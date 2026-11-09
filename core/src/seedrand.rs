//! Aleatoriedad determinista de la drum machine (réplica de la lógica Dice / semilla del Grid de Bitwig).
//!
//! Regla de fondo: **nada de `sin`, `fract` ni funciones transcendentes en coma flotante**.
//! Todo sale de un hash entero (splitmix64), así que el resultado es idéntico en x86_64, aarch64
//! y en cualquier sistema. Un "valor unitario" es un entero `u24` en `0..2^24`; se convierte a
//! `f32` dividiendo por una potencia de dos (exacto).

/// Un número en [0, 1) con 24 bits de resolución (`0..UNIT_MAX`).
pub type U24 = u32;
pub const UNIT_ONE: u32 = 1 << 24;

/// Desfases por parámetro: cada parámetro del pad usa uno distinto para no correlacionarse.
pub const OFFSET_SAMPLE: u32 = 1;
pub const OFFSET_PITCH: u32 = 2;
pub const OFFSET_DECAY: u32 = 3;
pub const OFFSET_FILTER: u32 = 4;
pub const PARAM_OFFSETS: [u32; 4] = [OFFSET_SAMPLE, OFFSET_PITCH, OFFSET_DECAY, OFFSET_FILTER];

/// splitmix64 (Steele, Lea y Flood): una pasada de mezcla sobre `z` (ya incrementado).
pub fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Hash de `(semilla del pad, desfase del parámetro)`.
pub fn hash_seed(seed: u32, offset: u32) -> u64 {
    let a = splitmix64(u64::from(seed) ^ 0xCA7A_0000_0000_0000);
    splitmix64(a ^ (u64::from(offset) << 32 | u64::from(offset)))
}

/// Los 24 bits altos del hash como valor unitario.
pub fn unit_from_hash(h: u64) -> U24 {
    (h >> 40) as u32
}

/// Valor unitario determinista de `(semilla, parámetro)`.
pub fn seed_unit(seed: u32, offset: u32) -> U24 {
    unit_from_hash(hash_seed(seed, offset))
}

/// `u24 -> f32` en [0, 1) (exacto: 24 bits caben en la mantisa).
pub fn unit_f32(u: U24) -> f32 {
    u as f32 / UNIT_ONE as f32
}

/// `floor(u * n)` solo con enteros. `n == 0` -> `None`.
pub fn index_from_unit(u: U24, n: usize) -> Option<usize> {
    if n == 0 {
        return None;
    }
    let i = ((u64::from(u) * n as u64) >> 24) as usize;
    Some(i.min(n - 1))
}

/// Generador del modo ROLL (el "Dice"): un flujo splitmix64.
#[derive(Clone, Debug)]
pub struct RollRng {
    state: u64,
}

impl RollRng {
    pub fn new(seed: u64) -> Self {
        RollRng { state: seed }
    }
    pub fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    pub fn next_unit(&mut self) -> U24 {
        unit_from_hash(self.next_u64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix64_reference_vector() {
        // Primera salida de SplitMix64 con estado inicial 0 (vector de referencia publicado).
        assert_eq!(splitmix64(0), 0xE220_A839_7B1D_CDAF);
        let mut r = RollRng::new(0);
        assert_eq!(r.next_u64(), 0xE220_A839_7B1D_CDAF);
    }

    /// Vectores fijos (golden): solo aritmética entera, no pueden cambiar entre x86_64 y aarch64.
    #[test]
    fn golden_vectors_are_frozen() {
        let got: Vec<u64> = [(0u32, 1u32), (0, 2), (1, 1), (12345, 1), (12345, 2), (12345, 3), (12345, 4), (99999, 1)]
            .iter()
            .map(|&(s, o)| hash_seed(s, o))
            .collect();
        assert_eq!(got, GOLDEN_HASHES, "{:#x?}", got);
        assert_eq!(seed_unit(12345, OFFSET_SAMPLE), unit_from_hash(GOLDEN_HASHES[3]));
    }

    const GOLDEN_HASHES: [u64; 8] = [
        0xd8d5_52d3_12b7_37d2,
        0x4ac4_da42_fc66_1103,
        0x814f_1ad0_8fc2_0745,
        0xf842_4617_e214_3d83,
        0xee64_a488_3080_836f,
        0x229a_9f6a_b45d_76f4,
        0x5ce3_426f_d5dc_ad2c,
        0xc052_b74c_1e87_9365,
    ];

    #[test]
    fn params_of_one_pad_are_not_correlated() {
        let us: Vec<U24> = PARAM_OFFSETS.iter().map(|&o| seed_unit(777, o)).collect();
        for i in 0..4 {
            for j in i + 1..4 {
                assert_ne!(us[i], us[j]);
            }
        }
    }

    #[test]
    fn index_is_stable_and_adapts_to_n() {
        // floor(u * N): con N distinto el mismo u da un índice proporcional, siempre < N.
        for seed in [0u32, 1, 2, 12345, 99999] {
            let u = seed_unit(seed, OFFSET_SAMPLE);
            for n in [1usize, 2, 7, 8, 32, 100, 128] {
                let i = index_from_unit(u, n).unwrap();
                assert!(i < n);
                assert_eq!(i, index_from_unit(u, n).unwrap());
            }
            assert_eq!(index_from_unit(u, 0), None);
            // proporcional: duplicar N duplica (o casi) el índice
            let a = index_from_unit(u, 32).unwrap();
            let b = index_from_unit(u, 64).unwrap();
            assert_eq!(b / 2, a);
        }
        assert_eq!(index_from_unit(0, 10), Some(0));
        assert_eq!(index_from_unit(UNIT_ONE - 1, 10), Some(9));
        assert_eq!(index_from_unit(UNIT_ONE / 2, 10), Some(5));
    }

    #[test]
    fn units_are_in_range_and_roughly_uniform() {
        let mut r = RollRng::new(42);
        let mut buckets = [0u32; 8];
        for _ in 0..8000 {
            let u = r.next_unit();
            assert!(u < UNIT_ONE);
            buckets[(u >> 21) as usize] += 1;
        }
        for b in buckets {
            assert!((800..1200).contains(&b), "{buckets:?}");
        }
    }
}
