//! Estado y lógica del drum machine con samples: la réplica, sin audio, de lo que hace el preset
//! de Bitwig (Dice / semilla / Select / Merge / Counter) más el pin de muestra.
//!
//! Qué vive dónde:
//! * `PadLive` (semilla activa, slot, modos): son **parámetros del plugin** (automatizables); aquí
//!   se guardan también en el JSON para poder inspeccionar/exportar el estado.
//! * `PadCfg` (valores base, rangos, probabilidades, banco de 8 semillas, pin, ganancia...): solo
//!   en el estado JSON, con número de versión de esquema. **Nunca audio.**

use crate::json::{self, quote, Value};
use crate::library::{DEFAULT_BANK, MAX_BANK, MIN_BANK};
use crate::pieces::{CH, CH2, NUM_PIECES, OH, OH2, PAD_ORDER};
use crate::seedrand::{index_from_unit, seed_unit, RollRng, PARAM_OFFSETS, U24, UNIT_ONE};

pub const NUM_PADS: usize = NUM_PIECES;
pub const SLOTS: usize = 8;
pub const SCHEMA_VERSION: u32 = 1;
pub const SEED_MAX: u32 = 99_999;

/// Los 4 parámetros de cada pad (el orden es el de `PARAM_OFFSETS`).
pub const P_SAMPLE: usize = 0;
pub const P_PITCH: usize = 1;
pub const P_DECAY: usize = 2;
pub const P_FILTER: usize = 3;
pub const PARAM_NAMES: [&str; 4] = ["Muestra", "Pitch", "Decay", "Filtro"];

/// Máscara de modos: el bit k en 1 = LOCK (semilla); en 0 = ROLL (aleatorio).
pub fn is_lock(mode: u8, param: usize) -> bool {
    mode >> param & 1 == 1
}
pub const ALL_ROLL: u8 = 0;
pub const ALL_LOCK: u8 = 0b1111;

/// Valor en milésimas (0..=1000): exacto en JSON y sin sorpresas de redondeo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParamCfg {
    pub base: u16,
    pub range: u16,
    pub prob: u16,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PadCfg {
    pub cfg: [ParamCfg; 4],
    /// Banco de hasta 8 semillas por pad.
    pub bank: [u32; SLOTS],
    /// Ruta relativa de la muestra bloqueada (pin).
    pub pin: Option<String>,
    /// Semilla con la que se fijó el pin: el pin solo manda mientras esa semilla siga activa
    /// (cambiar la semilla elige otra muestra, como en el Grid; la interfaz re-fija el pin).
    pub pin_seed: u32,
    /// Ganancia en milésimas (1000 = 0 dB, máx. 2000).
    pub gain: u16,
    /// Pan en milésimas, -1000..=1000.
    pub pan: i16,
    pub mute: bool,
    pub solo: bool,
    /// Polifonía 1..=4.
    pub poly: u8,
    /// Grupo de choke (0 = ninguno).
    pub choke_group: u8,
    /// Si el golpe de este pad corta las voces de su grupo.
    pub choker: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PadLive {
    pub seed: u32,
    pub slot: u8,
    pub mode: u8,
}

fn default_seed(pad: usize, slot: usize) -> u32 {
    ((pad as u32) * 7919 + (slot as u32) * 10_403 + 17) % (SEED_MAX + 1)
}

impl PadCfg {
    pub fn default_for(pad: usize) -> PadCfg {
        let piece = PAD_ORDER[pad.min(NUM_PADS - 1)];
        // Los hats comparten grupo: el cerrado corta al abierto.
        let (choke_group, choker) = match piece {
            p if p == CH || p == CH2 => (1, true),
            p if p == OH || p == OH2 => (1, false),
            _ => (0, false),
        };
        PadCfg {
            cfg: [
                ParamCfg { base: 500, range: 1000, prob: 1000 }, // Muestra: todo el banco
                ParamCfg { base: 500, range: 60, prob: 1000 },   // Pitch: ±~1.4 st
                ParamCfg { base: 1000, range: 0, prob: 1000 },   // Decay: sin recorte
                ParamCfg { base: 1000, range: 0, prob: 1000 },   // Filtro: abierto
            ],
            bank: std::array::from_fn(|s| default_seed(pad, s)),
            pin: None,
            pin_seed: 0,
            gain: 1000,
            pan: 0,
            mute: false,
            solo: false,
            poly: 2,
            choke_group,
            choker,
        }
    }
}

impl PadLive {
    pub fn default_for(pad: usize) -> PadLive {
        // Muestra y Pitch en ROLL; Decay y Filtro en LOCK (la base ya es "sin cambio").
        PadLive { seed: default_seed(pad, 0), slot: 0, mode: 0b1100 }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct KitState {
    pub folder: String,
    /// Tope del banco por celda (8..=128).
    pub bank_cap: usize,
    /// Máximo de la muestra en ms (se recorta lo que pase).
    pub max_len_ms: u32,
    /// Tope de RAM en MB.
    pub ram_cap_mb: u32,
    /// Muestras precargadas por pad en ROLL (subconjunto "pool").
    pub pool: usize,
    /// ROLL reproducible: semilla global del PRNG, reiniciada al arrancar el transporte.
    pub roll_repro: bool,
    pub roll_seed: u32,
    pub pads: Vec<PadCfg>,
    pub live: Vec<PadLive>,
    /// Asignaciones manuales de la bandeja "Sin clasificar" (ruta relativa -> pad).
    pub assign: Vec<(String, usize)>,
    /// Tabla de palabras clave en uso (el JSON tal cual; vacío = la integrada).
    pub keywords: String,
}

impl Default for KitState {
    fn default() -> Self {
        KitState {
            folder: String::new(),
            bank_cap: DEFAULT_BANK,
            max_len_ms: 3000,
            ram_cap_mb: 512,
            pool: 16,
            roll_repro: false,
            roll_seed: 1,
            pads: (0..NUM_PADS).map(PadCfg::default_for).collect(),
            live: (0..NUM_PADS).map(PadLive::default_for).collect(),
            assign: Vec::new(),
            keywords: String::new(),
        }
    }
}

// ───────────────────────────── elección por golpe ─────────────────────────────

/// Estado por pad que solo vive en el hilo de audio: el último valor sorteado de cada parámetro
/// (el Dice "retiene" el valor si no se re-sortea).
#[derive(Clone, Copy, Debug)]
pub struct PadRuntime {
    pub last_u: [U24; 4],
}

impl PadRuntime {
    pub fn new(seed: u32) -> PadRuntime {
        PadRuntime { last_u: std::array::from_fn(|k| seed_unit(seed, PARAM_OFFSETS[k])) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitParams {
    /// Índice de muestra por semilla/dado dentro del banco (`None` si el banco está vacío).
    pub index: Option<usize>,
    /// Valores 0..=1 ya con rango aplicado; el plugin los convierte a semitonos, segundos y Hz.
    pub pitch: f32,
    pub decay: f32,
    pub filter: f32,
}

/// Valor 0..=1 de un parámetro continuo: base ± rango según `u`.
pub fn modulated(c: ParamCfg, u: U24) -> f32 {
    let uf = f64::from(u) / f64::from(UNIT_ONE);
    let v = f64::from(c.base) / 1000.0 + (uf * 2.0 - 1.0) * f64::from(c.range) / 1000.0;
    v.clamp(0.0, 1.0) as f32
}

/// Índice de muestra: `floor(p * N)` con `p = base + (u - 0.5) * rango` (con base 0.5 y rango 1
/// es exactamente `floor(u * N)`: se adapta solo al tamaño real del banco).
pub fn sample_index(c: ParamCfg, u: U24, n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let uf = f64::from(u) / f64::from(UNIT_ONE);
    let p = f64::from(c.base) / 1000.0 + (uf - 0.5) * f64::from(c.range) / 1000.0;
    let p = p.clamp(0.0, 1.0 - 1.0 / f64::from(UNIT_ONE));
    Some(((p * n as f64) as usize).min(n - 1))
}

/// Resuelve un golpe: LOCK usa el valor de la semilla; ROLL re-sortea con la probabilidad del
/// parámetro (y si no toca, repite el último valor, como el Dice sin disparo).
pub fn resolve_hit(cfg: &[ParamCfg; 4], live: PadLive, rt: &mut PadRuntime, roll: &mut RollRng, bank_len: usize) -> HitParams {
    let mut u = [0u32; 4];
    for k in 0..4 {
        if is_lock(live.mode, k) {
            u[k] = seed_unit(live.seed, PARAM_OFFSETS[k]);
        } else {
            let p = u32::from(cfg[k].prob.min(1000));
            let fire = p >= 1000 || (u64::from(roll.next_unit()) * 1000 >> 24) < u64::from(p);
            if fire {
                rt.last_u[k] = roll.next_unit();
            }
            u[k] = rt.last_u[k];
        }
    }
    HitParams {
        index: sample_index(cfg[P_SAMPLE], u[P_SAMPLE], bank_len),
        pitch: modulated(cfg[P_PITCH], u[P_PITCH]),
        decay: modulated(cfg[P_DECAY], u[P_DECAY]),
        filter: modulated(cfg[P_FILTER], u[P_FILTER]),
    }
}

/// Como `resolve_hit` pero sin re-sortear: LOCK usa semilla, ROLL usa el último valor guardado.
/// Sirve para ritmos de cambio lentos (cada N compases) manteniendo el sonido entre cambios.
pub fn resolve_hit_hold(cfg: &[ParamCfg; 4], live: PadLive, rt: &PadRuntime, bank_len: usize) -> HitParams {
    let mut u = [0u32; 4];
    for k in 0..4 {
        if is_lock(live.mode, k) {
            u[k] = seed_unit(live.seed, PARAM_OFFSETS[k]);
        } else {
            u[k] = rt.last_u[k];
        }
    }
    HitParams {
        index: sample_index(cfg[P_SAMPLE], u[P_SAMPLE], bank_len),
        pitch: modulated(cfg[P_PITCH], u[P_PITCH]),
        decay: modulated(cfg[P_DECAY], u[P_DECAY]),
        filter: modulated(cfg[P_FILTER], u[P_FILTER]),
    }
}

/// Resultado de elegir la muestra de un golpe.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SamplePick {
    pub index: Option<usize>,
    /// El pin existe pero su archivo ya no está en el banco: se usó el índice por semilla.
    pub pin_missing: bool,
}

/// El pin manda en LOCK mientras su archivo exista en el banco; si falta, índice por semilla y aviso.
pub fn pick_sample(pad: &PadCfg, sample_locked: bool, live_seed: u32, seed_index: Option<usize>, bank: &[String]) -> SamplePick {
    if sample_locked && pad.pin_seed == live_seed {
        if let Some(pin) = &pad.pin {
            return match bank.iter().position(|r| r == pin) {
                Some(i) => SamplePick { index: Some(i), pin_missing: false },
                None => SamplePick { index: seed_index, pin_missing: true },
            };
        }
    }
    SamplePick { index: seed_index, pin_missing: false }
}

/// Índice que daría la semilla de `pad` para el parámetro Muestra (para fijar el pin al pasar a LOCK).
pub fn locked_index(pad: &PadCfg, seed: u32, bank_len: usize) -> Option<usize> {
    let u = seed_unit(seed, PARAM_OFFSETS[P_SAMPLE]);
    sample_index(pad.cfg[P_SAMPLE], u, bank_len)
}

pub fn index_of_unit(u: U24, n: usize) -> Option<usize> {
    index_from_unit(u, n)
}

// ───────────────────────────── estado JSON ─────────────────────────────

impl KitState {
    pub fn to_json(&self) -> String {
        let mut s = String::with_capacity(8192);
        s.push_str(&format!("{{\"schema\":{SCHEMA_VERSION},\"folder\":{},\"bank_cap\":{},\"max_len_ms\":{},\"ram_cap_mb\":{},\"pool\":{},\"roll_repro\":{},\"roll_seed\":{}", quote(&self.folder), self.bank_cap, self.max_len_ms, self.ram_cap_mb, self.pool, self.roll_repro, self.roll_seed));
        s.push_str(&format!(",\"keywords\":{}", quote(&self.keywords)));
        s.push_str(",\"assign\":[");
        for (i, (r, p)) in self.assign.iter().enumerate() {
            if i > 0 {
                s.push(',');
            }
            s.push_str(&format!("[{},{}]", quote(r), p));
        }
        s.push_str("],\"pads\":[");
        for (i, (p, l)) in self.pads.iter().zip(&self.live).enumerate() {
            if i > 0 {
                s.push(',');
            }
            let cfg: Vec<String> = p.cfg.iter().map(|c| format!("[{},{},{}]", c.base, c.range, c.prob)).collect();
            let bank: Vec<String> = p.bank.iter().map(u32::to_string).collect();
            let pin = p.pin.as_deref().map_or("null".to_string(), quote);
            s.push_str(&format!(
                "{{\"cfg\":[{}],\"bank\":[{}],\"pin\":{},\"pin_seed\":{},\"gain\":{},\"pan\":{},\"mute\":{},\"solo\":{},\"poly\":{},\"choke\":{},\"choker\":{},\"seed\":{},\"slot\":{},\"mode\":{}}}",
                cfg.join(","), bank.join(","), pin, p.pin_seed, p.gain, p.pan, p.mute, p.solo, p.poly, p.choke_group, p.choker, l.seed, l.slot, l.mode
            ));
        }
        s.push_str("]}");
        s
    }

    /// Carga un estado. Rechaza lo corrupto (tipos, longitudes, versión futura) sin tocar nada.
    pub fn from_json(src: &str) -> Result<KitState, String> {
        let v = json::parse(src)?;
        let schema = num(&v, "schema")? as u32;
        if schema == 0 || schema > SCHEMA_VERSION {
            return Err(format!("versión de esquema {schema} no soportada (máx. {SCHEMA_VERSION})"));
        }
        let mut k = KitState::default();
        k.folder = text(&v, "folder")?;
        k.bank_cap = (num(&v, "bank_cap")? as usize).clamp(MIN_BANK, MAX_BANK);
        k.max_len_ms = (num(&v, "max_len_ms")? as u32).clamp(100, 30_000);
        k.ram_cap_mb = (num(&v, "ram_cap_mb")? as u32).clamp(16, 16_384);
        k.pool = (num(&v, "pool")? as usize).clamp(1, MAX_BANK);
        k.roll_repro = v.get("roll_repro").and_then(Value::as_bool).ok_or("falta roll_repro")?;
        k.roll_seed = num(&v, "roll_seed")? as u32;
        k.keywords = text(&v, "keywords")?;
        k.assign = Vec::new();
        for a in v.get("assign").and_then(Value::as_arr).ok_or("falta assign")? {
            let a = a.as_arr().filter(|a| a.len() == 2).ok_or("assign mal formado")?;
            let pad = a[1].as_f64().ok_or("assign: pad")? as usize;
            if pad >= NUM_PADS {
                return Err("assign: pad fuera de rango".into());
            }
            k.assign.push((a[0].as_str().ok_or("assign: ruta")?.to_string(), pad));
        }
        let pads = v.get("pads").and_then(Value::as_arr).ok_or("faltan los pads")?;
        if pads.len() != NUM_PADS {
            return Err(format!("hay {} pads y debe haber {NUM_PADS}", pads.len()));
        }
        for (i, p) in pads.iter().enumerate() {
            let mut c = PadCfg::default_for(i);
            let cfg = p.get("cfg").and_then(Value::as_arr).filter(|a| a.len() == 4).ok_or("cfg mal formado")?;
            for (j, e) in cfg.iter().enumerate() {
                let e = e.as_arr().filter(|a| a.len() == 3).ok_or("cfg mal formado")?;
                let g = |x: &Value| x.as_f64().ok_or("cfg: número").map(|n| (n as i64).clamp(0, 1000) as u16);
                c.cfg[j] = ParamCfg { base: g(&e[0])?, range: g(&e[1])?, prob: g(&e[2])? };
            }
            let bank = p.get("bank").and_then(Value::as_arr).filter(|a| a.len() == SLOTS).ok_or("bank mal formado")?;
            for (j, b) in bank.iter().enumerate() {
                c.bank[j] = (b.as_f64().ok_or("bank: número")? as i64).clamp(0, i64::from(SEED_MAX)) as u32;
            }
            c.pin = match p.get("pin") {
                Some(Value::Null) => None,
                Some(Value::Str(s)) => Some(s.clone()),
                _ => return Err("pin mal formado".into()),
            };
            c.pin_seed = (num(p, "pin_seed")? as i64).clamp(0, i64::from(SEED_MAX)) as u32;
            c.gain = (num(p, "gain")? as i64).clamp(0, 2000) as u16;
            c.pan = (num(p, "pan")? as i64).clamp(-1000, 1000) as i16;
            c.mute = p.get("mute").and_then(Value::as_bool).ok_or("falta mute")?;
            c.solo = p.get("solo").and_then(Value::as_bool).ok_or("falta solo")?;
            c.poly = (num(p, "poly")? as i64).clamp(1, 4) as u8;
            c.choke_group = (num(p, "choke")? as i64).clamp(0, 8) as u8;
            c.choker = p.get("choker").and_then(Value::as_bool).ok_or("falta choker")?;
            k.pads[i] = c;
            k.live[i] = PadLive {
                seed: (num(p, "seed")? as i64).clamp(0, i64::from(SEED_MAX)) as u32,
                slot: (num(p, "slot")? as i64).clamp(0, SLOTS as i64 - 1) as u8,
                mode: (num(p, "mode")? as i64).clamp(0, 15) as u8,
            };
        }
        Ok(k)
    }
}

fn num(v: &Value, key: &str) -> Result<f64, String> {
    v.get(key).and_then(Value::as_f64).ok_or_else(|| format!("falta o no es número: {key}"))
}

fn text(v: &Value, key: &str) -> Result<String, String> {
    v.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("falta o no es texto: {key}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{pad_of_piece, KeywordTable, Library};
    use crate::pieces::KICK;
    use crate::seedrand::{index_from_unit, OFFSET_SAMPLE};

    fn busy_state() -> KitState {
        let mut k = KitState::default();
        k.folder = "/home/david/Samples ñ \"x\"".into();
        k.bank_cap = 64;
        k.roll_repro = true;
        k.roll_seed = 99;
        k.pads[0].pin = Some("kicks/Kick_007.wav".into());
        k.pads[0].pin_seed = 31337;
        k.pads[0].bank[3] = 4242;
        k.pads[0].cfg[P_PITCH] = ParamCfg { base: 410, range: 250, prob: 330 };
        k.pads[5].mute = true;
        k.pads[7].solo = true;
        k.pads[7].poly = 4;
        k.live[0] = PadLive { seed: 31337, slot: 3, mode: 0b0101 };
        k.assign.push(("vox/odd.wav".into(), 9));
        k.keywords = "{\"x\":1}".into();
        k
    }

    #[test]
    fn state_roundtrip_keeps_pins_seeds_and_modes() {
        let k = busy_state();
        let back = KitState::from_json(&k.to_json()).unwrap();
        assert_eq!(back, k);
        assert_eq!(back.pads[0].pin.as_deref(), Some("kicks/Kick_007.wav"));
        assert_eq!(back.live[0], PadLive { seed: 31337, slot: 3, mode: 0b0101 });
        assert_eq!(back.pads[0].bank[3], 4242);
    }

    #[test]
    fn default_state_roundtrips() {
        let k = KitState::default();
        assert_eq!(KitState::from_json(&k.to_json()).unwrap(), k);
    }

    #[test]
    fn corrupt_state_is_rejected() {
        let good = busy_state().to_json();
        assert!(KitState::from_json("").is_err());
        assert!(KitState::from_json("{").is_err());
        assert!(KitState::from_json("[]").is_err());
        assert!(KitState::from_json(&good[..good.len() / 2]).is_err());
        assert!(KitState::from_json(&good.replace("\"schema\":1", "\"schema\":2")).is_err(), "versión futura");
        assert!(KitState::from_json(&good.replace("\"schema\":1", "\"schema\":0")).is_err());
        assert!(KitState::from_json(&good.replace("\"pads\":[", "\"pads\":[null,")).is_err());
        assert!(KitState::from_json(&good.replace("\"gain\":1000", "\"gain\":\"x\"")).is_err());
        assert!(KitState::from_json(&good.replace("\"pin\":null", "\"pin\":5")).is_err());
        // fuera de rango: se recorta, no revienta
        let k = KitState::from_json(&good.replace("\"gain\":1000", "\"gain\":999999")).unwrap();
        assert_eq!(k.pads[1].gain, 2000);
    }

    #[test]
    fn default_roll_range_one_is_exactly_floor_u_times_n() {
        let c = ParamCfg { base: 500, range: 1000, prob: 1000 };
        for seed in [0u32, 1, 77, 12345, 99999] {
            let u = seed_unit(seed, OFFSET_SAMPLE);
            for n in [1usize, 3, 8, 31, 32, 100, 128] {
                assert_eq!(sample_index(c, u, n), index_from_unit(u, n), "seed {seed} n {n}");
            }
        }
        assert_eq!(sample_index(c, 123, 0), None);
    }

    #[test]
    fn sample_index_adapts_when_the_bank_size_changes() {
        let c = ParamCfg { base: 500, range: 1000, prob: 1000 };
        let u = seed_unit(4242, OFFSET_SAMPLE);
        let small = sample_index(c, u, 10).unwrap();
        let big = sample_index(c, u, 100).unwrap();
        assert!(small < 10 && big < 100);
        assert_eq!(big / 10, small); // misma posición relativa
        // un rango estrecho acota la zona del banco
        let narrow = ParamCfg { base: 100, range: 100, prob: 1000 };
        for s in 0..200 {
            let i = sample_index(narrow, seed_unit(s, OFFSET_SAMPLE), 100).unwrap();
            assert!((5..=15).contains(&i), "{i}");
        }
    }

    #[test]
    fn lock_is_deterministic_and_roll_holds_when_probability_fails() {
        let mut pad = PadCfg::default_for(0);
        pad.cfg[P_SAMPLE].prob = 0; // nunca re-sortea
        let live = PadLive { seed: 555, slot: 0, mode: ALL_ROLL };
        let mut rt = PadRuntime::new(555);
        let mut roll = RollRng::new(1);
        let first = resolve_hit(&pad.cfg, live, &mut rt, &mut roll, 32);
        for _ in 0..50 {
            assert_eq!(resolve_hit(&pad.cfg, live, &mut rt, &mut roll, 32).index, first.index);
        }
        // el primer valor sale de la semilla (determinista antes del primer dado)
        assert_eq!(first.index, locked_index(&pad, 555, 32));
        // LOCK no consume el generador ni cambia nunca
        let lock = PadLive { mode: ALL_LOCK, ..live };
        let mut r1 = RollRng::new(5);
        let a = resolve_hit(&pad.cfg, lock, &mut rt, &mut r1, 32);
        assert_eq!(r1.next_u64(), RollRng::new(5).next_u64());
        assert_eq!(a, resolve_hit(&pad.cfg, lock, &mut rt, &mut RollRng::new(9), 32));
    }

    #[test]
    fn roll_with_full_probability_varies_and_is_reproducible_with_the_same_seed() {
        let pad = PadCfg::default_for(0);
        let live = PadLive { seed: 1, slot: 0, mode: ALL_ROLL };
        let run = |seed: u64| {
            let mut rt = PadRuntime::new(1);
            let mut roll = RollRng::new(seed);
            (0..64).map(|_| resolve_hit(&pad.cfg, live, &mut rt, &mut roll, 32).index.unwrap()).collect::<Vec<_>>()
        };
        let a = run(7);
        assert_eq!(a, run(7));
        assert_ne!(a, run(8));
        assert!(a.iter().collect::<std::collections::BTreeSet<_>>().len() > 10);
    }

    #[test]
    fn probability_matches_the_configured_rate() {
        let mut pad = PadCfg::default_for(0);
        pad.cfg[P_SAMPLE].prob = 250;
        let live = PadLive { seed: 1, slot: 0, mode: ALL_ROLL };
        let mut rt = PadRuntime::new(1);
        let mut roll = RollRng::new(3);
        let mut changes = 0;
        let mut prev = resolve_hit(&pad.cfg, live, &mut rt, &mut roll, 128).index;
        for _ in 0..4000 {
            let cur = resolve_hit(&pad.cfg, live, &mut rt, &mut roll, 128).index;
            if cur != prev {
                changes += 1;
            }
            prev = cur;
        }
        assert!((800..1100).contains(&changes), "{changes}"); // ~25 % de 4000 (algo menos por repetidos)
    }

    /// Escenario "reabrir proyecto": con Muestra en LOCK y pin guardado, tras recargar el estado
    /// suena la misma muestra aunque cambie el orden/contenido de la carpeta.
    #[test]
    fn reopen_project_plays_the_pinned_sample_even_if_the_folder_changed() {
        let table = KeywordTable::builtin();
        let pad_i = pad_of_piece(KICK);
        let files: Vec<String> = (0..20).map(|i| format!("kicks/Kick_{:03}.wav", i * 2)).collect();
        let lib = Library::build("/s", &files, &table, &[]);
        let (bank1, n1) = (lib.bank(pad_i, 32), lib.bank(pad_i, 32).len());

        // Una semilla cualquiera; buscamos una que, tras el cambio de carpeta, apuntaría a OTRA muestra.
        let probe = |seed: u32, files2: &[String]| {
            let lib2 = Library::build("/s", files2, &table, &[]);
            let b2 = lib2.bank(pad_i, 32).to_vec();
            let cfg = PadCfg::default_for(pad_i);
            let before = bank1[locked_index(&cfg, seed, n1).unwrap()].clone();
            let after = b2[locked_index(&cfg, seed, b2.len()).unwrap()].clone();
            (before, after)
        };
        let mut files2: Vec<String> = files.clone();
        files2.retain(|f| f != "kicks/Kick_002.wav");
        files2.extend((0..9).map(|i| format!("kicks/Kick_{:03}.wav", i * 2 + 1)));
        let seed = (0..500).find(|&s| {
            let (b, a) = probe(s, &files2);
            b != a && b != "kicks/Kick_002.wav"
        });
        let seed = seed.expect("alguna semilla debe moverse al cambiar la carpeta");

        let mut kit = KitState::default();
        kit.live[pad_i] = PadLive { seed, slot: 0, mode: ALL_LOCK };
        // al pasar a LOCK se fija el pin con lo que sonaba
        let chosen = bank1[locked_index(&kit.pads[pad_i], seed, n1).unwrap()].clone();
        kit.pads[pad_i].pin = Some(chosen.clone());
        kit.pads[pad_i].pin_seed = seed;

        let reopened = KitState::from_json(&kit.to_json()).unwrap();

        let lib2 = Library::build("/s", &files2, &table, &[]);
        let bank2 = lib2.bank(pad_i, 32);
        let seed_idx = locked_index(&reopened.pads[pad_i], seed, bank2.len());
        let pick = pick_sample(&reopened.pads[pad_i], true, seed, seed_idx, bank2);
        assert!(!pick.pin_missing);
        assert_eq!(bank2[pick.index.unwrap()], chosen, "el pin debe sonar igual");
        assert_ne!(bank2[seed_idx.unwrap()], chosen, "sin pin habría sonado otra muestra");

        // si el archivo del pin desapareció: índice por semilla + aviso
        let files3: Vec<String> = files2.iter().filter(|f| **f != chosen).cloned().collect();
        let lib3 = Library::build("/s", &files3, &table, &[]);
        let bank3 = lib3.bank(pad_i, 32);
        let seed_idx3 = locked_index(&reopened.pads[pad_i], seed, bank3.len());
        let pick3 = pick_sample(&reopened.pads[pad_i], true, seed, seed_idx3, bank3);
        assert!(pick3.pin_missing);
        assert_eq!(pick3.index, seed_idx3);

        // si la semilla cambió, el pin ya no manda (elige otra muestra por semilla)
        let other = pick_sample(&reopened.pads[pad_i], true, seed + 1, seed_idx, bank2);
        assert!(!other.pin_missing && other.index == seed_idx);

        // en ROLL el pin no se usa
        let pick4 = pick_sample(&reopened.pads[pad_i], false, seed, seed_idx, bank2);
        assert_eq!(pick4, SamplePick { index: seed_idx, pin_missing: false });
    }
}
