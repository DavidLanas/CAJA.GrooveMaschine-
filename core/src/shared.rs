//! Estado compartido sin bloqueos entre el hilo de audio y la interfaz (solo atómicos).

use crate::spsc::Spsc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};

#[derive(Default)]
pub struct HostInfo {
    /// Paso actual dentro del bucle; -1 = parado.
    pub step: AtomicI32,
    pub playing: AtomicBool,
    tempo_bits: AtomicU32,
    /// (contador << 8) | nota: última nota recibida por la entrada (modo Aprender).
    last_note: AtomicU32,
    /// "Probar": reproduce el bucle sin transporte.
    pub audition: AtomicBool,
    /// Notas de prueba de la interfaz: (nota) | (velocidad << 8).
    pub preview: Spsc,
}

impl HostInfo {
    pub fn new() -> Self {
        let h = HostInfo::default();
        h.step.store(-1, Ordering::Relaxed);
        h
    }

    pub fn set_tempo(&self, bpm: f32) {
        self.tempo_bits.store(bpm.to_bits(), Ordering::Relaxed);
    }

    pub fn tempo(&self) -> f32 {
        f32::from_bits(self.tempo_bits.load(Ordering::Relaxed))
    }

    /// Hilo de audio (único escritor).
    pub fn note_seen(&self, note: u8) {
        let n = self.last_note.load(Ordering::Relaxed);
        self.last_note.store(((n >> 8).wrapping_add(1) << 8) | u32::from(note & 0x7f), Ordering::Relaxed);
    }

    /// (contador, nota). Si el contador cambió, llegó una nota nueva.
    pub fn last_note(&self) -> (u32, u8) {
        let n = self.last_note.load(Ordering::Relaxed);
        (n >> 8, (n & 0x7f) as u8)
    }

    pub fn push_preview(&self, note: u8, vel: u8) -> bool {
        self.preview.push(u32::from(note & 0x7f) | (u32::from(vel.clamp(1, 127)) << 8))
    }
}
