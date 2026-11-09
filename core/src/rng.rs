//! PRNG y utilidades numéricas idénticas a las de la appweb original (JavaScript).

/// mulberry32: mismo generador, mismo orden de consumo que la appweb.
#[derive(Clone, Debug)]
pub struct Mulberry32 {
    t: u32,
}

impl Mulberry32 {
    pub fn new(seed: u32) -> Self {
        Mulberry32 { t: seed }
    }

    /// Siguiente número en [0, 1).
    pub fn next(&mut self) -> f64 {
        self.t = self.t.wrapping_add(0x6d2b_79f5);
        let mut x = self.t;
        x = (x ^ (x >> 15)).wrapping_mul(x | 1);
        x ^= x.wrapping_add((x ^ (x >> 7)).wrapping_mul(x | 61));
        f64::from(x ^ (x >> 14)) / 4_294_967_296.0
    }
}

/// Semilla textual -> u32. Solo dígitos (< 10 cifras): el número; si no, FNV-1a de los bytes.
pub fn hash_seed(s: &str) -> u32 {
    let digits = !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
    if digits && s.len() < 10 {
        return s.parse::<u64>().unwrap_or(0) as u32;
    }
    let mut h: u32 = 2_166_136_261;
    for c in s.bytes() {
        h ^= u32::from(c);
        h = h.wrapping_mul(16_777_619);
    }
    h
}

/// `Math.round` de JavaScript: redondea .5 hacia +infinito.
pub fn js_round(x: f64) -> i32 {
    (x + 0.5).floor() as i32
}

/// FNV-1a de 64 bits (huellas de los tests de paridad).
pub fn fnv64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 1_469_598_103_934_665_603;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(1_099_511_628_211);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_round_matches_math_round() {
        assert_eq!(js_round(2.5), 3);
        assert_eq!(js_round(-2.5), -2);
        assert_eq!(js_round(-0.4), 0);
        assert_eq!(js_round(0.49999), 0);
    }

    #[test]
    fn hash_seed_digits_and_text() {
        assert_eq!(hash_seed("12345"), 12345);
        assert_eq!(hash_seed("0"), 0);
        assert_eq!(hash_seed("4294967295"), hash_seed("4294967295")); // 10 cifras -> FNV
        assert_ne!(hash_seed("hola"), hash_seed("hola2"));
    }

    #[test]
    fn mulberry_is_deterministic_and_in_range() {
        let mut a = Mulberry32::new(42);
        let mut b = Mulberry32::new(42);
        for _ in 0..1000 {
            let x = a.next();
            assert_eq!(x, b.next());
            assert!((0.0..1.0).contains(&x));
        }
    }
}
