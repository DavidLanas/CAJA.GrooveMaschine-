//! Géneros: plantillas de 16 pasos × pieza con probabilidades, cargadas de `genres.json`.
//!
//! El archivo integrado vive en `data/genres.json`. El usuario puede sustituirlo (ver `paths`).
//! La validación es estricta y acumula todos los errores para mostrarlos juntos.

use crate::json::{self, Value};
use crate::pieces::{piece_index, NUM_PIECES};

pub const BUILTIN_JSON: &str = include_str!("../data/genres.json");
/// Tope de géneros: el parámetro `Genero` tiene un rango fijo.
pub const MAX_GENRES: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent {
    Standard,
    Offbeat,
    Backbeat,
    Sync,
}

impl Accent {
    pub fn from_id(s: &str) -> Option<Accent> {
        match s {
            "standard" => Some(Accent::Standard),
            "offbeat" => Some(Accent::Offbeat),
            "backbeat" => Some(Accent::Backbeat),
            "sync" => Some(Accent::Sync),
            _ => None,
        }
    }

    /// Ajuste de velocidad por paso, aplicado a los golpes "seguros" (p >= 75).
    pub fn profile(self) -> &'static [i32; 16] {
        const STANDARD: [i32; 16] = [6, 0, 0, 0, 6, 0, 0, 0, 6, 0, 0, 0, 6, 0, 0, 0];
        const OFFBEAT: [i32; 16] = [4, 0, 8, 0, 4, 0, 8, 0, 4, 0, 8, 0, 4, 0, 8, 0];
        const BACKBEAT: [i32; 16] = [8, 0, 0, 0, 10, 0, 0, 0, 6, 0, 0, 0, 10, 0, 0, 0];
        const SYNC: [i32; 16] = [6, 0, 0, 6, 4, 0, 6, 0, 6, 0, 0, 6, 4, 0, 6, 0];
        match self {
            Accent::Standard => &STANDARD,
            Accent::Offbeat => &OFFBEAT,
            Accent::Backbeat => &BACKBEAT,
            Accent::Sync => &SYNC,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PieceTemplate {
    /// Velocidad base.
    pub v: i32,
    /// Probabilidad 0-100 por semicorchea.
    pub p: [u8; 16],
}

#[derive(Clone, Debug)]
pub struct Genre {
    pub family: String,
    pub id: String,
    pub name: String,
    /// Género nuevo propuesto (afinar de oído), no portado de la appweb.
    pub proposal: bool,
    pub bpm: i32,
    pub swing: i32,
    pub human: i32,
    pub accent: Accent,
    pub triplet_push: bool,
    /// Desempate del parser entre alias de igual longitud (menor gana).
    pub alias_priority: i64,
    pub aliases: Vec<String>,
    /// Plantilla por pieza (índice de pieza); `None` = la pieza no suena.
    pub pieces: [Option<PieceTemplate>; NUM_PIECES],
}

#[derive(Clone, Debug)]
pub struct GenreSet {
    pub genres: Vec<Genre>,
}

impl GenreSet {
    pub fn builtin() -> GenreSet {
        GenreSet::from_json(BUILTIN_JSON).expect("genres.json integrado válido")
    }

    pub fn len(&self) -> usize {
        self.genres.len()
    }

    pub fn is_empty(&self) -> bool {
        self.genres.is_empty()
    }

    pub fn index_of(&self, id: &str) -> Option<usize> {
        self.genres.iter().position(|g| g.id == id)
    }

    pub fn get(&self, i: usize) -> &Genre {
        &self.genres[i.min(self.genres.len() - 1)]
    }

    /// Familias en orden de primera aparición, con los índices de sus géneros.
    pub fn families(&self) -> Vec<(String, Vec<usize>)> {
        let mut out: Vec<(String, Vec<usize>)> = Vec::new();
        for (i, g) in self.genres.iter().enumerate() {
            match out.iter_mut().find(|(f, _)| *f == g.family) {
                Some((_, v)) => v.push(i),
                None => out.push((g.family.clone(), vec![i])),
            }
        }
        out
    }

    /// Valida y construye. Devuelve TODOS los errores encontrados, uno por línea.
    pub fn from_json(src: &str) -> Result<GenreSet, Vec<String>> {
        let root = json::parse(src).map_err(|e| vec![e])?;
        let Some(list) = root.get("genres").and_then(Value::as_arr) else {
            return Err(vec!["falta la lista \"genres\"".to_string()]);
        };
        let mut errors: Vec<String> = Vec::new();
        let mut genres: Vec<Genre> = Vec::new();
        for (gi, gv) in list.iter().enumerate() {
            match parse_genre(gv, gi) {
                Ok(g) => {
                    if genres.iter().any(|x| x.id == g.id) {
                        errors.push(format!("género {} (\"{}\"): id repetido", gi + 1, g.id));
                    }
                    genres.push(g);
                }
                Err(mut e) => errors.append(&mut e),
            }
        }
        if list.is_empty() {
            errors.push("la lista \"genres\" está vacía".to_string());
        }
        if list.len() > MAX_GENRES {
            errors.push(format!("hay {} géneros y el máximo es {MAX_GENRES}", list.len()));
        }
        if errors.is_empty() {
            Ok(GenreSet { genres })
        } else {
            Err(errors)
        }
    }
}

fn parse_genre(v: &Value, gi: usize) -> Result<Genre, Vec<String>> {
    let mut errs: Vec<String> = Vec::new();
    let id = v.get("id").and_then(Value::as_str).unwrap_or("").to_string();
    let tag = if id.is_empty() { format!("género {}", gi + 1) } else { format!("género {} (\"{}\")", gi + 1, id) };
    if id.is_empty() {
        errs.push(format!("{tag}: falta \"id\""));
    }
    let name = v.get("name").and_then(Value::as_str).unwrap_or("").to_string();
    if name.is_empty() {
        errs.push(format!("{tag}: falta \"name\""));
    }
    let family = v.get("family").and_then(Value::as_str).unwrap_or("Otros").to_string();
    let mut int = |key: &str, lo: i64, hi: i64, def: Option<i64>| -> i64 {
        match v.get(key).and_then(Value::as_f64) {
            Some(x) if x.fract() == 0.0 && (lo as f64..=hi as f64).contains(&x) => x as i64,
            Some(x) => {
                errs.push(format!("{tag}: \"{key}\" = {x} fuera de {lo}-{hi} (o no es entero)"));
                lo
            }
            None => match def {
                Some(d) => d,
                None => {
                    errs.push(format!("{tag}: falta \"{key}\""));
                    lo
                }
            },
        }
    };
    let bpm = int("bpm", 60, 210, None);
    let swing = int("swing", 0, 100, Some(0));
    let human = int("human", 0, 100, Some(14));
    let alias_priority = int("aliasPriority", -100_000, 100_000, Some(1000 + gi as i64));
    let accent = match v.get("accent").and_then(Value::as_str) {
        None => Accent::Standard,
        Some(s) => Accent::from_id(s).unwrap_or_else(|| {
            errs.push(format!("{tag}: acento desconocido \"{s}\" (standard, offbeat, backbeat o sync)"));
            Accent::Standard
        }),
    };
    let triplet_push = v.get("tripletPush").and_then(Value::as_bool).unwrap_or(false);
    let proposal = v.get("proposal").and_then(Value::as_bool).unwrap_or(false);

    let mut aliases: Vec<String> = Vec::new();
    if let Some(a) = v.get("aliases") {
        for lang in ["es", "en"] {
            if let Some(list) = a.get(lang).and_then(Value::as_arr) {
                for item in list {
                    match item.as_str() {
                        Some(s) if !s.trim().is_empty() => aliases.push(s.to_string()),
                        _ => errs.push(format!("{tag}: alias \"{lang}\" no es texto")),
                    }
                }
            }
        }
    }

    let mut pieces: [Option<PieceTemplate>; NUM_PIECES] = std::array::from_fn(|_| None);
    match v.get("pieces").and_then(Value::as_obj) {
        None => errs.push(format!("{tag}: falta \"pieces\"")),
        Some(items) => {
            if items.is_empty() {
                errs.push(format!("{tag}: no tiene ninguna pieza"));
            }
            for (pid, pv) in items {
                let Some(pi) = piece_index(pid) else {
                    errs.push(format!("{tag}: pieza desconocida \"{pid}\""));
                    continue;
                };
                if pieces[pi].is_some() {
                    errs.push(format!("{tag}: pieza \"{pid}\" repetida"));
                    continue;
                }
                let vel = match pv.get("v").and_then(Value::as_f64) {
                    Some(x) if x.fract() == 0.0 && (1.0..=127.0).contains(&x) => x as i32,
                    _ => {
                        errs.push(format!("{tag}: pieza \"{pid}\": \"v\" debe ser un entero de 1 a 127"));
                        100
                    }
                };
                let Some(row) = pv.get("p").and_then(Value::as_arr) else {
                    errs.push(format!("{tag}: pieza \"{pid}\": falta la fila \"p\""));
                    continue;
                };
                if row.len() != 16 {
                    errs.push(format!("{tag}: pieza \"{pid}\": la fila mide {} y debe medir 16", row.len()));
                    continue;
                }
                let mut p = [0u8; 16];
                let mut ok = true;
                for (s, cell) in row.iter().enumerate() {
                    match cell.as_f64() {
                        Some(x) if x.fract() == 0.0 && (0.0..=100.0).contains(&x) => p[s] = x as u8,
                        _ => {
                            errs.push(format!("{tag}: pieza \"{pid}\", paso {}: la probabilidad debe ser un entero de 0 a 100", s + 1));
                            ok = false;
                        }
                    }
                }
                if ok {
                    pieces[pi] = Some(PieceTemplate { v: vel, p });
                }
            }
        }
    }
    if errs.is_empty() {
        Ok(Genre { family, id, name, proposal, bpm: bpm as i32, swing: swing as i32, human: human as i32, accent, triplet_push, alias_priority, aliases, pieces })
    } else {
        Err(errs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_loads_with_53_genres_in_original_order() {
        let g = GenreSet::builtin();
        assert_eq!(g.len(), 53);
        let first: Vec<&str> = g.genres.iter().take(22).map(|x| x.id.as_str()).collect();
        assert_eq!(
            first,
            ["funk", "neosoul", "rnb", "afrobeat", "afrobeats", "boombap", "gfunk", "trap", "newjack", "house", "reggaeton", "rock", "jazz", "bossa", "samba", "techno", "techhouse", "deephouse", "lofi", "dnb", "synthwave", "dembow"]
        );
        assert!(g.genres.iter().take(22).all(|x| !x.proposal));
        assert!(g.genres.iter().skip(22).all(|x| x.proposal));
        assert!(g.genres.iter().find(|x| x.id == "jazz").unwrap().triplet_push);
    }

    fn one(extra: &str) -> String {
        format!(r#"{{"genres":[{{"id":"x","name":"X","bpm":100,{extra}}}]}}"#)
    }

    #[test]
    fn rejects_unknown_piece_out_of_range_and_bad_rows() {
        let row16 = "0,0,0,0,0,0,0,0,0,0,0,0,0,0,0,0";
        assert!(GenreSet::from_json(&one(&format!(r#""pieces":{{"kick":{{"v":100,"p":[{row16}]}}}}"#))).is_ok());
        let e = GenreSet::from_json(&one(&format!(r#""pieces":{{"bombo":{{"v":100,"p":[{row16}]}}}}"#))).unwrap_err();
        assert!(e.iter().any(|m| m.contains("pieza desconocida")));
        let e = GenreSet::from_json(&one(r#""pieces":{"kick":{"v":100,"p":[0,0,0]}}"#)).unwrap_err();
        assert!(e.iter().any(|m| m.contains("debe medir 16")));
        let e = GenreSet::from_json(&one(r#""pieces":{"kick":{"v":100,"p":[101,0,0,0,0,0,0,0,0,0,0,0,0,0,0,-1]}}"#)).unwrap_err();
        assert_eq!(e.iter().filter(|m| m.contains("0 a 100")).count(), 2);
    }

    #[test]
    fn collects_all_errors_and_rejects_duplicates_and_garbage() {
        let src = r#"{"genres":[{"id":"a","name":"A","bpm":999,"pieces":{}},{"id":"a","name":"B","bpm":90,"accent":"zzz","pieces":{"kick":{"v":0,"p":[]}}}]}"#;
        let e = GenreSet::from_json(src).unwrap_err();
        assert!(e.len() >= 5, "{e:?}");
        assert!(GenreSet::from_json("{").is_err());
        assert!(GenreSet::from_json(r#"{"genres":[]}"#).is_err());
    }

    #[test]
    fn families_group_in_order() {
        let g = GenreSet::builtin();
        let f = g.families();
        assert!(f.iter().any(|(n, _)| n == "Soul y R&B"));
        let total: usize = f.iter().map(|(_, v)| v.len()).sum();
        assert_eq!(total, 53);
    }
}
