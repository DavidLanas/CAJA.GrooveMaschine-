//! CAJA core: motor de patrones de batería, géneros, parser de frases, escritor .mid,
//! scheduler de transporte y estado de sesión. No depende de nih-plug ni de egui:
//! se compila y se prueba en cualquier parte (`cd core && cargo test`).

pub mod drum_map;
pub mod genres;
pub mod host_sim;
pub mod json;
pub mod kit;
pub mod library;
pub mod model;
pub mod parser;
pub mod paths;
pub mod pattern;
pub mod pattern_rack;
pub mod pieces;
pub mod rng;
pub mod schedule;
pub mod seedrand;
pub mod scheduler;
pub mod session;
pub mod shared;
pub mod smf;
pub mod spsc;
pub mod timing;
