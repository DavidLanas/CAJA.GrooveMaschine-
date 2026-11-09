//! Generación estocástica del patrón. Port fiel de `generatePattern` (C++/appweb): mismo PRNG,
//! mismo orden de consumo de números aleatorios y mismo redondeo que JavaScript.
//!
//! Las 5 piezas nuevas (índices 20..24) usan un segundo flujo aleatorio independiente, así los
//! 22 géneros originales dan exactamente los mismos patrones que antes.

use crate::genres::{Genre, GenreSet};
use crate::parser::*;
use crate::pieces::*;
use crate::rng::{hash_seed, js_round, Mulberry32};

pub const STEPS_PER_BAR: usize = 16;
pub const MAX_BARS: usize = 8;
pub const MAX_STEPS: usize = STEPS_PER_BAR * MAX_BARS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    /// 0 = paso vacío.
    pub vel: u8,
    pub ghost: bool,
}

impl Cell {
    pub fn on(&self) -> bool {
        self.vel > 0
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pattern {
    pub bars: usize,
    pub cell: [[Cell; MAX_STEPS]; NUM_PIECES],
}

impl Default for Pattern {
    fn default() -> Self {
        Pattern { bars: 2, cell: [[Cell::default(); MAX_STEPS]; NUM_PIECES] }
    }
}

impl Pattern {
    pub fn steps(&self) -> usize {
        self.bars * STEPS_PER_BAR
    }

    pub fn clear(&mut self) {
        self.cell = [[Cell::default(); MAX_STEPS]; NUM_PIECES];
    }

    pub fn count_on(&self) -> usize {
        (0..NUM_PIECES).map(|p| (0..self.steps()).filter(|&i| self.cell[p][i].on()).count()).sum()
    }

    /// Valor con signo para la interfaz: 0 vacío, velocidad, o velocidad negativa si es ghost.
    pub fn signed(&self, piece: usize, step: usize) -> i32 {
        let c = self.cell[piece][step];
        if !c.on() {
            0
        } else if c.ghost {
            -i32::from(c.vel)
        } else {
            i32::from(c.vel)
        }
    }
}

#[derive(Clone, Debug)]
pub struct Spec {
    pub genre: usize,
    pub mods: u32,
    pub bars: usize,
    /// 0-100
    pub density: i32,
    pub seed: String,
    /// 0-100; 50 = perfil tal cual.
    pub accent: i32,
}

impl Default for Spec {
    fn default() -> Self {
        Spec { genre: 0, mods: 0, bars: 2, density: 50, seed: "0".to_string(), accent: 50 }
    }
}

fn adjust(p0: f64, pi: usize, s: usize, bar: usize, spec: &Spec, jitter: f64) -> f64 {
    let mut p = p0;
    let group = PIECES[pi].group;
    let m = spec.mods;
    if bar > 0 && p > 16.0 && p < 90.0 {
        p += (jitter - 0.5) * 12.0;
    }
    if m & MOD_SINCOPADO != 0 && group == Group::Kick {
        if matches!(s, 3 | 6 | 10 | 13 | 14) {
            p += 38.0;
        }
        if s == 4 || s == 12 {
            p *= 0.35;
        }
    }
    if m & MOD_SINCOPADO != 0 && pi == SNARE && matches!(s, 3 | 6 | 10 | 14) {
        p += 22.0;
    }
    if m & MOD_VERSO != 0 {
        p *= 0.84;
        if matches!(pi, OH | OH2 | CLAP | CRASH) {
            p *= 0.35;
        }
    }
    if m & MOD_CORO != 0 {
        p *= 1.06;
        if pi == CLAP && (s == 4 || s == 12) {
            p = p.max(94.0);
        }
        if (pi == OH || pi == OH2) && s % 4 == 2 {
            p = p.max(48.0);
        }
        if pi == CRASH && s == 0 && bar % 2 == 0 {
            p = 100.0;
        }
    }
    if m & MOD_SEISAVOS != 0 && (pi == CH || pi == CH2) && s % 2 == 1 {
        p = p.max(60.0);
    }
    if m & MOD_OCHOS != 0 && (pi == CH || pi == CH2 || pi == SHAKER) && s % 2 == 1 {
        p = 0.0;
    }
    if pi == CRASH {
        if s != 0 {
            p = 0.0;
        } else if bar > 0 && m & MOD_CORO == 0 {
            p = 0.0;
        }
    }
    let factor = 1.0 + (f64::from(spec.density - 50) / 50.0) * 0.75;
    if !(p >= 97.0 && factor < 1.0 && spec.density >= 18) {
        p *= factor;
    }
    p.clamp(0.0, 100.0)
}

fn apply_half_time(pat: &mut Pattern) {
    for i in 0..pat.steps() {
        let s = i % 16;
        if s == 4 || s == 12 {
            for id in [SNARE, SN2, CLAP] {
                let c = &mut pat.cell[id][i];
                if c.on() && !c.ghost {
                    *c = Cell::default();
                }
            }
        }
        if s == 8 {
            let sn = &mut pat.cell[SNARE][i];
            let prev = if sn.on() && !sn.ghost { sn.vel } else { 0 };
            sn.vel = prev.max(116);
            sn.ghost = false;
        }
    }
}

fn apply_fill(pat: &mut Pattern) {
    let base = (pat.bars - 1) * 16;
    pat.cell[CRASH][base] = Cell { vel: 122, ghost: false };
    const SEQ: [(usize, usize, u8); 8] =
        [(TOM1, 8, 106), (TOM1, 9, 110), (TOM2, 10, 112), (TOM2, 11, 116), (TOM2, 12, 118), (TOM3, 13, 120), (TOM3, 14, 122), (TOM3, 15, 126)];
    for (id, s, v) in SEQ {
        pat.cell[id][base + s] = Cell { vel: v, ghost: false };
    }
    for s in 8..14 {
        pat.cell[CH][base + s] = Cell::default();
        pat.cell[OH][base + s] = Cell::default();
        let sn = &mut pat.cell[SNARE][base + s];
        if sn.on() && !sn.ghost {
            *sn = Cell::default();
        }
    }
    pat.cell[SNARE][base + 14] = Cell { vel: 104, ghost: false };
    pat.cell[SNARE][base + 15] = Cell { vel: 124, ghost: false };
}

fn apply_choke(pat: &mut Pattern) {
    for i in 0..pat.steps() {
        if pat.cell[OH][i].on() || pat.cell[OH2][i].on() {
            pat.cell[CH][i] = Cell::default();
            pat.cell[CH2][i] = Cell::default();
        }
    }
}

/// Decide la celda de una pieza en un paso a partir de las tres tiradas ya consumidas.
fn decide(g: &Genre, pi: usize, s: usize, bar: usize, spec: &Spec, draws: (f64, f64, f64), acc_scale: f64) -> Option<Cell> {
    let (jitter, tiro, vrand) = draws;
    let def = g.pieces[pi].as_ref()?;
    let acc = g.accent.profile();
    let p = adjust(f64::from(def.p[s]), pi, s, bar, spec, jitter);
    let can_ghost = matches!(pi, SNARE | SN2 | CH | CH2 | RIM);
    if tiro * 100.0 < p {
        let mut ghost = false;
        let mut vel: f64;
        if p < 40.0 && can_ghost {
            ghost = true;
            vel = f64::from(22 + js_round(p * 0.45 + vrand * 8.0));
        } else if p < 55.0 && PIECES[pi].group == Group::Kick {
            vel = f64::from(js_round(f64::from(def.v) * 0.66 + (vrand - 0.5) * 6.0));
        } else if p < 75.0 {
            vel = f64::from(js_round(f64::from(def.v) * 0.84 + (vrand - 0.5) * 6.0));
        } else {
            vel = f64::from(def.v) + f64::from(acc[s]) * acc_scale + f64::from(js_round((vrand - 0.5) * 8.0));
        }
        if spec.mods & MOD_GHOST != 0 && can_ghost && p < 72.0 && p >= 40.0 {
            ghost = true;
            vel = vel.min(f64::from(36 + js_round(vrand * 16.0)));
        }
        let (lo, hi) = if ghost { (12, 64) } else { (24, 127) };
        let v = js_round(vel).clamp(lo, hi);
        Some(Cell { vel: v as u8, ghost })
    } else if spec.mods & MOD_GHOST != 0 && can_ghost && s % 2 == 1 && tiro > 0.8 {
        Some(Cell { vel: (18 + js_round(vrand * 20.0)) as u8, ghost: true })
    } else {
        None
    }
}

pub fn generate_pattern(spec: &Spec, genres: &GenreSet) -> Pattern {
    let mut pat = Pattern::default();
    pat.bars = spec.bars.clamp(1, MAX_BARS);
    let g = genres.get(spec.genre);
    let seed = hash_seed(&spec.seed);
    let mut rnd = Mulberry32::new(seed);
    // Flujo aparte para las piezas nuevas: no altera el orden de consumo del flujo original.
    let mut rnd_ext = Mulberry32::new(seed ^ 0x5bd1_e995);
    let acc_scale = f64::from(spec.accent) / 50.0;
    for bar in 0..pat.bars {
        for pi in 0..LEGACY_PIECES {
            for s in 0..16 {
                let draws = (rnd.next(), rnd.next(), rnd.next());
                if let Some(c) = decide(g, pi, s, bar, spec, draws, acc_scale) {
                    pat.cell[pi][bar * 16 + s] = c;
                }
            }
        }
        for pi in LEGACY_PIECES..NUM_PIECES {
            for s in 0..16 {
                let draws = (rnd_ext.next(), rnd_ext.next(), rnd_ext.next());
                if let Some(c) = decide(g, pi, s, bar, spec, draws, acc_scale) {
                    pat.cell[pi][bar * 16 + s] = c;
                }
            }
        }
    }
    if spec.mods & MOD_HALFTIME != 0 {
        apply_half_time(&mut pat);
    }
    if spec.mods & MOD_FILL != 0 {
        apply_fill(&mut pat);
    }
    apply_choke(&mut pat);
    pat
}

// ── Serialización JSON del patrón (compartida por Session y PatternRack) ─────

use crate::json::Value;
use crate::pieces::NUM_PIECES;

/// Escribe `{"bars":N,"c":[pi,st,code, ...]}` en el buffer.
pub fn write_pattern_value(s: &mut String, p: &Pattern) {
    s.push_str(&format!("{{\"bars\":{},\"c\":[", p.bars));
    let mut first = true;
    for pi in 0..NUM_PIECES {
        for i in 0..p.steps() {
            let c = p.cell[pi][i];
            if c.on() {
                if !first {
                    s.push(',');
                }
                first = false;
                s.push_str(&format!("{pi},{i},{}", u32::from(c.vel) * 2 + u32::from(c.ghost)));
            }
        }
    }
    s.push_str("]}");
}

/// Lee un patrón desde un Value JSON producido por `write_pattern_value`.
pub fn read_pattern_value(v: &Value) -> Result<Pattern, String> {
    let bars_f = v.get("bars").and_then(Value::as_f64).ok_or("patrón sin bars")?;
    if bars_f.fract() != 0.0 || bars_f < 1.0 || bars_f > MAX_BARS as f64 {
        return Err(format!("bars = {bars_f} fuera de 1-{MAX_BARS}"));
    }
    let bars = bars_f as usize;
    let arr = v.get("c").and_then(Value::as_arr).ok_or("patrón sin celdas")?;
    if arr.len() % 3 != 0 {
        return Err("lista de celdas incompleta".into());
    }
    let mut pat = Pattern::default();
    pat.bars = bars;
    for t in arr.chunks(3) {
        let n = |x: &Value| -> Result<i64, String> {
            let f = x.as_f64().ok_or("celda no numérica")?;
            if f.fract() != 0.0 {
                return Err("celda no entera".to_string());
            }
            Ok(f as i64)
        };
        let (pi, st, code) = (n(&t[0])?, n(&t[1])?, n(&t[2])?);
        if !(0..NUM_PIECES as i64).contains(&pi)
            || !(0..(bars * STEPS_PER_BAR) as i64).contains(&st)
            || !(2..=255).contains(&code)
        {
            return Err(format!("celda fuera de rango ({pi},{st},{code})"));
        }
        let cell = &mut pat.cell[pi as usize][st as usize];
        if cell.on() {
            return Err("celda repetida".into());
        }
        *cell = Cell {
            vel: (code / 2) as u8,
            ghost: code % 2 == 1,
        };
    }
    Ok(pat)
}
