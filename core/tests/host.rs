//! Host simulado: precisión de muestra, bucle idéntico, parada, saltos, Thru, CC y modos.

use caja_core::genres::GenreSet;
use caja_core::host_sim::*;
use caja_core::pattern::*;
use caja_core::pieces::*;
use caja_core::schedule::Schedule;
use caja_core::scheduler::*;
use caja_core::timing::*;

fn schedule_for(pat: &Pattern, swing: i32, human: i32) -> Schedule {
    let g = GenreSet::builtin();
    let o = TimingOpts { genre: 0, mods: 0, swing, human, seed: "1".into() };
    let hits = build_hits(pat, &o, &g, &default_notes());
    let mut s = Schedule::default();
    s.fill(pat.steps() as u32 * TICKS_PER_STEP, &hits);
    s
}

fn four_on_floor(bars: usize) -> Pattern {
    let mut p = Pattern::default();
    p.bars = bars;
    for b in 0..bars {
        for k in 0..4 {
            p.cell[KICK][b * 16 + k * 4] = Cell { vel: 100, ghost: false };
        }
        p.cell[SNARE][b * 16 + 4] = Cell { vel: 90, ghost: false };
    }
    p
}

fn ons(ev: &[Timed], note: u8) -> Vec<u64> {
    ev.iter().filter_map(|e| match e.kind { OutKind::NoteOn { note: n, .. } if n == note => Some(e.sample), _ => None }).collect()
}

#[test]
fn kicks_land_on_the_exact_sample_at_120bpm_48k() {
    let sch = schedule_for(&four_on_floor(1), 0, 0);
    let mut s = Scheduler::new();
    s.activate(48_000.0);
    for block in [1u32, 64, 127, 512, 1000] {
        s.reset();
        let o = SimOpts { block, ..Default::default() };
        let ev = pump(&mut s, &sch, &RunSettings::default(), 0.0, 12.0, &o, |_| vec![]);
        let k = ons(&ev, 36);
        assert_eq!(k.len(), 12, "bloque {block}");
        for (i, &t) in k.iter().enumerate() {
            let want = (i as i64) * 24_000;
            assert!((t as i64 - want).abs() <= 1, "bloque {block}, kick {i}: {t} vs {want}");
        }
    }
}

#[test]
fn loop_is_identical_every_lap_and_starts_mid_bar() {
    let sch = schedule_for(&four_on_floor(2), 20, 14);
    let mut s = Scheduler::new();
    s.activate(48_000.0);
    let o = SimOpts::default();
    let ev = pump(&mut s, &sch, &RunSettings::default(), 0.0, 32.0, &o, |_| vec![]);
    let lap = (8.0 * 24_000.0) as u64; // 2 compases
    let first: Vec<(u64, OutKind)> = ev.iter().filter(|e| e.sample < lap).map(|e| (e.sample, e.kind)).collect();
    assert!(!first.is_empty());
    for l in 1..4u64 {
        let this: Vec<(u64, u8)> = ev.iter().filter(|e| e.sample >= l * lap && e.sample < (l + 1) * lap).filter_map(|e| match e.kind { OutKind::NoteOn { note, .. } => Some((e.sample - l * lap, note)), _ => None }).collect();
        let base: Vec<(u64, u8)> = first.iter().filter_map(|(t, k)| match k { OutKind::NoteOn { note, .. } => Some((*t, *note)), _ => None }).collect();
        assert_eq!(this.len(), base.len());
        for (a, b) in this.iter().zip(&base) {
            assert_eq!(a.1, b.1);
            assert!((a.0 as i64 - b.0 as i64).abs() <= 1, "vuelta {l}: {a:?} vs {b:?}");
        }
    }
    // arrancar en medio del compás da los mismos golpes que el bucle continuo
    let mut s2 = Scheduler::new();
    s2.activate(48_000.0);
    let straight = schedule_for(&four_on_floor(2), 0, 0);
    let ev2 = pump(&mut s2, &straight, &RunSettings::default(), 2.0, 2.0, &o, |_| vec![]);
    let kicks2 = ons(&ev2, 36);
    assert_eq!(kicks2.len(), 2);
    assert!((kicks2[0] as i64 - 2 * 24_000).abs() <= 1, "beat 2: {kicks2:?}");
    assert!((kicks2[1] as i64 - 3 * 24_000).abs() <= 1, "beat 3 (paso 12)");
}

#[test]
fn every_note_on_gets_its_note_off() {
    let sch = schedule_for(&four_on_floor(1), 0, 0);
    let mut s = Scheduler::new();
    s.activate(48_000.0);
    let ev = pump(&mut s, &sch, &RunSettings::default(), 0.0, 16.0, &SimOpts::default(), |_| vec![]);
    let mut open = std::collections::HashMap::new();
    for e in &ev {
        match e.kind {
            OutKind::NoteOn { id, .. } => { assert!(open.insert(id, e.sample).is_none()); }
            OutKind::NoteOff { id, .. } => { assert!(open.remove(&id).is_some(), "off sin on"); }
            _ => {}
        }
    }
    assert!(open.len() <= 2, "como mucho las notas del último instante: {}", open.len());
}

#[test]
fn stop_is_silent_and_sends_cc123_once() {
    let sch = schedule_for(&four_on_floor(1), 0, 0);
    let mut s = Scheduler::new();
    s.activate(48_000.0);
    let st = RunSettings::default();
    let _ = pump(&mut s, &sch, &st, 0.0, 4.0, &SimOpts::default(), |_| vec![]);
    let stopped = SimOpts { playing: false, ..Default::default() };
    let ev = pump(&mut s, &sch, &st, 4.0, 8.0, &stopped, |_| vec![]);
    let ccs: Vec<_> = ev.iter().filter(|e| matches!(e.kind, OutKind::Cc { cc: 123, .. })).collect();
    assert_eq!(ccs.len(), 1);
    assert!(ev.iter().all(|e| !matches!(e.kind, OutKind::NoteOn { .. })), "silencio al parar");
}

#[test]
fn position_jump_cuts_sounding_notes() {
    let mut pat = Pattern::default();
    pat.bars = 1;
    pat.cell[OH][0] = Cell { vel: 100, ghost: false }; // hat abierto: 180 ticks
    let sch = schedule_for(&pat, 0, 0);
    let mut s = Scheduler::new();
    s.activate(48_000.0);
    let st = RunSettings::default();
    let tr = |beat: f64| Transport { playing: true, song_pos_beats: Some(beat), tempo: Some(120.0) };
    let (_, e1) = s.process(64, &tr(0.0), &st, &sch, &[], &[]);
    assert!(e1.iter().any(|e| matches!(e.kind, OutKind::NoteOn { .. })));
    // el host salta al beat 100 con la nota aún sonando
    let (_, e2) = s.process(64, &tr(100.0 + 0.5), &st, &sch, &[], &[]);
    assert!(e2.iter().any(|e| matches!(e.kind, OutKind::NoteOff { .. })), "debe cortar la nota");
}

#[test]
fn thru_passes_input_in_order_and_off_by_default() {
    let sch = schedule_for(&four_on_floor(1), 0, 0);
    let mut s = Scheduler::new();
    s.activate(48_000.0);
    let input = vec![
        InEvent { time: 10, kind: InKind::NoteOn { note: 60 } },
        InEvent { time: 20, kind: InKind::Other },
        InEvent { time: 30, kind: InKind::NoteOff { note: 60 } },
    ];
    let tr = Transport { playing: false, song_pos_beats: Some(0.0), tempo: Some(120.0) };
    let off = RunSettings { thru: false, ..Default::default() };
    let (_, ev) = s.process(64, &tr, &off, &sch, &input, &[]);
    assert!(ev.is_empty());
    let on = RunSettings { thru: true, ..Default::default() };
    let (_, ev) = s.process(64, &tr, &on, &sch, &input, &[]);
    let passes: Vec<(u32, usize)> = ev.iter().filter_map(|e| match e.kind { OutKind::Pass(i) => Some((e.time, i)), _ => None }).collect();
    assert_eq!(passes, vec![(10, 0), (20, 1), (30, 2)]);
}

#[test]
fn cc_lane_sends_step_velocity_and_channel_and_velocity_scale_apply() {
    let sch = schedule_for(&four_on_floor(1), 0, 0);
    let mut s = Scheduler::new();
    s.activate(48_000.0);
    let st = RunSettings { cc_num: 74, channel: 3, vel_scale: 50, ..Default::default() };
    let ev = pump(&mut s, &sch, &st, 0.0, 4.0, &SimOpts::default(), |_| vec![]);
    let ccs: Vec<(u64, u8, f32)> = ev.iter().filter_map(|e| match e.kind { OutKind::Cc { cc: 74, channel: 3, value } => Some((e.sample, 74, value)), _ => None }).collect();
    assert_eq!(ccs.len(), 16, "una por semicorchea");
    assert!((ccs[0].2 * 127.0 - 100.0).abs() < 0.6, "paso 0 = kick 100");
    assert!((ccs[4].2 * 127.0 - 100.0).abs() < 0.6, "paso 4 = kick 100 (snare 90)");
    assert_eq!(ccs[1].2, 0.0);
    for e in &ev {
        if let OutKind::NoteOn { channel, vel, note, .. } = e.kind {
            assert_eq!(channel, 3);
            let want = if note == 36 { 50.0 } else { 45.0 };
            assert!((vel * 127.0 - want).abs() < 0.6, "{vel}");
        }
    }
}

#[test]
fn run_modes_always_and_held_key_and_audition() {
    let sch = schedule_for(&four_on_floor(1), 0, 0);
    let stopped = SimOpts { playing: false, send_beats: false, ..Default::default() };
    // siempre: suena sin transporte
    let mut s = Scheduler::new();
    s.activate(48_000.0);
    let always = RunSettings { run_mode: 1, ..Default::default() };
    let ev = pump(&mut s, &sch, &always, 0.0, 4.0, &stopped, |_| vec![]);
    assert_eq!(ons(&ev, 36).len(), 4);
    // con una tecla: solo mientras se sostiene
    let mut s = Scheduler::new();
    s.activate(48_000.0);
    let key = RunSettings { run_mode: 2, ..Default::default() };
    let none = pump(&mut s, &sch, &key, 0.0, 2.0, &stopped, |_| vec![]);
    assert!(ons(&none, 36).is_empty());
    let held = pump(&mut s, &sch, &key, 0.0, 4.0, &stopped, |b| if b == 0 { vec![InEvent { time: 0, kind: InKind::NoteOn { note: 60 } }] } else { vec![] });
    assert_eq!(ons(&held, 36).len(), 4);
    let released = pump(&mut s, &sch, &key, 0.0, 2.0, &stopped, |b| if b == 0 { vec![InEvent { time: 0, kind: InKind::NoteOff { note: 60 } }] } else { vec![] });
    assert!(ons(&released, 36).is_empty());
    assert!(released.iter().any(|e| matches!(e.kind, OutKind::Cc { cc: 123, .. })));
    // probar: audition fuerza la marcha en modo transporte
    let mut s = Scheduler::new();
    s.activate(48_000.0);
    let aud = RunSettings { audition: true, ..Default::default() };
    let ev = pump(&mut s, &sch, &aud, 0.0, 4.0, &stopped, |_| vec![]);
    assert_eq!(ons(&ev, 36).len(), 4);
}

#[test]
fn preview_note_sounds_then_releases() {
    let sch = Schedule::default();
    let mut s = Scheduler::new();
    s.activate(48_000.0);
    let tr = Transport::default();
    let st = RunSettings::default();
    let (_, ev) = s.process(256, &tr, &st, &sch, &[], &[Preview { note: 38, vel: 100 }]);
    assert!(matches!(ev[0].kind, OutKind::NoteOn { note: 38, .. }));
    let mut off = false;
    for _ in 0..40 {
        let (_, ev) = s.process(256, &tr, &st, &sch, &[], &[]);
        off |= ev.iter().any(|e| matches!(e.kind, OutKind::NoteOff { note: 38, .. }));
    }
    assert!(off);
}

