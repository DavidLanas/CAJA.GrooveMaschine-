//! Sesión: todo lo que vive en el hilo principal (patrón, historial, frase, mapa de kit) y se
//! persiste como una cadena JSON propia. No toca el hilo de audio: publica un `Schedule`.

use crate::drum_map::{DrumMap, Learn, MapSet};
use crate::genres::GenreSet;
use crate::json::{self, quote, Value};
use crate::parser::{mods_to_text, parse_phrase};
use crate::pattern::*;
use crate::pattern_rack::{PatternEntry, PatternParams, PatternRack};
use crate::paths;
use crate::pieces::*;
use crate::rng::hash_seed;
use crate::smf::build_midi_file;
use crate::timing::*;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub const BARS_CHOICES: [usize; 4] = [1, 2, 4, 8];
pub const HISTORY_MAX: usize = 10;
/// v2: rack multi-patrón (cada entrada con seed/genre/swing/human/accent/density propios).
pub const STATE_VERSION: i64 = 2;
const ALL_MODS: u32 = (1 << 9) - 1;

pub fn bars_from_idx(i: i32) -> usize {
    BARS_CHOICES[i.clamp(0, 3) as usize]
}

pub fn idx_from_bars(b: i32) -> i32 {
    BARS_CHOICES.iter().position(|&x| x as i32 == b).unwrap_or(1) as i32
}

/// Los 8 parámetros del host que afectan al patrón o a su tiempo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamSnap {
    pub genre: i32,
    pub bars_idx: i32,
    pub swing: i32,
    pub human: i32,
    pub density: i32,
    pub seed: i32,
    pub accent: i32,
    pub bpm: i32,
}

impl Default for ParamSnap {
    fn default() -> Self {
        ParamSnap { genre: 1, bars_idx: 1, swing: 0, human: 14, density: 50, seed: 12345, accent: 50, bpm: 100 }
    }
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub phrase: String,
    pub warning: String,
    pub mods: u32,
    pub p: ParamSnap,
    pub pattern: Pattern,
}

pub struct Session {
    pub genres: GenreSet,
    pub genre_origin: String,
    pub genre_errors: Vec<String>,
    pub maps: MapSet,
    pub maps_origin: String,
    pub maps_errors: Vec<String>,
    pub map_index: usize,
    /// Notas del kit actual (puede diferir del mapa guardado si se editó o se aprendió).
    pub notes: [u8; NUM_PIECES],
    pub learn: Learn,
    /// Copia de trabajo del patrón activo (espejo de `rack.entries[active]`).
    pub phrase: String,
    pub warning: String,
    pub mods: u32,
    pub pattern: Pattern,
    /// Rack de 32 patrones (2 bancos × 16). Cada uno con params propios.
    pub rack: PatternRack,
    pub history: Vec<Snapshot>,
    pub state_error: Option<String>,
    pub message: String,
    pub last_export: Option<PathBuf>,
    snap: ParamSnap,
    started: bool,
    pub sched_dirty: bool,
    pub state_dirty: bool,
    pub last_serialized: String,
}

static SEED_COUNTER: AtomicU64 = AtomicU64::new(0x9e37_79b9_7f4a_7c15);

/// Semilla nueva 100000..999999999 sin dependencias externas (SplitMix64 sobre reloj + contador).
pub fn random_seed() -> i32 {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(1);
    let mut z = t ^ SEED_COUNTER.fetch_add(0x9e37_79b9_7f4a_7c15, Ordering::Relaxed);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^= z >> 31;
    (100_000 + z % 999_900_000) as i32
}

impl Session {
    pub fn new(genres: paths::Loaded<GenreSet>, maps: paths::Loaded<MapSet>) -> Session {
        let mut maps = maps;
        maps.value.upgrade_legacy_bitwig();
        let notes = maps.value.maps.first().map(|m| m.notes).unwrap_or_else(default_notes);
        Session {
            genres: genres.value,
            genre_origin: genres.origin,
            genre_errors: genres.errors,
            maps: maps.value,
            maps_origin: maps.origin,
            maps_errors: maps.errors,
            map_index: 0,
            notes,
            learn: Learn::default(),
            phrase: String::new(),
            warning: String::new(),
            mods: 0,
            pattern: Pattern::default(),
            rack: PatternRack::default(),
            history: Vec::new(),
            state_error: None,
            message: String::new(),
            last_export: None,
            snap: ParamSnap::default(),
            started: false,
            sched_dirty: true,
            state_dirty: true,
            last_serialized: String::new(),
        }
    }

    /// Sesión con los géneros y mapas integrados (pruebas).
    pub fn builtin() -> Session {
        Session::new(
            paths::Loaded { value: GenreSet::builtin(), origin: "integrado".into(), errors: vec![] },
            paths::Loaded { value: MapSet::builtin(), origin: "integrados".into(), errors: vec![] },
        )
    }

    pub fn snap(&self) -> ParamSnap {
        self.snap
    }

    pub fn genre_index(&self, p: &ParamSnap) -> usize {
        (p.genre.max(0) as usize).min(self.genres.len() - 1)
    }

    fn spec(&self, p: &ParamSnap) -> Spec {
        Spec { genre: self.genre_index(p), mods: self.mods, bars: bars_from_idx(p.bars_idx), density: p.density, seed: p.seed.to_string(), accent: p.accent }
    }

    fn regenerate(&mut self) {
        let spec = self.spec(&self.snap);
        self.pattern = generate_pattern(&spec, &self.genres);
        self.store_active();
        self.sched_dirty = true;
        self.state_dirty = true;
    }

    /// Escribe la copia de trabajo (pattern/phrase/mods + params del snap) en el rack activo.
    fn store_active(&mut self) {
        let e = self.rack.active_entry_mut();
        e.pattern = self.pattern.clone();
        e.phrase = self.phrase.clone();
        e.warning = self.warning.clone();
        e.mods = self.mods;
        e.params = PatternParams::from_snap(&self.snap);
        // Asegurar coherencia bars
        e.pattern.bars = e.params.bars().clamp(1, MAX_BARS);
        self.pattern.bars = e.pattern.bars;
    }

    /// Carga la entrada activa del rack a la copia de trabajo y al snap (conserva bpm).
    fn load_active(&mut self) {
        let e = self.rack.active_entry().clone();
        self.pattern = e.pattern;
        self.phrase = e.phrase;
        self.warning = e.warning;
        self.mods = e.mods;
        e.params.apply_to_snap(&mut self.snap);
        self.sched_dirty = true;
        self.state_dirty = true;
    }

    /// Índice del patrón activo (0..31).
    pub fn active_pattern_index(&self) -> usize {
        self.rack.active
    }

    /// Selecciona un patrón del rack. Guarda el actual, carga el nuevo y actualiza el snap.
    /// Devuelve el ParamSnap resultante (para que el host actualice macros).
    pub fn select_pattern(&mut self, index: usize) -> ParamSnap {
        self.store_active();
        if self.rack.select(index) {
            self.load_active();
            self.message = format!(
                "Patrón {}{}{}",
                if index < 16 { "A" } else { "B" },
                (index % 16) + 1,
                if self.rack.active_entry().name.is_empty() {
                    String::new()
                } else {
                    format!(" — {}", self.rack.active_entry().name)
                }
            );
        }
        self.snap
    }

    /// Copia el patrón activo sobre otra ranura.
    pub fn copy_active_to(&mut self, to: usize) {
        self.store_active();
        let from = self.rack.active;
        self.rack.copy(from, to);
        self.state_dirty = true;
        self.message = format!("Copiado → {}{}", if to < 16 { "A" } else { "B" }, (to % 16) + 1);
    }

    /// Borra una ranura del rack (si es la activa, deja la copia de trabajo vacía).
    pub fn clear_pattern_slot(&mut self, index: usize) {
        self.rack.clear(index);
        if index == self.rack.active {
            self.pattern = Pattern::default();
            self.phrase.clear();
            self.warning.clear();
            self.mods = 0;
            self.sched_dirty = true;
        }
        self.state_dirty = true;
    }

    /// Asigna nombre corto al patrón activo.
    pub fn set_active_name(&mut self, name: &str) {
        self.rack.active_entry_mut().name = name.trim().to_string();
        self.state_dirty = true;
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot { phrase: self.phrase.clone(), warning: self.warning.clone(), mods: self.mods, p: self.snap, pattern: self.pattern.clone() }
    }

    fn push_history(&mut self) {
        let s = self.snapshot();
        self.history.insert(0, s);
        self.history.truncate(HISTORY_MAX);
        self.state_dirty = true;
    }

    fn sync_history_edit(&mut self) {
        if !self.history.is_empty() {
            self.history[0] = self.snapshot();
            self.state_dirty = true;
        }
    }

    /// Compara con los parámetros del host. Regenera si cambió algo del patrón, o solo republica
    /// si cambió el tiempo. Un cambio de swing/humanización no pierde las ediciones a mano.
    pub fn sync(&mut self, p: ParamSnap) {
        if !self.started {
            self.started = true;
            self.snap = p;
            self.regenerate();
            return;
        }
        let o = self.snap;
        if p == o {
            return;
        }
        self.snap = p;
        let shapes = p.genre != o.genre || p.bars_idx != o.bars_idx || p.density != o.density || p.seed != o.seed || p.accent != o.accent;
        if shapes {
            self.regenerate(); // también hace store_active
        } else {
            // swing/human/bpm: no regenera rejilla, pero sí guarda params del patrón activo
            self.store_active();
            self.sched_dirty = true;
            self.state_dirty = true;
        }
    }

    // ---- acciones de la interfaz: devuelven los parámetros que hay que fijar en el host ----

    /// "Generar": frase -> parámetros y patrón nuevo; entra al historial.
    pub fn generate(&mut self, phrase: &str, cur: ParamSnap) -> ParamSnap {
        let r = parse_phrase(phrase, &self.genres);
        let mut p = cur;
        p.genre = r.genre as i32;
        p.bpm = r.bpm;
        p.swing = r.swing;
        p.density = if r.density >= 0 { r.density } else { 50 };
        if r.human >= 0 {
            p.human = r.human;
        } else {
            p.human = self.genres.get(r.genre).human;
        }
        if r.bars >= 0 {
            p.bars_idx = idx_from_bars(r.bars);
        }
        self.phrase = phrase.trim().to_string();
        self.mods = r.mods;
        self.warning = r.warning;
        self.snap = p;
        self.started = true;
        self.regenerate();
        self.push_history();
        p
    }

    /// Elegir género: aplica sus valores por defecto y borra los modificadores.
    pub fn set_genre(&mut self, idx: usize, cur: ParamSnap) -> ParamSnap {
        let idx = idx.min(self.genres.len() - 1);
        let g = self.genres.get(idx);
        let mut p = cur;
        p.genre = idx as i32;
        p.swing = g.swing;
        p.bpm = g.bpm;
        p.human = g.human;
        self.mods = 0;
        self.warning.clear();
        self.snap = p;
        self.started = true;
        self.regenerate();
        self.sync_history_edit();
        p
    }

    /// Semilla nueva (dado). Con `history` entra al historial ("Regenerar").
    pub fn reroll(&mut self, cur: ParamSnap, history: bool) -> ParamSnap {
        let mut p = cur;
        p.seed = random_seed();
        self.snap = p;
        self.started = true;
        self.regenerate();
        if history {
            self.push_history();
        }
        p
    }

    /// Restaura una tarjeta del historial (con sus ediciones a mano) sobre el patrón activo.
    pub fn restore(&mut self, i: usize) -> Option<ParamSnap> {
        let s = self.history.get(i)?.clone();
        self.phrase = s.phrase;
        self.warning = s.warning;
        self.mods = s.mods;
        self.pattern = s.pattern;
        self.snap = s.p;
        self.started = true;
        self.store_active();
        self.sched_dirty = true;
        self.state_dirty = true;
        Some(s.p)
    }

    fn touch(&mut self) {
        self.store_active();
        self.sched_dirty = true;
        self.sync_history_edit();
        self.state_dirty = true;
    }

    /// Clic en una celda: alterna el golpe (con `ghost`, lo crea fantasma).
    pub fn toggle_cell(&mut self, piece: usize, step: usize, ghost: bool) {
        if piece >= NUM_PIECES || step >= self.pattern.steps() {
            return;
        }
        let c = &mut self.pattern.cell[piece][step];
        *c = if c.on() {
            Cell::default()
        } else if ghost {
            Cell { vel: 40, ghost: true }
        } else {
            Cell { vel: default_vel(piece), ghost: false }
        };
        self.touch();
    }

    /// Arrastre vertical: velocidad 1..127.
    pub fn set_cell_vel(&mut self, piece: usize, step: usize, vel: i32) {
        if piece >= NUM_PIECES || step >= self.pattern.steps() {
            return;
        }
        let c = &mut self.pattern.cell[piece][step];
        if c.on() {
            c.vel = vel.clamp(1, 127) as u8;
            self.touch();
        }
    }

    /// Clic derecho: ghost on/off (si no hay golpe, crea uno fantasma).
    pub fn toggle_ghost(&mut self, piece: usize, step: usize) {
        if piece >= NUM_PIECES || step >= self.pattern.steps() {
            return;
        }
        let c = &mut self.pattern.cell[piece][step];
        if c.on() {
            c.ghost = !c.ghost;
            if c.ghost {
                c.vel = c.vel.min(64);
            }
        } else {
            *c = Cell { vel: 40, ghost: true };
        }
        self.touch();
    }

    // ---- mapa de kit ----

    pub fn select_map(&mut self, i: usize) {
        if let Some(m) = self.maps.maps.get(i) {
            self.map_index = i;
            self.notes = m.notes;
            self.sched_dirty = true;
            self.state_dirty = true;
        }
    }

    pub fn set_note(&mut self, piece: usize, note: u8) {
        if piece < NUM_PIECES {
            self.notes[piece] = note & 0x7f;
            self.sched_dirty = true;
            self.state_dirty = true;
        }
    }

    pub fn learn_note(&mut self, note: u8) -> Option<(usize, u8)> {
        let mut notes = self.notes;
        let r = self.learn.on_note(note, &mut notes);
        if r.is_some() {
            self.notes = notes;
            self.sched_dirty = true;
            self.state_dirty = true;
        }
        r
    }

    pub fn current_map(&self, name: &str) -> DrumMap {
        DrumMap { name: name.trim().to_string(), notes: self.notes }
    }

    pub fn save_current_map(&mut self, name: &str) -> Result<PathBuf, String> {
        if name.trim().is_empty() {
            return Err("escribe un nombre para el mapa".into());
        }
        let m = self.current_map(name);
        let path = paths::save_map(&m)?;
        self.map_index = self.maps.upsert(m);
        self.state_dirty = true;
        Ok(path)
    }

    /// Botón "Recargar": vuelve a leer genres.json, drum-maps.json y maps/.
    pub fn reload_files(&mut self) {
        let g = paths::load_genres();
        self.genres = g.value;
        self.genre_origin = g.origin;
        self.genre_errors = g.errors;
        let m = paths::load_maps();
        let keep_name = self.maps.maps.get(self.map_index).map(|x| x.name.clone());
        self.maps = m.value;
        self.maps.upgrade_legacy_bitwig();
        if self.notes == legacy_notes() {
            self.notes = default_notes();
        }
        self.maps_origin = m.origin;
        self.maps_errors = m.errors;
        self.map_index = keep_name.and_then(|n| self.maps.index_of(&n)).unwrap_or(0);
        // el patrón actual se conserva (puede estar editado); las notas del kit también
        self.sched_dirty = true;
        self.state_dirty = true;
    }

    // ---- lectura y salida ----

    pub fn genre_name(&self, p: &ParamSnap) -> &str {
        &self.genres.get(self.genre_index(p)).name
    }

    /// Línea "Lectura": lo generado, en palabras.
    pub fn reading(&self) -> String {
        let p = self.snap;
        let g = self.genres.get(self.genre_index(&p));
        let bars = bars_from_idx(p.bars_idx);
        let mut s = format!("{} · {} BPM · swing {} · {} {}", g.name, p.bpm, p.swing, bars, if bars == 1 { "compás" } else { "compases" });
        if g.proposal {
            s.push_str(" · propuesta");
        }
        let m = mods_to_text(self.mods);
        if !m.is_empty() {
            s.push_str(" · ");
            s.push_str(&m);
        }
        if !self.warning.is_empty() {
            s.push_str(" · ");
            s.push_str(&self.warning);
        }
        s
    }

    pub fn timing_opts(&self) -> TimingOpts {
        TimingOpts { genre: self.genre_index(&self.snap), mods: self.mods, swing: self.snap.swing, human: self.snap.human, seed: self.snap.seed.to_string() }
    }

    /// Golpes del calendario con las notas del kit actual.
    pub fn hits(&self) -> (u32, Vec<Hit>) {
        let hits = build_hits(&self.pattern, &self.timing_opts(), &self.genres, &self.notes);
        (self.pattern.steps() as u32 * TICKS_PER_STEP, hits)
    }

    /// .mid en memoria (formato 0 o 1) con el mapa del kit actual o con GM.
    pub fn midi_bytes(&self, separate: bool, use_gm: bool) -> Vec<u8> {
        let gm = self.maps.index_of("GM").map(|i| self.maps.maps[i].notes);
        let notes = if use_gm { gm.unwrap_or(self.notes) } else { self.notes };
        build_midi_file(&self.pattern, &self.timing_opts(), self.snap.bpm, separate, &self.genres, &notes)
    }

    pub fn export(&mut self, separate: bool, use_gm: bool) -> Result<PathBuf, String> {
        let bytes = self.midi_bytes(separate, use_gm);
        let id = self.genres.get(self.genre_index(&self.snap)).id.clone();
        let path = paths::export_midi(&id, self.snap.bpm, &self.snap.seed.to_string(), &bytes)?;
        self.last_export = Some(path.clone());
        Ok(path)
    }

    // ---- persistencia ----

    pub fn to_state_json(&self) -> String {
        let mut s = String::with_capacity(16384);
        // Espejo del activo (compat + lectura rápida) + rack completo
        s.push_str(&format!(
            "{{\"v\":{STATE_VERSION},\"phrase\":{},\"warning\":{},\"mods\":{},\"pattern\":",
            quote(&self.phrase),
            quote(&self.warning),
            self.mods
        ));
        write_pattern_value(&mut s, &self.pattern);
        s.push_str(&format!(
            ",\"map\":{{\"name\":{},\"notes\":[{}]}}",
            quote(self.maps.maps.get(self.map_index).map_or("", |m| m.name.as_str())),
            self.notes.iter().map(|n| n.to_string()).collect::<Vec<_>>().join(",")
        ));
        s.push_str(",\"history\":[");
        for (i, h) in self.history.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!(
                "{{\"phrase\":{},\"warning\":{},\"mods\":{},\"genre\":{},\"bars\":{},\"swing\":{},\"human\":{},\"density\":{},\"seed\":{},\"accent\":{},\"bpm\":{},\"pattern\":",
                quote(&h.phrase),
                quote(&h.warning),
                h.mods,
                h.p.genre,
                h.p.bars_idx,
                h.p.swing,
                h.p.human,
                h.p.density,
                h.p.seed,
                h.p.accent,
                h.p.bpm
            ));
            write_pattern_value(&mut s, &h.pattern);
            s.push('}');
        }
        s.push_str("],\"rack\":");
        s.push_str(&self.rack.to_json_value());
        s.push('}');
        s
    }

    /// Carga un estado guardado. Acepta v1 (un solo patrón) y v2 (rack).
    /// Todo o nada: si algo no cuadra, no se toca la sesión y se devuelve el motivo.
    pub fn load_state_json(&mut self, src: &str, cur: ParamSnap) -> Result<(), String> {
        let root = json::parse(src).map_err(|e| format!("estado ilegible: {e}"))?;
        let ver = root.get("v").and_then(Value::as_f64).ok_or("estado sin versión")?;
        if ver != 1.0 && ver != 2.0 {
            return Err(format!("versión de estado {ver} no soportada"));
        }
        let phrase = str_field(&root, "phrase")?;
        let warning = str_field(&root, "warning")?;
        let mods = int_field(&root, "mods", 0, i64::from(ALL_MODS))? as u32;
        let pattern = read_pattern_value(root.get("pattern").ok_or("falta el patrón")?)?;
        let map = root.get("map").ok_or("falta el mapa")?;
        let notes_v = map.get("notes").and_then(Value::as_arr).ok_or("faltan las notas del mapa")?;
        if notes_v.len() != NUM_PIECES {
            return Err(format!("el mapa tiene {} notas y debe tener {NUM_PIECES}", notes_v.len()));
        }
        let mut notes = [0u8; NUM_PIECES];
        for (i, n) in notes_v.iter().enumerate() {
            let x = n.as_f64().ok_or("nota de mapa no numérica")?;
            if x.fract() != 0.0 || !(0.0..=127.0).contains(&x) {
                return Err("nota de mapa fuera de 0-127".into());
            }
            notes[i] = x as u8;
        }
        let map_name = map.get("name").and_then(Value::as_str).unwrap_or("").to_string();
        let hist_v = root.get("history").and_then(Value::as_arr).ok_or("falta el historial")?;
        if hist_v.len() > HISTORY_MAX {
            return Err(format!("el historial tiene {} tarjetas y el máximo es {HISTORY_MAX}", hist_v.len()));
        }
        let mut history = Vec::new();
        for h in hist_v {
            let p = ParamSnap {
                genre: int_field(h, "genre", 0, 4095)? as i32,
                bars_idx: int_field(h, "bars", 0, 3)? as i32,
                swing: int_field(h, "swing", 0, 100)? as i32,
                human: int_field(h, "human", 0, 100)? as i32,
                density: int_field(h, "density", 0, 100)? as i32,
                seed: int_field(h, "seed", 0, 999_999_999)? as i32,
                accent: int_field(h, "accent", 0, 100)? as i32,
                bpm: int_field(h, "bpm", 40, 300)? as i32,
            };
            history.push(Snapshot {
                phrase: str_field(h, "phrase")?,
                warning: str_field(h, "warning")?,
                mods: int_field(h, "mods", 0, i64::from(ALL_MODS))? as u32,
                p,
                pattern: read_pattern_value(h.get("pattern").ok_or("tarjeta sin patrón")?)?,
            });
        }
        // Rack: v2 lo trae; v1 se construye con el patrón único en la ranura 0
        let rack = if ver >= 2.0 {
            match root.get("rack") {
                Some(rv) => PatternRack::from_json_value(rv)?,
                None => {
                    let mut r = PatternRack::default();
                    r.entries[0] = PatternEntry {
                        pattern: pattern.clone(),
                        phrase: phrase.clone(),
                        warning: warning.clone(),
                        mods,
                        name: String::new(),
                        params: PatternParams::from_snap(&cur),
                    };
                    r
                }
            }
        } else {
            let mut r = PatternRack::default();
            r.entries[0] = PatternEntry {
                pattern: pattern.clone(),
                phrase: phrase.clone(),
                warning: warning.clone(),
                mods,
                name: String::new(),
                params: PatternParams::from_snap(&cur),
            };
            r
        };
        // todo validado: ahora sí
        self.phrase = phrase;
        self.warning = warning;
        self.mods = mods;
        self.pattern = pattern;
        self.rack = rack;
        // Asegurar que el activo refleja la copia de trabajo
        self.store_active();
        self.history = history;
        self.notes = migrate_notes(notes);
        self.map_index = self.maps.index_of(&map_name).unwrap_or(self.map_index);
        self.snap = cur;
        self.started = true;
        self.sched_dirty = true;
        self.state_dirty = false;
        self.state_error = None;
        Ok(())
    }
}

fn default_vel(piece: usize) -> u8 {
    match PIECES[piece].group {
        Group::Kick => 104,
        Group::Snare => 100,
        Group::Hat => 78,
        Group::Cym => 90,
        _ => 86,
    }
}

fn str_field(v: &Value, k: &str) -> Result<String, String> {
    v.get(k).and_then(Value::as_str).map(String::from).ok_or_else(|| format!("falta el texto \"{k}\""))
}

fn int_field(v: &Value, k: &str, lo: i64, hi: i64) -> Result<i64, String> {
    let f = v.get(k).and_then(Value::as_f64).ok_or_else(|| format!("falta el número \"{k}\""))?;
    if f.fract() != 0.0 || f < lo as f64 || f > hi as f64 {
        return Err(format!("\"{k}\" = {f} fuera de {lo}-{hi}"));
    }
    Ok(f as i64)
}

/// Semilla de texto libre -> entero de parámetro (como `hashSeed(...) % 1e9`).
pub fn seed_from_text(s: &str) -> i32 {
    let s = s.trim();
    if !s.is_empty() && s.len() < 10 && s.bytes().all(|c| c.is_ascii_digit()) {
        s.parse::<i32>().unwrap_or(0)
    } else {
        (hash_seed(s) % 1_000_000_000) as i32
    }
}
