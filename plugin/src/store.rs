//! Estado compartido de la drum machine entre el hilo de audio, el hilo de carga y la interfaz.
//!
//! * Audio solo lee: `SampleStore` (ranuras `ArcSwapOption`, atómicos) y `KitAudio` (triple buffer).
//! * El hilo de carga escribe las ranuras y la interfaz edita `KitState` bajo un `Mutex` que el
//!   hilo de audio **nunca** toca.

use crate::sample::Sample;
use arc_swap::ArcSwapOption;
use caja_core::kit::{KitState, ParamCfg, NUM_PADS, SLOTS};
use caja_core::library::{Library, MAX_BANK};
use caja_core::pieces::PAD_ORDER;
use caja_core::schedule::TripleBuffer;
use caja_core::spsc::Spsc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, RwLock};

pub const NO_PAD: u8 = 255;

/// Muestras decodificadas: una ranura por (pad, índice de banco). Sin bloqueos para el audio.
pub struct SampleStore {
    pub slots: Vec<ArcSwapOption<Sample>>,
    /// El audio ya pidió esta ranura (evita llenar la cola de peticiones).
    pub requested: Vec<AtomicBool>,
    /// Último "reloj" en que sonó (para expulsar las menos usadas al pasar el tope de RAM).
    pub last_use: Vec<AtomicU32>,
    pub bank_len: [AtomicUsize; NUM_PADS],
    /// Índice (en el banco) del pin si existe y su archivo está en el banco; -1 si no.
    pub pin_idx: [AtomicI32; NUM_PADS],
    pub pin_missing: [AtomicBool; NUM_PADS],
    /// Índice de la última muestra que sonó en cada pad (-1 = ninguna todavía).
    pub cur_idx: [AtomicI32; NUM_PADS],
    /// Peticiones audio -> carga: `(pad << 8) | índice`.
    pub want: Spsc,
    pub ram_bytes: AtomicUsize,
    pub clock: AtomicU32,
}

impl SampleStore {
    pub fn new() -> SampleStore {
        SampleStore {
            slots: (0..NUM_PADS * MAX_BANK).map(|_| ArcSwapOption::empty()).collect(),
            requested: (0..NUM_PADS * MAX_BANK).map(|_| AtomicBool::new(false)).collect(),
            last_use: (0..NUM_PADS * MAX_BANK).map(|_| AtomicU32::new(0)).collect(),
            bank_len: std::array::from_fn(|_| AtomicUsize::new(0)),
            pin_idx: std::array::from_fn(|_| AtomicI32::new(-1)),
            pin_missing: std::array::from_fn(|_| AtomicBool::new(false)),
            cur_idx: std::array::from_fn(|_| AtomicI32::new(-1)),
            want: Spsc::default(),
            ram_bytes: AtomicUsize::new(0),
            clock: AtomicU32::new(0),
        }
    }
    pub fn slot(pad: usize, idx: usize) -> usize {
        pad * MAX_BANK + idx
    }
    /// Cuántas ranuras de ese pad tienen audio (para la interfaz).
    pub fn loaded_in_pad(&self, pad: usize) -> usize {
        (0..MAX_BANK).filter(|&i| self.slots[Self::slot(pad, i)].load().is_some()).count()
    }
}

impl Default for SampleStore {
    fn default() -> Self {
        SampleStore::new()
    }
}

/// Lo que el audio necesita de cada pad (POD: pasa por el triple buffer sin reservas).
#[derive(Clone, Copy)]
pub struct PadAudio {
    pub cfg: [ParamCfg; 4],
    pub bank: [u32; SLOTS],
    pub gain: u16,
    pub pan: i16,
    pub mute: bool,
    pub solo: bool,
    pub poly: u8,
    pub choke_group: u8,
    pub choker: bool,
}

#[derive(Clone, Copy)]
pub struct KitAudio {
    pub pads: [PadAudio; NUM_PADS],
    pub any_solo: bool,
    pub roll_repro: bool,
    pub roll_seed: u32,
}

impl Default for KitAudio {
    fn default() -> Self {
        KitAudio::from_state(&KitState::default())
    }
}

impl KitAudio {
    pub fn from_state(k: &KitState) -> KitAudio {
        let pad = |i: usize| {
            let p = &k.pads[i];
            PadAudio { cfg: p.cfg, bank: p.bank, gain: p.gain, pan: p.pan, mute: p.mute, solo: p.solo, poly: p.poly, choke_group: p.choke_group, choker: p.choker }
        };
        KitAudio { pads: std::array::from_fn(pad), any_solo: k.pads.iter().any(|p| p.solo), roll_repro: k.roll_repro, roll_seed: k.roll_seed }
    }
}

#[derive(Default, Clone)]
pub struct ScanStatus {
    pub scanning: bool,
    pub files_seen: usize,
    /// Archivos ignorados (corruptos, ilegibles) y fallos de decodificación: visibles en la interfaz.
    pub errors: Vec<String>,
    /// De dónde salió la tabla de palabras clave (y sus errores).
    pub keywords_origin: String,
    pub keywords_errors: Vec<String>,
    /// Se alcanzó el tope de RAM y no se pudo cargar algo.
    pub ram_full: bool,
}

pub const MAX_ERRORS: usize = 200;

pub struct KitShared {
    pub state: Mutex<KitState>,
    pub audio: TripleBuffer<KitAudio>,
    pub store: SampleStore,
    pub library: RwLock<Arc<Library>>,
    pub status: Mutex<ScanStatus>,
    /// Cadena JSON del estado de la drum machine: es el campo `#[persist]` del plugin.
    pub persist: Arc<Mutex<String>>,
    last_serialized: Mutex<String>,
    /// nota MIDI -> pad (`NO_PAD` si no hay).
    pub note_pad: [AtomicU8; 128],
    /// Aviso de la última carga de estado rechazada.
    pub state_error: Mutex<Option<String>>,
    /// Hay cambios que el hilo de carga debe recoger (re-sincronizar bancos/pines).
    pub resync: AtomicBool,
    pub rescan: AtomicBool,
}

impl KitShared {
    pub fn new(persist: Arc<Mutex<String>>) -> Arc<KitShared> {
        let shared = KitShared {
            state: Mutex::new(KitState::default()),
            audio: TripleBuffer::default(),
            store: SampleStore::new(),
            library: RwLock::new(Arc::new(Library::empty())),
            status: Mutex::new(ScanStatus::default()),
            persist,
            last_serialized: Mutex::new(String::new()),
            note_pad: std::array::from_fn(|_| AtomicU8::new(NO_PAD)),
            state_error: Mutex::new(None),
            resync: AtomicBool::new(true),
            rescan: AtomicBool::new(false),
        };
        let a = KitAudio::from_state(&shared.state.lock().unwrap_or_else(|e| e.into_inner()));
        shared.audio.publish(|k| *k = a);
        Arc::new(shared)
    }

    /// Si el host cargó un estado nuevo en el campo persistente, lo lee (rechazando el corrupto).
    pub fn pull_persisted(&self) {
        let cur = self.persist.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut last = self.last_serialized.lock().unwrap_or_else(|e| e.into_inner());
        if cur == *last {
            return;
        }
        *last = cur.clone();
        if cur.is_empty() {
            return;
        }
        match KitState::from_json(&cur) {
            Ok(k) => {
                let audio = KitAudio::from_state(&k);
                let folder_changed = self.state.lock().unwrap_or_else(|e| e.into_inner()).folder != k.folder;
                *self.state.lock().unwrap_or_else(|e| e.into_inner()) = k;
                self.audio.publish(|a| *a = audio);
                *self.state_error.lock().unwrap_or_else(|e| e.into_inner()) = None;
                self.resync.store(true, Ordering::Release);
                if folder_changed {
                    self.rescan.store(true, Ordering::Release);
                }
            }
            Err(e) => {
                *self.state_error.lock().unwrap_or_else(|e| e.into_inner()) = Some(format!("Estado de la drum machine rechazado: {e}. Se usan los valores por defecto."));
            }
        }
    }

    /// Edita el estado (interfaz / tarea de fondo) y lo publica: audio, JSON persistente y aviso al cargador.
    pub fn edit<R>(&self, f: impl FnOnce(&mut KitState) -> R) -> R {
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let r = f(&mut s);
        let audio = KitAudio::from_state(&s);
        self.audio.publish(|a| *a = audio);
        let json = s.to_json();
        *self.last_serialized.lock().unwrap_or_else(|e| e.into_inner()) = json.clone();
        *self.persist.lock().unwrap_or_else(|e| e.into_inner()) = json;
        self.resync.store(true, Ordering::Release);
        r
    }

    pub fn snapshot(&self) -> KitState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn library(&self) -> Arc<Library> {
        self.library.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Tabla nota -> pad a partir del mapa de kit activo (notas por pieza). Gana el primer pad.
    pub fn set_notes(&self, notes: &[u8; caja_core::pieces::NUM_PIECES]) {
        let mut table = [NO_PAD; 128];
        for (pad, &piece) in PAD_ORDER.iter().enumerate() {
            let n = usize::from(notes[piece] & 0x7f);
            if table[n] == NO_PAD {
                table[n] = pad as u8;
            }
        }
        for (a, v) in self.note_pad.iter().zip(table) {
            a.store(v, Ordering::Relaxed);
        }
    }

    pub fn add_error(&self, msg: String) {
        let mut st = self.status.lock().unwrap_or_else(|e| e.into_inner());
        if st.errors.len() < MAX_ERRORS && !st.errors.contains(&msg) {
            st.errors.push(msg);
        }
    }
}
