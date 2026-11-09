//! Swing, humanización y calendario de golpes (hits) dentro del bucle.

use crate::genres::GenreSet;
use crate::parser::MOD_SHUFFLE;
use crate::pattern::{Pattern, MAX_STEPS};
use crate::pieces::*;
use crate::rng::{hash_seed, js_round, Mulberry32};

pub const PPQ: u32 = 480;
pub const TICKS_PER_STEP: u32 = PPQ / 4; // 120
pub const MAX_HITS: usize = NUM_PIECES * MAX_STEPS;

#[derive(Clone, Debug)]
pub struct TimingOpts {
    pub genre: usize,
    pub mods: u32,
    pub swing: i32,
    pub human: i32,
    pub seed: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Hit {
    /// 0 .. bars*1920-1, dentro del bucle.
    pub tick: u16,
    /// Duración en ticks.
    pub dur: u16,
    pub note: u8,
    /// 1-127
    pub vel: u8,
    pub piece: u8,
    /// Paso de rejilla de origen (0..127).
    pub step: u8,
}

pub fn displacement(step: usize, piece_id: &str, swing: i32, human: i32, seed: &str, triplet_push: bool, mods: u32) -> (i32, i32) {
    let mut ticks = 0;
    if step % 2 == 1 {
        ticks += js_round(f64::from(swing) / 100.0 * 48.0);
    }
    if (mods & MOD_SHUFFLE != 0 || triplet_push) && step % 4 == 2 {
        ticks += js_round(f64::from(swing) / 100.0 * 90.0);
    }
    let mut dvel = 0;
    if human > 0 {
        let mut r = Mulberry32::new(hash_seed(&format!("{seed}:{piece_id}:{step}")) ^ 0x9e37_79b9);
        ticks += js_round((r.next() - 0.5) * 2.0 * (f64::from(human) / 100.0) * 18.0);
        dvel = js_round((r.next() - 0.5) * 2.0 * (f64::from(human) / 100.0) * 16.0);
    }
    (ticks, dvel)
}

/// Golpes ordenados por (tick, nota). `notes` asigna una nota MIDI a cada pieza (mapa del kit).
pub fn build_hits(pat: &Pattern, o: &TimingOpts, genres: &GenreSet, notes: &[u8; NUM_PIECES]) -> Vec<Hit> {
    let g = genres.get(o.genre);
    let steps = pat.steps();
    let loop_ticks = (steps as u32 * TICKS_PER_STEP) as i32;
    let mut list: Vec<Hit> = Vec::new();
    for pi in 0..NUM_PIECES {
        for i in 0..steps {
            let c = pat.cell[pi][i];
            if !c.on() {
                continue;
            }
            let (dt, dv) = displacement(i, PIECES[pi].id, o.swing, o.human, &o.seed, g.triplet_push, o.mods);
            let tick = (i as i32 * TICKS_PER_STEP as i32 + dt).clamp(0, loop_ticks - 1);
            list.push(Hit {
                tick: tick as u16,
                dur: PIECES[pi].dur,
                note: notes[pi] & 0x7f,
                vel: (i32::from(c.vel) + dv).clamp(1, 127) as u8,
                piece: pi as u8,
                step: i as u8,
            });
        }
    }
    list.sort_by(|a, b| a.tick.cmp(&b.tick).then(a.note.cmp(&b.note)));
    // Recorta la duración si la misma nota vuelve a sonar antes de terminar.
    let mut next = [-1i32; 128];
    for k in (0..list.len()).rev() {
        let h = &mut list[k];
        let sig = next[h.note as usize];
        if sig >= 0 && i32::from(h.tick) + i32::from(h.dur) >= sig {
            h.dur = (sig - i32::from(h.tick) - 8).max(12) as u16;
        }
        next[h.note as usize] = i32::from(h.tick);
    }
    list
}
