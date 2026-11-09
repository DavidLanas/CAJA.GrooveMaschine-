//! Rack multi-patrón: 2 bancos × 16 patrones (32 entradas fijas).
//!
//! Cada entrada guarda su propia rejilla **y** sus parámetros de generación
//! (seed, genre, swing, human, accent, density). El tempo (BPM) sigue siendo
//! global del transporte/host.
//!
//! Compatible con el mapeo MK3:
//! - PATTERN          → banco A (pads 1–16 = índices 0–15)
//! - SHIFT + PATTERN  → banco B (pads 1–16 = índices 16–31)

use crate::json::{quote, Value};
use crate::pattern::{read_pattern_value, write_pattern_value, Pattern, MAX_BARS};
use crate::session::{bars_from_idx, ParamSnap};

/// Máscara de los 9 bits de modificadores de frase (misma que Session::ALL_MODS).
const ALL_MODS_MASK: u32 = (1 << 9) - 1;

/// Bancos A/B (coincide con pads del MK3).
pub const NUM_PATTERN_BANKS: usize = 2;
/// Patrones por banco (= 16 pads).
pub const PATTERNS_PER_BANK: usize = 16;
/// Total de entradas fijas del rack.
pub const NUM_PATTERNS: usize = NUM_PATTERN_BANKS * PATTERNS_PER_BANK;

/// Parámetros de generación propios de un patrón.
/// (BPM queda fuera a propósito: es del transporte global.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PatternParams {
    pub genre: i32,
    pub bars_idx: i32,
    pub swing: i32,
    pub human: i32,
    pub density: i32,
    pub seed: i32,
    pub accent: i32,
}

impl Default for PatternParams {
    fn default() -> Self {
        PatternParams {
            genre: 1,
            bars_idx: 1, // 2 compases
            swing: 0,
            human: 14,
            density: 50,
            seed: 12345,
            accent: 50,
        }
    }
}

impl PatternParams {
    pub fn from_snap(p: &ParamSnap) -> Self {
        PatternParams {
            genre: p.genre,
            bars_idx: p.bars_idx,
            swing: p.swing,
            human: p.human,
            density: p.density,
            seed: p.seed,
            accent: p.accent,
        }
    }

    /// Escribe estos valores sobre un ParamSnap (conserva bpm del host).
    pub fn apply_to_snap(&self, snap: &mut ParamSnap) {
        snap.genre = self.genre;
        snap.bars_idx = self.bars_idx;
        snap.swing = self.swing;
        snap.human = self.human;
        snap.density = self.density;
        snap.seed = self.seed;
        snap.accent = self.accent;
    }

    pub fn bars(&self) -> usize {
        bars_from_idx(self.bars_idx)
    }
}

/// Una ranura del rack: rejilla + metadatos + parámetros propios.
#[derive(Clone, Debug, PartialEq)]
pub struct PatternEntry {
    pub pattern: Pattern,
    pub phrase: String,
    pub warning: String,
    pub mods: u32,
    /// Etiqueta corta opcional (vacía = sin nombre).
    pub name: String,
    pub params: PatternParams,
}

impl Default for PatternEntry {
    fn default() -> Self {
        PatternEntry {
            pattern: Pattern::default(),
            phrase: String::new(),
            warning: String::new(),
            mods: 0,
            name: String::new(),
            params: PatternParams::default(),
        }
    }
}

impl PatternEntry {
    /// ¿Tiene al menos una celda activa o alguna frase/mods distintos del vacío?
    pub fn is_empty(&self) -> bool {
        self.pattern.count_on() == 0 && self.phrase.is_empty() && self.mods == 0 && self.name.is_empty()
    }

    pub fn bank(&self, index: usize) -> usize {
        index / PATTERNS_PER_BANK
    }

    pub fn slot_in_bank(index: usize) -> usize {
        index % PATTERNS_PER_BANK
    }
}

/// Rack de 32 patrones con un índice activo.
#[derive(Clone, Debug)]
pub struct PatternRack {
    pub entries: [PatternEntry; NUM_PATTERNS],
    /// Índice activo 0..31.
    pub active: usize,
}

impl Default for PatternRack {
    fn default() -> Self {
        PatternRack {
            entries: std::array::from_fn(|_| PatternEntry::default()),
            active: 0,
        }
    }
}

impl PatternRack {
    pub fn active_entry(&self) -> &PatternEntry {
        &self.entries[self.active.min(NUM_PATTERNS - 1)]
    }

    pub fn active_entry_mut(&mut self) -> &mut PatternEntry {
        let i = self.active.min(NUM_PATTERNS - 1);
        &mut self.entries[i]
    }

    /// Cambia el patrón activo. Devuelve `true` si cambió.
    pub fn select(&mut self, index: usize) -> bool {
        let i = index.min(NUM_PATTERNS - 1);
        if i == self.active {
            return false;
        }
        self.active = i;
        true
    }

    pub fn bank_of_active(&self) -> usize {
        self.active / PATTERNS_PER_BANK
    }

    /// Copia la entrada `from` sobre `to` (incluye rejilla y params).
    pub fn copy(&mut self, from: usize, to: usize) {
        if from >= NUM_PATTERNS || to >= NUM_PATTERNS || from == to {
            return;
        }
        self.entries[to] = self.entries[from].clone();
    }

    /// Borra la entrada (deja Default).
    pub fn clear(&mut self, index: usize) {
        if index < NUM_PATTERNS {
            self.entries[index] = PatternEntry::default();
        }
    }

    /// Intercambia dos entradas.
    pub fn swap(&mut self, a: usize, b: usize) {
        if a < NUM_PATTERNS && b < NUM_PATTERNS && a != b {
            self.entries.swap(a, b);
        }
    }

    // ── Serialización JSON (solo entradas no vacías para no inflar el preset) ──

    pub fn to_json_value(&self) -> String {
        let mut s = String::from("{\"active\":");
        s.push_str(&self.active.to_string());
        s.push_str(",\"e\":[");
        let mut first = true;
        for (i, e) in self.entries.iter().enumerate() {
            if e.is_empty() {
                continue;
            }
            if !first {
                s.push(',');
            }
            first = false;
            s.push('{');
            s.push_str(&format!("\"i\":{},", i));
            if !e.name.is_empty() {
                s.push_str(&format!("\"name\":{},", quote(&e.name)));
            }
            if !e.phrase.is_empty() {
                s.push_str(&format!("\"phrase\":{},", quote(&e.phrase)));
            }
            if !e.warning.is_empty() {
                s.push_str(&format!("\"warning\":{},", quote(&e.warning)));
            }
            if e.mods != 0 {
                s.push_str(&format!("\"mods\":{},", e.mods));
            }
            let p = &e.params;
            s.push_str(&format!(
                "\"genre\":{},\"bars\":{},\"swing\":{},\"human\":{},\"density\":{},\"seed\":{},\"accent\":{},",
                p.genre, p.bars_idx, p.swing, p.human, p.density, p.seed, p.accent
            ));
            s.push_str("\"pattern\":");
            write_pattern_value(&mut s, &e.pattern);
            s.push('}');
        }
        s.push_str("]}");
        s
    }

    pub fn from_json_value(v: &Value) -> Result<PatternRack, String> {
        let active = v
            .get("active")
            .and_then(Value::as_f64)
            .map(|f| f as usize)
            .unwrap_or(0)
            .min(NUM_PATTERNS - 1);
        let mut rack = PatternRack { entries: std::array::from_fn(|_| PatternEntry::default()), active };
        let arr = match v.get("e").and_then(Value::as_arr) {
            Some(a) => a,
            None => return Ok(rack),
        };
        for item in arr {
            let idx = item
                .get("i")
                .and_then(Value::as_f64)
                .ok_or("entrada de rack sin índice")? as usize;
            if idx >= NUM_PATTERNS {
                return Err(format!("índice de patrón fuera de rango: {idx}"));
            }
            let mut e = PatternEntry::default();
            e.name = item.get("name").and_then(Value::as_str).unwrap_or("").to_string();
            e.phrase = item.get("phrase").and_then(Value::as_str).unwrap_or("").to_string();
            e.warning = item.get("warning").and_then(Value::as_str).unwrap_or("").to_string();
            e.mods = item
                .get("mods")
                .and_then(Value::as_f64)
                .map(|f| f as u32)
                .unwrap_or(0)
                & ALL_MODS_MASK;
            e.params = PatternParams {
                genre: int_or(item, "genre", 0, 4095, 1),
                bars_idx: int_or(item, "bars", 0, 3, 1),
                swing: int_or(item, "swing", 0, 100, 0),
                human: int_or(item, "human", 0, 100, 14),
                density: int_or(item, "density", 0, 100, 50),
                seed: int_or(item, "seed", 0, 999_999_999, 12345),
                accent: int_or(item, "accent", 0, 100, 50),
            };
            if let Some(pv) = item.get("pattern") {
                e.pattern = read_pattern_value(pv)?;
                // coherencia: bars del Pattern alineado con bars_idx
                e.pattern.bars = e.params.bars().clamp(1, MAX_BARS);
            }
            rack.entries[idx] = e;
        }
        Ok(rack)
    }
}

fn int_or(v: &Value, k: &str, lo: i64, hi: i64, default: i32) -> i32 {
    v.get(k)
        .and_then(Value::as_f64)
        .map(|f| (f as i64).clamp(lo, hi) as i32)
        .unwrap_or(default)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::json;
    use crate::pattern::Cell;

    #[test]
    fn default_rack_is_empty() {
        let r = PatternRack::default();
        assert_eq!(r.active, 0);
        assert!(r.entries.iter().all(|e| e.is_empty()));
    }

    #[test]
    fn select_and_copy() {
        let mut r = PatternRack::default();
        r.entries[0].pattern.cell[0][0] = Cell { vel: 100, ghost: false };
        r.entries[0].params.seed = 42;
        r.entries[0].phrase = "boom bap".into();
        r.copy(0, 3);
        assert_eq!(r.entries[3].params.seed, 42);
        assert_eq!(r.entries[3].phrase, "boom bap");
        assert!(r.select(3));
        assert_eq!(r.active, 3);
        assert!(!r.select(3));
    }

    #[test]
    fn json_roundtrip_sparse() {
        let mut r = PatternRack::default();
        r.active = 5;
        r.entries[5].params.seed = 777;
        r.entries[5].params.genre = 3;
        r.entries[5].phrase = "techno".into();
        r.entries[5].pattern.cell[1][4] = Cell { vel: 110, ghost: false };
        r.entries[5].name = "A6".into();
        let js = r.to_json_value();
        // Solo debe aparecer una entrada
        assert!(js.contains("\"i\":5"));
        assert!(!js.contains("\"i\":0"));
        let v = json::parse(&js).expect("parse");
        let r2 = PatternRack::from_json_value(&v).expect("from");
        assert_eq!(r2.active, 5);
        assert_eq!(r2.entries[5].params.seed, 777);
        assert_eq!(r2.entries[5].params.genre, 3);
        assert_eq!(r2.entries[5].phrase, "techno");
        assert_eq!(r2.entries[5].name, "A6");
        assert!(r2.entries[5].pattern.cell[1][4].on());
        assert!(r2.entries[0].is_empty());
    }
}
