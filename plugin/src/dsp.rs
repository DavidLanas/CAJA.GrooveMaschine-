//! Motor de audio de la drum machine: voces por pad con interpolación cúbica, envolvente de
//! decay, filtro pasabajos (SVF TPT), ganancia, pan, mute/solo, polifonía 1–4 y choke groups.
//!
//! `process` solo llama a `begin_block`, `trigger` y `render`: ninguna de las tres reserva memoria,
//! toma locks, decodifica ni toca archivos (las muestras llegan listas en `SampleStore`).

use crate::sample::Sample;
use crate::store::{KitAudio, SampleStore};
use caja_core::kit::{is_lock, resolve_hit, resolve_hit_hold, PadLive, PadRuntime, SamplePick, NUM_PADS, P_SAMPLE, SLOTS};
use caja_core::library::MAX_BANK;
use caja_core::seedrand::RollRng;
use std::sync::atomic::Ordering;
use std::sync::Arc;

/// Voces físicas por pad: hasta 4 sonando + una más para el fundido de la que se roba.
pub const VOICES_PER_PAD: usize = 5;
const FADE_MS: f32 = 4.0;

// ───────────────────────────── conversiones de valor 0..1 ─────────────────────────────

/// 0..1 -> semitonos (-24..+24, 0.5 = sin cambio).
pub fn pitch_semitones(v: f32) -> f32 {
    (v * 2.0 - 1.0) * 24.0
}

/// 0..1 -> decay en segundos (10 ms .. 6 s, escala exponencial). `None` = sin recorte (v ≥ 0.999).
pub fn decay_seconds(v: f32) -> Option<f32> {
    (v < 0.999).then(|| 0.01 * 600.0f32.powf(v))
}

/// 0..1 -> frecuencia de corte en Hz (100 Hz .. 20 kHz). `None` = filtro abierto (v ≥ 0.999).
pub fn cutoff_hz(v: f32) -> Option<f32> {
    (v < 0.999).then(|| 100.0 * 200.0f32.powf(v))
}

// ───────────────────────────── voz ─────────────────────────────

#[derive(Clone)]
struct Voice {
    active: bool,
    fading: bool,
    sample: Option<Arc<Sample>>,
    pos: f64,
    rate: f64,
    env: f32,
    env_mul: f32,
    gl: f32,
    gr: f32,
    lp: bool,
    a1: f32,
    a2: f32,
    a3: f32,
    ic1: [f32; 2],
    ic2: [f32; 2],
    age: u64,
}

impl Voice {
    const fn idle() -> Voice {
        Voice { active: false, fading: false, sample: None, pos: 0.0, rate: 1.0, env: 1.0, env_mul: 1.0, gl: 0.0, gr: 0.0, lp: false, a1: 0.0, a2: 0.0, a3: 0.0, ic1: [0.0; 2], ic2: [0.0; 2], age: 0 }
    }
}

#[inline]
fn cubic(y0: f32, y1: f32, y2: f32, y3: f32, t: f32) -> f32 {
    // Catmull-Rom
    let c1 = 0.5 * (y2 - y0);
    let c2 = y0 - 2.5 * y1 + 2.0 * y2 - 0.5 * y3;
    let c3 = 0.5 * (y3 - y0) + 1.5 * (y1 - y2);
    ((c3 * t + c2) * t + c1) * t + y1
}

#[inline]
fn tap(v: &[f32], i: usize, t: f32) -> f32 {
    let n = v.len();
    if i >= 1 && i + 2 < n {
        cubic(v[i - 1], v[i], v[i + 1], v[i + 2], t)
    } else {
        let g = |j: isize| if j < 0 || j as usize >= n { 0.0 } else { v[j as usize] };
        let i = i as isize;
        cubic(g(i - 1), g(i), g(i + 1), g(i + 2), t)
    }
}

/// Un paso del SVF TPT pasabajos (Zavalishin); Q ≈ 0.707.
#[inline]
fn svf(ic1: &mut f32, ic2: &mut f32, x: f32, a1: f32, a2: f32, a3: f32) -> f32 {
    let v3 = x - *ic2;
    let v1 = a1 * *ic1 + a2 * v3;
    let v2 = *ic2 + a2 * *ic1 + a3 * v3;
    *ic1 = 2.0 * v1 - *ic1;
    *ic2 = 2.0 * v2 - *ic2;
    v2
}

pub struct Engine {
    voices: Vec<Voice>,
    sr: f32,
    clock: u64,
    rt: [PadRuntime; NUM_PADS],
    /// Valores efectivos por pad (semilla activa, slot, modos).
    eff: [PadLive; NUM_PADS],
    prev_param: [Option<(u32, u8)>; NUM_PADS],
    last: [Option<Arc<Sample>>; NUM_PADS],
    pub roll: RollRng,
    fade_mul: f32,
    pub pin_missing_hits: u32,
    mute: [bool; NUM_PADS],
    pan: [f32; NUM_PADS],
    /// Ganancia lineal por pad (1.0 = 0 dB), desde parámetro del host.
    gain: [f32; NUM_PADS],
    /// Ritmo ROLL por pad: 0 = cada golpe, 1/2/3 = cada 1/2/4 compases.
    rate: [u8; NUM_PADS],
    /// Último compás (floor(beats/4)) en que se re-sorteó cada pad.
    last_roll_bar: [i64; NUM_PADS],
}

impl Engine {
    pub fn new(sr: f32) -> Engine {
        let mut e = Engine {
            voices: vec![Voice::idle(); NUM_PADS * VOICES_PER_PAD],
            sr,
            clock: 0,
            rt: std::array::from_fn(|i| PadRuntime::new(PadLive::default_for(i).seed)),
            eff: std::array::from_fn(PadLive::default_for),
            prev_param: [None; NUM_PADS],
            last: std::array::from_fn(|_| None),
            roll: RollRng::new(0x5EED_CA7A),
            fade_mul: 1.0,
            pin_missing_hits: 0,
            mute: [false; NUM_PADS],
            pan: [0.0; NUM_PADS],
            gain: [1.0; NUM_PADS],
            rate: [0; NUM_PADS],
            last_roll_bar: [i64::MIN; NUM_PADS],
        };
        e.set_sample_rate(sr);
        e
    }

    pub fn set_sample_rate(&mut self, sr: f32) {
        self.sr = sr.max(8000.0);
        self.fade_mul = (-6.9078 / (FADE_MS * 0.001 * self.sr)).exp();
    }

    /// Corta todo (reinicio del transporte, desactivar).
    pub fn reset(&mut self) {
        for v in &mut self.voices {
            v.active = false;
            v.sample = None;
        }
        self.last = std::array::from_fn(|_| None);
    }

    /// Reinicia el dado de ROLL (con "ROLL reproducible" al arrancar el transporte).
    pub fn reseed_roll(&mut self, seed: u64) {
        self.roll = RollRng::new(seed);
    }

    /// Una vez por bloque: recoge los parámetros del host. La semilla efectiva sale de `padseed`
    /// si cambió ese parámetro, o del banco si cambió `padslot`.
    pub fn begin_block(&mut self, kit: &KitAudio, host: &[(u32, u8, u8); NUM_PADS]) {
        for (pad, &(seed, slot, mode)) in host.iter().enumerate() {
            let slot = slot.min(SLOTS as u8 - 1);
            match self.prev_param[pad] {
                None => self.eff[pad].seed = seed,
                Some((ps, pslot)) => {
                    if seed != ps {
                        self.eff[pad].seed = seed;
                    } else if slot != pslot {
                        self.eff[pad].seed = kit.pads[pad].bank[usize::from(slot)];
                    }
                }
            }
            self.prev_param[pad] = Some((seed, slot));
            self.eff[pad].slot = slot;
            self.eff[pad].mode = mode & 0b1111;
        }
    }

    #[allow(dead_code)]
    /// Mute, paneo, ganancia y ritmo ROLL por pad (parámetros del host, asignables a macros). Una vez por bloque.
    pub fn set_mixer(
        &mut self,
        mute: &[bool; NUM_PADS],
        pan: &[f32; NUM_PADS],
        gain: &[f32; NUM_PADS],
        rate: &[u8; NUM_PADS],
    ) {
        self.mute = *mute;
        self.pan = *pan;
        self.gain = *gain;
        self.rate = *rate;
    }

    #[allow(dead_code)]
    pub fn effective_seed(&self, pad: usize) -> u32 {
        self.eff[pad].seed
    }

    fn voice_range(pad: usize) -> std::ops::Range<usize> {
        pad * VOICES_PER_PAD..(pad + 1) * VOICES_PER_PAD
    }

    fn start_fade(v: &mut Voice, mul: f32) {
        if v.active {
            v.fading = true;
            v.env_mul = mul;
        }
    }

    /// Dispara un golpe del pad `pad` con velocidad `vel` (0..1).
    /// `song_beats` es la posición actual en beats del transporte (para el ritmo de ROLL por compases).
    /// Si no hay transporte válido, pasar un valor negativo para forzar comportamiento "cada golpe".
    pub fn trigger(&mut self, pad: usize, vel: f32, kit: &KitAudio, store: &SampleStore, song_beats: f64) {
        if pad >= NUM_PADS {
            return;
        }
        let cfg = &kit.pads[pad];
        if self.mute[pad] || (kit.any_solo && !cfg.solo) {
            return;
        }
        self.clock += 1;
        let live = self.eff[pad];
        let n = store.bank_len[pad].load(Ordering::Relaxed).min(MAX_BANK);

        // Ritmo de re-sorteo: 0 = cada golpe (comportamiento original); 1/2/3 = cada 1/2/4 compases.
        // Entre cambios se mantiene el último valor de ROLL (resolve_hit_hold).
        let rate = self.rate[pad];
        let hp = if rate > 0 && song_beats >= 0.0 {
            let bars_per = match rate {
                1 => 1i64,
                2 => 2,
                _ => 4,
            };
            let bar = (song_beats / 4.0).floor() as i64;
            let due = self.last_roll_bar[pad] == i64::MIN || bar - self.last_roll_bar[pad] >= bars_per;
            if due {
                self.last_roll_bar[pad] = bar;
                resolve_hit(&cfg.cfg, live, &mut self.rt[pad], &mut self.roll, n)
            } else {
                resolve_hit_hold(&cfg.cfg, live, &self.rt[pad], n)
            }
        } else {
            resolve_hit(&cfg.cfg, live, &mut self.rt[pad], &mut self.roll, n)
        };

        // El pin manda en LOCK mientras su archivo exista en el banco (la tienda ya lo resolvió);
        // si falta, índice por semilla y aviso (la lógica completa está probada en `caja_core::kit`).
        let pin = store.pin_idx[pad].load(Ordering::Relaxed);
        let locked = is_lock(live.mode, P_SAMPLE);
        let pick = if locked && pin >= 0 {
            SamplePick { index: Some(pin as usize), pin_missing: false }
        } else {
            let missing = locked && store.pin_missing[pad].load(Ordering::Relaxed);
            if missing {
                self.pin_missing_hits = self.pin_missing_hits.wrapping_add(1);
            }
            SamplePick { index: hp.index, pin_missing: missing }
        };
        let sample = match pick.index {
            Some(i) if i < n => {
                let slot = SampleStore::slot(pad, i);
                store.cur_idx[pad].store(i as i32, Ordering::Relaxed);
                store.last_use[slot].store(store.clock.load(Ordering::Relaxed), Ordering::Relaxed);
                match store.slots[slot].load_full() {
                    Some(s) => {
                        self.last[pad] = Some(s.clone());
                        Some(s)
                    }
                    None => {
                        // Aún no está lista: pide la carga y usa la anterior (o calla). Nunca bloquea.
                        if !store.requested[slot].swap(true, Ordering::Relaxed) {
                            let _ = store.want.push(((pad as u32) << 8) | i as u32);
                        }
                        self.last[pad].clone()
                    }
                }
            }
            _ => self.last[pad].clone(),
        };
        let Some(sample) = sample else { return };
        if sample.is_empty() {
            return;
        }

        // choke: este golpe corta a los demás pads de su grupo
        if cfg.choke_group != 0 && cfg.choker {
            for q in 0..NUM_PADS {
                if q != pad && kit.pads[q].choke_group == cfg.choke_group {
                    let fm = self.fade_mul;
                    for v in &mut self.voices[Self::voice_range(q)] {
                        Self::start_fade(v, fm);
                    }
                }
            }
        }

        // polifonía: si ya suenan `poly` voces sin fundido, la más antigua se desvanece
        let poly = usize::from(cfg.poly.clamp(1, 4));
        let range = Self::voice_range(pad);
        let fm = self.fade_mul;
        let live_count = self.voices[range.clone()].iter().filter(|v| v.active && !v.fading).count();
        if live_count >= poly {
            if let Some(oldest) = self.voices[range.clone()].iter_mut().filter(|v| v.active && !v.fading).min_by_key(|v| v.age) {
                Self::start_fade(oldest, fm);
            }
        }
        let idx = self.voices[range.clone()]
            .iter()
            .position(|v| !v.active)
            .map(|i| range.start + i)
            .unwrap_or_else(|| range.start + self.voices[range.clone()].iter().enumerate().min_by_key(|(_, v)| v.age).map_or(0, |(i, _)| i));

        // parámetros de la voz
        let rate = f64::from((pitch_semitones(hp.pitch) / 12.0).exp2());
        let env_mul = match decay_seconds(hp.decay) {
            Some(d) => (-6.9078 / (d * self.sr)).exp(),
            None => 1.0,
        };
        // Preferir ganancia del parámetro del host (macro); el kit.gain es respaldo / persistencia.
        let g = if self.gain[pad] > 0.0 { self.gain[pad] } else { f32::from(cfg.gain) / 1000.0 };
        let gain = g * vel.clamp(0.0, 1.0);
        let pan = self.pan[pad].clamp(-1.0, 1.0);
        let ang = (pan + 1.0) * std::f32::consts::FRAC_PI_4;
        let (gl, gr) = (ang.cos() * std::f32::consts::SQRT_2 * gain, ang.sin() * std::f32::consts::SQRT_2 * gain);
        let v = &mut self.voices[idx];
        *v = Voice::idle();
        v.active = true;
        v.sample = Some(sample);
        v.rate = rate;
        v.env_mul = env_mul;
        v.gl = gl;
        v.gr = gr;
        v.age = self.clock;
        if let Some(fc) = cutoff_hz(hp.filter) {
            let fc = fc.min(self.sr * 0.45);
            let g = (std::f32::consts::PI * fc / self.sr).tan();
            v.lp = true;
            v.a1 = 1.0 / (1.0 + g * (g + 1.4142));
            v.a2 = g * v.a1;
            v.a3 = g * v.a2;
        }
    }

    /// Suma todas las voces activas en `l`/`r` (que ya vienen a cero).
    pub fn render(&mut self, l: &mut [f32], r: &mut [f32]) {
        let frames = l.len().min(r.len());
        for v in &mut self.voices {
            if !v.active {
                continue;
            }
            let Some(s) = v.sample.take() else {
                v.active = false;
                continue;
            };
            let len = s.left.len();
            let stereo = s.right.is_some();
            let right = s.right.as_deref().unwrap_or(&s.left);
            for i in 0..frames {
                let ip = v.pos as usize;
                if ip >= len {
                    v.active = false;
                    break;
                }
                let t = (v.pos - ip as f64) as f32;
                let mut xl = tap(&s.left, ip, t);
                let mut xr = if stereo { tap(right, ip, t) } else { xl };
                if v.lp {
                    xl = svf(&mut v.ic1[0], &mut v.ic2[0], xl, v.a1, v.a2, v.a3);
                    xr = if stereo { svf(&mut v.ic1[1], &mut v.ic2[1], xr, v.a1, v.a2, v.a3) } else { xl };
                }
                l[i] += xl * v.env * v.gl;
                r[i] += xr * v.env * v.gr;
                v.env *= v.env_mul;
                v.pos += v.rate;
                if v.env < 1.0e-4 && (v.fading || v.env_mul < 1.0) {
                    v.active = false;
                    break;
                }
            }
            if v.active {
                v.sample = Some(s);
            }
            // si terminó, `s` se suelta aquí: solo baja el contador (el cementerio del cargador guarda
            // otra referencia a lo expulsado, así que nunca se libera memoria en el hilo de audio
            // salvo que la muestra siga viva en la caché)
        }
    }

    #[allow(dead_code)]
    pub fn active_voices(&self) -> usize {
        self.voices.iter().filter(|v| v.active).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use caja_core::kit::{KitState, ALL_LOCK, ALL_ROLL};

    const SR: f32 = 48000.0;

    fn noise(frames: usize, stereo: bool, seed: u32) -> Arc<Sample> {
        let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        let mut gen = |decay: bool| {
            (0..frames)
                .map(|i| {
                    x ^= x << 13;
                    x ^= x >> 17;
                    x ^= x << 5;
                    let env = if decay { (-(i as f32) / (frames as f32 * 0.3)).exp() } else { 1.0 };
                    ((x as f32 / u32::MAX as f32) * 2.0 - 1.0) * 0.5 * env
                })
                .collect::<Vec<f32>>()
        };
        let left = gen(true);
        Arc::new(Sample { left, right: if stereo { Some(gen(true)) } else { None } })
    }

    fn rig(samples_per_pad: usize, frames: usize) -> (SampleStore, KitAudio) {
        let store = SampleStore::new();
        for pad in 0..NUM_PADS {
            store.bank_len[pad].store(samples_per_pad, Ordering::Relaxed);
            for i in 0..samples_per_pad {
                store.slots[SampleStore::slot(pad, i)].store(Some(noise(frames, pad % 2 == 0, (pad * 31 + i) as u32)));
            }
        }
        (store, KitAudio::from_state(&KitState::default()))
    }

    fn host_all(mode: u8) -> [(u32, u8, u8); NUM_PADS] {
        std::array::from_fn(|i| (1000 + i as u32 * 17, 0, mode))
    }

    fn energy(l: &[f32], r: &[f32]) -> f32 {
        l.iter().chain(r).map(|v| v * v).sum()
    }

    #[test]
    fn value_mappings() {
        assert_eq!(pitch_semitones(0.5), 0.0);
        assert!((pitch_semitones(1.0) - 24.0).abs() < 1e-5);
        assert_eq!(decay_seconds(1.0), None);
        assert!((decay_seconds(0.0).unwrap() - 0.01).abs() < 1e-6);
        assert_eq!(cutoff_hz(1.0), None);
        assert!((cutoff_hz(0.0).unwrap() - 100.0).abs() < 1e-3);
    }

    #[test]
    fn a_hit_makes_sound_and_unmapped_banks_are_silent() {
        let (store, ka) = rig(4, 24000);
        let mut e = Engine::new(SR);
        e.begin_block(&ka, &host_all(ALL_LOCK));
        e.trigger(0, 1.0, &ka, &store, -1.0);
        let (mut l, mut r) = (vec![0.0; 512], vec![0.0; 512]);
        e.render(&mut l, &mut r);
        assert!(energy(&l, &r) > 1.0);
        // un pad con banco vacío calla y no revienta
        store.bank_len[1].store(0, Ordering::Relaxed);
        let mut e2 = Engine::new(SR);
        e2.begin_block(&ka, &host_all(ALL_LOCK));
        e2.trigger(1, 1.0, &ka, &store, -1.0);
        let (mut l, mut r) = (vec![0.0; 512], vec![0.0; 512]);
        e2.render(&mut l, &mut r);
        assert_eq!(energy(&l, &r), 0.0);
    }

    #[test]
    fn lock_is_bit_exact_repeatable_and_roll_changes_with_the_dice() {
        let (store, ka) = rig(16, 12000);
        let render = |mode: u8, roll_seed: u64| {
            let mut e = Engine::new(SR);
            e.reseed_roll(roll_seed);
            e.begin_block(&ka, &host_all(mode));
            let (mut l, mut r) = (vec![0.0f32; 4096], vec![0.0f32; 4096]);
            for k in 0..4 {
                e.trigger(4, 1.0, &ka, &store, -1.0);
                e.render(&mut l[k * 1024..(k + 1) * 1024], &mut r[k * 1024..(k + 1) * 1024]);
            }
            l
        };
        assert_eq!(render(ALL_LOCK, 1), render(ALL_LOCK, 2), "LOCK no depende del dado");
        assert_eq!(render(ALL_ROLL, 7), render(ALL_ROLL, 7), "ROLL con la misma semilla global se repite");
        assert_ne!(render(ALL_ROLL, 7), render(ALL_ROLL, 8));
    }

    #[test]
    fn closed_hat_chokes_the_open_hat() {
        let (store, ka) = rig(1, 48000);
        let mut e = Engine::new(SR);
        e.begin_block(&ka, &host_all(ALL_LOCK));
        // pad 3 = hat abierto, pad 2 = hat cerrado (grupo 1)
        e.trigger(3, 1.0, &ka, &store, -1.0);
        let (mut l, mut r) = (vec![0.0; 256], vec![0.0; 256]);
        e.render(&mut l, &mut r);
        assert_eq!(e.active_voices(), 1);
        e.trigger(2, 1.0, &ka, &store, -1.0);
        let (mut l, mut r) = (vec![0.0; 1024], vec![0.0; 1024]);
        e.render(&mut l, &mut r);
        // el abierto se apagó en pocos ms; solo queda el cerrado
        assert_eq!(e.active_voices(), 1);
    }

    #[test]
    fn polyphony_is_limited_per_pad() {
        let (store, mut ka) = rig(1, 48000);
        ka.pads[0].poly = 1;
        let mut e = Engine::new(SR);
        e.begin_block(&ka, &host_all(ALL_LOCK));
        for _ in 0..4 {
            e.trigger(0, 1.0, &ka, &store, -1.0);
            let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
            e.render(&mut l, &mut r);
        }
        let (mut l, mut r) = (vec![0.0; 2048], vec![0.0; 2048]);
        e.render(&mut l, &mut r);
        assert_eq!(e.active_voices(), 1);
    }

    #[test]
    fn mute_and_solo_work() {
        let (store, mut ka) = rig(1, 4800);
        let mut e = Engine::new(SR);
        let mut mute = [false; NUM_PADS];
        mute[0] = true;
        e.set_mixer(&mute, &[0.0; NUM_PADS], &[1.0; NUM_PADS], &[0; NUM_PADS]);
        e.begin_block(&ka, &host_all(ALL_LOCK));
        e.trigger(0, 1.0, &ka, &store, -1.0);
        assert_eq!(e.active_voices(), 0);
        e.set_mixer(&[false; NUM_PADS], &[0.0; NUM_PADS], &[1.0; NUM_PADS], &[0; NUM_PADS]);
        ka.pads[5].solo = true;
        ka.any_solo = true;
        e.trigger(0, 1.0, &ka, &store, -1.0);
        assert_eq!(e.active_voices(), 0, "con solo en otro pad, este calla");
        e.trigger(5, 1.0, &ka, &store, -1.0);
        assert_eq!(e.active_voices(), 1);
    }

    #[test]
    fn pan_param_moves_the_sound_between_channels() {
        let (store, ka) = rig(1, 4800);
        let mut e = Engine::new(SR);
        let mut pan = [0.0; NUM_PADS];
        pan[1] = -1.0;
        e.set_mixer(&[false; NUM_PADS], &pan, &[1.0; NUM_PADS], &[0; NUM_PADS]);
        e.begin_block(&ka, &host_all(ALL_LOCK));
        e.trigger(1, 1.0, &ka, &store, -1.0);
        let (mut l, mut r) = (vec![0.0; 512], vec![0.0; 512]);
        e.render(&mut l, &mut r);
        assert!(energy(&l, &[]) > 0.5 && energy(&r, &[]) < 1e-6, "pan -1 = todo a la izquierda");
    }

    #[test]
    fn slot_change_loads_the_bank_seed_and_seed_change_wins() {
        let (_store, mut ka) = rig(1, 100);
        ka.pads[0].bank[3] = 4321;
        let mut e = Engine::new(SR);
        let mut h = host_all(ALL_LOCK);
        e.begin_block(&ka, &h);
        assert_eq!(e.effective_seed(0), 1000);
        h[0].1 = 3; // el host automatiza el slot
        e.begin_block(&ka, &h);
        assert_eq!(e.effective_seed(0), 4321);
        h[0].0 = 77; // y luego la semilla: manda la semilla
        e.begin_block(&ka, &h);
        assert_eq!(e.effective_seed(0), 77);
    }

    /// Ni el disparo ni el render reservan ni liberan memoria (el asignador de nih-plug aborta
    /// si algo reserva dentro de `assert_no_alloc` en debug).
    #[test]
    fn hot_path_does_not_allocate() {
        let (store, ka) = rig(8, 24000);
        let mut e = Engine::new(SR);
        let mut l = vec![0.0f32; 512];
        let mut r = vec![0.0f32; 512];
        assert_no_alloc::assert_no_alloc(|| {
            for block in 0..200 {
                let mode = if block % 2 == 0 { ALL_ROLL } else { ALL_LOCK };
                e.begin_block(&ka, &host_all(mode));
                for pad in 0..NUM_PADS {
                    if (block + pad) % 3 == 0 {
                        e.trigger(pad, 0.8, &ka, &store, -1.0);
                    }
                }
                l.fill(0.0);
                r.fill(0.0);
                e.render(&mut l, &mut r);
            }
        });
    }

    /// `cargo test --release -p caja -- --ignored --nocapture bench`
    #[test]
    #[ignore]
    fn bench_cpu_16_pads() {
        let (store, mut ka) = rig(32, (SR * 1.5) as usize);
        for p in ka.pads.iter_mut() {
            p.poly = 4;
            p.cfg[3] = caja_core::kit::ParamCfg { base: 600, range: 200, prob: 1000 }; // filtro activo
        }
        let mut host = host_all(ALL_ROLL);
        for h in host.iter_mut() {
            h.2 = 0b0000; // todo en ROLL
        }
        let mut e = Engine::new(SR);
        let block = 512usize;
        let blocks = (SR as usize * 60) / block; // 60 s de audio
        let (mut l, mut r) = (vec![0.0f32; block], vec![0.0f32; block]);
        let mut worst = std::time::Duration::ZERO;
        let mut peak_voices = 0;
        let t0 = std::time::Instant::now();
        let mut since = 0usize;
        for _ in 0..blocks {
            let b0 = std::time::Instant::now();
            e.begin_block(&ka, &host);
            since += block;
            // semicorcheas a 140 bpm en los 16 primeros pads, todos a la vez
            if since >= (SR * 60.0 / 140.0 / 4.0) as usize {
                since = 0;
                for pad in 0..16 {
                    e.trigger(pad, 0.9, &ka, &store, -1.0);
                }
            }
            l.fill(0.0);
            r.fill(0.0);
            e.render(&mut l, &mut r);
            peak_voices = peak_voices.max(e.active_voices());
            worst = worst.max(b0.elapsed());
        }
        let total = t0.elapsed();
        let rt_block = std::time::Duration::from_secs_f64(block as f64 / f64::from(SR));
        println!(
            "BENCH 16 pads x poly 4 + filtro, 48 kHz, bloque {block}: 60 s de audio en {:.1} ms -> {:.2} % de un nucleo; peor bloque {:.3} ms de {:.2} ms disponibles; voces pico {peak_voices}",
            total.as_secs_f64() * 1000.0,
            total.as_secs_f64() / 60.0 * 100.0,
            worst.as_secs_f64() * 1000.0,
            rt_block.as_secs_f64() * 1000.0
        );
    }
}
