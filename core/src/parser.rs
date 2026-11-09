//! Frase -> especificación. Port fiel del parser C++ (que a su vez replica la appweb), sin regex:
//! el texto se normaliza a [a-z0-9 ] con espacios simples, así que los patrones se reducen a
//! subcadenas, palabras completas y pares de palabras.

use crate::genres::GenreSet;
use crate::rng::js_round;

pub const MOD_GHOST: u32 = 1 << 0;
pub const MOD_HALFTIME: u32 = 1 << 1;
pub const MOD_SHUFFLE: u32 = 1 << 2;
pub const MOD_SINCOPADO: u32 = 1 << 3;
pub const MOD_FILL: u32 = 1 << 4;
pub const MOD_VERSO: u32 = 1 << 5;
pub const MOD_CORO: u32 = 1 << 6;
pub const MOD_SEISAVOS: u32 = 1 << 7;
pub const MOD_OCHOS: u32 = 1 << 8;

/// "ghost notes · half-time ..."
pub fn mods_to_text(m: u32) -> String {
    const T: [(u32, &str); 9] = [
        (MOD_GHOST, "ghost notes"),
        (MOD_HALFTIME, "half-time"),
        (MOD_SHUFFLE, "shuffle"),
        (MOD_SINCOPADO, "sincopado"),
        (MOD_FILL, "fill"),
        (MOD_VERSO, "verso"),
        (MOD_CORO, "coro"),
        (MOD_SEISAVOS, "semicorcheas"),
        (MOD_OCHOS, "corcheas"),
    ];
    let mut out = String::new();
    for (bit, txt) in T {
        if m & bit != 0 {
            if !out.is_empty() {
                out.push_str(" · ");
            }
            out.push_str(txt);
        }
    }
    out
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParsedPhrase {
    pub genre: usize,
    pub bpm: i32,
    pub swing: i32,
    /// -1 = la frase no dice nada.
    pub density: i32,
    pub bars: i32,
    pub human: i32,
    pub mods: u32,
    pub warning: String,
}

fn latin_base(cp: u32) -> u8 {
    match cp {
        0xC0..=0xC5 | 0xE0..=0xE5 => b'a',
        0xC7 | 0xE7 => b'c',
        0xC8..=0xCB | 0xE8..=0xEB => b'e',
        0xCC..=0xCF | 0xEC..=0xEF => b'i',
        0xD1 | 0xF1 => b'n',
        0xD2..=0xD6 | 0xF2..=0xF6 => b'o',
        0xD9..=0xDC | 0xF9..=0xFC => b'u',
        0xDD | 0xFD | 0xFF => b'y',
        _ => b' ',
    }
}

fn is_c_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// Minúsculas ASCII, letras latinas acentuadas -> base, "r & b" -> "rnb", todo lo demás -> espacio.
pub fn normalize(input: &str) -> String {
    let inb = input.as_bytes();
    let mut s: Vec<u8> = Vec::with_capacity(inb.len());
    let mut i = 0;
    while i < inb.len() {
        let c = inb[i];
        if c < 0x80 {
            s.push(c.to_ascii_lowercase());
            i += 1;
            continue;
        }
        let (mut cp, n) = if c & 0xE0 == 0xC0 {
            (u32::from(c & 0x1F), 1)
        } else if c & 0xF0 == 0xE0 {
            (u32::from(c & 0x0F), 2)
        } else if c & 0xF8 == 0xF0 {
            (u32::from(c & 0x07), 3)
        } else {
            s.push(b' ');
            i += 1;
            continue;
        };
        if i + n >= inb.len() {
            s.push(b' ');
            break;
        }
        for k in 1..=n {
            cp = (cp << 6) | u32::from(inb[i + k] & 0x3F);
        }
        i += n + 1;
        s.push(latin_base(cp));
    }
    // regex "r\s*&\s*b" -> " rnb " (de izquierda a derecha, sin solaparse)
    let mut t: Vec<u8> = Vec::with_capacity(s.len() + 8);
    let mut i = 0;
    while i < s.len() {
        if s[i] == b'r' {
            let mut j = i + 1;
            while j < s.len() && is_c_space(s[j]) {
                j += 1;
            }
            if j < s.len() && s[j] == b'&' {
                j += 1;
                while j < s.len() && is_c_space(s[j]) {
                    j += 1;
                }
                if j < s.len() && s[j] == b'b' {
                    t.extend_from_slice(b" rnb ");
                    i = j + 1;
                    continue;
                }
            }
        }
        t.push(s[i]);
        i += 1;
    }
    let mut out = String::with_capacity(t.len());
    let mut pending = false;
    for c in t {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            if pending && !out.is_empty() {
                out.push(' ');
            }
            pending = false;
            out.push(c as char);
        } else {
            pending = true;
        }
    }
    out
}

fn has_sub(t: &str, alts: &[&str]) -> bool {
    alts.iter().any(|a| t.contains(a))
}

fn tokens(t: &str) -> Vec<&str> {
    t.split(' ').filter(|x| !x.is_empty()).collect()
}

fn has_word(toks: &[&str], alts: &[&str]) -> bool {
    toks.iter().any(|w| alts.contains(w))
}

fn nearest_bars(n: i64) -> i32 {
    const OPS: [i64; 4] = [1, 2, 4, 8];
    let mut best = OPS[0];
    for cand in OPS {
        if (cand - n).abs() < (best - n).abs() {
            best = cand;
        }
    }
    best as i32
}

fn digits(w: &str) -> bool {
    !w.is_empty() && w.bytes().all(|c| c.is_ascii_digit())
}

fn parse_num(w: &str) -> i64 {
    w.bytes().fold(0i64, |a, c| a.saturating_mul(10).saturating_add(i64::from(c - b'0')))
}

/// `\b(\d{2,3})\s*bpm\b` : primer acierto de izquierda a derecha.
fn find_bpm_tagged(toks: &[&str]) -> Option<i64> {
    for (i, w) in toks.iter().enumerate() {
        if digits(w) && (2..=3).contains(&w.len()) && toks.get(i + 1) == Some(&"bpm") {
            return Some(parse_num(w));
        }
        if let Some(num) = w.strip_suffix("bpm") {
            if digits(num) && (2..=3).contains(&num.len()) {
                return Some(parse_num(num));
            }
        }
    }
    None
}

/// `\b(\d+|un|uno|dos|cuatro|ocho)\s*(compases|compas|bars|bar|comp)\b`
fn find_bars(toks: &[&str]) -> Option<i64> {
    const UNITS: [&str; 5] = ["compases", "compas", "bars", "bar", "comp"];
    const WORDS: [(&str, i64); 5] = [("un", 1), ("uno", 1), ("dos", 2), ("cuatro", 4), ("ocho", 8)];
    for (i, w) in toks.iter().enumerate() {
        // "N compases" en dos palabras
        if UNITS.contains(toks.get(i + 1).unwrap_or(&"")) {
            if digits(w) {
                return Some(parse_num(w));
            }
            if let Some((_, v)) = WORDS.iter().find(|(x, _)| x == w) {
                return Some(*v);
            }
        }
        // pegado: "4compases"
        let lead = w.bytes().take_while(u8::is_ascii_digit).count();
        if lead > 0 && UNITS.contains(&&w[lead..]) {
            return Some(parse_num(&w[..lead]));
        }
        for (word, v) in WORDS {
            if let Some(rest) = w.strip_prefix(word) {
                if UNITS.contains(&rest) {
                    return Some(v);
                }
            }
        }
    }
    None
}

pub fn parse_phrase(phrase: &str, genres: &GenreSet) -> ParsedPhrase {
    let t = normalize(phrase);
    let toks = tokens(&t);
    let mut warning = String::new();

    // Alias más largo; a igual longitud gana el de menor prioridad (y luego el primero).
    let mut found: Option<(usize, usize, i64)> = None; // (género, longitud, prioridad)
    for (gi, g) in genres.genres.iter().enumerate() {
        for name in &g.aliases {
            let n = normalize(name);
            if n.is_empty() || !t.contains(&n) {
                continue;
            }
            let better = match found {
                None => true,
                Some((_, len, prio)) => n.len() > len || (n.len() == len && g.alias_priority < prio),
            };
            if better {
                found = Some((gi, n.len(), g.alias_priority));
            }
        }
    }
    let by_id = |id: &str| genres.index_of(id).unwrap_or(0);
    let genre = if let Some((gi, _, _)) = found {
        gi
    } else if has_word(&toks, &["swing", "shuffle"]) {
        by_id("jazz")
    } else {
        if !t.is_empty() {
            warning = "No reconocí el género, así que armé un funk.".to_string();
        }
        by_id("funk")
    };
    let base = genres.get(genre);

    let mut m = 0u32;
    if has_sub(&t, &["ghost", "fantasma"]) {
        m |= MOD_GHOST;
    }
    if has_sub(&t, &["half time", "halftime", "medio tiempo"]) {
        m |= MOD_HALFTIME;
    }
    if has_word(&toks, &["shuffle"]) || t.contains("arrastre") {
        m |= MOD_SHUFFLE;
    }
    if has_sub(&t, &["sincop", "syncop", "contratiempo"]) {
        m |= MOD_SINCOPADO;
    }
    if has_word(&toks, &["fill"]) || has_sub(&t, &["relleno", "redoble"]) {
        m |= MOD_FILL;
    }
    if has_word(&toks, &["verso", "verse"]) {
        m |= MOD_VERSO;
    }
    if has_word(&toks, &["coro", "chorus"]) || t.contains("estribillo") {
        m |= MOD_CORO;
    }
    if has_word(&toks, &["semicorcheas", "16avos", "16th", "dieciseisavos"]) {
        m |= MOD_SEISAVOS;
    } else if has_word(&toks, &["corcheas", "8vos", "8th"]) {
        m |= MOD_OCHOS;
    }

    let mut density = -1;
    if has_sub(&t, &["minimal", "minimo", "minimalista", "sparse", "escaso"]) {
        density = 8;
    }
    if has_sub(&t, &["busy", "cargado", "denso", "lleno", "complejo"]) {
        density = 92;
    }
    let mut human = -1;
    if has_sub(&t, &["cuantiz", "robotic", "tight", "mecanico"]) {
        human = 0;
    }
    if has_word(&toks, &["humano"]) || has_sub(&t, &["flojo", "loose", "suelto"]) {
        human = 36;
    }

    let mut bpm = 90;
    let mut bpm_set = false;
    if let Some(n) = find_bpm_tagged(&toks) {
        bpm = n.clamp(60, 210) as i32;
        bpm_set = true;
    } else {
        for w in &toks {
            if digits(w) && (2..=3).contains(&w.len()) {
                let n = parse_num(w);
                if (60..=210).contains(&n) {
                    bpm = n as i32;
                    bpm_set = true;
                    break;
                }
            }
        }
    }
    if !bpm_set {
        bpm = base.bpm;
        if has_sub(&t, &["lento", "slow", "downtempo", "despacio"]) {
            bpm = js_round(f64::from(base.bpm) * 0.86).clamp(60, 210);
        }
        if has_sub(&t, &["rapido", "fast", "uptempo", "veloz"]) {
            bpm = js_round(f64::from(base.bpm) * 1.12).clamp(60, 210);
        }
    }

    let swing = if has_sub(&t, &["sin swing", "recto", "straight", "no swing"]) {
        0
    } else if m & MOD_SHUFFLE != 0 {
        66.max(base.swing)
    } else if has_word(&toks, &["swing"]) {
        40.max(base.swing)
    } else {
        base.swing
    };

    let mut bars = -1;
    if let Some(crude) = find_bars(&toks) {
        bars = nearest_bars(crude);
        if crude > 8 {
            if !warning.is_empty() {
                warning.push(' ');
            }
            warning.push_str("Máximo 8 compases.");
        }
    }
    ParsedPhrase { genre, bpm, swing, density, bars, human, mods: m, warning }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_accents_rnb_and_punctuation() {
        assert_eq!(normalize("Ñandú ROCK, estribillo!"), "nandu rock estribillo");
        assert_eq!(normalize("R&B"), "rnb");
        assert_eq!(normalize("  a   b  "), "a b");
        assert_eq!(normalize("reggaetón"), "reggaeton");
        assert_eq!(normalize("日本 funk"), "funk");
    }

    #[test]
    fn bars_and_bpm_forms() {
        let t = |s: &str| tokens(Box::leak(normalize(s).into_boxed_str()).as_ref()).len();
        assert!(t("a b") == 2);
        let n = normalize("cuatro compases 4compases 120bpm");
        let toks = tokens(&n);
        assert_eq!(find_bars(&toks), Some(4));
        assert_eq!(find_bpm_tagged(&toks), Some(120));
        assert_eq!(find_bpm_tagged(&tokens(&normalize("1234 bpm"))), None);
        assert_eq!(find_bars(&tokens(&normalize("99999999999999999999 bars"))), Some(i64::MAX));
        assert_eq!(nearest_bars(3), 2);
        assert_eq!(nearest_bars(6), 4);
        assert_eq!(nearest_bars(0), 1);
    }
}
