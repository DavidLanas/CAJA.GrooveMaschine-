//! Biblioteca de samples: clasificación por nombre y orden estable. Lógica pura (sin E/S ni audio):
//! el escaneo de la carpeta y la decodificación viven en el plugin.
//!
//! El plugin **no incluye ni copia samples**: aquí solo hay rutas relativas (en minúsculas para
//! ordenar, con `/` como separador) respecto de la carpeta que elige el usuario.

use crate::json::{self, Value};
use crate::pieces::{NUM_PIECES, PAD_ORDER, PIECES};

pub const KEYWORDS_JSON: &str = include_str!("../data/sample-keywords.json");
pub const MIN_BANK: usize = 8;
pub const MAX_BANK: usize = 128;
pub const DEFAULT_BANK: usize = 32;

/// Posición de cada pieza en `PAD_ORDER` (el pad k suena con la pieza `PAD_ORDER[k]`).
pub fn pad_of_piece(piece: usize) -> usize {
    PAD_ORDER.iter().position(|&p| p == piece).unwrap_or(0)
}

// ───────────────────────────── normalización ─────────────────────────────

fn strip_accent(c: char) -> char {
    match c {
        'á' | 'à' | 'â' | 'ä' | 'ã' | 'å' | 'Á' | 'À' | 'Â' | 'Ä' | 'Ã' | 'Å' => 'a',
        'é' | 'è' | 'ê' | 'ë' | 'É' | 'È' | 'Ê' | 'Ë' => 'e',
        'í' | 'ì' | 'î' | 'ï' | 'Í' | 'Ì' | 'Î' | 'Ï' => 'i',
        'ó' | 'ò' | 'ô' | 'ö' | 'õ' | 'Ó' | 'Ò' | 'Ô' | 'Ö' | 'Õ' => 'o',
        'ú' | 'ù' | 'û' | 'ü' | 'Ú' | 'Ù' | 'Û' | 'Ü' => 'u',
        'ñ' | 'Ñ' => 'n',
        'ç' | 'Ç' => 'c',
        _ => c,
    }
}

/// Texto -> palabras: sin acentos, minúsculas, separadores (`-`, `_`, espacios, puntos...) y
/// camelCase/dígitos como cortes, plural `-s` quitado ("kicks" -> "kick", "bass" se queda).
pub fn tokens(s: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev: Option<char> = None;
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() {
            out.push(std::mem::take(cur));
        }
    };
    for raw in s.chars() {
        let c = strip_accent(raw);
        if !c.is_alphanumeric() {
            flush(&mut cur, &mut out);
            prev = None;
            continue;
        }
        if let Some(p) = prev {
            let camel = p.is_lowercase() && c.is_uppercase();
            let digit_edge = p.is_ascii_digit() != c.is_ascii_digit();
            if camel || digit_edge {
                flush(&mut cur, &mut out);
            }
        }
        cur.extend(c.to_lowercase());
        prev = Some(c);
    }
    flush(&mut cur, &mut out);
    for t in &mut out {
        if t.chars().count() > 3 && t.ends_with('s') && !t.ends_with("ss") {
            t.pop();
        }
    }
    out
}

// ───────────────────────────── tabla de palabras clave ─────────────────────────────

#[derive(Clone, Debug, PartialEq)]
struct Phrase {
    toks: Vec<String>,
    strong: bool,
    piece: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeywordTable {
    phrases: Vec<Phrase>,
    /// El JSON original (se guarda en el estado para que el proyecto reabra igual).
    source: String,
}

impl KeywordTable {
    pub fn builtin() -> KeywordTable {
        KeywordTable::from_json(KEYWORDS_JSON).expect("sample-keywords.json integrado es válido")
    }

    pub fn from_json(src: &str) -> Result<KeywordTable, String> {
        let v = json::parse(src)?;
        let pieces = v.get("pieces").and_then(Value::as_obj).ok_or("falta \"pieces\"")?;
        let mut phrases = Vec::new();
        for (id, entry) in pieces {
            let piece = PIECES.iter().position(|p| p.id == id).ok_or_else(|| format!("pieza desconocida \"{id}\""))?;
            for (key, strong) in [("strong", true), ("weak", false)] {
                let Some(list) = entry.get(key) else { continue };
                let list = list.as_arr().ok_or_else(|| format!("\"{id}.{key}\" debe ser una lista"))?;
                for w in list {
                    let w = w.as_str().ok_or_else(|| format!("\"{id}.{key}\": solo texto"))?;
                    let toks = tokens(w);
                    if !toks.is_empty() {
                        phrases.push(Phrase { toks, strong, piece });
                    }
                }
            }
        }
        if phrases.is_empty() {
            return Err("la tabla no tiene ninguna palabra clave".into());
        }
        Ok(KeywordTable { phrases, source: src.to_string() })
    }

    pub fn source(&self) -> &str {
        &self.source
    }

    /// Pieza (índice de `PIECES`) de una ruta relativa, o `None` si no clasifica.
    ///
    /// Se mira el nombre del archivo y los de sus carpetas. Gana la coincidencia más específica:
    /// 1) las palabras claras antes que las débiles, 2) más palabras ("open hat" antes que "hat"),
    /// 3) la del nombre del archivo antes que la de una carpeta, 4) más letras, 5) la pieza de
    /// menor índice (así el resultado nunca depende del orden de la tabla).
    pub fn classify(&self, rel_path: &str) -> Option<usize> {
        let comps: Vec<&str> = rel_path.split(['/', '\\']).filter(|c| !c.is_empty()).collect();
        let (file, folders) = comps.split_last()?;
        let stem = file.rsplit_once('.').map_or(*file, |(a, _)| a);
        let mut stream: Vec<(String, bool)> = Vec::new();
        for f in folders {
            stream.extend(tokens(f).into_iter().map(|t| (t, false)));
        }
        stream.extend(tokens(stem).into_iter().map(|t| (t, true)));

        let mut best: Option<((bool, usize, bool, usize), usize)> = None;
        for ph in &self.phrases {
            let n = ph.toks.len();
            if n > stream.len() {
                continue;
            }
            for start in 0..=stream.len() - n {
                if (0..n).all(|k| stream[start + k].0 == ph.toks[k]) {
                    let chars: usize = ph.toks.iter().map(|t| t.chars().count()).sum();
                    let from_file = (0..n).any(|k| stream[start + k].1);
                    let key = (ph.strong, n, from_file, chars);
                    let better = match best {
                        None => true,
                        Some((bk, bp)) => key > bk || (key == bk && ph.piece < bp),
                    };
                    if better {
                        best = Some((key, ph.piece));
                    }
                }
            }
        }
        best.map(|(_, p)| p)
    }
}

// ───────────────────────────── biblioteca ─────────────────────────────

/// Orden estable: ruta relativa en minúsculas (orden Unicode), con la ruta original como desempate.
pub fn sort_key(rel: &str) -> (String, &str) {
    (rel.to_lowercase(), rel)
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Library {
    /// Carpeta raíz (solo la ruta; nunca copiamos nada de ella).
    pub root: String,
    /// Una celda por pad (`PAD_ORDER`): rutas relativas ordenadas de forma estable.
    pub cells: Vec<Vec<String>>,
    /// Bandeja "Sin clasificar".
    pub unclassified: Vec<String>,
}

/// `rel` normalizada: separador `/`, sin `./` inicial.
pub fn norm_rel(rel: &str) -> String {
    rel.replace('\\', "/").trim_start_matches("./").trim_start_matches('/').to_string()
}

impl Library {
    pub fn empty() -> Library {
        Library { root: String::new(), cells: vec![Vec::new(); NUM_PIECES], unclassified: Vec::new() }
    }

    /// Clasifica `files` (rutas relativas). `assign` = asignaciones manuales (`ruta -> pad`), que
    /// mandan sobre la clasificación automática.
    pub fn build(root: &str, files: &[String], table: &KeywordTable, assign: &[(String, usize)]) -> Library {
        let mut lib = Library { root: root.to_string(), cells: vec![Vec::new(); NUM_PIECES], unclassified: Vec::new() };
        for f in files {
            let rel = norm_rel(f);
            let manual = assign.iter().find(|(r, _)| *r == rel).map(|(_, p)| *p).filter(|p| *p < NUM_PIECES);
            match manual.or_else(|| table.classify(&rel).map(pad_of_piece)) {
                Some(pad) => lib.cells[pad].push(rel),
                None => lib.unclassified.push(rel),
            }
        }
        for c in lib.cells.iter_mut().chain(std::iter::once(&mut lib.unclassified)) {
            c.sort_by(|a, b| sort_key(a).cmp(&sort_key(b)));
            c.dedup();
        }
        lib
    }

    /// El banco de un pad: los primeros `cap` (8..=128) de la celda, en el orden estable.
    pub fn bank(&self, pad: usize, cap: usize) -> &[String] {
        let cap = cap.clamp(MIN_BANK, MAX_BANK);
        let c = self.cells.get(pad).map_or(&[][..], |v| v.as_slice());
        &c[..c.len().min(cap)]
    }

    /// Posición de `rel` dentro del banco del pad (para resolver un pin).
    pub fn find_in_bank(&self, pad: usize, cap: usize, rel: &str) -> Option<usize> {
        let rel = norm_rel(rel);
        self.bank(pad, cap).iter().position(|r| *r == rel)
    }

    pub fn total_classified(&self) -> usize {
        self.cells.iter().map(Vec::len).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pieces::*;

    fn t() -> KeywordTable {
        KeywordTable::builtin()
    }

    #[test]
    fn tokens_normalize_separators_camel_accents_and_plurals() {
        assert_eq!(tokens("Kick_Heavy-01"), ["kick", "heavy", "01"]);
        assert_eq!(tokens("OpenHat"), ["open", "hat"]);
        assert_eq!(tokens("hihatOpen2"), ["hihat", "open", "2"]);
        assert_eq!(tokens("Güiro Corto"), ["guiro", "corto"]);
        assert_eq!(tokens("Cañón"), ["canon"]);
        assert_eq!(tokens("Claps"), ["clap"]);
        assert_eq!(tokens("bass"), ["bass"]);
    }

    /// 50 nombres realistas (inglés y español) con la pieza esperada, incluidos los desempates.
    #[test]
    fn classification_table() {
        let cases: &[(&str, Option<usize>)] = &[
            ("Kick_01.wav", Some(KICK)),
            ("drums/Bombo grave 3.wav", Some(KICK)),
            ("BD_Deep.flac", Some(KICK)),
            ("808 Bass Drum.wav", Some(KICK)),
            ("Snare_Tight_04.wav", Some(SNARE)),
            ("Caja/Caja clara 02.wav", Some(SNARE)),
            ("SD-909.wav", Some(SNARE)),
            ("closed_hat_03.wav", Some(CH)),
            ("HH Closed 05.wav", Some(CH)),
            ("CHH_vintage.wav", Some(CH)),
            ("hat cerrado 2 tight.wav", Some(CH2)),
            ("Hi-Hat Cerrado.wav", Some(CH)),
            ("open_hat_01.wav", Some(OH)),
            ("OpenHat.wav", Some(OH)),
            ("OHH 12.wav", Some(OH)),
            ("hihat_abierto.ogg", Some(OH)),
            ("Hat Abierto 2.wav", Some(OH2)),
            ("hat.wav", Some(CH)),
            ("Clap_Wide.wav", Some(CLAP)),
            ("Palmas_01.wav", Some(CLAP)),
            ("handclap 808.wav", Some(CLAP)),
            ("Rimshot_01.wav", Some(RIM)),
            ("Rim Shot dry.wav", Some(RIM)),
            ("sidestick.wav", Some(RIM)),
            ("Ride_Bell_01.wav", Some(RIDE)),
            ("Ride Cymbal Long.wav", Some(RIDE)),
            ("Shaker_Loose.wav", Some(SHAKER)),
            ("maracas 02.wav", Some(SHAKER)),
            ("Tom_High.wav", Some(TOM1)),
            ("Tom 2.wav", Some(TOM2)),
            ("floor tom.wav", Some(TOM3)),
            ("Tom Bajo grave.wav", Some(TOM3)),
            ("Crash_01.wav", Some(CRASH)),
            ("Cymbal_Splash.wav", Some(CRASH)),
            ("Cowbell_808.wav", Some(COWBELL)),
            ("Cencerro agudo.wav", Some(COWBELL)),
            ("Conga Alta 1.wav", Some(CONGA_H)),
            ("conga_high_slap.wav", Some(CONGA_H)),
            ("Conga Baja.wav", Some(CONGA_L)),
            ("conga low.wav", Some(CONGA_L)),
            ("Clave_01.wav", Some(CLAVE)),
            ("claves madera.wav", Some(CLAVE)),
            ("Timbal Alto.wav", Some(TIMBAL_H)),
            ("timbales bajo.wav", Some(TIMBAL_L)),
            ("Güiro corto.wav", Some(GUIRO)),
            ("guiro_largo.wav", Some(GUIRO)),
            ("Pandereta_01.wav", Some(PANDERETA)),
            ("Tambourine 8th.wav", Some(PANDERETA)),
            ("Egg Shaker.wav", Some(EGG)),
            ("vocal_chop_01.wav", None),
            ("FX/riser.wav", None),
        ];
        let mut bad = Vec::new();
        for (name, want) in cases {
            let got = t().classify(name);
            if got != *want {
                bad.push(format!("{name}: esperado {want:?}, salió {got:?}"));
            }
        }
        assert!(cases.len() >= 40);
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    #[test]
    fn tie_break_rules() {
        let t = t();
        // "open hat" (2 palabras) gana a "hat" (1) y a "open" solo.
        assert_eq!(t.classify("open hat.wav"), Some(OH));
        // "closed" gana a "hat" suelto de la carpeta.
        assert_eq!(t.classify("Hats/closed_01.wav"), Some(CH));
        // carpeta con la pieza y archivo sin pista: sirve la carpeta.
        assert_eq!(t.classify("Kicks/Heavy_01.wav"), Some(KICK));
        assert_eq!(t.classify("Hats/Open/x_01.wav"), Some(OH));
        // empate de especificidad: el nombre del archivo gana a la carpeta.
        assert_eq!(t.classify("Snare/Kick_03.wav"), Some(KICK));
        assert_eq!(t.classify("Kick/Snare_03.wav"), Some(SNARE));
        // las palabras claras ganan a las débiles aunque la débil esté en el archivo.
        assert_eq!(t.classify("Kicks/open_01.wav"), Some(KICK));
    }

    #[test]
    fn classify_does_not_depend_on_case_or_separator() {
        let t = t();
        for n in ["KICK-HEAVY.WAV", "kick_heavy.wav", "KickHeavy.wav", "kick heavy.wav", "Kick.Heavy.wav"] {
            assert_eq!(t.classify(n), Some(KICK), "{n}");
        }
        assert_eq!(t.classify("a\\b\\Snare.wav"), Some(SNARE));
    }

    #[test]
    fn user_table_overrides_and_bad_tables_are_rejected() {
        let custom = r#"{"version":1,"pieces":{"kick":{"strong":["thump"]},"snare":{"strong":["crack"]}}}"#;
        let t = KeywordTable::from_json(custom).unwrap();
        assert_eq!(t.classify("thump_01.wav"), Some(KICK));
        assert_eq!(t.classify("Kick_01.wav"), None);
        assert!(KeywordTable::from_json(r#"{"pieces":{"nope":{"strong":["x"]}}}"#).is_err());
        assert!(KeywordTable::from_json("{").is_err());
        assert!(KeywordTable::from_json(r#"{"pieces":{}}"#).is_err());
    }

    #[test]
    fn cells_follow_pad_order_and_banks_are_stable() {
        let files: Vec<String> = ["b/Kick_02.wav", "A/kick_01.wav", "Kick_10.wav", "Snare_1.wav", "x.wav", "Kick_03.wav"].iter().map(|s| s.to_string()).collect();
        let lib = Library::build("/s", &files, &t(), &[]);
        assert_eq!(lib.cells[pad_of_piece(KICK)], ["A/kick_01.wav", "b/Kick_02.wav", "Kick_03.wav", "Kick_10.wav"]);
        assert_eq!(lib.cells[pad_of_piece(SNARE)].len(), 1);
        assert_eq!(lib.unclassified, ["x.wav"]);
        // el orden no depende del orden de entrada
        let mut rev = files.clone();
        rev.reverse();
        assert_eq!(Library::build("/s", &rev, &t(), &[]), lib);
        // el tope recorta con una regla estable: los primeros N en ese orden (mínimo 8)
        let many: Vec<String> = (0..50).map(|i| format!("Kick_{i:03}.wav")).collect();
        let big = Library::build("/s", &many, &t(), &[]);
        assert_eq!(big.bank(0, 8).len(), 8);
        assert_eq!(big.bank(0, 3).len(), 8); // mínimo 8
        assert_eq!(big.bank(0, 32).len(), 32);
        assert_eq!(big.bank(0, 128).len(), 50);
        assert_eq!(big.bank(0, 8)[0], "Kick_000.wav");
    }

    #[test]
    fn manual_assignment_beats_automatic() {
        let files = vec!["weird_name.wav".to_string(), "Kick_1.wav".to_string()];
        let lib = Library::build("/s", &files, &t(), &[("weird_name.wav".into(), pad_of_piece(CLAP)), ("Kick_1.wav".into(), pad_of_piece(SNARE))]);
        assert_eq!(lib.cells[pad_of_piece(CLAP)], ["weird_name.wav"]);
        assert_eq!(lib.cells[pad_of_piece(SNARE)], ["Kick_1.wav"]);
        assert!(lib.unclassified.is_empty());
    }
}
