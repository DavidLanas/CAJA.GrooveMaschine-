//! 12 instancias simultáneas: el coste por bloque del scheduler debe quedar muy por debajo del
//! presupuesto de tiempo real. Se tolera un 0,1 % de bloques fuera de presupuesto (preempción del SO).

use caja_core::genres::GenreSet;
use caja_core::pattern::*;
use caja_core::pieces::*;
use caja_core::schedule::Schedule;
use caja_core::scheduler::*;
use caja_core::timing::*;
use std::time::Instant;

const INSTANCES: usize = 12;
const BLOCK: u32 = 256;
const SR: f64 = 48_000.0;

fn dense_schedule() -> Schedule {
    let g = GenreSet::builtin();
    let spec = Spec { genre: g.index_of("metal").unwrap(), mods: 0, bars: 8, density: 100, seed: "7".into(), accent: 50 };
    let pat = generate_pattern(&spec, &g);
    let o = TimingOpts { genre: spec.genre, mods: 0, swing: 20, human: 30, seed: "7".into() };
    let hits = build_hits(&pat, &o, &g, &default_notes());
    let mut s = Schedule::default();
    s.fill(pat.steps() as u32 * TICKS_PER_STEP, &hits);
    assert!(s.count > 150, "calendario denso: {}", s.count);
    s
}

#[test]
fn twelve_instances_stay_far_below_the_block_budget() {
    let budget_ns = f64::from(BLOCK) / SR * 1e9; // ~5,33 ms
    let blocks = 4000usize;
    let mut handles = Vec::new();
    for k in 0..INSTANCES {
        handles.push(std::thread::spawn(move || {
            let sch = dense_schedule();
            let mut s = Scheduler::new();
            s.activate(SR);
            let st = RunSettings { cc_num: 74, thru: true, ..Default::default() };
            let input = [InEvent { time: 3, kind: InKind::NoteOn { note: 60 } }, InEvent { time: 9, kind: InKind::NoteOff { note: 60 } }];
            let mut times: Vec<f64> = Vec::with_capacity(blocks);
            let mut events = 0usize;
            for b in 0..blocks {
                let beat = b as f64 * f64::from(BLOCK) / SR * (120.0 + k as f64) / 60.0;
                let tr = Transport { playing: true, song_pos_beats: Some(beat), tempo: Some(120.0 + k as f64) };
                let t0 = Instant::now();
                let (_, ev) = s.process(BLOCK, &tr, &st, &sch, if b % 7 == 0 { &input } else { &[] }, &[]);
                events += ev.len();
                times.push(t0.elapsed().as_nanos() as f64);
            }
            assert!(events > 1000);
            times
        }));
    }
    let mut all: Vec<f64> = handles.into_iter().flat_map(|h| h.join().unwrap()).collect();
    all.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mean = all.iter().sum::<f64>() / all.len() as f64;
    let p999 = all[(all.len() as f64 * 0.999) as usize - 1];
    let over = all.iter().filter(|&&t| t > budget_ns).count() as f64 / all.len() as f64;
    println!("12 instancias: media {:.1} us, p99.9 {:.1} us, presupuesto {:.0} us, fuera de presupuesto {:.4} %", mean / 1e3, p999 / 1e3, budget_ns / 1e3, over * 100.0);
    assert!(mean < budget_ns * 0.05, "media demasiado alta");
    assert!(over <= 0.001, "más de un 0,1 % de bloques fuera de presupuesto");
    assert!(p999 < budget_ns, "p99.9 por encima del presupuesto");
}
