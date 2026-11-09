//! Host simulado para pruebas: llama al scheduler bloque a bloque con un transporte sintético
//! (como la función `pump` de ChordGlow, pero con reloj).

use crate::scheduler::*;
use crate::schedule::Schedule;

/// Evento con tiempo absoluto en muestras.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timed {
    pub sample: u64,
    pub kind: OutKind,
}

pub struct SimOpts {
    pub sample_rate: f64,
    pub block: u32,
    pub bpm: f64,
    pub playing: bool,
    /// Si es false el host no da posición en beats (reloj libre).
    pub send_beats: bool,
    pub send_tempo: bool,
}

impl Default for SimOpts {
    fn default() -> Self {
        SimOpts { sample_rate: 48_000.0, block: 512, bpm: 120.0, playing: true, send_beats: true, send_tempo: true }
    }
}

/// Procesa `beats` negras empezando en `start_beat`. `inputs(block_index)` da los eventos de cada bloque.
pub fn pump(
    sched: &mut Scheduler,
    sch: &Schedule,
    st: &RunSettings,
    start_beat: f64,
    beats: f64,
    o: &SimOpts,
    mut inputs: impl FnMut(usize) -> Vec<InEvent>,
) -> Vec<Timed> {
    let frames_total = beats * 60.0 / o.bpm * o.sample_rate;
    let start_sample = (start_beat * 60.0 / o.bpm * o.sample_rate).round() as u64;
    let mut done: u64 = 0;
    let mut out = Vec::new();
    let mut idx = 0usize;
    while (done as f64) < frames_total {
        let tr = Transport {
            playing: o.playing,
            song_pos_beats: if o.send_beats { Some(start_beat + done as f64 / o.sample_rate * o.bpm / 60.0) } else { None },
            tempo: if o.send_tempo { Some(o.bpm) } else { None },
        };
        let input = inputs(idx);
        let (_, evs) = sched.process(o.block, &tr, st, sch, &input, &[]);
        for w in evs.windows(2) {
            assert!(w[0].time <= w[1].time, "salida desordenada en el bloque {idx}");
        }
        for e in evs {
            assert!(e.time < o.block, "evento fuera del bloque");
            if (done + u64::from(e.time)) as f64 + 1.0 >= frames_total {
                continue; // el último bloque puede pasarse del tramo pedido (±1 muestra de tolerancia en el borde)
            }
            out.push(Timed { sample: start_sample + done + u64::from(e.time), kind: e.kind });
        }
        done += u64::from(o.block);
        idx += 1;
    }
    out
}
