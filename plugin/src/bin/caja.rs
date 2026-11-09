//! CAJA Drum Machine — binario standalone.
//!
//! Abre la GUI, conecta al audio/MIDI del sistema (por defecto JACK si está
//! disponible; si no, el backend de cpal) y bloquea hasta cerrar la ventana.
//!
//! Compilar y ejecutar:
//! ```text
//! cargo run -p caja --features standalone --release
//! cargo run -p caja --features standalone --release -- --help
//! ```
//!
//! Opciones útiles (las lista `--help`):
//! - dispositivos de audio / sample rate / buffer
//! - puertos MIDI in/out (Mikro MK3 → in; LEDs → out del mismo dispositivo)
//!
//! Recomendado en standalone:
//! - Parámetro **Modo** = "Siempre" (no depende del transporte del host)
//! - **MK3 nativo** = sí (LEDs y modos Pattern/Rack)
//! - Carpeta de samples en la pestaña Biblioteca

use nih_plug::prelude::*;

fn main() {
    // `false` si falla la inicialización o hay un error fatal de audio.
    let ok = nih_export_standalone::<caja::Caja>();
    if !ok {
        eprintln!("CAJA standalone: no se pudo iniciar (audio/MIDI/GUI). Prueba `caja --help`.");
        std::process::exit(1);
    }
}
