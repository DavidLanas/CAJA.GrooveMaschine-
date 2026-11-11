//! Parámetros del plugin (CAJA Drum Machine). Los 13 primeros son los de la versión C++ (mismos nombres, rangos y
//! orden); detrás van 4 ajustes de aspecto como en ChordGlow. El estado de usuario (patrón editado,
//! historial, frase, mapa de kit) viaja en un campo `#[persist]` como cadena JSON del core.

use caja_core::genres::MAX_GENRES;
use caja_core::kit::{PadLive, NUM_PADS, SEED_MAX, SLOTS};
use caja_core::session::ParamSnap;
use nih_plug::prelude::*;
use nih_plug_egui::EguiState;
use std::sync::{Arc, Mutex, RwLock};

/// Tamaño inicial de la ventana (lógico). Es redimensionable.
pub const DEFAULT_SIZE: (u32, u32) = (1280, 860);

pub const RUN_MODES: [&str; 3] = ["Con el transporte", "Siempre", "Con una tecla"];

#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteParam {
    #[id = "arcade"]
    #[name = "Arcade"]
    Arcade,
    /// Paleta Okabe-Ito: distinguible con las formas más comunes de daltonismo.
    #[id = "cvd"]
    #[name = "Daltonismo"]
    ColorBlind,
}

#[derive(Enum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSizeParam {
    /// 8 px: la rejilla nativa de la fuente (nítida).
    #[id = "s"]
    #[name = "Pequeno"]
    Small,
    #[id = "m"]
    #[name = "Mediano"]
    Medium,
    /// 16 px: doble de la rejilla (nítida).
    #[id = "l"]
    #[name = "Grande"]
    Large,
}

impl TextSizeParam {
    pub fn px(self) -> f32 {
        match self {
            TextSizeParam::Small => 8.0,
            TextSizeParam::Medium => 12.0,
            TextSizeParam::Large => 16.0,
        }
    }
}

#[derive(Params)]
pub struct CajaParams {
    /// Tamaño de la ventana, guardado con el proyecto.
    #[persist = "editor-state"]
    pub editor_state: Arc<EguiState>,
    /// Estado de usuario como cadena JSON del core (rechaza lo corrupto al cargar).
    #[persist = "caja-state"]
    pub state: Arc<Mutex<String>>,
    /// Estado de la drum machine (carpeta, pins, bancos de semillas...) con número de versión de
    /// esquema. Nunca contiene audio.
    #[persist = "kit-state"]
    pub kit_state: Arc<Mutex<String>>,

    // ---- los 13 parámetros originales, en su orden ----
    #[id = "genre"]
    pub genre: IntParam,
    /// 0..3 = 1, 2, 4, 8 compases.
    #[id = "bars"]
    pub bars: IntParam,
    #[id = "swing"]
    pub swing: IntParam,
    #[id = "human"]
    pub human: IntParam,
    #[id = "density"]
    pub density: IntParam,
    #[id = "seed"]
    pub seed: IntParam,
    #[id = "accent"]
    pub accent: IntParam,
    /// 0 con el transporte, 1 siempre, 2 solo con una tecla.
    #[id = "runmode"]
    pub run_mode: IntParam,
    #[id = "thru"]
    pub thru: BoolParam,
    #[id = "velscale"]
    pub vel_scale: IntParam,
    #[id = "ccnum"]
    pub cc_num: IntParam,
    /// Tempo libre cuando el host no da tempo (y tempo del .mid).
    #[id = "bpm"]
    pub bpm: IntParam,
    #[id = "channel"]
    pub channel: IntParam,

    // ---- drum machine ----
    /// Solo drum machine: no corre el patrón, solo suena con MIDI entrante.
    #[id = "dmonly"]
    pub dm_only: BoolParam,
    #[id = "master"]
    pub master: FloatParam,
    /// Prioridad al clip: si llegan notas de un clip a los pads, el patrón del plugin calla
    /// mientras dure el clip (y 1 compás después). No actúa en el modo "Con una tecla".
    #[id = "clipprio"]
    pub clip_prio: BoolParam,
    /// Índice del patrón activo (0..31 = A1–A16, B1–B16). Asignable a macros de Bitwig.
    #[id = "pattern"]
    pub pattern: IntParam,
    /// Banco de patrones visible: 0 = A (1–16), 1 = B (17–32). Macro Bitwig.
    #[id = "bank"]
    pub bank: IntParam,
    /// Pestaña de la GUI: 0=Perform, 1=Library, 2=Pads, 3=Step. Macro Bitwig.
    #[id = "uitab"]
    pub ui_tab: IntParam,
    /// Trigger (rising edge): generar patrón desde la frase. Macro botón Bitwig.
    #[id = "gen"]
    pub generate: BoolParam,
    /// Trigger (rising edge): regenerar con la misma frase. Macro botón Bitwig.
    #[id = "regen"]
    pub regenerate: BoolParam,
    /// Por pad (orden de `PAD_ORDER`, n = 1..25): `padseed_n`, `padslot_n`, `padmode_n`.
    /// Automatizar la semilla o el slot produce SALTOS entre sonidos, no transiciones.
    #[nested(array)]
    pub pads: [PadParams; NUM_PADS],

    // ---- aspecto (extra) ----
    #[id = "palette"]
    pub palette: EnumParam<PaletteParam>,
    #[id = "textsz"]
    pub text_size: EnumParam<TextSizeParam>,
    #[id = "dark"]
    pub dark: BoolParam,
    #[id = "scan"]
    pub scanlines: BoolParam,
    /// Puente Maschine Mikro MK3 (Pattern/Rack + LEDs). Off si usas el script Bitwig.
    #[id = "mk3"]
    pub mk3_enabled: BoolParam,
    /// Exporta el estado de los 16 pads a `/tmp/caja-leds.json` para el script de Bitwig
    /// (preferencia "CAJA LED Bridge"). Usar con "MK3 nativo = No".
    #[id = "mk3leds"]
    pub mk3_leds_to_script: BoolParam,
}

/// Parámetros automatizables de un pad. IDs estables: `padseed_{n}`, `padslot_{n}`, `padmode_{n}`
/// (n = posición del pad en `PAD_ORDER`, empezando en 1).
#[derive(Params)]
pub struct PadParams {
    /// Semilla activa (0..99999). Entero sin suavizado.
    #[id = "padseed"]
    pub seed: IntParam,
    /// Slot activo del banco de 8 semillas (1..8 en pantalla, 0..7 en el valor).
    #[id = "padslot"]
    pub slot: IntParam,
    /// Modos: bit 0 Muestra, 1 Pitch, 2 Decay, 3 Filtro; el bit en 1 = LOCK, en 0 = ROLL.
    #[id = "padmode"]
    pub mode: IntParam,
    /// Mute del pad (asignable a macros de Bitwig). `padmute_{n}`.
    #[id = "padmute"]
    pub mute: BoolParam,
    /// Solo del pad (asignable a macros de Bitwig). `padsolo_{n}`.
    #[id = "padsolo"]
    pub solo: BoolParam,
    /// Paneo del pad, -1 (izq) .. +1 (der). `padpan_{n}`. Se aplica al disparar cada golpe.
    #[id = "padpan"]
    pub pan: FloatParam,
    /// Ganancia del pad, 0..2 (1 = 0 dB). `padgain_{n}`. Asignable a macros.
    #[id = "padgain"]
    pub gain: FloatParam,
    /// Botón "Guardar en slot": al pasar de apagado a encendido guarda la semilla activa en el slot
    /// activo del pad. `padsave_{n}`. Pensado para un botón de macro.
    #[id = "padsave"]
    pub save: BoolParam,
    /// Ritmo de re-sorteo en ROLL: 0 = cada golpe (rápido), 1/2/3 = cada 1/2/4 compases.
    /// `padrate_{n}`. Asignable a macros.
    #[id = "padrate"]
    pub rate: IntParam,
}

impl PadParams {
    fn new(pad: usize) -> PadParams {
        let d = PadLive::default_for(pad);
        let no_smooth = |p: IntParam| p.with_smoother(SmoothingStyle::None);
        let rate_labels = ["golpe", "1 compas", "2 compases", "4 compases"];
        // Nombres con índice para macros de Bitwig (P1..P25).
        let n = pad + 1;
        PadParams {
            seed: no_smooth(IntParam::new(
                format!("P{n} Semilla"),
                d.seed as i32,
                IntRange::Linear { min: 0, max: SEED_MAX as i32 },
            )),
            slot: no_smooth(
                IntParam::new(
                    format!("P{n} Slot"),
                    i32::from(d.slot),
                    IntRange::Linear { min: 0, max: SLOTS as i32 - 1 },
                )
                .with_value_to_string(Arc::new(|v| (v + 1).to_string()))
                .with_string_to_value(Arc::new(|s| s.trim().parse::<i32>().ok().map(|x| x - 1))),
            ),
            mute: BoolParam::new(format!("P{n} Mute"), false),
            solo: BoolParam::new(format!("P{n} Solo"), false),
            pan: FloatParam::new(
                format!("P{n} Pan"),
                0.0,
                FloatRange::Linear { min: -1.0, max: 1.0 },
            )
            .with_step_size(0.01),
            gain: FloatParam::new(
                format!("P{n} Gain"),
                1.0,
                FloatRange::Linear { min: 0.0, max: 2.0 },
            )
            .with_step_size(0.01)
            .with_unit(" x")
            .with_value_to_string(Arc::new(|v| {
                if v <= 0.0001 {
                    "-inf dB".to_string()
                } else {
                    format!("{:.1} dB", 20.0 * v.log10())
                }
            })),
            save: BoolParam::new(format!("P{n} Guardar slot"), false),
            rate: no_smooth(
                IntParam::new(format!("P{n} Ritmo ROLL"), 3, IntRange::Linear { min: 0, max: 3 })
                    .with_value_to_string(Arc::new(move |v| rate_labels[v.clamp(0, 3) as usize].to_string()))
                    .with_string_to_value(Arc::new(|s| match s.trim().to_lowercase().as_str() {
                        "golpe" | "0" | "rapido" | "rápido" => Some(0),
                        "1" | "1 compas" | "1 compás" => Some(1),
                        "2" | "2 compases" => Some(2),
                        "4" | "4 compases" => Some(3),
                        _ => None,
                    })),
            ),
            mode: no_smooth(
                IntParam::new(
                    format!("P{n} Modos"),
                    i32::from(d.mode),
                    IntRange::Linear { min: 0, max: 15 },
                )
                .with_value_to_string(Arc::new(|v| mode_text(v as u8)))
                .with_string_to_value(Arc::new(parse_mode)),
            ),
        }
    }

    pub fn live(&self) -> PadLive {
        PadLive { seed: self.seed.value() as u32, slot: self.slot.value() as u8, mode: self.mode.value() as u8 & 0b1111 }
    }
}

/// Lee "M:R P:L D:L F:L" (cualquier orden de letras L/R tras cada inicial) o un entero 0..15.
pub fn parse_mode(s: &str) -> Option<i32> {
    let s = s.trim();
    if let Ok(n) = s.parse::<i32>() {
        return (0..=15).contains(&n).then_some(n);
    }
    let mut mode = 0;
    let mut seen = 0;
    for tok in s.split_whitespace() {
        let (k, v) = tok.split_once(':')?;
        let bit = match k.to_ascii_uppercase().as_str() {
            "M" => 0,
            "P" => 1,
            "D" => 2,
            "F" => 3,
            _ => return None,
        };
        match v.to_ascii_uppercase().as_str() {
            "L" => mode |= 1 << bit,
            "R" => {}
            _ => return None,
        }
        seen |= 1 << bit;
    }
    (seen == 0b1111).then_some(mode)
}

/// "M:R P:L D:L F:L" (Muestra, Pitch, Decay, Filtro; R = ROLL, L = LOCK).
pub fn mode_text(mode: u8) -> String {
    let n = ["M", "P", "D", "F"];
    (0..4).map(|k| format!("{}:{}", n[k], if mode >> k & 1 == 1 { "L" } else { "R" })).collect::<Vec<_>>().join(" ")
}

pub(crate) fn int(name: &str, def: i32, min: i32, max: i32) -> IntParam {
    IntParam::new(name, def, IntRange::Linear { min, max })
}

impl CajaParams {
    pub fn new(state: Arc<Mutex<String>>, kit_state: Arc<Mutex<String>>, genre_names: Arc<RwLock<Vec<String>>>) -> Self {
        let names_a = genre_names.clone();
        let names_b = genre_names;
        let bars_labels = ["1", "2", "4", "8"];
        Self {
            editor_state: EguiState::from_size(DEFAULT_SIZE.0, DEFAULT_SIZE.1),
            state,
            kit_state,
            // el nombre sale del mapa de géneros cargado (integrado o del usuario)
            genre: int("Genero", 1, 0, MAX_GENRES as i32 - 1)
                .with_value_to_string(Arc::new(move |v| {
                    names_a.read().ok().and_then(|n| n.get(v.max(0) as usize).cloned()).unwrap_or_else(|| format!("({v})"))
                }))
                .with_string_to_value(Arc::new(move |s| {
                    let s = s.trim();
                    // primero el nombre exacto (hay nombres que acaban en ")"); luego "(53)" o "53"
                    names_b
                        .read()
                        .ok()
                        .and_then(|n| n.iter().position(|x| x.eq_ignore_ascii_case(s)))
                        .map(|i| i as i32)
                        .or_else(|| s.trim_matches(|c| c == '(' || c == ')').parse().ok())
                })),
            bars: int("Compases", 1, 0, 3)
                .with_value_to_string(Arc::new(move |v| bars_labels[v.clamp(0, 3) as usize].to_string()))
                .with_string_to_value(Arc::new(|s| match s.trim() {
                    "1" => Some(0),
                    "2" => Some(1),
                    "4" => Some(2),
                    "8" => Some(3),
                    _ => None,
                })),
            swing: int("Swing", 0, 0, 100).with_unit(" %"),
            human: int("Humanizacion", 14, 0, 100).with_unit(" %"),
            density: int("Densidad", 50, 0, 100).with_unit(" %"),
            seed: int("Semilla", 12345, 0, 999_999_999),
            accent: int("Acento", 50, 0, 100).with_unit(" %"),
            run_mode: int("Modo", 0, 0, 2)
                .with_value_to_string(Arc::new(|v| RUN_MODES[v.clamp(0, 2) as usize].to_string()))
                .with_string_to_value(Arc::new(|s| RUN_MODES.iter().position(|m| m.eq_ignore_ascii_case(s.trim())).map(|i| i as i32))),
            thru: BoolParam::new("Thru", false)
                .with_value_to_string(Arc::new(|v| if v { "Si".to_string() } else { "No".to_string() }))
                .with_string_to_value(Arc::new(|s| match s.trim().to_lowercase().as_str() {
                    "si" | "sí" | "on" | "1" => Some(true),
                    "no" | "off" | "0" => Some(false),
                    _ => None,
                })),
            vel_scale: int("Velocidad %", 100, 25, 200).with_unit(" %"),
            cc_num: int("CC por paso", 0, 0, 127),
            bpm: int("BPM libre", 100, 60, 210),
            channel: int("Canal MIDI", 10, 1, 16),
            dm_only: BoolParam::new("Solo drum machine", false),
            clip_prio: BoolParam::new("Prioridad al clip", true),
            master: FloatParam::new("Volumen", 0.0, FloatRange::Linear { min: -24.0, max: 6.0 }).with_unit(" dB").with_step_size(0.1),
            pattern: IntParam::new("Patron activo", 0, IntRange::Linear { min: 0, max: 31 })
                .with_smoother(SmoothingStyle::None)
                .with_value_to_string(Arc::new(|v| {
                    let v = v.clamp(0, 31) as usize;
                    let bank = if v < 16 { "A" } else { "B" };
                    format!("{bank}{}", (v % 16) + 1)
                }))
                .with_string_to_value(Arc::new(|s| {
                    let s = s.trim().to_uppercase();
                    if let Ok(n) = s.parse::<i32>() {
                        return Some(n.clamp(0, 31));
                    }
                    // "A1".."A16", "B1".."B16"
                    if s.len() >= 2 {
                        let bank = match s.chars().next() {
                            Some('A') => 0,
                            Some('B') => 16,
                            _ => return None,
                        };
                        if let Ok(slot) = s[1..].parse::<i32>() {
                            if (1..=16).contains(&slot) {
                                return Some(bank + slot - 1);
                            }
                        }
                    }
                    None
                })),
            bank: IntParam::new("Banco", 0, IntRange::Linear { min: 0, max: 1 })
                .with_smoother(SmoothingStyle::None)
                .with_value_to_string(Arc::new(|v| if v == 0 { "A".into() } else { "B".into() }))
                .with_string_to_value(Arc::new(|s| match s.trim().to_uppercase().as_str() {
                    "A" | "0" => Some(0),
                    "B" | "1" => Some(1),
                    _ => None,
                })),
            ui_tab: IntParam::new("Pestana", 0, IntRange::Linear { min: 0, max: 3 })
                .with_smoother(SmoothingStyle::None)
                .with_value_to_string(Arc::new(|v| {
                    match v.clamp(0, 3) {
                        0 => "Perform".into(),
                        1 => "Library".into(),
                        2 => "Pads".into(),
                        _ => "Step".into(),
                    }
                }))
                .with_string_to_value(Arc::new(|s| {
                    match s.trim().to_lowercase().as_str() {
                        "perform" | "patron" | "pattern" | "0" => Some(0),
                        "library" | "libreria" | "biblioteca" | "1" => Some(1),
                        "pads" | "kit" | "2" => Some(2),
                        "step" | "rack" | "3" => Some(3),
                        _ => None,
                    }
                })),
            generate: BoolParam::new("Generar", false),
            regenerate: BoolParam::new("Regenerar", false),
            pads: std::array::from_fn(PadParams::new),
            palette: EnumParam::new("Paleta", PaletteParam::Arcade),
            text_size: EnumParam::new("Tamano de texto", TextSizeParam::Small),
            dark: BoolParam::new("Modo oscuro", true),
            scanlines: BoolParam::new("Scanlines", true),
            mk3_enabled: BoolParam::new("MK3 nativo", true)
                .with_value_to_string(Arc::new(|v| if v { "Si".into() } else { "No".into() }))
                .with_string_to_value(Arc::new(|s| match s.trim().to_lowercase().as_str() {
                    "si" | "sí" | "on" | "1" => Some(true),
                    "no" | "off" | "0" => Some(false),
                    _ => None,
                })),
            mk3_leds_to_script: BoolParam::new("MK3 LEDs a script", false)
                .with_value_to_string(Arc::new(|v| if v { "Si".into() } else { "No".into() }))
                .with_string_to_value(Arc::new(|s| match s.trim().to_lowercase().as_str() {
                    "si" | "sí" | "on" | "1" => Some(true),
                    "no" | "off" | "0" => Some(false),
                    _ => None,
                })),
        }
    }

    /// Los 8 valores que cambian el patrón o su tiempo (lectura atómica: vale en el hilo de audio).
    pub fn snap(&self) -> ParamSnap {
        ParamSnap {
            genre: self.genre.value(),
            bars_idx: self.bars.value(),
            swing: self.swing.value(),
            human: self.human.value(),
            density: self.density.value(),
            seed: self.seed.value(),
            accent: self.accent.value(),
            bpm: self.bpm.value(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> CajaParams {
        let names = Arc::new(RwLock::new(vec!["Funk".to_string(), "Neo soul".to_string(), "Reggae (one drop)".to_string()]));
        CajaParams::new(Arc::new(Mutex::new(String::new())), Arc::new(Mutex::new(String::new())), names)
    }

    #[test]
    fn the_13_original_params_keep_names_ranges_and_order() {
        let p = params();
        let list = p.param_map();
        let got: Vec<(String, String)> = list.iter().take(13).map(|(id, ptr, _)| (id.clone(), unsafe { ptr.name().to_string() })).collect();
        let names: Vec<&str> = got.iter().map(|(_, n)| n.as_str()).collect();
        assert_eq!(
            names,
            ["Genero", "Compases", "Swing", "Humanizacion", "Densidad", "Semilla", "Acento", "Modo", "Thru", "Velocidad %", "CC por paso", "BPM libre", "Canal MIDI"]
        );
        // rangos y valores por defecto (plano)
        let range = |i: usize| unsafe { (list[i].1.preview_plain(0.0), list[i].1.preview_plain(1.0), list[i].1.default_plain_value()) };
        assert_eq!(range(1), (0.0, 3.0, 1.0));
        assert_eq!(range(2), (0.0, 100.0, 0.0));
        assert_eq!(range(3), (0.0, 100.0, 14.0));
        assert_eq!(range(5), (0.0, 999_999_999.0, 12345.0));
        assert_eq!(range(9), (25.0, 200.0, 100.0));
        assert_eq!(range(10), (0.0, 127.0, 0.0));
        assert_eq!(range(11), (60.0, 210.0, 100.0));
        assert_eq!(range(12), (1.0, 16.0, 10.0));
    }

    #[test]
    fn genre_param_shows_names_from_the_map() {
        let p = params();
        assert_eq!(p.genre.normalized_value_to_string(p.genre.preview_normalized(2), false), "Reggae (one drop)");
        assert_eq!(p.genre.string_to_normalized_value("Reggae (one drop)").map(|n| p.genre.preview_plain(n)), Some(2));
        assert_eq!(p.genre.string_to_normalized_value("neo soul").map(|n| p.genre.preview_plain(n)), Some(1));
        let beyond = p.genre.normalized_value_to_string(p.genre.preview_normalized(53), false);
        assert_eq!(beyond, "(53)");
        assert_eq!(p.genre.string_to_normalized_value(&beyond).map(|n| p.genre.preview_plain(n)), Some(53));
        assert_eq!(p.bars.normalized_value_to_string(p.bars.preview_normalized(3), false), "8");
        assert_eq!(p.run_mode.normalized_value_to_string(p.run_mode.preview_normalized(2), false), "Con una tecla");
    }

    #[test]
    fn mode_text_roundtrips_for_every_mask() {
        for m in 0..16u8 {
            assert_eq!(parse_mode(&mode_text(m)), Some(i32::from(m)));
        }
        assert_eq!(parse_mode("7"), Some(7));
        assert_eq!(parse_mode("16"), None);
        assert_eq!(parse_mode("M:L"), None);
        assert_eq!(parse_mode("hola"), None);
    }

    #[test]
    fn pad_params_have_stable_ids() {
        let p = params();
        let ids: Vec<String> = p.param_map().into_iter().map(|(id, _, _)| id).collect();
        for id in ["padseed_1", "padslot_1", "padmode_1", "padmute_1", "padpan_1", "padgain_1", "padsave_1", "padrate_1", "padmute_25", "clipprio", "padseed_25", "padmode_25", "dmonly", "master"] {
            assert!(ids.contains(&id.to_string()), "falta {id}");
        }
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "IDs duplicados");
    }

    #[test]
    fn snap_reads_defaults() {
        let s = params().snap();
        assert_eq!((s.genre, s.bars_idx, s.swing, s.human, s.density, s.seed, s.accent, s.bpm), (1, 1, 0, 14, 50, 12345, 50, 100));
    }
}
