//! Sesión y modelo: parámetros -> patrón, ediciones a mano, historial, estado persistente y rechazo de estado corrupto.

use caja_core::model::Model;
use caja_core::pieces::*;
use caja_core::session::*;
use std::sync::{Arc, Mutex};

fn model() -> Model {
    Model::with_session(Session::builtin(), Arc::new(Mutex::new(String::new())))
}

fn snap() -> ParamSnap {
    ParamSnap { genre: 1, bars_idx: 1, swing: 20, human: 14, density: 50, seed: 4242, accent: 50, bpm: 90 }
}

#[test]
fn first_sync_generates_and_publishes_a_schedule() {
    let m = model();
    assert_eq!(m.schedule.latest().count, 0);
    m.sync(snap());
    let sch = m.schedule.latest();
    assert!(sch.count > 10);
    assert_eq!(sch.loop_ticks, 2 * 1920);
    assert!(!m.persist.lock().unwrap().is_empty(), "el estado se serializa");
}

#[test]
fn manual_edits_survive_timing_changes_but_not_pattern_changes() {
    let m = model();
    m.sync(snap());
    m.with(snap(), |s| {
        s.toggle_cell(CLAVE, 3, false);
        assert!(s.pattern.cell[CLAVE][3].on());
    });
    let before = m.schedule.latest().count;
    // swing/humanización: se republica pero se conserva lo editado
    let mut p = snap();
    p.swing = 55;
    m.sync(p);
    m.with(p, |s| assert!(s.pattern.cell[CLAVE][3].on()));
    assert_eq!(m.schedule.latest().count, before);
    // densidad: regenera (se pierde la edición, como en la versión C++)
    p.density = 90;
    m.sync(p);
    m.with(p, |s| assert!(!s.pattern.cell[CLAVE][3].on()));
}

#[test]
fn cell_editing_velocity_and_ghost() {
    let m = model();
    m.with(snap(), |s| {
        s.pattern.clear();
        s.toggle_cell(KICK, 0, false);
        let v = s.pattern.cell[KICK][0].vel;
        assert!(v > 0);
        s.set_cell_vel(KICK, 0, 300);
        assert_eq!(s.pattern.cell[KICK][0].vel, 127);
        s.set_cell_vel(KICK, 0, -5);
        assert_eq!(s.pattern.cell[KICK][0].vel, 1);
        s.toggle_ghost(KICK, 0);
        assert!(s.pattern.cell[KICK][0].ghost);
        s.toggle_ghost(SNARE, 4);
        assert!(s.pattern.cell[SNARE][4].ghost && s.pattern.cell[SNARE][4].on());
        s.toggle_cell(KICK, 0, false);
        assert!(!s.pattern.cell[KICK][0].on());
        s.toggle_cell(KICK, 9999, false); // fuera de rango: ignorado
    });
}

#[test]
fn generate_from_phrase_sets_params_and_history_caps_at_10() {
    let m = model();
    let mut p = snap();
    for i in 0..12 {
        p = m.with(p, |s| s.generate(&format!("techno {} bpm con fill 4 compases", 120 + i), p));
    }
    m.with(p, |s| {
        assert_eq!(s.history.len(), 10);
        assert_eq!(s.genres.get(s.genre_index(&s.snap())).id, "techno");
        assert_eq!(s.snap().bars_idx, 2);
        assert_eq!(s.snap().bpm, 131);
        assert!(s.reading().contains("Techno") && s.reading().contains("fill"));
        let got = s.restore(5).unwrap();
        assert_eq!(got.bpm, 120 + 6); // las más recientes primero: 0 -> 131 ... 5 -> 126
    });
}

#[test]
fn state_roundtrip_restores_edits_history_and_map() {
    let a = model();
    let mut p = snap();
    p = a.with(p, |s| s.generate("funk 100 bpm", p));
    a.with(p, |s| {
        s.toggle_cell(GUIRO, 5, false);
        s.select_map(1);
        s.set_note(KICK, 41);
    });
    let json = a.persist.lock().unwrap().clone();
    // otra instancia (el host cargó el proyecto): mismos parámetros + cadena persistida
    let b = Model::with_session(Session::builtin(), Arc::new(Mutex::new(json.clone())));
    b.sync(p);
    b.with(p, |s| {
        assert!(s.pattern.cell[GUIRO][5].on(), "la edición a mano se conserva");
        assert_eq!(s.history.len(), 1);
        assert_eq!(s.notes[KICK], 41);
        assert_eq!(s.maps.maps[s.map_index].name, "GM");
        assert_eq!(s.phrase, "funk 100 bpm");
        assert!(s.state_error.is_none());
    });
    assert_eq!(b.persist.lock().unwrap().clone().len() > 100, true);
}

#[test]
fn corrupt_state_is_rejected_without_touching_the_session() {
    let bad_states = [
        "".to_string(),
        "{".to_string(),
        r#"{"v":2,"phrase":"","warning":"","mods":0,"pattern":{"bars":2,"c":[]},"map":{"name":"x","notes":[]},"history":[]}"#.to_string(),
        r#"{"v":1,"phrase":"","warning":"","mods":0,"pattern":{"bars":99,"c":[]},"map":{"name":"x","notes":[1]},"history":[]}"#.to_string(),
        r#"{"v":1,"phrase":"","warning":"","mods":0,"pattern":{"bars":2,"c":[0,40,100]},"map":{"name":"x","notes":[36]},"history":[]}"#.to_string(),
        r#"{"v":1,"phrase":"","warning":"","mods":999999,"pattern":{"bars":2,"c":[]},"map":{"name":"x","notes":[1,2]},"history":[]}"#.to_string(),
    ];
    for (i, bad) in bad_states.iter().enumerate() {
        let m = Model::with_session(Session::builtin(), Arc::new(Mutex::new(bad.clone())));
        m.sync(snap());
        m.with(snap(), |s| {
            if !bad.is_empty() {
                assert!(s.state_error.is_some(), "estado {i} debía rechazarse");
            }
            assert!(s.pattern.count_on() > 10, "se generó desde los parámetros ({i})");
        });
    }
    // también con celdas válidas pero 25 notas con una fuera de rango
    let mut s = Session::builtin();
    s.sync(snap());
    let good = s.to_state_json();
    let broken = good.replacen("\"notes\":[36", "\"notes\":[300", 1);
    let mut s2 = Session::builtin();
    assert!(s2.load_state_json(&good, snap()).is_ok());
    assert!(s2.load_state_json(&broken, snap()).is_err());
}

#[test]
fn midi_export_uses_kit_map_or_gm() {
    let mut s = Session::builtin();
    s.sync(snap());
    s.pattern.clear();
    s.toggle_cell(CH, 0, false); // hat cerrado
    s.set_note(CH, 99);
    let find = |b: &[u8], note: u8| b.windows(3).any(|w| w == [0x99, note, w[2]] && w[2] > 0);
    assert!(find(&s.midi_bytes(false, false), 99));
    assert!(find(&s.midi_bytes(false, true), 42), "GM: hat cerrado = 42");
    let b1 = s.midi_bytes(true, false);
    assert_eq!(&b1[8..10], &[0, 1], "formato 1");
}

#[test]
fn learn_mode_assigns_notes_in_order_and_publishes() {
    let m = model();
    m.sync(snap());
    m.with(snap(), |s| {
        s.pattern.clear();
        s.toggle_cell(KICK, 0, false);
        s.learn.start(KICK);
    });
    m.with(snap(), |s| {
        assert_eq!(s.learn_note(60), Some((KICK, 60)));
        assert_eq!(s.learn.piece, SNARE);
    });
    assert_eq!(m.schedule.latest().hits()[0].note, 60);
}

#[test]
fn seed_text_and_random_seed() {
    assert_eq!(seed_from_text("12345"), 12345);
    assert!(seed_from_text("hola") < 1_000_000_000 && seed_from_text("hola") >= 0);
    assert_eq!(seed_from_text("hola"), seed_from_text(" hola "));
    for _ in 0..100 {
        let r = random_seed();
        assert!((100_000..1_000_000_000).contains(&r));
    }
}

#[test]
fn old_default_map_in_a_saved_project_moves_snare_off_the_closed_hat() {
    let mut s = Session::builtin();
    s.sync(snap());
    assert_eq!(s.notes[SNARE], 37);
    assert_eq!(s.notes[CH], 38);
    s.notes = legacy_notes();
    let raw = s.to_state_json();
    let mut loaded = Session::builtin();
    loaded.load_state_json(&raw, snap()).unwrap();
    assert_eq!(loaded.notes[SNARE], 37);
    assert_eq!(loaded.notes[CH], 38);
    assert_eq!(loaded.notes[SHAKER], 44);
    s.notes = legacy_notes();
    s.notes[KICK] = 40;
    let custom = s.to_state_json();
    loaded.load_state_json(&custom, snap()).unwrap();
    assert_eq!(loaded.notes[KICK], 40);
    assert_eq!(loaded.notes[SNARE], 38, "un mapa ya editado no se reescribe");
}
