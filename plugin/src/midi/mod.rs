//! Puente MIDI de controladores (Maschine Mikro MK3 y similares).
//!
//! El hilo de audio empuja comandos a una cola sin bloquear mucho; el frame de la GUI
//! los drena y aplica cambios de sesión / parámetros del host.

pub mod mk3;

pub use mk3::{Mk3Bridge, Mk3Cmd, Mk3LedState, Mk3Mode, Mk3Out};
