//! Archivo MIDI estándar (SMF): formato 0 o 1, 480 PPQ, canal 10.

use crate::genres::GenreSet;
use crate::pattern::Pattern;
use crate::pieces::*;
use crate::rng::js_round;
use crate::timing::*;

struct Ev {
    tick: i32,
    ord: i32,
    note: i32,
    bytes: Vec<u8>,
}

fn put_vlq(out: &mut Vec<u8>, mut n: u32) {
    let mut stack = [0u8; 5];
    let mut k = 0;
    stack[k] = (n & 0x7f) as u8;
    k += 1;
    n >>= 7;
    while n > 0 {
        stack[k] = ((n & 0x7f) | 0x80) as u8;
        k += 1;
        n >>= 7;
    }
    while k > 0 {
        k -= 1;
        out.push(stack[k]);
    }
}

fn meta_text(kind: u8, text: &str) -> Vec<u8> {
    let mut b = vec![0xff, kind, text.len() as u8];
    b.extend(text.bytes().map(|c| c & 127));
    b
}

fn build_track(mut events: Vec<Ev>) -> Vec<u8> {
    events.sort_by(|a, b| a.tick.cmp(&b.tick).then(a.ord.cmp(&b.ord)).then(a.note.cmp(&b.note)));
    let mut body: Vec<u8> = Vec::new();
    let mut last = 0;
    for e in &events {
        let delta = (e.tick - last).max(0);
        last = last.max(e.tick);
        put_vlq(&mut body, delta as u32);
        body.extend_from_slice(&e.bytes);
    }
    let mut trk = b"MTrk".to_vec();
    trk.extend_from_slice(&(body.len() as u32).to_be_bytes());
    trk.extend_from_slice(&body);
    trk
}

fn add_hits(ev: &mut Vec<Ev>, hits: &[Hit], filter: Option<usize>) {
    for h in hits {
        if filter.is_some_and(|p| usize::from(h.piece) != p) {
            continue;
        }
        let tick = i32::from(h.tick);
        ev.push(Ev { tick, ord: 1, note: i32::from(h.note), bytes: vec![0x99, h.note, h.vel] });
        ev.push(Ev { tick: tick + i32::from(h.dur), ord: 0, note: i32::from(h.note), bytes: vec![0x89, h.note, 0] });
    }
}

pub fn build_midi_file(pat: &Pattern, o: &TimingOpts, bpm: i32, separate: bool, genres: &GenreSet, notes: &[u8; NUM_PIECES]) -> Vec<u8> {
    let hits = build_hits(pat, o, genres, notes);
    let end = ((pat.steps() as u32 * TICKS_PER_STEP) as i32).max(120);
    let us = js_round(60_000_000.0 / f64::from(bpm.max(1))).clamp(1, 0xff_ffff);
    let head = || {
        vec![
            Ev { tick: 0, ord: -2, note: 0, bytes: vec![0xff, 0x58, 0x04, 0x04, 0x02, 0x18, 0x08] },
            Ev { tick: 0, ord: -1, note: 0, bytes: vec![0xff, 0x51, 0x03, ((us >> 16) & 255) as u8, ((us >> 8) & 255) as u8, (us & 255) as u8] },
        ]
    };
    let end_ev = || Ev { tick: end, ord: 9, note: 0, bytes: vec![0xff, 0x2f, 0x00] };
    let mut tracks: Vec<Vec<u8>> = Vec::new();
    let format: u16;
    if separate {
        format = 1;
        let mut cond = head();
        cond.push(Ev { tick: 0, ord: -3, note: 0, bytes: meta_text(0x03, "CAJA") });
        cond.push(end_ev());
        tracks.push(build_track(cond));
        for pi in 0..NUM_PIECES {
            if !hits.iter().any(|h| usize::from(h.piece) == pi) {
                continue;
            }
            let mut ev = vec![Ev { tick: 0, ord: -3, note: 0, bytes: meta_text(0x03, PIECES[pi].midi_name) }];
            add_hits(&mut ev, &hits, Some(pi));
            ev.push(end_ev());
            tracks.push(build_track(ev));
        }
    } else {
        format = 0;
        let mut ev = head();
        ev.push(Ev { tick: 0, ord: -3, note: 0, bytes: meta_text(0x03, "CAJA") });
        add_hits(&mut ev, &hits, None);
        ev.push(end_ev());
        tracks.push(build_track(ev));
    }
    let mut out = b"MThd".to_vec();
    out.extend_from_slice(&6u32.to_be_bytes());
    out.extend_from_slice(&format.to_be_bytes());
    out.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
    out.extend_from_slice(&(PPQ as u16).to_be_bytes());
    for t in tracks {
        out.extend_from_slice(&t);
    }
    out
}
