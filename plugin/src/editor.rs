//! Interfaz 8 bits (mismo estilo que ChordGlow): todo se dibuja a mano con rectángulos de píxeles
//! enteros y la fuente Press Start 2P (SIL OFL 1.1, ver `assets/OFL-PressStart2P.txt`): sin
//! esquinas redondeadas, sin suavizado, bordes gruesos. Solo palabras: nada depende de símbolos.
//!
//! De arriba abajo: título y ajustes, frase + "Generar" + atajos, sugerencias de género
//! (ocultas hasta pulsar el botón), piano roll (grave abajo, como Bitwig), kit, controles,
//! exportar, historial y línea "Lectura".

use crate::midi::{Mk3Bridge, Mk3Cmd, Mk3LedState, Mk3Mode};
use crate::params::*;
use crate::theme::Theme;
use crate::store::KitShared;
use caja_core::model::Model;
use caja_core::pattern::{Pattern, STEPS_PER_BAR};
use caja_core::kit::{is_lock, ALL_ROLL, ALL_LOCK, P_SAMPLE, NUM_PADS};
use caja_core::seedrand::splitmix64;
use std::time::{SystemTime, UNIX_EPOCH};
use caja_core::pieces::*;
use caja_core::session::{ParamSnap, BARS_CHOICES};
use nih_plug::prelude::*;
#[cfg(test)]
use nih_plug::context::gui::GuiContext;
use nih_plug_egui::egui::{self, pos2, vec2, Align2, Color32, FontId, Painter, Pos2, Rect, Sense};
use nih_plug_egui::{create_egui_editor, resizable_window::ResizableWindow};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

mod kitview;

const PAD_X: f32 = 8.0;
const PAD_Y: f32 = 5.0;
const GAP: f32 = 6.0;

/// Frases de atajo: un clic las escribe y genera.
const SHORTCUTS: [&str; 8] = [
    "neo soul 78 bpm ghost notes 4 compases",
    "techno 132 bpm con fill 4 compases",
    "boom bap 90 bpm shuffle verso",
    "reggaeton coro 4 compases",
    "salsa clave 3-2 180 bpm",
    "blues shuffle lento",
    "drum and bass 174 bpm minimal",
    "disco 118 bpm busy",
];

// ───────────────────────────── Estado de la ventana ─────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
enum Focus {
    None,
    Phrase,
    MapName,
    Path,
    Seed,
    /// Nombre corto del patrón activo del rack.
    PatName,
}

#[derive(Clone, Copy)]
struct Drag {
    piece: usize,
    step: usize,
    start_vel: i32,
    existed: bool,
}

struct Card {
    genre: String,
    bpm: i32,
    bars: usize,
    phrase: String,
}

/// Copia ligera de lo que la interfaz necesita de la sesión en este fotograma.
struct View {
    pattern: Pattern,
    notes: [u8; NUM_PIECES],
    reading: String,
    families: Vec<(String, Vec<(usize, String, bool)>)>,
    genre_idx: usize,
    history: Vec<Card>,
    maps: Vec<String>,
    map_index: usize,
    learn_active: bool,
    learn_piece: usize,
    errors: Vec<String>,
    message: String,
    origins: String,
    has_last_export: bool,
    /// Índice activo del rack (0..31).
    pattern_active: usize,
    /// Ranura con contenido (no vacía).
    pattern_filled: [bool; 32],
    /// Nombres cortos (vacío = sin etiqueta).
    pattern_names: [String; 32],
}

struct Gui {
    model: Arc<Model>,
    kit: Arc<KitShared>,
    mk3: Arc<std::sync::Mutex<Mk3Bridge>>,
    tab: usize,
    path: String,
    seed_text: String,
    sel_pad: usize,
    sel_unclassified: Option<String>,
    pick: Arc<std::sync::Mutex<Option<Result<String, String>>>>,
    note: String,
    phrase: String,
    map_name: String,
    /// Nombre corto editable del patrón activo.
    pat_name: String,
    focus: Focus,
    page: usize,
    selected: usize,
    show_all: bool,
    show_suggestions: bool,
    format1: bool,
    use_gm: bool,
    note_counter: u32,
    drag: Option<Drag>,
    status: String,
    panel_fill: Option<Color32>,
    /// Banco visible del selector de patrones: 0 = A (0..15), 1 = B (16..31).
    pattern_bank: usize,
    /// Overlay de ayuda MK3 / mapeo.
    show_help: bool,
    /// Compás visible en la pantalla RACK (0 .. bars-1).
    rack_bar: usize,
}

impl Gui {
    fn new(model: Arc<Model>, kit: Arc<KitShared>, mk3: Arc<std::sync::Mutex<Mk3Bridge>>) -> Self {
        let (counter, _) = model.host.last_note();
        Gui {
            model,
            kit,
            mk3,
            tab: 0,
            path: String::new(),
            seed_text: String::new(),
            sel_pad: 0,
            sel_unclassified: None,
            pick: Arc::new(std::sync::Mutex::new(None)),
            note: String::new(),
            phrase: String::new(),
            map_name: String::new(),
            pat_name: String::new(),
            focus: Focus::None,
            page: 0,
            selected: 0,
            show_all: false,
            show_suggestions: false,
            format1: false,
            use_gm: false,
            note_counter: counter,
            drag: None,
            status: String::new(),
            panel_fill: None,
            pattern_bank: 0,
            show_help: false,
            rack_bar: 0,
        }
    }

    /// Publica el estado de LEDs del MK3 (la salida MIDI real ocurre en process).
    fn push_mk3_leds(&mut self, view: &View, p: &CajaParams) {
        if !p.mk3_enabled.value() { return; }
        let mode = self.mk3.lock().map(|m| m.mode()).unwrap_or_default();
        let bank = self.pattern_bank.min(1) as u8;
        let mut filled: u16 = 0;
        let base = bank as usize * 16;
        for i in 0..16 { if view.pattern_filled[base + i] { filled |= 1 << i; } }
        let first = self.rack_bar * STEPS_PER_BAR;
        let mut rack_steps = [0u8; 16];
        let pi = self.selected.min(NUM_PIECES - 1);
        for i in 0..16 {
            let st = first + i;
            if st < view.pattern.steps() {
                let c = view.pattern.cell[pi][st];
                if c.on() { rack_steps[i] = c.vel; }
            }
        }
        let step_now = self.model.host.step.load(Ordering::Relaxed);
        let playing = self.model.host.playing.load(Ordering::Relaxed) && step_now >= 0;
        let playhead = if playing && step_now as usize / STEPS_PER_BAR == self.rack_bar { (step_now as usize % STEPS_PER_BAR) as u8 } else { 255 };
        
        let mut pad_mutes = [false; NUM_PADS];
        let mut pad_locks = [0u8; NUM_PADS];
        for i in 0..NUM_PADS {
            pad_mutes[i] = p.pads[i].mute.value();
            pad_locks[i] = p.pads[i].mode.value() as u8;
        }
        
        let st = Mk3LedState {
            mode, pattern_bank: bank, pattern_active: view.pattern_active as u8,
            pattern_filled: filled, rack_steps, playhead, playing, pad_mutes, pad_locks,
        };
        if let Ok(mut m) = self.mk3.lock() { m.update_leds(&st); }
    }

    /// Aplica comandos del Mikro MK3 (Pattern / Rack / Mixer / Samples / Library) en el hilo de la GUI.
    fn drain_mk3(&mut self, setter: &ParamSetter, p: &CajaParams) {
        let cmds = self.mk3.lock().map(|m| m.drain(32)).unwrap_or_default();
        if cmds.is_empty() { return; }
        let snap = p.snap();
        for cmd in cmds {
            match cmd {
                Mk3Cmd::SelectPattern(idx) => {
                    let n = self.model.with(snap, |s| s.select_pattern(idx as usize));
                    apply_snap(setter, p, n);
                    self.pattern_bank = (idx as usize) / 16;
                    self.tab = 0;
                    self.status = format!("MK3 Patron {}{}", if idx < 16 { "A" } else { "B" }, (idx % 16) + 1);
                }
                Mk3Cmd::RackBarDelta(d) => {
                    let bars = self.model.with(snap, |s| s.pattern.bars.max(1));
                    if d < 0 && self.rack_bar > 0 { self.rack_bar -= 1; }
                    else if d > 0 && self.rack_bar + 1 < bars { self.rack_bar += 1; }
                    self.tab = 3;
                }
                Mk3Cmd::SelectRow(pad) => {
                    let pi = PAD_ORDER.get(pad as usize).copied().unwrap_or(0);
                    self.selected = pi; self.tab = 3;
                    self.status = format!("Fila: {}", PIECES[pi].name);
                }
                Mk3Cmd::ToggleStep { piece, step } => {
                    let pi = if piece == 255 { self.selected } else { PAD_ORDER.get(piece as usize).copied().unwrap_or(self.selected) };
                    let st = self.rack_bar * STEPS_PER_BAR + (step as usize % STEPS_PER_BAR);
                    self.model.with(snap, |s| { if st < s.pattern.steps() { s.toggle_cell(pi, st, false); } });
                    self.selected = pi; self.tab = 3;
                }
                Mk3Cmd::ToggleGhost { step } => {
                    let pi = self.selected;
                    let st = self.rack_bar * STEPS_PER_BAR + (step as usize % STEPS_PER_BAR);
                    self.model.with(snap, |s| { if st < s.pattern.steps() { s.toggle_ghost(pi, st); } });
                    self.tab = 3;
                }
                Mk3Cmd::SetBank(b) => {
                    self.pattern_bank = (b as usize).min(1); self.tab = 0;
                    self.status = if b == 0 { "Banco A".into() } else { "Banco B".into() };
                }
                Mk3Cmd::BpmDelta(d) => {
                    let next = (p.bpm.value() + i32::from(d)).clamp(40, 240);
                    set(setter, &p.bpm, next); self.status = format!("BPM {next}");
                }
                Mk3Cmd::MasterDelta(d) => {
                    let next = (p.master.value() + f32::from(d) * 0.5).clamp(-24.0, 6.0);
                    set(setter, &p.master, next); self.status = format!("Master {next:.1} dB");
                }
                Mk3Cmd::SwingDelta(d) => { let next = (p.swing.value() + i32::from(d) * 2).clamp(0, 100); set(setter, &p.swing, next); }
                Mk3Cmd::AccentDelta(d) => { let next = (p.accent.value() + i32::from(d) * 2).clamp(0, 100); set(setter, &p.accent, next); }
                Mk3Cmd::HumanDelta(d) => { let next = (p.human.value() + i32::from(d) * 2).clamp(0, 100); set(setter, &p.human, next); }
                Mk3Cmd::SelectCell(pad) => { self.sel_pad = pad.min(NUM_PADS - 1); self.tab = 2; }
                Mk3Cmd::CellVolAbsolute(v) => {
                    let gain = (v as f32 / 127.0) * 2.0;
                    set(setter, &p.pads[self.sel_pad].gain, gain);
                    self.kit.edit(|k| k.pads[self.sel_pad].gain = (gain * 1000.0).round() as u16);
                }
                Mk3Cmd::CellPanDelta(d) => {
                    let next = (p.pads[self.sel_pad].pan.value() + f32::from(d) * 0.05).clamp(-1.0, 1.0);
                    set(setter, &p.pads[self.sel_pad].pan, next);
                    self.kit.edit(|k| k.pads[self.sel_pad].pan = (next * 1000.0).round() as i16);
                }
                Mk3Cmd::ToggleCellMute(pad) => {
                    let pad = pad.min(NUM_PADS - 1);
                    let muted = p.pads[pad].mute.value();
                    set(setter, &p.pads[pad].mute, !muted);
                    self.kit.edit(|k| k.pads[pad].mute = !muted);
                }
                Mk3Cmd::SamplesDice(pad) => {
                    let pad = pad.min(NUM_PADS - 1);
                    let n = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
                    let seed = (splitmix64(n ^ (pad as u64).wrapping_mul(0x9E37_79B9)) % 100000) as u32;
                    self.set_seed(setter, p, pad, seed);
                }
                Mk3Cmd::SamplesSave(pad) => { self.pin_pad(p, pad.min(NUM_PADS - 1)); }
                Mk3Cmd::ToggleCellLockBit(pad, bit) => {
                    let pad = pad.min(NUM_PADS - 1);
                    let live = p.pads[pad].live();
                    let lock = is_lock(live.mode, bit as usize);
                    let new_mode = if lock { live.mode & !(1u8 << bit) } else { live.mode | (1u8 << bit) };
                    set(setter, &p.pads[pad].mode, i32::from(new_mode));
                }
                Mk3Cmd::GlobalAction(action) => {
                    match action {
                        0 => { let n = self.model.with(snap, |s| s.reroll(snap, true)); apply_snap(setter, p, n); self.status = "Regenerar".into(); }
                        1 => { self.do_generate(setter, p); self.status = "Generar".into(); }
                        2 => { self.status = "Tap Tempo".into(); }
                        3 => { let next = (p.run_mode.value() + 1) % 3; set(setter, &p.run_mode, next); }
                        4 => { for q in 0..NUM_PADS { set(setter, &p.pads[q].mode, i32::from(ALL_ROLL)); } self.status = "All ROLL".into(); }
                        5 => { for q in 0..NUM_PADS { self.pin_pad(p, q); set(setter, &p.pads[q].mode, i32::from(ALL_LOCK)); } self.status = "All LOCK".into(); }
                        6 => { for q in 0..NUM_PADS { set(setter, &p.pads[q].rate, 3); } self.status = "Todos 4c".into(); }
                        7 => { for q in 0..NUM_PADS { set(setter, &p.pads[q].rate, 0); } self.status = "Todos golpe".into(); }
                        8 => { set(setter, &p.run_mode, 1); self.status = "Modo: Siempre".into(); }
                        9 => {
                            let playing = self.model.host.playing.load(Ordering::Relaxed);
                            let next = if playing { 0 } else { 1 };
                            set(setter, &p.run_mode, next);
                            self.status = if playing { "Detenido".into() } else { "Reproduciendo".into() };
                        }
                        10 => { set(setter, &p.run_mode, 0); self.status = "Detenido".into(); }
                        _ => {}
                    }
                }
                Mk3Cmd::OpenExplorer => { self.pick_folder(); }
                Mk3Cmd::LibrarySetting(row, col) => {
                    match row {
                        0 => { let caps = [8, 16, 32, 64]; self.kit.edit(|k| k.bank_cap = caps[col as usize]); }
                        1 => { let lens = [1000, 3000, 6000, 10000]; self.kit.edit(|k| k.max_len_ms = lens[col as usize]); self.kit.rescan.store(true, Ordering::Release); }
                        2 => { let rams = [128, 256, 512, 1024]; self.kit.edit(|k| k.ram_cap_mb = rams[col as usize]); }
                        3 => { let pools = [8, 16, 32, 64]; self.kit.edit(|k| k.pool = pools[col as usize]); }
                        _ => {}
                    }
                }
                Mk3Cmd::ShiftPadAction(pad) => {
                    match pad {
                        0 => { self.model.reload_files(snap); self.status = "Recargar".into(); }
                        1 => { set(setter, &p.palette, if p.palette.value() == PaletteParam::Arcade { PaletteParam::ColorBlind } else { PaletteParam::Arcade }); }
                        2 => { set(setter, &p.dark, !p.dark.value()); }
                        3 => { set(setter, &p.scanlines, !p.scanlines.value()); }
                        4 => { set(setter, &p.text_size, match p.text_size.value() { TextSizeParam::Small => TextSizeParam::Medium, TextSizeParam::Medium => TextSizeParam::Large, TextSizeParam::Large => TextSizeParam::Small }); }
                        5 => { let next = !p.mk3_enabled.value(); set(setter, &p.mk3_enabled, next); if let Ok(mut m) = self.mk3.lock() { m.set_enabled(next); } }
                        _ => {}
                    }
                }
                Mk3Cmd::ToggleShowAll => { self.show_all = !self.show_all; }
                Mk3Cmd::ToggleSuggestions => { self.show_suggestions = !self.show_suggestions; }
                Mk3Cmd::FocusPhrase => { self.focus = Focus::Phrase; self.tab = 0; }
                Mk3Cmd::SelectGenre(idx) => {
                    let n = self.model.with(snap, |s| s.set_genre(idx, snap));
                    apply_snap(setter, p, n);
                }
                Mk3Cmd::SetMode(mode) => {
                    match mode {
                        Mk3Mode::Mixer => { self.tab = 2; self.status = "MK3: modo Mixer".into(); }
                        Mk3Mode::Samples => { self.tab = 2; self.status = "MK3: modo Samples".into(); }
                        Mk3Mode::Library => { self.tab = 1; self.status = "MK3: modo Libreria".into(); }
                        Mk3Mode::Pattern => { self.tab = 0; self.status = "MK3: modo Pattern".into(); }
                        Mk3Mode::Rack => { self.tab = 3; self.status = "MK3: modo Rack".into(); }
                        Mk3Mode::Perform => { self.tab = 0; self.status = "MK3: modo Perform".into(); }
                    }
                }
            }
        }
    }

    /// Lee la sesión (y la sincroniza con los parámetros del host) una vez por fotograma.
    fn view(&mut self, snap: ParamSnap) -> View {
        let model = self.model.clone();
        model.with(snap, |s| {
            let mut errors: Vec<String> = Vec::new();
            if let Some(e) = &s.state_error {
                errors.push(e.clone());
            }
            errors.extend(s.genre_errors.iter().map(|e| format!("genres.json: {e}")));
            errors.extend(s.maps_errors.iter().map(|e| format!("mapas: {e}")));
            View {
                pattern: s.pattern.clone(),
                notes: s.notes,
                reading: s.reading(),
                families: s.genres.families().into_iter().map(|(f, v)| (f, v.into_iter().map(|i| (i, s.genres.genres[i].name.clone(), s.genres.genres[i].proposal)).collect())).collect(),
                genre_idx: s.genre_index(&s.snap()),
                history: s.history.iter().map(|h| Card { genre: s.genres.get(h.p.genre.max(0) as usize).name.clone(), bpm: h.p.bpm, bars: caja_core::session::bars_from_idx(h.p.bars_idx), phrase: h.phrase.clone() }).collect(),
                maps: s.maps.maps.iter().map(|m| m.name.clone()).collect(),
                map_index: s.map_index,
                learn_active: s.learn.active,
                learn_piece: s.learn.piece,
                errors,
                message: s.message.clone(),
                origins: format!("generos: {} · mapas: {}", s.genre_origin, s.maps_origin),
                has_last_export: s.last_export.is_some(),
                pattern_active: s.rack.active,
                pattern_filled: std::array::from_fn(|i| !s.rack.entries[i].is_empty()),
                pattern_names: std::array::from_fn(|i| s.rack.entries[i].name.clone()),
            }
        })
    }
}

pub fn create(
    params: Arc<CajaParams>,
    model: Arc<Model>,
    kit: Arc<KitShared>,
    mk3: Arc<std::sync::Mutex<Mk3Bridge>>,
) -> Option<Box<dyn Editor>> {
    let state = params.editor_state.clone();
    create_egui_editor(
        state,
        Gui::new(model, kit, mk3),
        |ctx, _gui| setup(ctx),
        move |ctx, setter, gui| frame(ctx, setter, gui, &params),
    )
}

fn setup(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("pixel".to_owned(), Arc::new(egui::FontData::from_static(include_bytes!("../assets/PressStart2P-Regular.ttf"))));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().insert(0, "pixel".to_owned());
    }
    ctx.set_fonts(fonts);
    // Píxeles duros: sin el degradado de 1 px que egui añade a los bordes.
    ctx.tessellation_options_mut(|o| o.feathering = false);
}

fn frame(ctx: &egui::Context, setter: &ParamSetter, gui: &mut Gui, p: &CajaParams) {
    let th = Theme::new(p.palette.value(), p.dark.value());
    if gui.panel_fill != Some(th.bg) {
        ctx.style_mut(|s| s.visuals.panel_fill = th.bg);
        gui.panel_fill = Some(th.bg);
    }
    // ~30 fps: suficiente para el cabezal y los controles.
    ctx.request_repaint_after(Duration::from_millis(33));

    let fs = p.text_size.value().px();
    ResizableWindow::new("caja-window").min_size(vec2(760.0, 520.0)).show(ctx, p.editor_state.as_ref(), |ui| {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| gui.paint(ui, setter, p, &th, fs));
    });
}

// ───────────────────────────── Utilidades de dibujo ─────────────────────────────

fn snap_rect(r: Rect) -> Rect {
    Rect::from_min_max(pos2(r.min.x.round(), r.min.y.round()), pos2(r.max.x.round(), r.max.y.round()))
}

/// Pincel con la paleta y el tamaño de texto actuales.
struct Cv<'a> {
    p: &'a Painter,
    th: Theme,
    fs: f32,
}

impl Cv<'_> {
    fn font(&self) -> FontId {
        FontId::proportional(self.fs)
    }

    fn fill(&self, r: Rect, c: Color32) {
        self.p.rect_filled(snap_rect(r), 0.0_f32, c);
    }

    /// Caja con borde grueso (el borde se dibuja como un rectángulo más grande debajo).
    fn boxed(&self, r: Rect, fill: Color32, border: Color32, thick: f32) {
        self.fill(r, border);
        self.fill(r.shrink(thick), fill);
    }

    fn text(&self, pos: Pos2, anchor: Align2, s: &str, c: Color32) -> Rect {
        self.p.text(pos, anchor, s, self.font(), c)
    }

    fn text_w(&self, s: &str) -> f32 {
        self.p.layout_no_wrap(s.to_string(), self.font(), Color32::WHITE).size().x
    }

    /// Texto con salto de línea automático. Devuelve el alto usado.
    fn wrapped(&self, pos: Pos2, width: f32, s: &str, c: Color32) -> f32 {
        if s.is_empty() {
            return 0.0;
        }
        let galley = self.p.layout(s.to_string(), self.font(), c, width.max(8.0));
        let h = galley.size().y;
        self.p.galley(pos, galley, c);
        h
    }

    fn clipped(&self, clip: Rect) -> Painter {
        self.p.with_clip_rect(clip)
    }
}

/// Coloca elementos de izquierda a derecha y salta de fila al llegar al borde.
struct Flow {
    left: f32,
    right: f32,
    x: f32,
    y: f32,
    row_h: f32,
}

impl Flow {
    fn new(left: f32, y: f32, right: f32) -> Self {
        Flow { left, right, x: left, y, row_h: 0.0 }
    }

    fn place(&mut self, w: f32, h: f32) -> Rect {
        if self.x > self.left && self.x + w > self.right {
            self.newline();
        }
        let r = Rect::from_min_size(pos2(self.x, self.y), vec2(w, h));
        self.x += w + GAP;
        self.row_h = self.row_h.max(h);
        r
    }

    fn newline(&mut self) {
        self.x = self.left;
        self.y += self.row_h + GAP;
        self.row_h = 0.0;
    }

    fn bottom(&self) -> f32 {
        self.y + self.row_h
    }
}

fn button_h(cv: &Cv) -> f32 {
    cv.fs + 2.0 * PAD_Y + 2.0
}

fn button_at(ui: &egui::Ui, cv: &Cv, r: Rect, id: &str, label: &str, on: bool) -> bool {
    let resp = ui.interact(snap_rect(r), ui.id().with(id), Sense::click());
    let th = &cv.th;
    let (fill, border, txt) = if on {
        (th.accent, th.border, th.bg)
    } else if resp.hovered() {
        (th.panel, th.accent, th.text)
    } else {
        (th.panel, th.text_dim, th.text)
    };
    cv.boxed(r, fill, border, 2.0);
    cv.text(r.center(), Align2::CENTER_CENTER, label, txt);
    resp.clicked()
}

/// Botón de género: el nombre y, si es una propuesta, el aviso escrito "propuesta" en tono apagado.
fn button_tag(ui: &egui::Ui, cv: &Cv, r: Rect, id: &str, name: &str, tag: &str, on: bool) -> bool {
    let resp = ui.interact(snap_rect(r), ui.id().with(id), Sense::click());
    let th = &cv.th;
    let (fill, border, txt, dim) = if on {
        (th.accent, th.border, th.bg, th.bg)
    } else if resp.hovered() {
        (th.panel, th.accent, th.text, th.text_dim)
    } else {
        (th.panel, th.text_dim, th.text, th.text_dim)
    };
    cv.boxed(r, fill, border, 2.0);
    let nw = cv.text_w(name);
    let tw = if tag.is_empty() { 0.0 } else { cv.text_w(tag) };
    let x = r.center().x - (nw + tw) / 2.0;
    cv.text(pos2(x, r.center().y), Align2::LEFT_CENTER, name, txt);
    if !tag.is_empty() {
        cv.text(pos2(x + nw, r.center().y), Align2::LEFT_CENTER, tag, dim);
    }
    resp.clicked()
}

fn button(ui: &egui::Ui, cv: &Cv, flow: &mut Flow, id: &str, label: &str, on: bool) -> bool {
    let w = cv.text_w(label) + 2.0 * PAD_X;
    let r = flow.place(w, button_h(cv));
    button_at(ui, cv, r, id, label, on)
}

fn row_label(cv: &Cv, flow: &mut Flow, label: &str) {
    let w = cv.text_w(label) + PAD_X;
    let r = flow.place(w, button_h(cv));
    cv.text(pos2(r.min.x, r.center().y), Align2::LEFT_CENTER, label, cv.th.text_dim);
}

fn set<P: Param>(setter: &ParamSetter, param: &P, value: P::Plain) {
    setter.begin_set_parameter(param);
    setter.set_parameter(param, value);
    setter.end_set_parameter(param);
}

/// Aplica al host los 8 valores que devolvió una acción de la sesión.
fn apply_snap(setter: &ParamSetter, p: &CajaParams, n: ParamSnap) {
    let o = p.snap();
    if n.genre != o.genre {
        set(setter, &p.genre, n.genre);
    }
    if n.bars_idx != o.bars_idx {
        set(setter, &p.bars, n.bars_idx);
    }
    if n.swing != o.swing {
        set(setter, &p.swing, n.swing);
    }
    if n.human != o.human {
        set(setter, &p.human, n.human);
    }
    if n.density != o.density {
        set(setter, &p.density, n.density);
    }
    if n.seed != o.seed {
        set(setter, &p.seed, n.seed);
    }
    if n.accent != o.accent {
        set(setter, &p.accent, n.accent);
    }
    if n.bpm != o.bpm {
        set(setter, &p.bpm, n.bpm);
    }
}

/// Control deslizante dibujado a mano para un `IntParam`. Devuelve el alto usado.
fn slider(ui: &egui::Ui, cv: &Cv, setter: &ParamSetter, r: Rect, id: &str, label: &str, param: &IntParam, lo: i32, hi: i32, value_text: &str) {
    let label_w = r.width() * 0.42;
    cv.text(pos2(r.min.x, r.center().y), Align2::LEFT_CENTER, label, cv.th.text_dim);
    let track = Rect::from_min_max(pos2(r.min.x + label_w, r.min.y), r.max);
    let resp = ui.interact(snap_rect(track), ui.id().with(id), Sense::click_and_drag());
    if resp.drag_started() {
        setter.begin_set_parameter(param);
    }
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let f = ((pos.x - track.min.x - 2.0) / (track.width() - 4.0)).clamp(0.0, 1.0);
            let v = lo + (f * (hi - lo) as f32).round() as i32;
            if resp.clicked() && !resp.dragged() {
                setter.begin_set_parameter(param);
                setter.set_parameter(param, v);
                setter.end_set_parameter(param);
            } else {
                setter.set_parameter(param, v);
            }
        }
    }
    if resp.drag_stopped() {
        setter.end_set_parameter(param);
    }
    let th = &cv.th;
    cv.boxed(track, th.panel, if resp.hovered() || resp.dragged() { th.accent } else { th.text_dim }, 2.0);
    let f = if hi > lo { (param.value() - lo) as f32 / (hi - lo) as f32 } else { 0.0 };
    let inner = track.shrink(4.0);
    cv.fill(Rect::from_min_size(inner.min, vec2((inner.width() * f).round().max(0.0), inner.height())), th.accent);
    cv.text(track.center(), Align2::CENTER_CENTER, value_text, if f > 0.5 { th.bg } else { th.text });
}

// ───────────────────────────── Dibujo principal ─────────────────────────────

impl Gui {
    fn paint(&mut self, ui: &mut egui::Ui, setter: &ParamSetter, p: &CajaParams, th: &Theme, fs: f32) {
        let area = ui.available_rect_before_wrap();
        let painter = ui.painter().clone();
        let cv = Cv { p: &painter, th: *th, fs };
        // Chasis MPC: fondo + bisel exterior
        cv.fill(area, th.chassis);
        cv.boxed(area.shrink(2.0), th.face, th.chassis_edge, 3.0);
        let left = area.min.x + 12.0;
        let right = (area.max.x - 12.0).max(left + 300.0);
        let mut y = area.min.y + 10.0;
        let snap = p.snap();
        // Comandos del Mikro MK3 encolados en el hilo de audio
        self.drain_mk3(setter, p);
        let view = self.view(snap);
        self.push_mk3_leds(&view, p);
        // drum machine: estado cargado por el host y mapa de notas actual (barato; sin audio)
        self.kit.pull_persisted();
        self.kit.set_notes(&view.notes);

        self.keyboard(ui, setter, p, &view);
        self.learn_tick(snap);

        y = self.top_bar(ui, &cv, setter, p, left, right, y, &view);
        y = self.tabs(ui, &cv, left, right, y);
        if self.tab == 1 {
            y = self.library_view(ui, &cv, setter, p, left, right, y);
            self.finish_paint(ui, &cv, p, th, area, right, y);
            return;
        }
        if self.tab == 2 {
            y = self.pads_view(ui, &cv, setter, p, left, right, y);
            self.finish_paint(ui, &cv, p, th, area, right, y);
            return;
        }
        if self.tab == 3 {
            y = self.rack_view(ui, &cv, setter, p, left, right, y, &view);
            self.finish_paint(ui, &cv, p, th, area, right, y);
            return;
        }
        y = self.transport_bar(ui, &cv, setter, p, left, right, y);
        y = self.lcd_strip(ui, &cv, setter, p, left, right, y, &view);

        let split = left + (right - left) * 0.62;
        let pads_right = split - GAP * 0.5;
        let ctrl_left = split + GAP * 0.5;
        let y_pads = self.perform_pad_grid(ui, &cv, setter, p, left, pads_right, y, &view);
        let y_ctrl = self.controls(ui, &cv, setter, p, ctrl_left, right, y);
        y = y_pads.max(y_ctrl) + GAP * 2.0;

        y = self.phrase_bar(ui, &cv, setter, p, left, right, y);
        y = self.pattern_selector(ui, &cv, setter, p, left, right, y, &view);
        y = self.genres(ui, &cv, setter, p, left, right, y, &view);
        y = self.kit(ui, &cv, left, right, y, &view);
        y = self.export_row(ui, &cv, setter, p, left, right, y, &view);
        y = self.history(ui, &cv, setter, p, left, right, y, &view);
        y = self.reading(&cv, left, right, y, &view);

        self.finish_paint(ui, &cv, p, th, area, right, y);
    }

    fn finish_paint(&mut self, ui: &mut egui::Ui, cv: &Cv, p: &CajaParams, th: &Theme, area: Rect, right: f32, y: f32) {
        if self.show_help {
            self.draw_help(ui, cv, area);
        }
        if p.scanlines.value() {
            let clip = ui.clip_rect();
            let mut sy = clip.min.y;
            while sy < clip.max.y {
                cv.p.rect_filled(snap_rect(Rect::from_min_size(pos2(clip.min.x, sy), vec2(clip.width(), 1.0))), 0.0_f32, th.scanline());
                sy += 3.0;
            }
        }
        // Tamaño real del contenido (para el desplazamiento vertical).
        ui.allocate_rect(Rect::from_min_max(area.min, pos2(right + 10.0, y + 10.0)), Sense::hover());
    }

    /// Texto escrito en el campo enfocado. Algunos hosts se quedan con el teclado: la interfaz
    /// sigue siendo usable con los atajos y el selector de género.
    fn keyboard(&mut self, ui: &egui::Ui, setter: &ParamSetter, p: &CajaParams, _view: &View) {
        if self.focus == Focus::None {
            return;
        }
        let events = ui.input(|i| i.events.clone());
        let mut generate = false;
        let (limit, digits_only) = match self.focus {
            Focus::Path => (400, false),
            Focus::Seed => (5, true),
            Focus::PatName => (16, false),
            _ => (120, false),
        };
        let mut apply = false;
        let buf = match self.focus {
            Focus::Phrase => &mut self.phrase,
            Focus::Path => &mut self.path,
            Focus::Seed => &mut self.seed_text,
            Focus::PatName => &mut self.pat_name,
            _ => &mut self.map_name,
        };
        for e in events {
            match e {
                egui::Event::Text(t) => {
                    for c in t.chars().filter(|c| !c.is_control() && (!digits_only || c.is_ascii_digit())) {
                        if buf.chars().count() < limit {
                            buf.push(c);
                        }
                    }
                }
                egui::Event::Paste(t) => {
                    for c in t.chars().filter(|c| !c.is_control() && (!digits_only || c.is_ascii_digit())) {
                        if buf.chars().count() < limit {
                            buf.push(c);
                        }
                    }
                }
                egui::Event::Key { key: egui::Key::Backspace, pressed: true, .. } => {
                    buf.pop();
                }
                egui::Event::Key { key: egui::Key::Enter, pressed: true, .. } => {
                    generate = self.focus == Focus::Phrase;
                    apply = matches!(self.focus, Focus::Path | Focus::Seed | Focus::PatName);
                }
                egui::Event::Key { key: egui::Key::Escape, pressed: true, .. } => {
                    self.focus = Focus::None;
                    break;
                }
                _ => {}
            }
        }
        if generate {
            self.do_generate(setter, p);
        }
        if apply {
            self.apply_typed(setter, p);
        }
    }

    fn do_generate(&mut self, setter: &ParamSetter, p: &CajaParams) {
        let snap = p.snap();
        let phrase = self.phrase.clone();
        let n = self.model.with(snap, |s| s.generate(&phrase, snap));
        apply_snap(setter, p, n);
        self.page = 0;
    }

    /// Modo "Aprender": la nota que llegó por la entrada se asigna a la pieza seleccionada.
    fn learn_tick(&mut self, snap: ParamSnap) {
        let (counter, note) = self.model.host.last_note();
        if counter != self.note_counter {
            self.note_counter = counter;
            let r = self.model.with(snap, |s| s.learn_note(note));
            if let Some((_, _)) = r {
                self.selected = self.model.with(snap, |s| s.learn.piece);
            }
        }
    }


    fn draw_help(&self, ui: &egui::Ui, cv: &Cv, area: Rect) {
        let th = &cv.th;
        let panel = area.shrink(24.0);
        cv.boxed(panel, Color32::from_rgba_unmultiplied(8, 8, 12, 230), th.accent, 3.0);
        let lines = [
            "CAJA.Groove.Pattern · Mapa Mikro MK3",
            "",
            "PAD MODE  → Perform",
            "KEYBOARD  → Mixer (Fader=Vol, Enc=Pan, Mute=Mute)",
            "CHORD     → Samples (Pad=Dado, Pad2=Pin)",
            "SAMPLING  → Librería (Pads 1-16 ajustes)",
            "PATTERN   → Selector de patrones",
            "STEP      → Secuenciador (Scene=pistas, SELECT+pad=fila)",
            "",
            "GLOBALES:",
            "Encoder → BPM | SHIFT+Enc → Master | VOLUME/TEMPO toggle",
            "SWING cicla: Swing → Acento → Human → Off",
            "PLUGIN abre carpeta de samples.",
            "FIXED VEL: fija velocidad a 100.",
            "BROWSE (Lupa): pads = géneros. STAR: oculta/muestra.",
            "",
            "En Perform: Play/Stop/REC controlan el transporte.",
            "Pulsa Ayuda otra vez para cerrar.",
        ];
        let mut yy = panel.min.y + 16.0;
        for line in lines {
            let col = if line.is_empty() {
                th.text_dim
            } else if line.starts_with("CAJA") {
                th.accent
            } else {
                th.text
            };
            cv.text(pos2(panel.min.x + 16.0, yy), Align2::LEFT_TOP, line, col);
            yy += cv.fs + 4.0;
        }
        let _ = ui;
    }

    fn top_bar(&mut self, ui: &egui::Ui, cv: &Cv, setter: &ParamSetter, p: &CajaParams, left: f32, right: f32, y: f32, view: &View) -> f32 {
        let mut f = Flow::new(left, y, right);
        let r = f.place(cv.text_w("CAJA") + PAD_X, button_h(cv));
        cv.text(pos2(r.min.x, r.center().y), Align2::LEFT_CENTER, "CAJA", cv.th.accent);
        row_label(cv, &mut f, "Drum Machine en Rust");
        if button(ui, cv, &mut f, "recargar", "Recargar", false) {
            self.model.reload_files(p.snap());
        }
        let pal = if p.palette.value() == PaletteParam::Arcade { "Paleta arcade" } else { "Paleta daltonismo" };
        if button(ui, cv, &mut f, "pal", pal, false) {
            set(setter, &p.palette, if p.palette.value() == PaletteParam::Arcade { PaletteParam::ColorBlind } else { PaletteParam::Arcade });
        }
        if button(ui, cv, &mut f, "dark", if p.dark.value() { "Oscuro" } else { "Claro" }, false) {
            set(setter, &p.dark, !p.dark.value());
        }
        if button(ui, cv, &mut f, "scan", if p.scanlines.value() { "Scanlines si" } else { "Scanlines no" }, p.scanlines.value()) {
            set(setter, &p.scanlines, !p.scanlines.value());
        }
        let txt = match p.text_size.value() {
            TextSizeParam::Small => "Texto pequeno",
            TextSizeParam::Medium => "Texto mediano",
            TextSizeParam::Large => "Texto grande",
        };
        if button(ui, cv, &mut f, "textsz", txt, false) {
            set(
                setter,
                &p.text_size,
                match p.text_size.value() {
                    TextSizeParam::Small => TextSizeParam::Medium,
                    TextSizeParam::Medium => TextSizeParam::Large,
                    TextSizeParam::Large => TextSizeParam::Small,
                },
            );
        }
        let mk3_on = p.mk3_enabled.value();
        if button(ui, cv, &mut f, "mk3", if mk3_on { "MK3 nativo si" } else { "MK3 nativo no" }, mk3_on) {
            set(setter, &p.mk3_enabled, !mk3_on);
            if let Ok(mut m) = self.mk3.lock() {
                m.set_enabled(!mk3_on);
            }
        }
        if button(ui, cv, &mut f, "btn_help", if self.show_help { "Ayuda si" } else { "Ayuda" }, self.show_help) {
            self.show_help = !self.show_help;
        }
        if mk3_on {
            let mode = self.mk3.lock().map(|m| m.mode().label()).unwrap_or("?");
            row_label(cv, &mut f, &format!("modo {mode}"));
        }
        f.newline();
        let w = right - left;
        let h = cv.wrapped(pos2(left, f.y), w, &view.origins, cv.th.text_dim);
        f.y += h + GAP;
        f.row_h = 0.0;
        f.bottom() + GAP
    }

    fn text_field(&mut self, ui: &egui::Ui, cv: &Cv, r: Rect, id: &str, which: Focus, hint: &str, now: f64) {
        let resp = ui.interact(snap_rect(r), ui.id().with(id), Sense::click());
        if resp.clicked() {
            self.focus = which;
        } else if ui.input(|i| i.pointer.any_pressed()) && !resp.hovered() && self.focus == which {
            self.focus = Focus::None;
        }
        let focused = self.focus == which;
        let th = &cv.th;
        cv.boxed(r, th.panel, if focused { th.accent } else { th.text_dim }, 2.0);
        let buf = match which {
            Focus::Phrase => &self.phrase,
            Focus::Path => &self.path,
            Focus::Seed => &self.seed_text,
            Focus::PatName => &self.pat_name,
            _ => &self.map_name,
        };
        let clip = cv.clipped(r.shrink(3.0));
        let mut shown = buf.clone();
        if shown.is_empty() && !focused {
            clip.text(pos2(r.min.x + PAD_X, r.center().y), Align2::LEFT_CENTER, hint, cv.font(), th.text_dim);
        } else {
            let caret = focused && (now * 2.0) as i64 % 2 == 0;
            if caret {
                shown.push('_');
            }
            // si no cabe, se ve el final
            let w = cv.text_w(&shown);
            let avail = r.width() - 2.0 * PAD_X;
            let x = if w > avail { r.max.x - PAD_X - w } else { r.min.x + PAD_X };
            clip.text(pos2(x, r.center().y), Align2::LEFT_CENTER, &shown, cv.font(), th.text);
        }
    }

    /// Selector de patrones del rack: banco A/B + 16 pads + copiar/borrar.
    /// Al elegir un pad se llama a `Session::select_pattern` y se aplica el ParamSnap
    /// al host (seed/genre/swing/human/accent/density/bars del patrón elegido).
    fn pattern_selector(
        &mut self,
        ui: &egui::Ui,
        cv: &Cv,
        setter: &ParamSetter,
        p: &CajaParams,
        left: f32,
        right: f32,
        y: f32,
        view: &View,
    ) -> f32 {
        let bh = button_h(cv);
        let mut f = Flow::new(left, y, right);
        row_label(cv, &mut f, "Patron");

        // Bancos A / B
        if button(ui, cv, &mut f, "pbankA", "Banco A", self.pattern_bank == 0) {
            self.pattern_bank = 0;
        }
        if button(ui, cv, &mut f, "pbankB", "Banco B", self.pattern_bank == 1) {
            self.pattern_bank = 1;
        }

        // Etiqueta + nombre editable del activo
        let active = view.pattern_active;
        let bank_letter = if active < 16 { 'A' } else { 'B' };
        let slot_n = (active % 16) + 1;
        row_label(cv, &mut f, &format!("activo {bank_letter}{slot_n}"));
        // Sincronizar campo de nombre si no estamos editando
        if self.focus != Focus::PatName {
            self.pat_name = view.pattern_names[active].clone();
        }
        let nw = (cv.text_w("Nombrexx") + 2.0 * PAD_X).min(140.0);
        let nr = f.place(nw, button_h(cv));
        let now = ui.input(|i| i.time);
        self.text_field(ui, cv, nr, "patname", Focus::PatName, "nombre", now);
        if button(ui, cv, &mut f, "patok", "OK", false) {
            let name = self.pat_name.clone();
            let snap = p.snap();
            self.model.with(snap, |s| s.set_active_name(&name));
            self.focus = Focus::None;
            self.status = format!("Patron {bank_letter}{slot_n} renombrado");
        }

        // Copiar activo → ranura vacía del banco visible (primera libre) o forzar dialog simple:
        // por simplicidad: "Copiar aqui" no; botones Copy/Clear sobre el activo
        if button(ui, cv, &mut f, "pcopy", "Copiar a…", false) {
            // Copia el activo a la primera ranura libre del banco visible; si no hay, a la 1 del banco
            let base = self.pattern_bank * 16;
            let snap = p.snap();
            let target = {
                let mut t = base;
                for i in 0..16 {
                    let idx = base + i;
                    if idx != active && !view.pattern_filled[idx] {
                        t = idx;
                        break;
                    }
                    t = base + ((active + 1) % 16); // fallback: siguiente del banco
                }
                t
            };
            self.model.with(snap, |s| s.copy_active_to(target));
            self.status = format!("Copiado → {}{}", if target < 16 { "A" } else { "B" }, (target % 16) + 1);
        }
        if button(ui, cv, &mut f, "pclear", "Borrar activo", false) {
            let snap = p.snap();
            self.model.with(snap, |s| s.clear_pattern_slot(s.rack.active));
            self.status = "Patron borrado".into();
        }

        let y2 = f.bottom() + GAP;

        // Rejilla 4×4 de pads del banco visible
        let gap = 4.0_f32;
        let cols = 4usize;
        let rows = 4usize;
        let avail_w = (right - left).max(120.0);
        let cell_w = ((avail_w - gap * (cols as f32 - 1.0)) / cols as f32).floor().max(28.0);
        let cell_h = bh.max(22.0);
        let base = self.pattern_bank * 16;

        for row in 0..rows {
            for col in 0..cols {
                let local = row * cols + col; // 0..15
                let idx = base + local;
                let x = left + col as f32 * (cell_w + gap);
                let yy = y2 + row as f32 * (cell_h + gap);
                let r = Rect::from_min_size(pos2(x, yy), vec2(cell_w, cell_h));
                let is_active = view.pattern_active == idx;
                let filled = view.pattern_filled[idx];
                let label = if view.pattern_names[idx].is_empty() {
                    format!("{}", local + 1)
                } else {
                    // nombre corto truncado
                    let n = &view.pattern_names[idx];
                    if n.len() > 4 {
                        format!("{}", &n[..4])
                    } else {
                        n.clone()
                    }
                };
                // Visual: activo = accent; con contenido = panel; vacío = apagado
                let id = format!("pslot{idx}");
                if button_at(ui, cv, r, &id, &label, is_active || filled) {
                    if !is_active {
                        let snap = p.snap();
                        let n = self.model.with(snap, |s| s.select_pattern(idx));
                        apply_snap(setter, p, n);
                        // sincronizar banco visible con el elegido
                        self.pattern_bank = idx / 16;
                        self.status = format!("Patron {}{}", if idx < 16 { "A" } else { "B" }, (idx % 16) + 1);
                    }
                }
                // Marca de “tiene contenido” cuando no está activo: punto sutil no; el `on` del botón ya cambia color
                if filled && !is_active {
                    // reborde extra vía segundo boxed tenue
                    let th = &cv.th;
                    cv.boxed(r, Color32::TRANSPARENT, th.accent.linear_multiply(0.45), 1.0);
                }
            }
        }

        y2 + rows as f32 * (cell_h + gap) + GAP
    }

    /// Barra de transporte sobre el LCD: Play/Stop + BPM ±.
    fn transport_bar(
        &mut self,
        ui: &egui::Ui,
        cv: &Cv,
        setter: &ParamSetter,
        p: &CajaParams,
        left: f32,
        right: f32,
        y: f32,
    ) -> f32 {
        let mut f = Flow::new(left, y, right);
        let playing = self.model.host.playing.load(Ordering::Relaxed);
        let label = if playing { "Parar" } else { "Play" };
        if button(ui, cv, &mut f, "tb_play", label, playing) {
            let next = if playing { 0 } else { 1 };
            set(setter, &p.run_mode, next);
            self.status = if playing { "Detenido (modo con el transporte)".into() } else { "Reproduciendo (modo Siempre)".into() };
        }

        row_label(cv, &mut f, "BPM");
        if button(ui, cv, &mut f, "tb_bpm_dn", "-", false) {
            set(setter, &p.bpm, (p.bpm.value() - 1).clamp(60, 210));
        }
        
        let v = p.bpm.value();
        let vw = cv.text_w("999") + 2.0 * PAD_X;
        let r = f.place(vw, button_h(cv));
        cv.boxed(r, cv.th.panel, cv.th.text_dim, 2.0);
        cv.text(r.center(), Align2::CENTER_CENTER, &v.to_string(), cv.th.text);

        if button(ui, cv, &mut f, "tb_bpm_up", "+", false) {
            set(setter, &p.bpm, (p.bpm.value() + 1).clamp(60, 210));
        }

        let mode_label = match p.run_mode.value() {
            0 => "transporte",
            1 => "siempre",
            _ => "tecla",
        };
        row_label(cv, &mut f, &format!("Modo: {mode_label}"));
        f.bottom() + GAP
    }

    /// Franja LCD estilo MPC: BPM grande, patrón activo, frase y estado.
    /// Franja LCD estilo MPC: BPM grande, patrón activo, frase y estado.
    fn lcd_strip(
        &mut self,
        ui: &egui::Ui,
        cv: &Cv,
        setter: &ParamSetter,
        p: &CajaParams,
        left: f32,
        right: f32,
        y: f32,
        view: &View,
    ) -> f32 {
        let th = &cv.th;
        let h = (cv.fs * 4.5 + 16.0).clamp(52.0, 88.0);
        let r = Rect::from_min_max(pos2(left, y), pos2(right, y + h));
        // bisel + cristal LCD
        cv.boxed(r, th.lcd_bg, th.chassis_edge, 3.0);
        let inner = r.shrink(5.0);
        cv.fill(inner, th.lcd_bg);

        // BPM grande a la derecha
        let bpm = p.bpm.value();
        let bpm_s = format!("{bpm}");
        let bpm_font = FontId::proportional((cv.fs * 2.2).round().max(16.0));
        let bpm_w = cv.p.layout_no_wrap(bpm_s.clone(), bpm_font.clone(), th.lcd_fg).size().x;
        let bpm_x = inner.max.x - 8.0;
        cv.p.text(
            pos2(bpm_x, inner.center().y - cv.fs * 0.2),
            Align2::RIGHT_CENTER,
            &bpm_s,
            bpm_font,
            th.lcd_fg,
        );
        cv.p.text(
            pos2(bpm_x, inner.center().y + cv.fs * 1.1),
            Align2::RIGHT_CENTER,
            "BPM",
            FontId::proportional(cv.fs * 0.85),
            th.lcd_dim,
        );

        // clic en BPM: no edita aquí; el control sigue en la fila de parámetros
        let _ = ui.interact(snap_rect(Rect::from_min_size(pos2(bpm_x - bpm_w - 8.0, inner.min.y + 4.0), vec2(bpm_w + 16.0, h * 0.6))), ui.id().with("lcd_bpm"), Sense::hover());

        // Columna izquierda: patrón + modo
        let pa = view.pattern_active;
        let bank = if pa < 16 { 'A' } else { 'B' };
        let slot = (pa % 16) + 1;
        let pname = if view.pattern_names[pa].is_empty() {
            format!("{bank}{slot}")
        } else {
            format!("{bank}{slot}:{name}", name = &view.pattern_names[pa])
        };
        let mode = self
            .mk3
            .lock()
            .map(|m| m.mode().label())
            .unwrap_or("Perform");
        let line1 = format!("{pname}  ·  {mode}");
        cv.p.text(
            pos2(inner.min.x + 8.0, inner.min.y + 6.0),
            Align2::LEFT_TOP,
            &line1,
            FontId::proportional(cv.fs),
            th.lcd_fg,
        );

        // Frase / status
        let phrase = if self.phrase.is_empty() {
            view.reading.chars().take(48).collect::<String>()
        } else {
            self.phrase.chars().take(48).collect::<String>()
        };
        let line2 = if self.status.is_empty() {
            if phrase.is_empty() {
                "CAJA  ·  ready".to_string()
            } else {
                phrase
            }
        } else {
            self.status.chars().take(48).collect::<String>()
        };
        cv.p.text(
            pos2(inner.min.x + 8.0, inner.min.y + 6.0 + cv.fs * 1.5),
            Align2::LEFT_TOP,
            &line2,
            FontId::proportional(cv.fs * 0.95),
            th.lcd_dim,
        );

        // LEDs decorativos (play / patrón lleno)
        let led_y = inner.max.y - 10.0;
        let playing = self.model.host.playing.load(Ordering::Relaxed);
        let led_play = if playing { th.led_green } else { th.led_off };
        let led_pat = if view.pattern_filled[pa] { th.led_amber } else { th.led_off };
        cv.fill(Rect::from_center_size(pos2(inner.min.x + 12.0, led_y), vec2(8.0, 8.0)), led_play);
        cv.fill(Rect::from_center_size(pos2(inner.min.x + 26.0, led_y), vec2(8.0, 8.0)), led_pat);

        // Botones rápidos bajo el LCD: Generar (si hay frase) queda en phrase_bar
        let _ = setter;
        y + h + GAP
    }

    /// Rejilla 4×4 de pads de goma (primeros 16 del kit) — pantalla Perform.
    fn perform_pad_grid(
        &mut self,
        ui: &egui::Ui,
        cv: &Cv,
        setter: &ParamSetter,
        p: &CajaParams,
        left: f32,
        right: f32,
        y: f32,
        view: &View,
    ) -> f32 {
        let th = &cv.th;
        let cols = 4usize;
        let rows = 4usize;
        let gap = 6.0_f32;
        let avail_w = (right - left).max(160.0);
        let cell = ((avail_w - gap * (cols as f32 - 1.0)) / cols as f32).floor().clamp(48.0, 96.0);
        // altura un poco menor que ancho para aspecto pad
        let cell_h = (cell * 0.92).round();

        let st = self.kit.snapshot();
        let lib = self.kit.library();

        for row in 0..rows {
            for col in 0..cols {
                let q = row * cols + col; // 0..15
                if q >= NUM_PIECES {
                    continue;
                }
                let x = left + col as f32 * (cell + gap);
                let yy = y + row as f32 * (cell_h + gap);
                let r = Rect::from_min_size(pos2(x, yy), vec2(cell, cell_h));

                let selected = self.sel_pad == q;
                let muted = p.pads[q].mute.value();
                let solo = st.pads[q].solo;
                let n_samples = lib.bank(q, st.bank_cap).len();

                // sombra
                cv.fill(r.translate(vec2(2.0, 2.0)), th.shadow);
                // goma
                let fill = if selected {
                    th.rubber_on
                } else if muted {
                    th.rubber.linear_multiply(0.55)
                } else {
                    th.rubber
                };
                let border = if selected {
                    th.accent
                } else if solo {
                    th.led_green
                } else {
                    th.chassis_edge
                };
                cv.boxed(r, fill, border, if selected { 3.0 } else { 2.0 });

                // LED de esquina (grupo de la pieza)
                let pi = PAD_ORDER.get(q).copied().unwrap_or(0);
                let led_c = if muted {
                    th.led_off
                } else {
                    th.group(PIECES[pi].group)
                };
                let led_r = Rect::from_min_size(pos2(r.min.x + 5.0, r.min.y + 5.0), vec2(7.0, 7.0));
                cv.fill(led_r, led_c);

                // nombre
                let name = PIECES[pi].name;
                let short = if name.len() > 8 { &name[..8] } else { name };
                let tc = if selected { th.text } else { th.text_dim };
                cv.text(pos2(r.center().x, r.center().y - 2.0), Align2::CENTER_CENTER, short, tc);
                // contador de muestras
                cv.text(
                    pos2(r.center().x, r.max.y - 8.0),
                    Align2::CENTER_CENTER,
                    &format!("{n_samples}"),
                    th.text_dim,
                );

                let resp = ui.interact(snap_rect(r), ui.id().with(("ppad", q)), Sense::click());
                if resp.clicked() {
                    self.sel_pad = q;
                    let note = view.notes[pi];
                    let _ = self.model.host.push_preview(note, 110);
                    self.status = format!("{} · note {}", name, note);
                }
                if resp.secondary_clicked() {
                    // clic derecho → mute
                    let muted = p.pads[q].mute.value();
                    set(setter, &p.pads[q].mute, !muted);
                    self.kit.edit(|k| k.pads[q].mute = !muted);
                    self.sel_pad = q;
                    self.status = format!("{} {}", name, if !muted { "MUTE" } else { "unmute" });
                }
                if resp.middle_clicked() {
                    // clic medio → solo
                    self.kit.edit(|k| k.pads[q].solo = !k.pads[q].solo);
                    let solo = self.kit.snapshot().pads[q].solo;
                    self.sel_pad = q;
                    self.status = format!("{} {}", name, if solo { "SOLO" } else { "unsolo" });
                }
            }
        }

        y + rows as f32 * (cell_h + gap) + GAP
    }

    fn phrase_bar(&mut self, ui: &egui::Ui, cv: &Cv, setter: &ParamSetter, p: &CajaParams, left: f32, right: f32, y: f32) -> f32 {
        let now = ui.input(|i| i.time);
        let bh = button_h(cv);
        let gen_w = cv.text_w("Generar") + 2.0 * PAD_X;
        let field = Rect::from_min_max(pos2(left, y), pos2(right - gen_w - GAP, y + bh));
        self.text_field(ui, cv, field, "phrase", Focus::Phrase, "techno 132 bpm con fill 4 compases", now);
        let gb = Rect::from_min_size(pos2(right - gen_w, y), vec2(gen_w, bh));
        if button_at(ui, cv, gb, "generar", "Generar", false) {
            self.do_generate(setter, p);
        }
        let mut f = Flow::new(left, y + bh + GAP, right);
        row_label(cv, &mut f, "Atajos");
        for (i, s) in SHORTCUTS.iter().enumerate() {
            if button(ui, cv, &mut f, &format!("sc{i}"), s, false) {
                self.phrase = (*s).to_string();
                self.do_generate(setter, p);
            }
        }
        f.bottom() + GAP * 2.0
    }

    fn genres(&mut self, ui: &egui::Ui, cv: &Cv, setter: &ParamSetter, p: &CajaParams, left: f32, right: f32, y: f32, view: &View) -> f32 {
        let mut f = Flow::new(left, y, right);
        if button(ui, cv, &mut f, "sug", if self.show_suggestions { "Ocultar sugerencias" } else { "Sugerencias" }, self.show_suggestions) {
            self.show_suggestions = !self.show_suggestions;
        }
        if !self.show_suggestions {
            return f.bottom() + GAP * 2.0;
        }
        for (fi, (family, list)) in view.families.iter().enumerate() {
            f.newline();
            row_label(cv, &mut f, &format!("{family}:"));
            for (idx, name, proposal) in list {
                let on = *idx == view.genre_idx;
                let tag = if *proposal { " propuesta" } else { "" };
                let w = cv.text_w(name) + cv.text_w(tag) + 2.0 * PAD_X;
                let r = f.place(w, button_h(cv));
                if button_tag(ui, cv, r, &format!("g{fi}_{idx}"), name, tag, on) {
                    let snap = p.snap();
                    let n = self.model.with(snap, |s| s.set_genre(*idx, snap));
                    apply_snap(setter, p, n);
                }
            }
        }
        cv.text(pos2(left, f.bottom() + GAP + cv.fs * 0.5), Align2::LEFT_CENTER, "Los generos con el aviso propuesta son nuevos: afinalos de oido.", cv.th.text_dim);
        f.bottom() + GAP * 2.0 + cv.fs + 4.0
    }

    // ───────────────────────── RACK (step sequencer) ─────────────────────────
    //
    // Pantalla alineada con el modo STEP del MK3:
    //   · 16 pasos del compás visible (celdas grandes)
    //   · LEFT/RIGHT de UI = cambio de compás
    //   · clic en nombre de pieza = seleccionar fila (SELECT+pad)
    //   · clic = golpe, arrastre vertical = velocidad, clic derecho = ghost

    fn rack_view(
        &mut self,
        ui: &egui::Ui,
        cv: &Cv,
        _setter: &ParamSetter,
        p: &CajaParams,
        left: f32,
        right: f32,
        y: f32,
        view: &View,
    ) -> f32 {
        let th = &cv.th;
        let bars = view.pattern.bars.max(1);
        self.rack_bar = self.rack_bar.min(bars - 1);

        // mini-LCD de la vista Step
        let lcd_h = (cv.fs * 2.2 + 10.0).clamp(28.0, 40.0);
        let lcd = Rect::from_min_max(pos2(left, y), pos2(right, y + lcd_h));
        cv.boxed(lcd, th.lcd_bg, th.chassis_edge, 2.0);
        let pa = view.pattern_active;
        let bank = if pa < 16 { 'A' } else { 'B' };
        let pname = if view.pattern_names[pa].is_empty() {
            format!("{bank}{}", (pa % 16) + 1)
        } else {
            format!("{bank}{}:{}", (pa % 16) + 1, &view.pattern_names[pa])
        };
        cv.p.text(
            pos2(left + 8.0, lcd.center().y),
            Align2::LEFT_CENTER,
            &format!("STEP  ·  {pname}  ·  compás {}/{}", self.rack_bar + 1, bars),
            FontId::proportional(cv.fs),
            th.lcd_fg,
        );
        if !self.status.is_empty() {
            cv.p.text(
                pos2(right - 8.0, lcd.center().y),
                Align2::RIGHT_CENTER,
                &self.status.chars().take(28).collect::<String>(),
                FontId::proportional(cv.fs * 0.9),
                th.lcd_dim,
            );
        }
        let y = y + lcd_h + GAP;

        // Seguir el playhead del host cuando está sonando
        let step_now = self.model.host.step.load(Ordering::Relaxed);
        let running = self.model.host.playing.load(Ordering::Relaxed) && step_now >= 0;
        if running {
            self.rack_bar = (step_now as usize / STEPS_PER_BAR).min(bars - 1);
        }

        // ── barra de navegación ──
        let mut f = Flow::new(left, y, right);
        row_label(cv, &mut f, "RACK");
        if button(ui, cv, &mut f, "rbarprev", "Compas <", false) && self.rack_bar > 0 {
            self.rack_bar -= 1;
        }
        row_label(cv, &mut f, &format!("Compas {} / {}", self.rack_bar + 1, bars));
        if button(ui, cv, &mut f, "rbarnext", "Compas >", false) && self.rack_bar + 1 < bars {
            self.rack_bar += 1;
        }
        if button(
            ui,
            cv,
            &mut f,
            "rshowall",
            if self.show_all { "Todas las pistas" } else { "Solo con golpes" },
            self.show_all,
        ) {
            self.show_all = !self.show_all;
        }
        // indicador de patrón activo
        let pa = view.pattern_active;
        let bank = if pa < 16 { 'A' } else { 'B' };
        row_label(cv, &mut f, &format!("Patron {bank}{}", (pa % 16) + 1));
        f.newline();

        // ── filas visibles ──
        let used = |i: usize| (0..view.pattern.steps()).any(|s| view.pattern.cell[i][s].on());
        let mut rows: Vec<usize> = (0..NUM_PIECES)
            .filter(|&i| self.show_all || used(i) || i == self.selected)
            .collect();
        // Orden de pads del kit (grave → agudo), no por nota MIDI invertida
        rows.sort_by_key(|&i| PAD_ORDER.iter().position(|&p| p == i).unwrap_or(999));

        let name_w = cv.text_w("Hat cer. sec.") + 2.0 * PAD_X + 8.0;
        // Fila seleccionada más alta (estilo “pad”); el resto compacto
        let rh_sel = (cv.fs + 28.0).max(40.0); // fila foco = pad alto
        let rh_dim = (cv.fs + 12.0).max(22.0);
        let selected_pi = self.selected;
        let row_h = |pi: usize| -> f32 {
            if pi == selected_pi {
                rh_sel
            } else {
                rh_dim
            }
        };
        let cols = STEPS_PER_BAR;
        let first_step = self.rack_bar * STEPS_PER_BAR;
        let cw = ((right - left - name_w - 4.0) / cols as f32).floor().max(16.0);
        let x0 = left + name_w;
        let head_h = cv.fs + 8.0;
        let y_head = f.y;
        let y0 = y_head + head_h;
        let mut y_of: Vec<f32> = Vec::with_capacity(rows.len());
        let mut acc = y0;
        for &pi in &rows {
            y_of.push(acc);
            acc += row_h(pi);
        }
        let grid_h = acc - y0;

        // cabecera de pasos 1..16
        for c in 0..cols {
            let cx = x0 + c as f32 * cw;
            let lab = format!("{}", c + 1);
            let col = if c % 4 == 0 { th.text } else { th.text_dim };
            cv.text(pos2(cx + cw / 2.0, y_head + head_h / 2.0), Align2::CENTER_CENTER, &lab, col);
        }

        let grid = Rect::from_min_size(pos2(x0, y0), vec2(cols as f32 * cw, grid_h));
        let names_area = Rect::from_min_size(pos2(left, y0), vec2(name_w, grid_h));

        // nombres de fila (SELECT fila)
        for (r, &pi) in rows.iter().enumerate() {
            let ry = y_of[r];
            let rh = row_h(pi);
            let row = Rect::from_min_size(pos2(left, ry), vec2(name_w - 2.0, rh - 1.0));
            let selected = self.selected == pi;
            let fill = if selected { th.accent } else { th.panel.linear_multiply(0.85) };
            let border = if selected { th.text } else { th.border };
            cv.boxed(row, fill, border, if selected { 3.0 } else { 2.0 });
            let group_col = th.group(PIECES[pi].group);
            cv.fill(Rect::from_min_size(pos2(row.min.x, ry + 2.0), vec2(4.0, rh - 4.0)), group_col);
            let tc = if selected { th.bg } else { th.text_dim };
            cv.text(pos2(row.min.x + 10.0, ry + rh / 2.0), Align2::LEFT_CENTER, PIECES[pi].name, tc);

            let resp = ui.interact(snap_rect(row), ui.id().with(("rrow", pi)), Sense::click());
            if resp.clicked() {
                self.selected = pi;
                let note = view.notes[pi];
                let _ = self.model.host.push_preview(note, 100);
            }
        }

        // celdas estilo pad MPC (goma + velocity + ghost + playhead)
        let cells = cv.clipped(grid.expand(1.0));
        let cc = Cv { p: &cells, th: *th, fs: cv.fs };
        for (r, &pi) in rows.iter().enumerate() {
            let ry = y_of[r];
            let rh = row_h(pi);
            let col = th.group(PIECES[pi].group);
            let row_sel = self.selected == pi;
            for c in 0..cols {
                let st = first_step + c;
                let cell = view.pattern.cell[pi][st];
                let cx = x0 + c as f32 * cw;
                let gap = 1.5_f32;
                let slot = Rect::from_min_size(pos2(cx + gap, ry + gap), vec2(cw - gap * 2.0, rh - gap * 2.0));
                let beat = c % 4 == 0;
                // base goma
                let base = if row_sel { th.rubber_on } else { th.rubber };
                let border = if beat { th.chassis_edge } else { th.chassis.linear_multiply(1.2) };
                cc.boxed(slot, base, border, 1.5);
                if cell.on() {
                    if cell.ghost {
                        // ghost: contorno del grupo, relleno bajo
                        let h = (slot.height() * 0.4).round().max(3.0);
                        let g = Rect::from_min_max(pos2(slot.min.x + 2.0, slot.max.y - h - 1.0), pos2(slot.max.x - 2.0, slot.max.y - 1.0));
                        cc.boxed(g, th.face, col, 1.5);
                    } else {
                        // velocity de abajo hacia arriba
                        let h = (slot.height() * f32::from(cell.vel) / 127.0).round().max(4.0);
                        let body = Rect::from_min_max(
                            pos2(slot.min.x + 2.0, slot.max.y - h - 1.0),
                            pos2(slot.max.x - 2.0, slot.max.y - 1.0),
                        );
                        cc.fill(body, col);
                        // LED esquina (hit)
                        let led = Rect::from_min_size(pos2(slot.min.x + 3.0, slot.min.y + 3.0), vec2(5.0, 5.0));
                        cc.fill(led, if cell.vel > 100 { th.led_white } else { col });
                    }
                }
            }
        }

        // playhead: columna activa con borde led_white
        if running {
            let st = step_now as usize;
            if st >= first_step && st < first_step + cols {
                let c = st - first_step;
                let cx = x0 + c as f32 * cw;
                // velo suave sobre la columna
                cc.fill(
                    Rect::from_min_size(pos2(cx, y_head), vec2(cw, head_h + grid_h)),
                    Color32::from_rgba_unmultiplied(th.led_white.r(), th.led_white.g(), th.led_white.b(), 28),
                );
                // marco superior del paso
                cc.boxed(
                    Rect::from_min_size(pos2(cx + 1.0, y_head + 1.0), vec2(cw - 2.0, head_h - 2.0)),
                    th.lcd_bg,
                    th.led_white,
                    2.0,
                );
            }
        }

        // ── interacción (igual filosofía que el piano roll) ──
        let cell_at = |pos: Pos2| -> Option<(usize, usize)> {
            if !grid.contains(pos) {
                return None;
            }
            let c = ((pos.x - x0) / cw) as usize;
            if c >= cols {
                return None;
            }
            // Buscar fila por y acumulada (alturas variables)
            for (r, &pi) in rows.iter().enumerate() {
                let ry = y_of[r];
                let rh = row_h(pi);
                if pos.y >= ry && pos.y < ry + rh {
                    return Some((pi, first_step + c));
                }
            }
            None
        };
        let resp = ui.interact(snap_rect(grid), ui.id().with("rack_grid"), Sense::click_and_drag());
        let snap = p.snap();
        let notes = view.notes;

        if resp.drag_started() {
            if let Some(pos) = resp.interact_pointer_pos() {
                if let Some((pi, st)) = cell_at(pos) {
                    let cell = view.pattern.cell[pi][st];
                    self.drag = Some(Drag {
                        piece: pi,
                        step: st,
                        start_vel: if cell.on() { i32::from(cell.vel) } else { 100 },
                        existed: cell.on(),
                    });
                    self.selected = pi;
                }
            }
        }
        if resp.dragged() {
            if let (Some(d), Some(pos)) = (self.drag, resp.interact_pointer_pos()) {
                let vel = if let Some((r_idx, _)) = rows.iter().enumerate().find(|(_, &pi)| pi == d.piece) {
                    let ry = y_of[r_idx];
                    let rh = row_h(d.piece);
                    let rel = ((pos.y - ry) / rh).clamp(0.0, 1.0);
                    (127.0 * (1.0 - rel)).round().clamp(1.0, 127.0) as i32
                } else {
                    d.start_vel
                };
                self.model.with(snap, |s| {
                    if !s.pattern.cell[d.piece][d.step].on() {
                        s.toggle_cell(d.piece, d.step, false);
                    }
                    s.set_cell_vel(d.piece, d.step, vel);
                });
                let _ = self.model.host.push_preview(notes[d.piece], vel.clamp(1, 127) as u8);
            }
        }
        if resp.drag_stopped() {
            self.drag = None;
        }
        if resp.clicked() && self.drag.is_none() {
            if let Some(pos) = resp.interact_pointer_pos() {
                if let Some((pi, st)) = cell_at(pos) {
                    self.selected = pi;
                    let was_on = view.pattern.cell[pi][st].on();
                    self.model.with(snap, |s| s.toggle_cell(pi, st, false));
                    if !was_on {
                        let _ = self.model.host.push_preview(notes[pi], 100);
                    }
                }
            }
        }
        if resp.secondary_clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                if let Some((pi, st)) = cell_at(pos) {
                    self.selected = pi;
                    self.model.with(snap, |s| s.toggle_ghost(pi, st));
                    let _ = self.model.host.push_preview(notes[pi], 40);
                }
            }
        }

        let _ = names_area;

        let mut f2 = Flow::new(left, y0 + grid_h + GAP * 2.0, right);
        let help = "RACK: clic = golpe · arrastre vertical = velocidad · clic derecho = ghost · clic en nombre = elegir fila · Compas </> cambia el bloque de 16 pasos.";
        let h = cv.wrapped(pos2(left, f2.y), right - left, help, th.text_dim);
        f2.y += h;
        f2.y + GAP * 2.0
    }

    // ───────────────────────── piano roll ─────────────────────────

    fn roll(&mut self, ui: &egui::Ui, cv: &Cv, _setter: &ParamSetter, p: &CajaParams, left: f32, right: f32, y: f32, view: &View) -> f32 {
        let th = &cv.th;
        let bars = view.pattern.bars;
        let per_page = if right - left > 1000.0 { 2 } else { 1 }.min(bars);
        let pages = bars.div_ceil(per_page);
        let step_now = self.model.host.step.load(Ordering::Relaxed);
        let running = self.model.host.playing.load(Ordering::Relaxed) && step_now >= 0;
        if running {
            self.page = (step_now as usize / STEPS_PER_BAR) / per_page;
        }
        self.page = self.page.min(pages - 1);

        let mut f = Flow::new(left, y, right);
        row_label(cv, &mut f, "Piano roll");
        if button(ui, cv, &mut f, "pgprev", "Pagina anterior", false) && self.page > 0 {
            self.page -= 1;
        }
        row_label(cv, &mut f, &format!("Pagina {} de {}", self.page + 1, pages));
        if button(ui, cv, &mut f, "pgnext", "Pagina siguiente", false) && self.page + 1 < pages {
            self.page += 1;
        }
        if button(ui, cv, &mut f, "showall", if self.show_all { "Todas las pistas" } else { "Solo pistas con golpes" }, self.show_all) {
            self.show_all = !self.show_all;
        }
        f.newline();

        let used = |i: usize| (0..view.pattern.steps()).any(|s| view.pattern.cell[i][s].on());
        let mut rows: Vec<usize> = (0..NUM_PIECES).filter(|&i| self.show_all || used(i) || i == self.selected).collect();
        // Piano roll de Bitwig: la nota más aguda arriba, la más grave (C1 / kick) abajo.
        rows.sort_by(|&a, &b| view.notes[b].cmp(&view.notes[a]).then(a.cmp(&b)));
        let name_w = cv.text_w("Hat cer. sec. 127") + 2.0 * PAD_X;
        let rh = (cv.fs + 10.0).max(18.0);
        let cols_full = per_page * STEPS_PER_BAR;
        let first_bar = self.page * per_page;
        let cols = ((bars - first_bar).min(per_page)) * STEPS_PER_BAR;
        let cw = ((right - left - name_w - 4.0) / cols_full as f32).floor().max(6.0);
        let x0 = left + name_w;
        let head_h = cv.fs + 6.0;
        let y_head = f.y;
        let y0 = y_head + head_h;
        let grid_h = rows.len() as f32 * rh;
        let grid = Rect::from_min_size(pos2(x0, y0), vec2(cw * cols as f32, grid_h));

        // fondo y cabecera con el número de compás
        cv.boxed(Rect::from_min_max(pos2(left, y_head), pos2(x0 + cw * cols as f32 + 2.0, y0 + grid_h + 2.0)), th.panel, th.border, 2.0);
        for b in 0..(cols / STEPS_PER_BAR) {
            cv.text(pos2(x0 + (b * STEPS_PER_BAR) as f32 * cw + 4.0, y_head + head_h / 2.0), Align2::LEFT_CENTER, &format!("Compas {}", first_bar + b + 1), th.text_dim);
        }

        // pistas: nombre (en palabras) + nota del kit. Clic: elegir pieza. Clic en el número: +1 / clic derecho: -1.
        let snap = p.snap();
        for (r, &pi) in rows.iter().enumerate() {
            let ry = y0 + r as f32 * rh;
            let row = Rect::from_min_size(pos2(left + 2.0, ry), vec2(name_w - 2.0, rh));
            let sel = pi == self.selected;
            let learning = view.learn_active && view.learn_piece == pi;
            if r % 2 == 1 {
                cv.fill(Rect::from_min_size(pos2(x0, ry), vec2(cw * cols as f32, rh)), th.bg.linear_multiply(0.35));
            }
            if sel || learning {
                cv.fill(row, if learning { th.accent } else { th.text_dim.linear_multiply(0.35) });
            }
            let note_w = cv.text_w("127") + PAD_X;
            let name_rect = Rect::from_min_size(row.min, vec2(row.width() - note_w, rh));
            let note_rect = Rect::from_min_max(pos2(row.max.x - note_w, row.min.y), row.max);
            let rn = ui.interact(snap_rect(name_rect), ui.id().with(("pn", pi)), Sense::click());
            if rn.clicked() {
                self.selected = pi;
                self.model.with(snap, |s| {
                    if s.learn.active {
                        s.learn.piece = pi;
                    }
                });
            }
            let rv = ui.interact(snap_rect(note_rect), ui.id().with(("pv", pi)), Sense::click());
            let cur = view.notes[pi];
            if rv.clicked() {
                self.model.with(snap, |s| s.set_note(pi, cur.saturating_add(1).min(127)));
            }
            if rv.secondary_clicked() {
                self.model.with(snap, |s| s.set_note(pi, cur.saturating_sub(1)));
            }
            let group_col = th.group(PIECES[pi].group);
            cv.fill(Rect::from_min_size(pos2(row.min.x, ry + 2.0), vec2(4.0, rh - 4.0)), group_col);
            let tc = if learning { th.bg } else { th.text };
            cv.text(pos2(row.min.x + 8.0, ry + rh / 2.0), Align2::LEFT_CENTER, PIECES[pi].name, tc);
            cv.text(pos2(row.max.x - 4.0, ry + rh / 2.0), Align2::RIGHT_CENTER, &cur.to_string(), if learning { th.bg } else { th.text_dim });
        }

        // celdas
        let cells = cv.clipped(grid.expand(1.0));
        let cc = Cv { p: &cells, th: *th, fs: cv.fs };
        for (r, &pi) in rows.iter().enumerate() {
            let ry = y0 + r as f32 * rh;
            let col = th.group(PIECES[pi].group);
            for c in 0..cols {
                let st = first_bar * STEPS_PER_BAR + c;
                let cell = view.pattern.cell[pi][st];
                let cx = x0 + c as f32 * cw;
                let slot = Rect::from_min_size(pos2(cx + 1.0, ry + 1.0), vec2(cw - 2.0, rh - 2.0));
                if !cell.on() {
                    cc.fill(slot, th.bg.linear_multiply(0.6));
                } else if cell.ghost {
                    let h = ((rh - 2.0) * 0.5).round().max(4.0);
                    let r2 = Rect::from_min_max(pos2(slot.min.x, slot.max.y - h), slot.max);
                    cc.boxed(r2, th.panel, col, 2.0);
                } else {
                    let h = ((rh - 2.0) * f32::from(cell.vel) / 127.0).round().max(5.0);
                    cc.fill(Rect::from_min_max(pos2(slot.min.x, slot.max.y - h), slot.max), col);
                }
            }
        }
        // líneas de pulso y de compás
        for c in 0..=cols {
            let lx = x0 + c as f32 * cw;
            if c % STEPS_PER_BAR == 0 {
                cc.fill(Rect::from_min_size(pos2(lx - 1.0, y0), vec2(3.0, grid_h)), th.border);
            } else if c % 4 == 0 {
                cc.fill(Rect::from_min_size(pos2(lx - 1.0, y0), vec2(2.0, grid_h)), th.text_dim);
            }
        }
        // cabezal del host: barra de píxeles
        if running {
            let st = step_now as usize;
            if st >= first_bar * STEPS_PER_BAR && st < first_bar * STEPS_PER_BAR + cols {
                let cx = x0 + (st - first_bar * STEPS_PER_BAR) as f32 * cw;
                cc.fill(Rect::from_min_size(pos2(cx, y_head), vec2(cw, head_h + grid_h)), Color32::from_rgba_unmultiplied(th.preview.r(), th.preview.g(), th.preview.b(), 60));
                cc.fill(Rect::from_min_size(pos2(cx, y_head), vec2(4.0, head_h + grid_h)), th.preview);
            }
        }

        // interacción: clic = golpe (y suena), arrastre = velocidad, clic derecho = ghost
        let cell_at = |pos: Pos2| -> Option<(usize, usize)> {
            if !grid.contains(pos) {
                return None;
            }
            let c = ((pos.x - x0) / cw) as usize;
            let r = ((pos.y - y0) / rh) as usize;
            Some((*rows.get(r)?, first_bar * STEPS_PER_BAR + c))
        };
        let resp = ui.interact(snap_rect(grid), ui.id().with("grid"), Sense::click_and_drag());
        let sound = |pi: usize, vel: i32| {
            self.model.host.push_preview(view.notes[pi], vel.clamp(1, 127) as u8);
        };
        if resp.drag_started_by(egui::PointerButton::Primary) {
            if let Some((pi, st)) = ui.input(|i| i.pointer.press_origin()).and_then(cell_at) {
                let cell = view.pattern.cell[pi][st];
                self.drag = Some(Drag { piece: pi, step: st, start_vel: i32::from(cell.vel), existed: cell.on() });
                if cell.on() {
                    sound(pi, i32::from(cell.vel));
                }
            }
        }
        if resp.dragged_by(egui::PointerButton::Primary) {
            if let (Some(d), Some(pos), Some(o)) = (&self.drag, resp.interact_pointer_pos(), ui.input(|i| i.pointer.press_origin())) {
                if d.existed {
                    let vel = d.start_vel + ((o.y - pos.y) * 1.3) as i32;
                    let (pi, st) = (d.piece, d.step);
                    self.model.with(snap, |s| s.set_cell_vel(pi, st, vel));
                }
            }
        }
        if resp.drag_stopped() {
            self.drag = None;
        }
        if resp.clicked_by(egui::PointerButton::Primary) {
            if let Some((pi, st)) = resp.interact_pointer_pos().and_then(cell_at) {
                self.selected = pi;
                let before = view.pattern.cell[pi][st];
                self.model.with(snap, |s| s.toggle_cell(pi, st, false));
                sound(pi, if before.on() { i32::from(before.vel) } else { 100 });
            }
        }
        if resp.secondary_clicked() {
            if let Some((pi, st)) = resp.interact_pointer_pos().and_then(cell_at) {
                self.selected = pi;
                self.model.with(snap, |s| s.toggle_ghost(pi, st));
                sound(pi, 40);
            }
        }
        let mut f2 = Flow::new(left, y0 + grid_h + GAP * 2.0, right);
        let h = cv.wrapped(pos2(left, f2.y), right - left, "Clic: golpe (y suena el kit) · arrastre vertical: velocidad · clic derecho: ghost. Clic en el nombre: elegir pista; en el numero: nota del kit (derecho resta).", th.text_dim);
        f2.y += h;
        f2.y + GAP * 2.0
    }

    // ───────────────────────── kit (mapa de notas) ─────────────────────────

    fn kit(&mut self, ui: &egui::Ui, cv: &Cv, left: f32, right: f32, y: f32, view: &View) -> f32 {
        let snap = self.model_snap();
        let now = ui.input(|i| i.time);
        let mut f = Flow::new(left, y, right);
        row_label(cv, &mut f, "Kit de la Drum Machine");
        for (i, name) in view.maps.iter().enumerate() {
            if button(ui, cv, &mut f, &format!("map{i}"), name, i == view.map_index) {
                self.model.with(snap, |s| s.select_map(i));
            }
        }
        f.newline();
        if button(ui, cv, &mut f, "learn", if view.learn_active { "Aprender: activo" } else { "Aprender" }, view.learn_active) {
            let sel = self.selected;
            self.model.with(snap, |s| {
                if s.learn.active {
                    s.learn.stop();
                } else {
                    s.learn.start(sel);
                }
            });
        }
        let w = (cv.text_w("Nombre del mapa") + 12.0 * cv.fs).min((right - left) * 0.4);
        let r = f.place(w, button_h(cv));
        self.text_field(ui, cv, r, "mapname", Focus::MapName, "Nombre del mapa", now);
        if button(ui, cv, &mut f, "savemap", "Guardar mapa", false) {
            let name = self.map_name.clone();
            let res = self.model.with(snap, |s| s.save_current_map(&name));
            self.status = match res {
                Ok(path) => format!("Mapa guardado en {}", path.display()),
                Err(e) => e,
            };
        }
        f.newline();
        let hint = if view.learn_active {
            format!("Aprender: pulsa una tecla de tu controlador para {}; luego pasa a la siguiente pieza.", PIECES[view.learn_piece.min(NUM_PIECES - 1)].name)
        } else {
            "Aprender: elige una pista y pulsa Aprender; cada tecla que toques se asigna a esa pieza.".to_string()
        };
        let h = cv.wrapped(pos2(left, f.y), right - left, &hint, cv.th.text_dim);
        f.y += h;
        f.y + GAP * 2.0
    }

    fn model_snap(&self) -> ParamSnap {
        self.model.session.lock().map(|s| s.snap()).unwrap_or_default()
    }

    // ───────────────────────── controles ─────────────────────────

    fn controls(&mut self, ui: &egui::Ui, cv: &Cv, setter: &ParamSetter, p: &CajaParams, left: f32, right: f32, y: f32) -> f32 {
        let mut f = Flow::new(left, y, right);
        row_label(cv, &mut f, "Compases");
        for (i, b) in BARS_CHOICES.iter().enumerate() {
            if button(ui, cv, &mut f, &format!("bars{i}"), &b.to_string(), p.bars.value() == i as i32) {
                set(setter, &p.bars, i as i32);
            }
        }
        row_label(cv, &mut f, "Modo");
        for (i, name) in RUN_MODES.iter().enumerate() {
            if button(ui, cv, &mut f, &format!("mode{i}"), name, p.run_mode.value() == i as i32) {
                set(setter, &p.run_mode, i as i32);
            }
        }
        if button(ui, cv, &mut f, "thru", if p.thru.value() { "Thru: si" } else { "Thru: no" }, p.thru.value()) {
            set(setter, &p.thru, !p.thru.value());
        }
        f.newline();
        let ctrl_cols = (((right - left) / 240.0).floor() as usize).max(1);
        let cell_w = ((right - left) - (ctrl_cols as f32 - 1.0) * GAP) / ctrl_cols as f32;
        let bh = button_h(cv);
        let mut g = Flow::new(left, f.y, right);
        let items: [(&str, &IntParam, i32, i32); 7] = [
            ("Swing", &p.swing, 0, 100),
            ("Humanizacion", &p.human, 0, 100),
            ("Densidad", &p.density, 0, 100),
            ("Acento", &p.accent, 0, 100),
            ("Velocidad %", &p.vel_scale, 25, 200),
            ("BPM libre", &p.bpm, 60, 210),
            ("Canal MIDI", &p.channel, 1, 16),
        ];
        for (name, param, lo, hi) in items {
            let r = g.place(cell_w, bh);
            slider(ui, cv, setter, r, name, name, param, lo, hi, &param.value().to_string());
        }
        let r = g.place(cell_w, bh);
        let cc = p.cc_num.value();
        slider(ui, cv, setter, r, "cc", "CC por paso", &p.cc_num, 0, 127, &if cc == 0 { "apagado".to_string() } else { cc.to_string() });
        // semilla: valor escrito + dado
        let r = g.place(cell_w, bh);
        let dice_w = cv.text_w("Dado") + 2.0 * PAD_X;
        cv.text(pos2(r.min.x, r.center().y), Align2::LEFT_CENTER, "Semilla", cv.th.text_dim);
        let label_w = r.width() * 0.42;
        let val = Rect::from_min_max(pos2(r.min.x + label_w, r.min.y), pos2(r.max.x - dice_w - GAP, r.max.y));
        cv.boxed(val, cv.th.panel, cv.th.text_dim, 2.0);
        cv.text(val.center(), Align2::CENTER_CENTER, &p.seed.value().to_string(), cv.th.text);
        let dice = Rect::from_min_max(pos2(r.max.x - dice_w, r.min.y), r.max);
        if button_at(ui, cv, dice, "dice", "Dado", false) {
            let snap = p.snap();
            let n = self.model.with(snap, |s| s.reroll(snap, false));
            apply_snap(setter, p, n);
        }
        g.bottom() + GAP * 2.0
    }

    // ───────────────────────── exportar y acciones ─────────────────────────

    fn export_row(&mut self, ui: &egui::Ui, cv: &Cv, setter: &ParamSetter, p: &CajaParams, left: f32, right: f32, y: f32, view: &View) -> f32 {
        let snap = p.snap();
        let mut f = Flow::new(left, y, right);
        if button(ui, cv, &mut f, "regen", "Regenerar", false) {
            let n = self.model.with(snap, |s| s.reroll(snap, true));
            apply_snap(setter, p, n);
        }
        let aud = self.model.host.audition.load(Ordering::Relaxed);
        if button(ui, cv, &mut f, "probar", if aud { "Probar: sonando" } else { "Probar" }, aud) {
            self.model.host.audition.store(!aud, Ordering::Relaxed);
        }
        row_label(cv, &mut f, "Exportar");
        if button(ui, cv, &mut f, "fmt", if self.format1 { "Formato 1" } else { "Formato 0" }, false) {
            self.format1 = !self.format1;
        }
        if button(ui, cv, &mut f, "gm", if self.use_gm { "Notas GM" } else { "Notas del kit actual" }, false) {
            self.use_gm = !self.use_gm;
        }
        if button(ui, cv, &mut f, "export", "Exportar .mid", false) {
            let (sep, gm) = (self.format1, self.use_gm);
            let res = self.model.with(snap, |s| s.export(sep, gm));
            self.status = match res {
                Ok(path) => format!("Guardado en {}", path.display()),
                Err(e) => e,
            };
        }
        if button(ui, cv, &mut f, "openfolder", "Abrir carpeta", view.has_last_export) {
            self.status = match caja_core::paths::export_dir() {
                Some(d) => match caja_core::paths::open_folder(&d) {
                    Ok(()) => format!("Abriendo {}", d.display()),
                    Err(e) => e,
                },
                None => "No encuentro la carpeta Documentos.".to_string(),
            };
        }
        f.newline();
        if !self.status.is_empty() {
            let h = cv.wrapped(pos2(left, f.y), right - left, &self.status, cv.th.accent);
            f.y += h + GAP;
        }
        f.y + GAP
    }

    // ───────────────────────── historial ─────────────────────────

    fn history(&mut self, ui: &egui::Ui, cv: &Cv, setter: &ParamSetter, p: &CajaParams, left: f32, right: f32, y: f32, view: &View) -> f32 {
        let mut f = Flow::new(left, y, right);
        row_label(cv, &mut f, "Historial");
        if view.history.is_empty() {
            row_label(cv, &mut f, "vacio: pulsa Generar o Regenerar");
        }
        for (i, c) in view.history.iter().enumerate() {
            let label = format!("{} · {} BPM · {} {}", c.genre, c.bpm, c.bars, if c.bars == 1 { "compas" } else { "compases" });
            if button(ui, cv, &mut f, &format!("h{i}"), &label, i == 0) {
                let snap = p.snap();
                if let Some(n) = self.model.with(snap, |s| s.restore(i)) {
                    self.phrase = c.phrase.clone();
                    apply_snap(setter, p, n);
                }
            }
        }
        f.bottom() + GAP * 2.0
    }

    // ───────────────────────── lectura y avisos ─────────────────────────

    fn reading(&mut self, cv: &Cv, left: f32, right: f32, y: f32, view: &View) -> f32 {
        let th = &cv.th;
        let w = right - left;
        let text_h = {
            let g = cv.p.layout(view.reading.clone(), cv.font(), th.text, w - 4.0 * PAD_X);
            g.size().y
        };
        let r = Rect::from_min_size(pos2(left, y), vec2(w, text_h + cv.fs + 4.0 * PAD_Y));
        cv.boxed(r, th.panel, th.border, 2.0);
        cv.text(pos2(r.min.x + PAD_X, r.min.y + PAD_Y + cv.fs / 2.0), Align2::LEFT_CENTER, "Lectura", th.accent);
        cv.wrapped(pos2(r.min.x + PAD_X, r.min.y + 2.0 * PAD_Y + cv.fs), w - 2.0 * PAD_X, &view.reading, th.text);
        let mut yy = r.max.y + GAP;
        if !view.message.is_empty() {
            yy += cv.wrapped(pos2(left, yy), w, &view.message, th.accent) + GAP;
        }
        for e in &view.errors {
            yy += cv.wrapped(pos2(left, yy), w, e, th.accent) + 2.0;
        }
        yy + GAP
    }
}

#[cfg(test)]
mod tests {
    //! Prueba de humo SIN pantalla: dibuja la interfaz con un contexto de egui y un `GuiContext`
    //! falso que REGISTRA los cambios de parámetros (los parámetros de nih-plug no se pueden escribir
    //! desde fuera de un host). Comprueba que no hay pánicos y que el teclado y las acciones producen los
    //! cambios de parámetros y de sesión esperados. NO comprueba el aspecto (eso hay que verlo en un host).

    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeCtx {
        sets: Mutex<Vec<(String, f32)>>,
    }

    impl GuiContext for FakeCtx {
        fn plugin_api(&self) -> PluginApi {
            PluginApi::Clap
        }
        fn request_resize(&self) -> bool {
            true
        }
        unsafe fn raw_begin_set_parameter(&self, _param: ParamPtr) {}
        unsafe fn raw_set_parameter_normalized(&self, param: ParamPtr, normalized: f32) {
            self.sets.lock().unwrap().push((param.name().to_string(), normalized));
        }
        unsafe fn raw_end_set_parameter(&self, _param: ParamPtr) {}
        fn get_state(&self) -> nih_plug::wrapper::state::PluginState {
            unimplemented!()
        }
        fn set_state(&self, _state: nih_plug::wrapper::state::PluginState) {}
    }

    fn setup_all(bars_idx: i32) -> (Arc<Model>, Arc<CajaParams>, Gui, egui::Context) {
        let state = Arc::new(Mutex::new(String::new()));
        let model = Arc::new(Model::with_session(caja_core::session::Session::builtin(), state.clone()));
        let mut params = CajaParams::new(state, Arc::new(Mutex::new(String::new())), model.genre_names.clone());
        params.bars = crate::params::int("Compases", bars_idx, 0, 3);
        let gui = Gui::new(
            model.clone(),
            KitShared::new(Arc::new(Mutex::new(String::new()))),
            Arc::new(Mutex::new(Mk3Bridge::default())),
        );
        let ctx = egui::Context::default();
        setup(&ctx);
        (model, Arc::new(params), gui, ctx)
    }

    fn run(ctx: &egui::Context, fake: &FakeCtx, gui: &mut Gui, params: &CajaParams, w: f32, h: f32, events: Vec<egui::Event>) {
        let setter = ParamSetter::new(fake);
        let mut input = egui::RawInput::default();
        input.screen_rect = Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(w, h)));
        input.events = events;
        let th = Theme::new(params.palette.value(), params.dark.value());
        let fs = params.text_size.value().px();
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| gui.paint(ui, &setter, params, &th, fs));
            });
        });
    }

    #[test]
    fn draws_at_several_sizes_without_panicking() {
        for bars_idx in [0, 1, 3] {
            let (_m, params, mut gui, ctx) = setup_all(bars_idx);
            let fake = FakeCtx::default();
            for (w, h) in [(1280.0, 860.0), (760.0, 520.0), (1900.0, 1000.0), (300.0, 200.0)] {
                for show_all in [false, true] {
                    gui.show_all = show_all;
                    run(&ctx, &fake, &mut gui, &params, w, h, vec![]);
                }
            }
        }
    }

    #[test]
    fn typing_a_phrase_and_pressing_enter_requests_the_right_parameters() {
        let (model, params, mut gui, ctx) = setup_all(1);
        let fake = FakeCtx::default();
        run(&ctx, &fake, &mut gui, &params, 1280.0, 860.0, vec![]);
        gui.focus = Focus::Phrase;
        let key = |k| egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE };
        run(&ctx, &fake, &mut gui, &params, 1280.0, 860.0, vec![egui::Event::Text("techno 132 bpm con fill 4 compases".into()), key(egui::Key::Enter)]);
        assert_eq!(gui.phrase, "techno 132 bpm con fill 4 compases");
        let sets = fake.sets.lock().unwrap().clone();
        let plain = |name: &str, f: &dyn Fn(f32) -> i32| sets.iter().rev().find(|(n, _)| n == name).map(|(_, v)| f(*v));
        assert_eq!(plain("Genero", &|v| params.genre.preview_plain(v)), Some(15), "Techno es el género 15");
        assert_eq!(plain("BPM libre", &|v| params.bpm.preview_plain(v)), Some(132));
        assert_eq!(plain("Compases", &|v| params.bars.preview_plain(v)), Some(2), "4 compases = índice 2");
        let snap_now = model.session.lock().unwrap().snap(); // el guard se suelta aquí (si no, `with` se bloquearía)
        model.with(snap_now, |s| {
            assert_eq!(s.history.len(), 1);
            assert!(s.reading().contains("fill"));
            assert_eq!(s.pattern.bars, 4);
        });
        // Backspace borra y Escape suelta el foco
        run(&ctx, &fake, &mut gui, &params, 1280.0, 860.0, vec![key(egui::Key::Backspace), key(egui::Key::Escape)]);
        assert!(gui.phrase.ends_with("compase"));
        assert!(gui.focus == Focus::None);
    }

    #[test]
    fn pages_learn_and_playhead() {
        let (model, params, mut gui, ctx) = setup_all(3); // 8 compases
        let fake = FakeCtx::default();
        gui.show_all = true;
        for page in 0..4 {
            gui.page = page;
            run(&ctx, &fake, &mut gui, &params, 1280.0, 860.0, vec![]);
            run(&ctx, &fake, &mut gui, &params, 700.0, 600.0, vec![]);
        }
        model.with(params.snap(), |s| assert_eq!(s.pattern.bars, 8));
        // modo aprender: una nota recibida por la entrada se asigna a la pieza elegida
        model.with(params.snap(), |s| s.learn.start(3));
        model.host.note_seen(61);
        run(&ctx, &fake, &mut gui, &params, 1280.0, 860.0, vec![]);
        model.with(params.snap(), |s| {
            assert_eq!(s.notes[3], 61);
            assert_eq!(s.learn.piece, 4);
        });
        // con el transporte en marcha, la página sigue al cabezal (compás 3 -> página 2 con 2 compases por página)
        model.host.playing.store(true, Ordering::Relaxed);
        model.host.step.store(37, Ordering::Relaxed);
        run(&ctx, &fake, &mut gui, &params, 1280.0, 860.0, vec![]);
        assert_eq!(gui.page, 1);
    }
}
