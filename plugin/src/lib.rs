//! CAJA Drum Machine (Rust): plugin CLAP **instrumento** que genera patrones de batería sincronizados
//! con el transporte del host y los toca con **sus propias voces de audio**: lee la carpeta de samples
//! del usuario (solo guarda la ruta), clasifica por celda y elige/modifica los sonidos con
//! aleatoriedad controlada por semillas (la lógica del preset de Bitwig Grid: Dice / semilla / Select).
//!
//! Hilo de audio (`process`): lee el transporte, ejecuta el scheduler del core, dispara voces y las
//! renderiza. No toma locks, no reserva memoria, no decodifica y no toca archivos (se comprueba con
//! `assert_process_allocs` en debug). Las muestras llegan ya decodificadas desde el hilo de carga por
//! ranuras sin bloqueos (`store.rs`); lo demás vive en el hilo principal / la interfaz / tareas de fondo.

use caja_core::model::Model;
use caja_core::scheduler::{InEvent, InKind, OutKind, RunSettings, Scheduler, Transport as CoreTransport};
use nih_plug::prelude::*;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex, RwLock};

mod dsp;
mod editor;
mod loader;
mod midi;
mod params;
mod sample;
mod store;
mod theme;

use dsp::Engine;
use loader::{LiveFn, LoaderHandle};
use midi::Mk3Bridge;
use params::CajaParams;
use store::{KitShared, NO_PAD};

/// Capacidad de los buffers de eventos de entrada (se reservan al crear el plugin).
const MAX_IN: usize = 4096;
const MAX_TRIG: usize = 4096;

#[derive(Clone, Copy, Debug)]
pub enum Task {
    /// Un parámetro cambió (automatización, host): regenerar y publicar el calendario.
    Sync,
    /// Botón de macro "Guardar en slot" del pad: semilla activa -> slot activo.
    SaveSlot(u8),
}

#[derive(Clone, Copy)]
struct Trig {
    time: u32,
    pad: u8,
    vel: f32,
}

pub struct Caja {
    params: Arc<CajaParams>,
    model: Arc<Model>,
    kit: Arc<KitShared>,
    sched: Scheduler,
    inbuf: Vec<InEvent>,
    trig: Vec<Trig>,
    last_snap: caja_core::session::ParamSnap,
    engine: Engine,
    loader: Option<LoaderHandle>,
    was_playing: bool,
    sample_rate: f32,
    /// Muestras que faltan para que el patrón vuelva a sonar tras la última nota de un clip.
    clip_hold: f64,
    prev_save: [bool; caja_core::kit::NUM_PADS],
    /// Puente Maschine Mikro MK3 (modos Pattern / Rack).
    mk3: Arc<std::sync::Mutex<Mk3Bridge>>,
}

impl Default for Caja {
    fn default() -> Self {
        let state = Arc::new(Mutex::new(String::new()));
        let kit_state = Arc::new(Mutex::new(String::new()));
        let names = Arc::new(RwLock::new(Vec::new()));
        let model = Arc::new(Model::new(state.clone(), names.clone()));
        let params = Arc::new(CajaParams::new(state, kit_state.clone(), names));
        let kit = KitShared::new(kit_state);
        let last_snap = params.snap();
        Self {
            params,
            model,
            kit,
            sched: Scheduler::new(),
            inbuf: Vec::with_capacity(MAX_IN),
            trig: Vec::with_capacity(MAX_TRIG),
            last_snap,
            engine: Engine::new(44100.0),
            loader: None,
            was_playing: false,
            sample_rate: 44100.0,
            clip_hold: 0.0,
            prev_save: [false; caja_core::kit::NUM_PADS],
            mk3: Arc::new(std::sync::Mutex::new(Mk3Bridge::default())),
        }
    }
}

impl Caja {
    fn publish_notes(model: &Model, kit: &KitShared) {
        let notes = model.session.lock().unwrap_or_else(|e| e.into_inner()).notes;
        kit.set_notes(&notes);
    }
}

impl Plugin for Caja {
    const NAME: &'static str = "CAJA Groove Pattern";
    const VENDOR: &'static str = "David Lanas";
    const URL: &'static str = "https://example.invalid/caja";
    const EMAIL: &'static str = "";

    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    // Instrumento: sin entrada de audio, salida estéreo principal.
    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    // Entrada MIDI: las notas del mapa de kit disparan los pads. Sin salida MIDI.
    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    /// LEDs del Mikro MK3 en modo nativo.
    const MIDI_OUTPUT: MidiConfig = MidiConfig::Basic;

    const SAMPLE_ACCURATE_AUTOMATION: bool = false;

    type SysExMessage = ();
    type BackgroundTask = Task;

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn task_executor(&mut self) -> TaskExecutor<Self> {
        let model = self.model.clone();
        let params = self.params.clone();
        let kit = self.kit.clone();
        Box::new(move |task| match task {
            Task::Sync => {
                model.sync(params.snap());
                kit.pull_persisted();
                Caja::publish_notes(&model, &kit);
            }
            Task::SaveSlot(pad) => {
                let pad = usize::from(pad).min(caja_core::kit::NUM_PADS - 1);
                let live = params.pads[pad].live();
                kit.edit(|k| k.pads[pad].bank[usize::from(live.slot).min(caja_core::kit::SLOTS - 1)] = live.seed);
            }
        })
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Box<dyn Editor>> {
        editor::create(self.params.clone(), self.model.clone(), self.kit.clone(), self.mk3.clone())
    }

    fn initialize(&mut self, _layout: &AudioIOLayout, config: &BufferConfig, _context: &mut impl InitContext<Self>) -> bool {
        self.sample_rate = config.sample_rate;
        self.sched.activate(f64::from(config.sample_rate));
        self.engine.set_sample_rate(config.sample_rate);
        // También recoge el estado que el host acaba de cargar en los campos persistentes.
        self.last_snap = self.params.snap();
        self.model.sync(self.last_snap);
        self.kit.pull_persisted();
        Caja::publish_notes(&self.model, &self.kit);
        let sr = config.sample_rate.round() as u32;
        match &self.loader {
            Some(l) => l.set_sample_rate(sr),
            None => {
                let params = self.params.clone();
                let live: LiveFn = Arc::new(move |pad| params.pads[pad.min(caja_core::kit::NUM_PADS - 1)].live());
                self.loader = LoaderHandle::spawn(self.kit.clone(), live, sr);
            }
        }
        true
    }

    fn reset(&mut self) {
        self.sched.reset();
        self.engine.reset();
        self.was_playing = false;
    }

    fn process(&mut self, buffer: &mut Buffer, _aux: &mut AuxiliaryBuffers, context: &mut impl ProcessContext<Self>) -> ProcessStatus {
        let frames = buffer.samples();
        let kit = self.kit.clone();
        let store = &kit.store;
        store.clock.fetch_add(1, Ordering::Relaxed);

        // 1. cambios de parámetros -> tarea de fondo (la sesión nunca se toca desde aquí)
        let snap = self.params.snap();
        if snap != self.last_snap {
            self.last_snap = snap;
            context.execute_background(Task::Sync);
        }

        // 2. parámetros por pad -> motor (semilla efectiva, modos)
        let ka = kit.audio.latest();
        let mut host = [(0u32, 0u8, 0u8); caja_core::kit::NUM_PADS];
        for (pad, h) in host.iter_mut().enumerate() {
            let l = self.params.pads[pad].live();
            *h = (l.seed, l.slot, l.mode);
        }
        let mut mute = [false; caja_core::kit::NUM_PADS];
        let mut solo = [false; caja_core::kit::NUM_PADS];
        let mut pan = [0.0f32; caja_core::kit::NUM_PADS];
        let mut gain = [1.0f32; caja_core::kit::NUM_PADS];
        let mut rate = [0u8; caja_core::kit::NUM_PADS];
        for pad in 0..caja_core::kit::NUM_PADS {
            let pp = &self.params.pads[pad];
            mute[pad] = pp.mute.value();
            solo[pad] = pp.solo.value();
            pan[pad] = pp.pan.value();
            gain[pad] = pp.gain.value().clamp(0.0, 2.0);
            rate[pad] = pp.rate.value().clamp(0, 3) as u8;
            let save = pp.save.value();
            if save && !self.prev_save[pad] {
                context.execute_background(Task::SaveSlot(pad as u8));
            }
            self.prev_save[pad] = save;
        }
        self.engine.set_mixer(&mute, &solo, &pan, &gain, &rate);
        self.engine.begin_block(ka, &host);

        // 3. eventos de entrada (dentro de la capacidad reservada: no hay reservas)
        self.inbuf.clear();
        self.trig.clear();
        let mut n_in = 0;
        let mut clip_notes = false;
        while let Some(ev) = context.next_event() {
            n_in += 1;
            if n_in > MAX_IN {
                continue;
            }
            // MK3: CCs de modo / navegación (no van al scheduler de notas)
            if let NoteEvent::MidiCC { cc, value, .. } = ev {
                if let Ok(mut m) = self.mk3.lock() {
                    m.handle_cc(cc, (value * 127.0).round().clamp(0.0, 127.0) as u8);
                }
                continue;
            }
            let kind = match ev {
                NoteEvent::NoteOn { note, velocity, timing, .. } if velocity > 0.0 => {
                    // ¿Lo consume el modo Pattern/Rack del MK3?
                    let consumed = self
                        .mk3
                        .lock()
                        .map(|mut m| m.handle_pad_note_on(note, (velocity * 127.0) as u8))
                        .unwrap_or(false);
                    if consumed {
                        InKind::Other
                    } else {
                        let fixed = self.mk3.lock().map(|m| m.fixed_vel).unwrap_or(false);
                        let final_vel = if fixed { 100.0 / 127.0 } else { velocity };
                        self.model.host.note_seen(note);
                        let pad = store_pad(&kit, note);
                        if pad != NO_PAD && self.trig.len() < MAX_TRIG {
                            self.trig.push(Trig { time: timing, pad, vel: final_vel });
                            clip_notes = true;
                        }
                        InKind::NoteOn { note }
                    }
                }
                // Un NoteOn con velocidad 0 es un NoteOff en MIDI clásico.
                NoteEvent::NoteOn { note, .. } | NoteEvent::NoteOff { note, .. } | NoteEvent::Choke { note, .. } => {
                    let consumed = self
                        .mk3
                        .lock()
                        .map(|mut m| m.handle_pad_note_off(note))
                        .unwrap_or(false);
                    if consumed {
                        InKind::Other
                    } else {
                        InKind::NoteOff { note }
                    }
                }
                _ => InKind::Other,
            };
            self.inbuf.push(InEvent { time: ev.timing(), kind });
        }

        // 4. notas de prueba de la interfaz: suenan directo (también en "solo drum machine")
        while let Some(v) = self.model.host.preview.pop() {
            let note = (v & 0x7f) as u8;
            let vel = f32::from(((v >> 8) & 0x7f) as u8) / 127.0;
            let pad = store_pad(&kit, note);
            if pad != NO_PAD && self.trig.len() < MAX_TRIG {
                self.trig.push(Trig { time: 0, pad, vel });
            }
        }

        // 5. patrón: scheduler puro del core (salvo en "solo drum machine")
        let tr = context.transport();
        let host_info = &self.model.host;
        // Reloj en beats para ROLL por compases (actualizado por el scheduler en modo libre).
        let mut clock_beats = -1.0_f64;
        if self.params.dm_only.value() {
            host_info.step.store(-1, Ordering::Relaxed);
            host_info.playing.store(false, Ordering::Relaxed);
            self.was_playing = false;
        } else {
            let transport = CoreTransport { playing: tr.playing, song_pos_beats: tr.pos_beats(), tempo: tr.tempo };
            let p = &*self.params;
            let st = RunSettings {
                run_mode: p.run_mode.value().clamp(0, 2) as u8,
                thru: false,
                vel_scale: p.vel_scale.value(),
                cc_num: 0,
                free_bpm: p.bpm.value(),
                channel: (p.channel.value().clamp(1, 16) - 1) as u8,
                audition: host_info.audition.load(Ordering::Relaxed),
            };
            let sch = self.model.schedule.latest();
            let (info, out) = self.sched.process(frames as u32, &transport, &st, sch, &self.inbuf, &[]);
            clock_beats = info.song_beats;
            host_info.step.store(if info.running { info.step } else { -1 }, Ordering::Relaxed);
            host_info.playing.store(info.running, Ordering::Relaxed);
            host_info.set_tempo(info.tempo as f32);
            // ROLL reproducible: el dado vuelve a su semilla al arrancar el patrón
            if info.running && !self.was_playing && ka.roll_repro {
                self.engine.reseed_roll(u64::from(ka.roll_seed));
            }
            self.was_playing = info.running;
            // Prioridad al clip: si un clip está tocando las notas de los pads, el patrón calla
            // (así no suenan a la vez). Dura hasta 1 compás después de la última nota del clip.
            // No aplica al modo "Con una tecla" (ahí las notas de entrada son el disparador).
            let bpm = if p.run_mode.value() == 0 {
                tr.tempo.unwrap_or(f64::from(p.bpm.value())).max(20.0)
            } else {
                f64::from(p.bpm.value()).max(20.0)
            };
            let hold_samples = 4.0 * 60.0 / bpm * f64::from(self.sample_rate);
            self.clip_hold = next_clip_hold(self.clip_hold, clip_notes && p.run_mode.value() != 2 && p.clip_prio.value(), hold_samples, frames, info.running);
            let muted_by_clip = self.clip_hold > 0.0 && !host_info.audition.load(Ordering::Relaxed);
            for e in out {
                if muted_by_clip {
                    break;
                }
                if let OutKind::NoteOn { note, vel, .. } = e.kind {
                    let pad = store_pad(&kit, note);
                    if pad != NO_PAD && self.trig.len() < MAX_TRIG {
                        self.trig.push(Trig { time: e.time, pad, vel });
                    }
                }
            }
        }

        // 6. audio: segmentos entre golpes, con precisión de muestra
        self.trig.sort_unstable_by_key(|t| t.time);
        let gain = util::db_to_gain(self.params.master.value());
        let chans = buffer.as_slice();
        let (first, rest) = chans.split_at_mut(1);
        let (l, r) = (&mut *first[0], &mut *rest[0]);
        l.fill(0.0);
        r.fill(0.0);
        // Posición en beats para ROLL por compases.
        // En modo libre (Siempre / tecla) usa el free_tick del scheduler (standalone avanza).
        // En modo transporte, la posición del host si existe.
        let run_mode = self.params.run_mode.value();
        let base_beats = if run_mode == 0 {
            tr.pos_beats().unwrap_or(clock_beats)
        } else {
            clock_beats
        };
        let tempo = if run_mode == 0 {
            tr.tempo.unwrap_or(f64::from(self.params.bpm.value())).max(20.0)
        } else {
            f64::from(self.params.bpm.value()).max(20.0)
        };
        let beats_per_sample = (tempo / 60.0) / f64::from(self.sample_rate);
        let mut pos = 0usize;
        for i in 0..self.trig.len() {
            let t = self.trig[i];
            let at = (t.time as usize).min(frames);
            if at > pos {
                self.engine.render(&mut l[pos..at], &mut r[pos..at]);
                pos = at;
            }
            let song_beats = if base_beats >= 0.0 {
                base_beats + f64::from(t.time) * beats_per_sample
            } else {
                -1.0
            };
            self.engine.trigger(usize::from(t.pad), t.vel, ka, store, song_beats);
        }
        if pos < frames {
            self.engine.render(&mut l[pos..], &mut r[pos..]);
        }
        // Master gain + Soft-clipper (tanh) para evitar distorsión digital
        for s in l.iter_mut().chain(r.iter_mut()) {
            *s = (*s * gain).tanh();
        }

        // 7. LEDs MK3 (modo + cola de cambios)
        {
            let enabled = self.params.mk3_enabled.value();
            let playing = self.model.host.playing.load(Ordering::Relaxed);
            if let Ok(mut m) = self.mk3.lock() {
                if m.enabled != enabled {
                    m.set_enabled(enabled);
                }
                if enabled {
                    // LEDs de modo desde audio (no depende de que la GUI repinte)
                    m.pulse_mode_leds(playing);
                }
                for msg in m.drain_out(48) {
                    match msg {
                        crate::midi::Mk3Out::PadLed { note, color } => {
                            let _ = context.send_event(NoteEvent::NoteOn {
                                timing: 0,
                                voice_id: None,
                                channel: 0,
                                note,
                                velocity: f32::from(color) / 127.0,
                            });
                        }
                        crate::midi::Mk3Out::ButtonLed { cc, value } => {
                            let _ = context.send_event(NoteEvent::MidiCC {
                                timing: 0,
                                channel: 0,
                                cc,
                                value: f32::from(value) / 127.0,
                            });
                        }
                    }
                }
            }
        }

        ProcessStatus::Normal
    }
}

/// Muestras que faltan para que el patrón vuelva a sonar tras un clip: se recarga con cada nota de
/// clip, baja con el tiempo y se anula al parar el patrón.
fn next_clip_hold(hold: f64, clip_note_now: bool, hold_samples: f64, frames: usize, running: bool) -> f64 {
    if !running {
        0.0
    } else if clip_note_now {
        hold_samples
    } else {
        (hold - frames as f64).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pattern_stays_silent_while_a_clip_plays_and_resumes_after_one_bar() {
        let hold = 4.0 * 60.0 / 120.0 * 48000.0; // 1 compás a 120 bpm
        let mut h = 0.0;
        h = next_clip_hold(h, true, hold, 512, true);
        assert_eq!(h, hold);
        // sin notas nuevas baja con el tiempo y sigue callado
        for _ in 0..10 {
            h = next_clip_hold(h, false, hold, 512, true);
            assert!(h > 0.0);
        }
        // una nota nueva lo recarga
        assert_eq!(next_clip_hold(h, true, hold, 512, true), hold);
        // al pasar un compás sin notas, el patrón vuelve
        let mut h = hold;
        for _ in 0..(hold as usize / 512 + 2) {
            h = next_clip_hold(h, false, hold, 512, true);
        }
        assert_eq!(h, 0.0);
        // al parar el patrón se anula
        assert_eq!(next_clip_hold(hold, true, hold, 512, false), 0.0);
    }
}

#[inline]
fn store_pad(kit: &KitShared, note: u8) -> u8 {
    kit.note_pad[usize::from(note & 0x7f)].load(Ordering::Relaxed)
}

impl ClapPlugin for Caja {
    // ID nuevo: no choca con la variante note-effect (`com.davidlanas.caja-notefx-rs`).
    const CLAP_ID: &'static str = "com.davidlanas.caja-groove-pattern-rs";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Drum machine con samples propios: patrones de bateria y semillas estilo Bitwig Grid");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[ClapFeature::Instrument, ClapFeature::Drum, ClapFeature::DrumMachine, ClapFeature::Stereo];
}

#[cfg(feature = "vst3")]
impl Vst3Plugin for Caja {
    const VST3_CLASS_ID: [u8; 16] = *b"CajaGroovePatRs1";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] = &[Vst3SubCategory::Instrument, Vst3SubCategory::Drum];
}

nih_export_clap!(Caja);
#[cfg(feature = "vst3")]
nih_export_vst3!(Caja);
