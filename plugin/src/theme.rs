//! Tema visual CAJA — lenguaje MPC / Maschine.
//!
//! Tokens de chasis, panel, goma de pad, LCD y LEDs. Las paletas Arcade /
//! Daltonismo siguen disponibles; en oscuro el look por defecto es MPC
//! (gris carbón + ámbar). Nada depende solo del color: nombres, altura de
//! golpes y bordes refuerzan el estado.

use crate::params::PaletteParam;
use caja_core::pieces::Group;
use nih_plug_egui::egui::Color32;

#[derive(Clone, Copy)]
#[allow(dead_code)]
pub struct Theme {
    // ── API legada (editor actual) ──────────────────────────────────────────
    pub bg: Color32,
    pub panel: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub accent: Color32,
    pub held: Color32,
    pub preview: Color32,
    pub flash_a: Color32,
    pub flash_b: Color32,
    pub key_white: Color32,
    pub key_black: Color32,
    pub key_edge: Color32,
    pub lanes: [Color32; 5],

    // ── Tokens MPC ──────────────────────────────────────────────────────────
    /// Plástico exterior del rack.
    pub chassis: Color32,
    /// Bisel interno / ranuras.
    pub chassis_edge: Color32,
    /// Superficie de panel bajo los controles.
    pub face: Color32,
    /// Goma del pad en reposo.
    pub rubber: Color32,
    /// Goma al pulsar / pad activo.
    pub rubber_on: Color32,
    /// Fondo del LCD.
    pub lcd_bg: Color32,
    /// Texto / pixel del LCD (verde fósforo o ámbar).
    pub lcd_fg: Color32,
    /// Texto secundario del LCD.
    pub lcd_dim: Color32,
    /// LED apagado.
    pub led_off: Color32,
    /// LED rojo (rec / hit fuerte).
    pub led_red: Color32,
    /// LED ámbar (modo / selección).
    pub led_amber: Color32,
    /// LED verde (play / paso activo).
    pub led_green: Color32,
    /// LED blanco (playhead / foco).
    pub led_white: Color32,
    /// Sombra suave bajo pads (alpha baja).
    pub shadow: Color32,
}

const fn rgb(r: u8, g: u8, b: u8) -> Color32 {
    Color32::from_rgb(r, g, b)
}

fn rgba(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(r, g, b, a)
}

impl Theme {
    pub fn new(palette: PaletteParam, dark: bool) -> Theme {
        match (palette, dark) {
            // ── MPC oscuro (look por defecto standalone) ─────────────────────
            (PaletteParam::Arcade, true) => Theme {
                bg: rgb(18, 18, 20),
                panel: rgb(32, 32, 36),
                border: rgb(72, 72, 78),
                text: rgb(236, 232, 220),
                text_dim: rgb(140, 136, 128),
                accent: rgb(232, 168, 48),
                held: rgb(72, 196, 96),
                preview: rgb(255, 244, 220),
                flash_a: rgb(255, 255, 255),
                flash_b: rgb(232, 168, 48),
                key_white: rgb(228, 228, 224),
                key_black: rgb(22, 22, 24),
                key_edge: rgb(18, 18, 20),
                lanes: [
                    rgb(220, 72, 64),   // kick
                    rgb(64, 168, 220),  // snare
                    rgb(232, 168, 48),  // hat
                    rgb(200, 120, 64),  // perc/tom
                    rgb(168, 120, 220), // cym
                ],
                chassis: rgb(12, 12, 14),
                chassis_edge: rgb(48, 48, 52),
                face: rgb(28, 28, 32),
                rubber: rgb(40, 40, 44),
                rubber_on: rgb(72, 68, 64),
                lcd_bg: rgb(8, 16, 12),
                lcd_fg: rgb(96, 220, 120),
                lcd_dim: rgb(40, 100, 56),
                led_off: rgb(28, 24, 24),
                led_red: rgb(220, 48, 40),
                led_amber: rgb(232, 168, 48),
                led_green: rgb(56, 200, 88),
                led_white: rgb(240, 236, 228),
                shadow: rgba(0, 0, 0, 90),
            },
            // ── MPC claro ───────────────────────────────────────────────────
            (PaletteParam::Arcade, false) => Theme {
                bg: rgb(210, 206, 198),
                panel: rgb(236, 232, 224),
                border: rgb(48, 44, 40),
                text: rgb(24, 22, 20),
                text_dim: rgb(96, 90, 82),
                accent: rgb(180, 100, 16),
                held: rgb(24, 140, 56),
                preview: rgb(0, 100, 160),
                flash_a: rgb(24, 22, 20),
                flash_b: rgb(180, 100, 16),
                key_white: rgb(250, 248, 240),
                key_black: rgb(36, 34, 32),
                key_edge: rgb(48, 44, 40),
                lanes: [
                    rgb(180, 40, 36),
                    rgb(0, 110, 160),
                    rgb(160, 110, 0),
                    rgb(180, 90, 30),
                    rgb(110, 70, 180),
                ],
                chassis: rgb(170, 166, 158),
                chassis_edge: rgb(120, 116, 108),
                face: rgb(224, 220, 212),
                rubber: rgb(200, 196, 188),
                rubber_on: rgb(160, 156, 148),
                lcd_bg: rgb(180, 190, 170),
                lcd_fg: rgb(20, 60, 30),
                lcd_dim: rgb(80, 110, 70),
                led_off: rgb(160, 156, 150),
                led_red: rgb(200, 40, 32),
                led_amber: rgb(200, 140, 20),
                led_green: rgb(20, 150, 50),
                led_white: rgb(40, 40, 40),
                shadow: rgba(0, 0, 0, 40),
            },
            // ── Daltonismo oscuro ───────────────────────────────────────────
            (PaletteParam::ColorBlind, true) => Theme {
                bg: rgb(14, 14, 18),
                panel: rgb(28, 28, 36),
                border: rgb(220, 220, 220),
                text: rgb(250, 250, 250),
                text_dim: rgb(170, 170, 180),
                accent: rgb(240, 228, 66),
                held: rgb(0, 158, 115),
                preview: rgb(86, 180, 233),
                flash_a: rgb(255, 255, 255),
                flash_b: rgb(240, 228, 66),
                key_white: rgb(236, 236, 236),
                key_black: rgb(20, 20, 26),
                key_edge: rgb(14, 14, 18),
                lanes: [
                    rgb(230, 159, 0),
                    rgb(86, 180, 233),
                    rgb(240, 228, 66),
                    rgb(213, 94, 0),
                    rgb(204, 121, 167),
                ],
                chassis: rgb(10, 10, 14),
                chassis_edge: rgb(60, 60, 70),
                face: rgb(24, 24, 32),
                rubber: rgb(36, 36, 44),
                rubber_on: rgb(60, 60, 72),
                lcd_bg: rgb(8, 12, 16),
                lcd_fg: rgb(86, 180, 233),
                lcd_dim: rgb(40, 80, 100),
                led_off: rgb(28, 28, 32),
                led_red: rgb(213, 94, 0),
                led_amber: rgb(240, 228, 66),
                led_green: rgb(0, 158, 115),
                led_white: rgb(250, 250, 250),
                shadow: rgba(0, 0, 0, 90),
            },
            // ── Daltonismo claro ────────────────────────────────────────────
            (PaletteParam::ColorBlind, false) => Theme {
                bg: rgb(246, 246, 244),
                panel: rgb(255, 255, 255),
                border: rgb(20, 20, 24),
                text: rgb(20, 20, 24),
                text_dim: rgb(90, 90, 100),
                accent: rgb(0, 114, 178),
                held: rgb(0, 122, 89),
                preview: rgb(0, 114, 178),
                flash_a: rgb(20, 20, 24),
                flash_b: rgb(213, 94, 0),
                key_white: rgb(255, 255, 255),
                key_black: rgb(28, 28, 34),
                key_edge: rgb(20, 20, 24),
                lanes: [
                    rgb(213, 94, 0),
                    rgb(0, 114, 178),
                    rgb(150, 130, 0),
                    rgb(204, 121, 167),
                    rgb(0, 122, 89),
                ],
                chassis: rgb(220, 220, 216),
                chassis_edge: rgb(140, 140, 148),
                face: rgb(240, 240, 236),
                rubber: rgb(210, 210, 206),
                rubber_on: rgb(180, 180, 176),
                lcd_bg: rgb(200, 210, 220),
                lcd_fg: rgb(0, 70, 120),
                lcd_dim: rgb(60, 100, 130),
                led_off: rgb(180, 180, 180),
                led_red: rgb(213, 94, 0),
                led_amber: rgb(150, 130, 0),
                led_green: rgb(0, 122, 89),
                led_white: rgb(20, 20, 24),
                shadow: rgba(0, 0, 0, 40),
            },
        }
    }

    pub fn lane(&self, i: usize) -> Color32 {
        self.lanes[i % self.lanes.len()]
    }

    /// Color de una pieza según su grupo (el nombre siempre va escrito).
    pub fn group(&self, g: Group) -> Color32 {
        self.lane(match g {
            Group::Kick => 0,
            Group::Snare => 1,
            Group::Hat => 2,
            Group::Perc => 3,
            Group::Cym => 4,
            Group::Tom => 3,
        })
    }

    pub fn scanline(&self) -> Color32 {
        Color32::from_rgba_unmultiplied(0, 0, 0, 40)
    }
}

/// Zonas del layout estilo MPC (coordenadas relativas al área de contenido).
#[derive(Clone, Copy, Debug)]
pub struct Zones {
    /// Franja superior: transport / banco / modo.
    pub top: f32,
    /// LCD central (frase, lectura, status).
    pub lcd: f32,
    /// Rejilla de pads o step grid.
    pub pads: f32,
    /// Franja inferior: knobs / params.
    pub bottom: f32,
}

impl Zones {
    /// Alturas sugeridas para una ventana de `h` píxeles de contenido.
    pub fn for_height(h: f32) -> Self {
        let top = (h * 0.08).clamp(28.0, 48.0);
        let lcd = (h * 0.14).clamp(40.0, 72.0);
        let bottom = (h * 0.16).clamp(48.0, 96.0);
        let pads = (h - top - lcd - bottom).max(120.0);
        Zones { top, lcd, pads, bottom }
    }
}
