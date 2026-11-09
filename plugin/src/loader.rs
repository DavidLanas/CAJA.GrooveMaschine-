//! Hilo de carga: escanea la carpeta, clasifica, y decodifica las muestras que hacen falta de forma
//! perezosa. **Nada de esto corre en `process`**: el audio solo recibe `Arc<Sample>` ya listos por las
//! ranuras de `SampleStore` y pide lo que le falta con una cola sin bloqueos.
//!
//! Memoria: un pad en LOCK solo mantiene 1 muestra en RAM (su pin o la de su semilla); en ROLL
//! mantiene un subconjunto "pool" repartido por el banco más lo que el audio vaya pidiendo. Si se
//! pasa el tope de RAM se expulsan las menos usadas. Las muestras expulsadas pasan por un
//! "cementerio" hasta que ninguna voz las use: así el hilo de audio nunca libera memoria.

use crate::sample::{self, Sample};
use crate::store::{KitShared, MAX_ERRORS};
use caja_core::kit::{is_lock, locked_index, PadLive, NUM_PADS, P_SAMPLE};
use caja_core::library::{KeywordTable, Library, MAX_BANK};
use caja_core::paths;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

pub type LiveFn = Arc<dyn Fn(usize) -> PadLive + Send + Sync>;

pub enum Cmd {
    SampleRate(u32),
    Shutdown,
}

pub struct LoaderHandle {
    tx: Sender<Cmd>,
    join: Option<JoinHandle<()>>,
}

impl LoaderHandle {
    pub fn spawn(shared: Arc<KitShared>, live: LiveFn, sample_rate: u32) -> Option<LoaderHandle> {
        let (tx, rx) = channel();
        let _ = tx.send(Cmd::SampleRate(sample_rate));
        let join = std::thread::Builder::new().name("caja-loader".into()).spawn(move || Loader::new(shared, live, rx).run()).ok()?;
        Some(LoaderHandle { tx, join: Some(join) })
    }
    pub fn set_sample_rate(&self, sr: u32) {
        let _ = self.tx.send(Cmd::SampleRate(sr));
    }
}

impl Drop for LoaderHandle {
    fn drop(&mut self) {
        // Sin `join`: destruir la instancia no debe bloquear (el hilo ve `Shutdown` en <= 25 ms).
        let _ = self.tx.send(Cmd::Shutdown);
        drop(self.join.take());
    }
}

const AUDIO_EXT: [&str; 4] = ["wav", "flac", "ogg", "oga"];
const MAX_FILES: usize = 250_000;
const MAX_DEPTH: usize = 16;

/// Lista recursivamente los archivos de audio (rutas relativas con `/`). No sigue enlaces de carpeta.
pub fn scan_dir(root: &Path, errors: &mut Vec<String>, seen: &mut usize) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack: Vec<(PathBuf, String, usize)> = vec![(root.to_path_buf(), String::new(), 0)];
    while let Some((dir, rel, depth)) = stack.pop() {
        let rd = match std::fs::read_dir(&dir) {
            Ok(r) => r,
            Err(e) => {
                if errors.len() < MAX_ERRORS {
                    errors.push(format!("{}: no se pudo leer la carpeta ({e})", dir.display()));
                }
                continue;
            }
        };
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            let name = entry.file_name().to_string_lossy().to_string();
            let r = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
            if ft.is_dir() {
                if depth < MAX_DEPTH {
                    stack.push((entry.path(), r, depth + 1));
                }
            } else if ft.is_file() || ft.is_symlink() {
                let ext = Path::new(&name).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
                if ext.as_deref().is_some_and(|e| AUDIO_EXT.contains(&e)) && !name.starts_with("._") {
                    *seen += 1;
                    if out.len() < MAX_FILES {
                        out.push(r);
                    }
                }
            }
        }
    }
    out
}

struct Cached {
    sample: Arc<Sample>,
    bytes: usize,
}

struct Loader {
    shared: Arc<KitShared>,
    live: LiveFn,
    rx: Receiver<Cmd>,
    sr: u32,
    lib: Arc<Library>,
    root: PathBuf,
    cache: HashMap<String, Cached>,
    graveyard: Vec<Arc<Sample>>,
    failed: HashSet<String>,
    wanted: HashSet<(usize, usize)>,
    scanned_folder: Option<String>,
}

impl Loader {
    fn new(shared: Arc<KitShared>, live: LiveFn, rx: Receiver<Cmd>) -> Loader {
        Loader { shared, live, rx, sr: 0, lib: Arc::new(Library::empty()), root: PathBuf::new(), cache: HashMap::new(), graveyard: Vec::new(), failed: HashSet::new(), wanted: HashSet::new(), scanned_folder: None }
    }

    fn run(&mut self) {
        loop {
            let mut did = false;
            // 1. órdenes
            let mut cmd = match self.rx.try_recv() {
                Ok(c) => Some(c),
                Err(std::sync::mpsc::TryRecvError::Empty) => None,
                Err(_) => return,
            };
            if cmd.is_none() && !did {
                // espera corta solo si no hay nada pendiente
                let pending = self.shared.resync.load(Ordering::Acquire) || self.shared.rescan.load(Ordering::Acquire);
                if !pending {
                    match self.rx.recv_timeout(Duration::from_millis(25)) {
                        Ok(c) => cmd = Some(c),
                        Err(RecvTimeoutError::Timeout) => {}
                        Err(RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
            match cmd {
                Some(Cmd::Shutdown) => return,
                Some(Cmd::SampleRate(sr)) if sr > 0 && sr != self.sr => {
                    self.sr = sr;
                    self.drop_everything();
                    self.shared.rescan.store(true, Ordering::Release);
                }
                _ => {}
            }
            // 2. reescaneo
            if self.sr > 0 && self.shared.rescan.swap(false, Ordering::AcqRel) {
                self.rescan();
                did = true;
            }
            // 3. peticiones del audio
            while let Some(v) = self.shared.store.want.pop() {
                let (pad, idx) = ((v >> 8) as usize, (v & 0xff) as usize);
                if pad < NUM_PADS && idx < MAX_BANK {
                    if self.wanted.len() > 512 {
                        self.wanted.clear();
                    }
                    self.wanted.insert((pad, idx));
                }
            }
            // 4. mantener lo necesario (una carga por vuelta para seguir atento a las órdenes)
            if self.sr > 0 {
                self.shared.resync.store(false, Ordering::Release);
                did |= self.maintain();
            }
            self.graveyard.retain(|a| Arc::strong_count(a) > 1);
            if !did {
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }

    fn drop_everything(&mut self) {
        let st = &self.shared.store;
        for (i, s) in st.slots.iter().enumerate() {
            if let Some(a) = s.swap(None) {
                self.graveyard.push(a);
            }
            st.requested[i].store(false, Ordering::Relaxed);
        }
        for (_, c) in self.cache.drain() {
            self.graveyard.push(c.sample);
        }
        st.ram_bytes.store(0, Ordering::Relaxed);
        self.failed.clear();
        self.wanted.clear();
    }

    fn keywords(&self) -> KitKeywords {
        let json = self.shared.state.lock().unwrap_or_else(|e| e.into_inner()).keywords.clone();
        if !json.is_empty() {
            if let Ok(t) = KeywordTable::from_json(&json) {
                return KitKeywords { table: t, json, origin: "estado del proyecto".into(), errors: vec![] };
            }
        }
        let loaded = paths::load_keywords();
        let (table, json) = loaded.value;
        KitKeywords { table, json, origin: loaded.origin, errors: loaded.errors }
    }

    fn rescan(&mut self) {
        {
            let mut st = self.shared.status.lock().unwrap_or_else(|e| e.into_inner());
            st.scanning = true;
            st.files_seen = 0;
            st.errors.clear();
            st.ram_full = false;
        }
        let state = self.shared.snapshot();
        let kw = self.keywords();
        let root = PathBuf::from(&state.folder);
        let mut errors = Vec::new();
        let mut seen = 0usize;
        let mut files = Vec::new();
        if state.folder.is_empty() {
            // sin carpeta: biblioteca vacía
        } else if !root.is_dir() {
            errors.push(format!("{}: la carpeta no existe", state.folder));
        } else {
            for rel in scan_dir(&root, &mut errors, &mut seen) {
                // Cabecera ilegible o archivo corrupto: se ignora y se sigue.
                match sample::probe(&root.join(&rel)) {
                    Ok(()) => files.push(rel),
                    Err(e) => {
                        if errors.len() < MAX_ERRORS {
                            errors.push(format!("{rel}: {e}"));
                        }
                    }
                }
            }
        }
        let lib = Arc::new(Library::build(&state.folder, &files, &kw.table, &state.assign));
        self.root = root;
        self.lib = lib.clone();
        self.scanned_folder = Some(state.folder.clone());
        // La biblioteca nueva invalida las ranuras (los índices cambian), no la caché por ruta.
        for (i, s) in self.shared.store.slots.iter().enumerate() {
            if let Some(a) = s.swap(None) {
                self.graveyard.push(a);
            }
            self.shared.store.requested[i].store(false, Ordering::Relaxed);
        }
        self.wanted.clear();
        self.failed.clear();
        *self.shared.library.write().unwrap_or_else(|e| e.into_inner()) = lib;
        {
            let mut st = self.shared.status.lock().unwrap_or_else(|e| e.into_inner());
            st.scanning = false;
            st.files_seen = seen;
            st.errors = errors;
            st.keywords_origin = kw.origin;
            st.keywords_errors = kw.errors;
        }
        // Guarda la tabla usada en el estado (para que el proyecto reabra igual).
        if state.keywords != kw.json {
            self.shared.edit(|k| k.keywords = kw.json.clone());
        }
    }

    /// Devuelve true si hizo trabajo (cargó o expulsó algo).
    fn maintain(&mut self) -> bool {
        let state = self.shared.snapshot();
        if self.scanned_folder.is_none() {
            self.shared.rescan.store(true, Ordering::Release);
            return false;
        }
        let shared = self.shared.clone();
        let st = &shared.store;
        let clock = st.clock.load(Ordering::Relaxed);
        // 1. qué hace falta
        let mut core: HashSet<(usize, usize)> = HashSet::new(); // imprescindibles (LOCK, pines)
        let mut extra: Vec<(usize, usize)> = Vec::new(); // pool de ROLL y peticiones
        for pad in 0..NUM_PADS {
            let bank = self.lib.bank(pad, state.bank_cap);
            let n = bank.len();
            st.bank_len[pad].store(n, Ordering::Relaxed);
            let live = (self.live)(pad);
            let locked = is_lock(live.mode, P_SAMPLE);
            let pin = state.pads[pad].pin.as_deref().filter(|_| state.pads[pad].pin_seed == live.seed);
            let pin_i = pin.and_then(|p| bank.iter().position(|r| r == p));
            st.pin_idx[pad].store(pin_i.map_or(-1, |i| i as i32), Ordering::Relaxed);
            st.pin_missing[pad].store(pin.is_some() && pin_i.is_none(), Ordering::Relaxed);
            if n == 0 {
                continue;
            }
            if locked {
                let i = pin_i.or_else(|| locked_index(&state.pads[pad], live.seed, n)).unwrap_or(0);
                core.insert((pad, i));
            } else {
                let pool = state.pool.clamp(1, n);
                for k in 0..pool {
                    extra.push((pad, k * n / pool));
                }
                let cur = st.cur_idx[pad].load(Ordering::Relaxed);
                if cur >= 0 && (cur as usize) < n {
                    extra.push((pad, cur as usize));
                }
            }
            if let Some(i) = pin_i {
                core.insert((pad, i));
            }
        }
        for &(pad, idx) in &self.wanted {
            let n = self.lib.bank(pad, state.bank_cap).len();
            if idx < n && !is_lock(((self.live)(pad)).mode, P_SAMPLE) {
                extra.push((pad, idx));
            } else if idx < n {
                core.insert((pad, idx)); // en LOCK sin pin el audio puede pedir la de la semilla
            }
        }
        // 2. en LOCK solo queda lo imprescindible; lo demás sale ya (1 muestra en RAM por pad)
        let mut did = false;
        for pad in 0..NUM_PADS {
            let locked = is_lock(((self.live)(pad)).mode, P_SAMPLE);
            if !locked {
                continue;
            }
            for idx in 0..MAX_BANK {
                if !core.contains(&(pad, idx)) && self.unload(pad, idx) {
                    did = true;
                }
            }
        }
        // 3. cargar lo que falta (primero lo imprescindible), una muestra por vuelta
        let cap_bytes = (state.ram_cap_mb as usize) * 1024 * 1024;
        let mut todo: Vec<(usize, usize)> = core.iter().copied().collect();
        todo.sort_unstable();
        let mut extra_sorted = extra;
        extra_sorted.sort_unstable();
        extra_sorted.dedup();
        todo.extend(extra_sorted);
        let mut ram_full = false;
        for (pad, idx) in todo {
            let slot = crate::store::SampleStore::slot(pad, idx);
            if st.slots[slot].load().is_some() {
                continue;
            }
            let bank = self.lib.bank(pad, state.bank_cap);
            let Some(rel) = bank.get(idx).cloned() else { continue };
            if self.failed.contains(&rel) {
                continue;
            }
            if let Some(c) = self.cache.get(&rel) {
                st.slots[slot].store(Some(c.sample.clone()));
                st.requested[slot].store(false, Ordering::Relaxed);
                continue;
            }
            // ¿cabe? si no, expulsa lo menos usado que no sea imprescindible
            if st.ram_bytes.load(Ordering::Relaxed) >= cap_bytes && !self.evict_one(&core, clock) {
                ram_full = true;
                continue;
            }
            match sample::load(&self.root.join(&rel), self.sr, state.max_len_ms) {
                Ok(s) => {
                    let bytes = s.bytes();
                    st.ram_bytes.fetch_add(bytes, Ordering::Relaxed);
                    let arc = Arc::new(s);
                    self.cache.insert(rel, Cached { sample: arc.clone(), bytes });
                    st.slots[slot].store(Some(arc));
                    st.requested[slot].store(false, Ordering::Relaxed);
                }
                Err(e) => {
                    self.failed.insert(rel.clone());
                    self.shared.add_error(format!("{rel}: {e}"));
                }
            }
            did = true;
            break;
        }
        // 4. por encima del tope: expulsa
        while st.ram_bytes.load(Ordering::Relaxed) > cap_bytes {
            if !self.evict_one(&core, clock) {
                ram_full = true;
                break;
            }
            did = true;
        }
        self.shared.status.lock().unwrap_or_else(|e| e.into_inner()).ram_full = ram_full;
        did
    }

    /// Vacía una ranura; si nadie más usa ese archivo, la caché lo suelta (pasa al cementerio).
    fn unload(&mut self, pad: usize, idx: usize) -> bool {
        let st = &self.shared.store;
        let slot = crate::store::SampleStore::slot(pad, idx);
        let Some(arc) = st.slots[slot].swap(None) else { return false };
        st.requested[slot].store(false, Ordering::Relaxed);
        self.graveyard.push(arc);
        self.collect_cache();
        true
    }

    /// Quita de la caché los archivos que ya no tiene ninguna ranura.
    fn collect_cache(&mut self) {
        let st = &self.shared.store;
        let mut in_use: HashSet<*const Sample> = HashSet::new();
        for s in &st.slots {
            if let Some(a) = s.load().as_ref() {
                in_use.insert(Arc::as_ptr(a));
            }
        }
        let dead: Vec<String> = self.cache.iter().filter(|(_, c)| !in_use.contains(&Arc::as_ptr(&c.sample))).map(|(k, _)| k.clone()).collect();
        for k in dead {
            if let Some(c) = self.cache.remove(&k) {
                st.ram_bytes.fetch_sub(c.bytes.min(st.ram_bytes.load(Ordering::Relaxed)), Ordering::Relaxed);
                self.graveyard.push(c.sample);
            }
        }
    }

    /// Expulsa la ranura menos usada que no sea imprescindible. false si no hay candidata.
    fn evict_one(&mut self, core: &HashSet<(usize, usize)>, _clock: u32) -> bool {
        let st = &self.shared.store;
        let mut best: Option<(u32, usize, usize)> = None;
        for pad in 0..NUM_PADS {
            for idx in 0..MAX_BANK {
                let slot = crate::store::SampleStore::slot(pad, idx);
                if core.contains(&(pad, idx)) || st.slots[slot].load().is_none() {
                    continue;
                }
                let lu = st.last_use[slot].load(Ordering::Relaxed);
                if best.map_or(true, |(b, _, _)| lu < b) {
                    best = Some((lu, pad, idx));
                }
            }
        }
        match best {
            Some((_, pad, idx)) => self.unload(pad, idx),
            None => false,
        }
    }
}

struct KitKeywords {
    table: KeywordTable,
    json: String,
    origin: String,
    errors: Vec<String>,
}
