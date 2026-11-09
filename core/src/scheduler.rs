//! Scheduler de tiempo real puro (sin nih-plug): convierte el calendario de golpes y el
//! transporte del host en eventos con precisión de muestra. Port de `processor.cpp`.
//!
//! Sin bloqueos y sin reservas en `process`: todos los buffers se crean en `new`.

use crate::schedule::Schedule;
use crate::timing::{PPQ, TICKS_PER_STEP};

pub const MAX_OUT: usize = 8192;
const MAX_ACTIVE: usize = 128;
const MAX_PREVIEW: usize = 16;
/// Duración de las notas de prueba de la interfaz.
const PREVIEW_MS: f64 = 120.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InKind {
    NoteOn { note: u8 },
    NoteOff { note: u8 },
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InEvent {
    pub time: u32,
    pub kind: InKind,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum OutKind {
    NoteOn { note: u8, channel: u8, vel: f32, id: i32 },
    NoteOff { note: u8, channel: u8, id: i32 },
    /// `value` en 0..1 (el CC de 7 bits es value*127).
    Cc { channel: u8, cc: u8, value: f32 },
    /// Reenvía el evento de entrada con ese índice, tal cual (Thru).
    Pass(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct OutEvent {
    pub time: u32,
    pub kind: OutKind,
    order: u8,
    seq: u32,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Transport {
    pub playing: bool,
    /// Posición en beats (negra) si el host la da.
    pub song_pos_beats: Option<f64>,
    pub tempo: Option<f64>,
}

#[derive(Clone, Copy, Debug)]
pub struct RunSettings {
    /// 0 con el transporte, 1 siempre, 2 solo con una tecla pulsada.
    pub run_mode: u8,
    pub thru: bool,
    /// 25..200 (%)
    pub vel_scale: i32,
    /// 0 = apagado
    pub cc_num: u8,
    /// Tempo cuando el host no lo da.
    pub free_bpm: i32,
    /// 0..15
    pub channel: u8,
    /// "Probar": corre aunque el transporte esté parado.
    pub audition: bool,
}

impl Default for RunSettings {
    fn default() -> Self {
        RunSettings { run_mode: 0, thru: false, vel_scale: 100, cc_num: 0, free_bpm: 100, channel: 9, audition: false }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Preview {
    pub note: u8,
    pub vel: u8,
}

#[derive(Clone, Copy, Debug)]
pub struct BlockInfo {
    /// Paso actual dentro del bucle; -1 = parado.
    pub step: i32,
    pub running: bool,
    pub tempo: f64,
    /// Posición en beats del reloj usado este bloque (host o free_tick).
    /// Sirve para el ritmo de ROLL por compases en standalone.
    pub song_beats: f64,
}

#[derive(Clone, Copy, Default)]
struct Active {
    used: bool,
    note: u8,
    channel: u8,
    off_tick: i64,
    id: i32,
}

#[derive(Clone, Copy, Default)]
struct PreviewActive {
    used: bool,
    note: u8,
    channel: u8,
    id: i32,
    frames_left: i64,
}

pub struct Scheduler {
    sr: f64,
    active: [Active; MAX_ACTIVE],
    previews: [PreviewActive; MAX_PREVIEW],
    out: Vec<OutEvent>,
    seq: u32,
    next_id: i32,
    was_running: bool,
    have_prev: bool,
    prev_end: i64,
    free_tick: f64,
    held: [bool; 128],
    held_count: i32,
}

impl Scheduler {
    pub fn new() -> Self {
        Scheduler {
            sr: 44100.0,
            active: [Active::default(); MAX_ACTIVE],
            previews: [PreviewActive::default(); MAX_PREVIEW],
            out: Vec::with_capacity(MAX_OUT),
            seq: 0,
            next_id: 1,
            was_running: false,
            have_prev: false,
            prev_end: 0,
            free_tick: 0.0,
            held: [false; 128],
            held_count: 0,
        }
    }

    pub fn activate(&mut self, sample_rate: f64) {
        self.sr = if sample_rate > 0.0 { sample_rate } else { 44100.0 };
        self.reset();
    }

    pub fn reset(&mut self) {
        self.active = [Active::default(); MAX_ACTIVE];
        self.previews = [PreviewActive::default(); MAX_PREVIEW];
        self.out.clear();
        self.was_running = false;
        self.have_prev = false;
        self.free_tick = 0.0;
        self.held = [false; 128];
        self.held_count = 0;
    }

    fn push(&mut self, time: u32, order: u8, kind: OutKind) {
        if self.out.len() >= MAX_OUT {
            return;
        }
        let seq = self.seq;
        self.seq = self.seq.wrapping_add(1);
        self.out.push(OutEvent { time, kind, order, seq });
    }

    fn note_off_slot(&mut self, time: u32, i: usize) {
        let a = self.active[i];
        self.push(time, 0, OutKind::NoteOff { note: a.note, channel: a.channel, id: a.id });
        self.active[i].used = false;
    }

    fn note_on(&mut self, time: u32, note: u8, channel: u8, vel: f32, off_tick: i64) {
        // Si la misma nota sigue sonando (cola que cruza el final del bucle), la cierra antes.
        for i in 0..MAX_ACTIVE {
            let a = self.active[i];
            if a.used && a.note == note && a.channel == channel {
                self.note_off_slot(time, i);
            }
        }
        let id = self.next_id;
        self.next_id = if self.next_id == i32::MAX { 1 } else { self.next_id + 1 };
        self.push(time, 1, OutKind::NoteOn { note, channel, vel, id });
        if let Some(slot) = self.active.iter().position(|a| !a.used) {
            self.active[slot] = Active { used: true, note, channel, off_tick, id };
        }
    }

    fn release_all(&mut self, time: u32) {
        for i in 0..MAX_ACTIVE {
            if self.active[i].used {
                self.note_off_slot(time, i);
            }
        }
    }

    /// Procesa un bloque. Devuelve los eventos ordenados por (tiempo, offs antes que ons, llegada).
    pub fn process(
        &mut self,
        frames: u32,
        transport: &Transport,
        st: &RunSettings,
        sch: &Schedule,
        input: &[InEvent],
        previews: &[Preview],
    ) -> (BlockInfo, &[OutEvent]) {
        self.out.clear();
        let n = frames;

        // 1. eventos de entrada: teclas sostenidas (modo "tecla") y Thru
        for (i, e) in input.iter().enumerate() {
            match e.kind {
                InKind::NoteOn { note } => {
                    let k = usize::from(note & 0x7f);
                    if !self.held[k] {
                        self.held[k] = true;
                        self.held_count += 1;
                    }
                    if st.thru {
                        self.push(e.time, 1, OutKind::Pass(i));
                    }
                }
                InKind::NoteOff { note } => {
                    let k = usize::from(note & 0x7f);
                    if self.held[k] {
                        self.held[k] = false;
                        self.held_count -= 1;
                    }
                    if st.thru {
                        self.push(e.time, 0, OutKind::Pass(i));
                    }
                }
                InKind::Other => {
                    if st.thru {
                        self.push(e.time, 1, OutKind::Pass(i));
                    }
                }
            }
        }

        // 2. transporte y reloj
        let host_playing = transport.playing;
        let has_beats = transport.song_pos_beats.is_some();
        // run_mode 0 = seguir host (tempo del DAW si existe).
        // run_mode 1/2 = reloj libre: SIEMPRE el BPM de la GUI (standalone / tecla).
        // Antes se prefería transport.tempo aunque viniera fijo a 120 desde el wrapper
        // standalone, y el slider de BPM no hacía nada.
        let tempo = if st.run_mode == 0 {
            match transport.tempo {
                Some(t) if t > 1.0 => t,
                _ => f64::from(st.free_bpm).max(20.0),
            }
        } else {
            f64::from(st.free_bpm).max(20.0)
        };
        let running = match st.run_mode {
            0 => host_playing,
            1 => true,
            _ => self.held_count > 0,
        } || st.audition;
        let ticks_per_frame = tempo * f64::from(PPQ) / (60.0 * self.sr);
        let lp = i64::from(sch.loop_ticks.max(480));

        let mut cur_step = -1;
        if running {
            let follow_host = host_playing && has_beats;
            let t0 = if follow_host {
                transport.song_pos_beats.unwrap_or(0.0) * f64::from(PPQ)
            } else {
                if !self.was_running {
                    self.free_tick = 0.0; // arranque/retrigger en el compás 1
                }
                self.free_tick
            };
            let t1 = t0 + f64::from(n) * ticks_per_frame;
            let n0 = (t0 - 1e-3).ceil() as i64;
            let n1 = (t1 - 1e-3).ceil() as i64;

            // salto de posición (loop del host, seek, cambio de reloj): corta lo que suena
            if self.have_prev && self.was_running && (n0 - self.prev_end).abs() > 24 {
                self.release_all(0);
            }
            if !self.was_running {
                self.release_all(0);
            }

            let vscale = f64::from(st.vel_scale);
            let channel = st.channel & 15;
            let frame_of = |tick: i64| -> u32 {
                let f = (tick as f64 - t0) / ticks_per_frame;
                f.clamp(0.0, f64::from(n.max(1)) - 1.0) as u32
            };

            // note-offs pendientes (también los atrasados)
            for i in 0..MAX_ACTIVE {
                let a = self.active[i];
                if a.used && a.off_tick < n1 {
                    self.note_off_slot(frame_of(a.off_tick), i);
                }
            }

            // golpes del calendario que caen en [n0, n1)
            let hits = sch.hits();
            let mut cycle = n0.div_euclid(lp) * lp;
            while cycle < n1 {
                let lo = (n0 - cycle).max(0);
                let hi = (n1 - cycle).min(lp);
                let start = hits.partition_point(|h| i64::from(h.tick) < lo);
                for h in &hits[start..] {
                    if i64::from(h.tick) >= hi {
                        break;
                    }
                    let abs = cycle + i64::from(h.tick);
                    let vel = (f64::from(h.vel) * (vscale / 100.0)).clamp(1.0, 127.0) / 127.0;
                    self.note_on(frame_of(abs), h.note, channel, vel as f32, abs + i64::from(h.dur));
                }
                // carril CC: un valor por semicorchea = velocidad máxima del paso
                if st.cc_num > 0 {
                    let tps = i64::from(TICKS_PER_STEP);
                    let mut tk = ((cycle + lo + tps - 1) / tps) * tps;
                    while tk < cycle + hi {
                        let step = (((tk - cycle) / tps) as usize) % sch.step_vel.len();
                        let value = f32::from(sch.step_vel[step]) / 127.0;
                        self.push(frame_of(tk), 1, OutKind::Cc { channel, cc: st.cc_num & 0x7f, value });
                        tk += tps;
                    }
                }
                cycle += lp;
            }
            let within = n0.rem_euclid(lp);
            cur_step = (within / i64::from(TICKS_PER_STEP)) as i32;
            self.prev_end = n1;
            self.have_prev = true;
            if !follow_host {
                self.free_tick = t1;
            }
        } else {
            if self.was_running {
                // acaba de parar: apaga todo y avisa con All Notes Off
                self.release_all(0);
                let channel = st.channel & 15;
                self.push(0, 1, OutKind::Cc { channel, cc: 123, value: 0.0 });
            }
            self.have_prev = false;
        }
        self.was_running = running;

        // 3. notas de prueba de la interfaz (clic en una celda)
        let channel = st.channel & 15;
        for i in 0..MAX_PREVIEW {
            let p = self.previews[i];
            if p.used {
                if p.frames_left < i64::from(n) {
                    self.push(p.frames_left.max(0) as u32, 0, OutKind::NoteOff { note: p.note, channel: p.channel, id: p.id });
                    self.previews[i].used = false;
                } else {
                    self.previews[i].frames_left -= i64::from(n);
                }
            }
        }
        for pv in previews {
            for i in 0..MAX_PREVIEW {
                let p = self.previews[i];
                if p.used && p.note == pv.note {
                    self.push(0, 0, OutKind::NoteOff { note: p.note, channel: p.channel, id: p.id });
                    self.previews[i].used = false;
                }
            }
            if let Some(slot) = self.previews.iter().position(|p| !p.used) {
                let id = self.next_id;
                self.next_id = if self.next_id == i32::MAX { 1 } else { self.next_id + 1 };
                self.push(0, 1, OutKind::NoteOn { note: pv.note, channel, vel: f32::from(pv.vel.clamp(1, 127)) / 127.0, id });
                let frames = (self.sr * PREVIEW_MS / 1000.0) as i64;
                self.previews[slot] = PreviewActive { used: true, note: pv.note, channel, id, frames_left: frames };
            }
        }

        // 4. salida ordenada por tiempo (sin reservas: sort_unstable ordena in situ)
        self.out.sort_unstable_by(|a, b| a.time.cmp(&b.time).then(a.order.cmp(&b.order)).then(a.seq.cmp(&b.seq)));
                // Posición en beats para consumidores (ROLL por compases, GUI).
        // Preferir el reloj libre cuando no seguimos al host, para que standalone avance.
        let song_beats = {
            let host_beats = transport.song_pos_beats.filter(|_| host_playing);
            match host_beats {
                Some(b) if st.run_mode == 0 => b,
                _ => self.free_tick / f64::from(PPQ),
            }
        };
        let info = BlockInfo { step: cur_step, running, tempo, song_beats };
        (info, &self.out)
    }
}

impl Default for Scheduler {
    fn default() -> Self {
        Scheduler::new()
    }
}
