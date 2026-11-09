//! Mapas de notas por kit. Caja no genera audio: los golpes suenan con el kit de la Drum Machine
//! siguiente, y un note-effect no puede leer los pads de la cadena, así que la adaptación es
//! un mapa pieza -> nota MIDI. Presets integrados en `data/drum-maps.json`.

use crate::json::{self, quote, Value};
use crate::pieces::*;

pub const BUILTIN_JSON: &str = include_str!("../data/drum-maps.json");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrumMap {
    pub name: String,
    pub notes: [u8; NUM_PIECES],
}

impl DrumMap {
    /// `{"name": "...", "notes": {"kick": 36, ...}}` (también es el formato de ~/.config/caja/maps/*.json).
    pub fn to_json(&self) -> String {
        let mut s = format!("{{\n  \"name\": {},\n  \"notes\": {{ ", quote(&self.name));
        for (i, p) in PIECES.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&format!("\"{}\": {}", p.id, self.notes[i]));
        }
        s.push_str(" }\n}\n");
        s
    }

    pub fn from_value(v: &Value, tag: &str, errs: &mut Vec<String>) -> Option<DrumMap> {
        let name = v.get("name").and_then(Value::as_str).unwrap_or("").trim().to_string();
        if name.is_empty() {
            errs.push(format!("{tag}: falta \"name\""));
        }
        let Some(items) = v.get("notes").and_then(Value::as_obj) else {
            errs.push(format!("{tag}: falta \"notes\""));
            return None;
        };
        let mut notes = [255u8; NUM_PIECES];
        let before = errs.len();
        for (k, nv) in items {
            let Some(pi) = piece_index(k) else {
                errs.push(format!("{tag}: pieza desconocida \"{k}\""));
                continue;
            };
            match nv.as_f64() {
                Some(x) if x.fract() == 0.0 && (0.0..=127.0).contains(&x) => notes[pi] = x as u8,
                _ => errs.push(format!("{tag}: la nota de \"{k}\" debe ser un entero de 0 a 127")),
            }
        }
        for (i, n) in notes.iter().enumerate() {
            if *n == 255 {
                errs.push(format!("{tag}: falta la nota de \"{}\"", PIECES[i].id));
            }
        }
        if errs.len() == before && !name.is_empty() {
            Some(DrumMap { name, notes })
        } else {
            None
        }
    }

    pub fn parse_one(src: &str) -> Result<DrumMap, Vec<String>> {
        let v = json::parse(src).map_err(|e| vec![e])?;
        let mut errs = Vec::new();
        match DrumMap::from_value(&v, "mapa", &mut errs) {
            Some(m) => Ok(m),
            None => Err(errs),
        }
    }
}

#[derive(Clone, Debug)]
pub struct MapSet {
    pub maps: Vec<DrumMap>,
}

impl MapSet {
    pub fn builtin() -> MapSet {
        MapSet::from_json(BUILTIN_JSON).expect("drum-maps.json integrado válido")
    }

    pub fn from_json(src: &str) -> Result<MapSet, Vec<String>> {
        let root = json::parse(src).map_err(|e| vec![e])?;
        let Some(list) = root.get("maps").and_then(Value::as_arr) else {
            return Err(vec!["falta la lista \"maps\"".to_string()]);
        };
        let mut errs = Vec::new();
        let mut maps: Vec<DrumMap> = Vec::new();
        for (i, mv) in list.iter().enumerate() {
            if let Some(m) = DrumMap::from_value(mv, &format!("mapa {}", i + 1), &mut errs) {
                if maps.iter().any(|x| x.name == m.name) {
                    errs.push(format!("mapa {}: nombre repetido \"{}\"", i + 1, m.name));
                }
                maps.push(m);
            }
        }
        if list.is_empty() {
            errs.push("la lista \"maps\" está vacía".to_string());
        }
        if errs.is_empty() {
            Ok(MapSet { maps })
        } else {
            Err(errs)
        }
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.maps.iter().position(|m| m.name == name)
    }

    /// El preset de Bitwig que todavía trae el mapa viejo (snare/hat cruzados) pasa al orden de pads.
    /// Un mapa editado a mano no se toca.
    pub fn upgrade_legacy_bitwig(&mut self) {
        for m in &mut self.maps {
            if m.name == "Bitwig Drum Machine" && m.notes == legacy_notes() {
                m.notes = default_notes();
            }
        }
    }

    /// Añade o reemplaza por nombre.
    pub fn upsert(&mut self, m: DrumMap) -> usize {
        match self.index_of(&m.name) {
            Some(i) => {
                self.maps[i] = m;
                i
            }
            None => {
                self.maps.push(m);
                self.maps.len() - 1
            }
        }
    }
}

/// Modo "Aprender": asigna la nota pulsada a la pieza seleccionada y pasa a la siguiente.
#[derive(Clone, Debug, Default)]
pub struct Learn {
    pub active: bool,
    pub piece: usize,
}

impl Learn {
    pub fn start(&mut self, piece: usize) {
        self.active = true;
        self.piece = piece.min(NUM_PIECES - 1);
    }

    pub fn stop(&mut self) {
        self.active = false;
    }

    /// Devuelve (pieza, nota) asignada. Tras la última pieza el modo se apaga solo.
    pub fn on_note(&mut self, note: u8, notes: &mut [u8; NUM_PIECES]) -> Option<(usize, u8)> {
        if !self.active {
            return None;
        }
        let p = self.piece;
        notes[p] = note & 0x7f;
        match pad_after(p) {
            Some(next) => self.piece = next,
            None => self.active = false,
        }
        Some((p, note & 0x7f))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_has_bitwig_and_gm_and_bitwig_equals_default_notes() {
        let m = MapSet::builtin();
        assert_eq!(m.maps[0].name, "Bitwig Drum Machine");
        assert_eq!(m.maps[0].notes, default_notes());
        assert_eq!(m.maps[0].notes[KICK], 36);
        assert_eq!(m.maps[0].notes[SNARE], 37);
        assert_eq!(m.maps[0].notes[CH], 38);
        assert_eq!(m.maps[1].name, "GM");
        assert_eq!(m.maps[1].notes[CH], 42);
        assert_eq!(m.maps[1].notes[OH], 46);
        assert_eq!(m.maps[1].notes[CLAVE], 75);
        assert_eq!(m.maps[1].notes[EGG], 69);
    }

    #[test]
    fn roundtrip_and_validation() {
        let m = MapSet::builtin().maps[1].clone();
        assert_eq!(DrumMap::parse_one(&m.to_json()).unwrap(), m);
        let bad = r#"{"name":"x","notes":{"kick":200,"bombo":3}}"#;
        let e = DrumMap::parse_one(bad).unwrap_err();
        assert!(e.iter().any(|x| x.contains("0 a 127")));
        assert!(e.iter().any(|x| x.contains("desconocida")));
        assert!(e.iter().any(|x| x.contains("falta la nota")));
        assert!(DrumMap::parse_one("{").is_err());
    }

    #[test]
    fn learn_walks_through_pieces() {
        let mut notes = default_notes();
        let mut l = Learn::default();
        assert!(l.on_note(60, &mut notes).is_none());
        l.start(1);
        assert_eq!(l.on_note(61, &mut notes), Some((1, 61)));
        assert_eq!(l.piece, 2);
        assert_eq!(notes[SNARE], 61);
        l.start(NUM_PIECES - 1);
        assert!(l.on_note(99, &mut notes).is_some());
        assert!(!l.active);
        l.start(EGG);
        assert_eq!(l.on_note(70, &mut notes), Some((EGG, 70)));
        assert_eq!(l.piece, TOM1, "después de egg sigue tom 1, no clap");
    }

    #[test]
    fn legacy_bitwig_preset_is_upgraded_and_a_custom_map_is_not() {
        let mut set = MapSet { maps: vec![DrumMap { name: "Bitwig Drum Machine".into(), notes: legacy_notes() }] };
        set.upgrade_legacy_bitwig();
        assert_eq!(set.maps[0].notes[SNARE], 37);
        assert_eq!(set.maps[0].notes[CH], 38);
        set.maps[0].notes[KICK] = 40;
        let kept = set.maps[0].notes;
        set.upgrade_legacy_bitwig();
        assert_eq!(set.maps[0].notes, kept);
    }
}
