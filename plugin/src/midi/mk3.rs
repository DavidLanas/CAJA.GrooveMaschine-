//! Runtime del Maschine Mikro MK3 para CAJA (modo NATIVO / standalone).
use std::collections::VecDeque;
use std::sync::Mutex;

pub const CC_SWING: u8 = 24;
pub const CC_PLUGIN: u8 = 26;
pub const CC_SAMPLING: u8 = 27;
pub const CC_GROUP: u8 = 34;
pub const CC_AUTO: u8 = 35;
pub const CC_LOCK: u8 = 36;
pub const CC_NOTE_REPEAT: u8 = 37;
pub const CC_RESTART: u8 = 38;
pub const CC_ERASE: u8 = 39;
pub const CC_TAP: u8 = 40;
pub const CC_FOLLOW: u8 = 41;
pub const CC_KEYBOARD: u8 = 48;
pub const CC_CHORDS: u8 = 49;
pub const CC_ENCODER: u8 = 1;
pub const CC_SLIDER: u8 = 9;
pub const CC_VOLUME: u8 = 23;
pub const CC_TEMPO: u8 = 25;
pub const CC_LEFT: u8 = 28;
pub const CC_RIGHT: u8 = 29;
pub const CC_PLAY: u8 = 42;
pub const CC_REC: u8 = 43;
pub const CC_STOP: u8 = 44;
pub const CC_SHIFT: u8 = 45;
pub const CC_PAD_MODE: u8 = 47;
pub const CC_STEP: u8 = 50;
pub const CC_PATTERN: u8 = 52;
pub const CC_SELECT: u8 = 56;
pub const CC_SOLO: u8 = 57;
pub const CC_MUTE: u8 = 58;
pub const CC_FIXED_VEL: u8 = 46;
pub const CC_BROWSE: u8 = 22;
pub const CC_STAR: u8 = 21;
pub const CC_SCENE: u8 = 51;

pub const PAD_NOTES: [u8; 16] = [48, 49, 50, 51, 44, 45, 46, 47, 40, 41, 42, 43, 36, 37, 38, 39];

pub mod led {
    pub const OFF: u8 = 0;
    pub const RED: u8 = 4;
    pub const ORANGE: u8 = 11;
    pub const WARM_YELLOW: u8 = 25;
    pub const YELLOW: u8 = 32;
    pub const GREEN: u8 = 46;
    pub const CYAN: u8 = 60;
    pub const BLUE: u8 = 74;
    pub const PURPLE: u8 = 95;
    pub const WHITE: u8 = 120;
    pub const DIM: u8 = 20;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mk3Mode { Perform, Pattern, Rack, Mixer, Samples, Library }
impl Default for Mk3Mode { fn default() -> Self { Mk3Mode::Perform } }
impl Mk3Mode {
    pub fn label(self) -> &'static str {
        match self {
            Mk3Mode::Perform => "Perform",
            Mk3Mode::Pattern => "Pattern",
            Mk3Mode::Rack => "Rack",
            Mk3Mode::Mixer => "Mixer",
            Mk3Mode::Samples => "Samples",
            Mk3Mode::Library => "Library",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum Mk3Cmd {
    SelectPattern(u8), RackBarDelta(i8), SelectRow(u8), ToggleStep { piece: u8, step: u8 }, ToggleGhost { step: u8 },
    SetMode(Mk3Mode), SetBank(u8), BpmDelta(i8), MasterDelta(i8), SwingDelta(i8), AccentDelta(i8), HumanDelta(i8),
    SelectCell(usize), CellVolAbsolute(u8), CellPanDelta(i8), ToggleCellMute(usize), SamplesDice(usize), SamplesSave(usize),
    ToggleCellLockBit(usize, u8), GlobalAction(u8), OpenExplorer, LibrarySetting(usize, u8), ShiftPadAction(usize),
    ToggleShowAll, ToggleSuggestions, FocusPhrase, SelectGenre(usize),
}

#[derive(Clone, Copy, Debug)]
pub enum Mk3Out { PadLed { note: u8, color: u8 }, ButtonLed { cc: u8, value: u8 } }

#[derive(Clone, Copy, Debug, Default)]
pub struct Mk3LedState {
    pub mode: Mk3Mode, pub pattern_bank: u8, pub pattern_active: u8, pub pattern_filled: u16,
    pub rack_steps: [u8; 16], pub playhead: u8, pub playing: bool,
    pub pad_mutes: [bool; caja_core::kit::NUM_PADS], pub pad_locks: [u8; caja_core::kit::NUM_PADS],
}

pub struct Mk3Bridge {
    pub enabled: bool, mode: Mk3Mode, shift: bool, select: bool, pattern_bank: u8, last_encoder: u8,
    enc_mode: u8, swing_mode: u8, selected_cell: usize, cell_offset: usize, dice_pad: Option<usize>,
    pub fixed_vel: bool, genre_pad_mode: bool, genre_page: usize,
    cmds: Mutex<VecDeque<Mk3Cmd>>, out: Mutex<VecDeque<Mk3Out>>, last_pads: [u8; 16], last_buttons: [u8; 12],
}

impl Default for Mk3Bridge {
    fn default() -> Self {
        Mk3Bridge {
            enabled: true, mode: Mk3Mode::Perform, shift: false, select: false, pattern_bank: 0, last_encoder: 64,
            enc_mode: 1, swing_mode: 0, selected_cell: 0, cell_offset: 0, dice_pad: None,
            fixed_vel: false, genre_pad_mode: false, genre_page: 0,
            cmds: Mutex::new(VecDeque::with_capacity(64)), out: Mutex::new(VecDeque::with_capacity(64)),
            last_pads: [0xff; 16], last_buttons: [0xff; 12],
        }
    }
}

impl Mk3Bridge {
    fn push(&self, cmd: Mk3Cmd) { if let Ok(mut q) = self.cmds.lock() { if q.len() < 128 { q.push_back(cmd); } } }
    pub fn drain(&self, max: usize) -> Vec<Mk3Cmd> {
        let mut out = Vec::new();
        if let Ok(mut q) = self.cmds.lock() { while out.len() < max { match q.pop_front() { Some(c) => out.push(c), None => break } } }
        out
    }
    pub fn drain_out(&self, max: usize) -> Vec<Mk3Out> {
        let mut out = Vec::new();
        if let Ok(mut q) = self.out.lock() { while out.len() < max { match q.pop_front() { Some(c) => out.push(c), None => break } } }
        out
    }
    pub fn mode(&self) -> Mk3Mode { self.mode }
    pub fn pattern_bank(&self) -> u8 { self.pattern_bank }
    pub fn set_enabled(&mut self, on: bool) { self.enabled = on; if !on { self.queue_all_pads_off(); } }
    fn queue_all_pads_off(&mut self) {
        if let Ok(mut q) = self.out.lock() {
            for (i, &note) in PAD_NOTES.iter().enumerate() { q.push_back(Mk3Out::PadLed { note, color: led::OFF }); self.last_pads[i] = led::OFF; }
        }
    }

    pub fn update_leds(&mut self, st: &Mk3LedState) {
        if !self.enabled { return; }
        let mut pad_colors = [led::OFF; 16];
        match st.mode {
            Mk3Mode::Perform => {
                if self.genre_pad_mode {
                    for i in 0..16 { pad_colors[i] = led::CYAN; }
                }
            }
            Mk3Mode::Pattern => {
                for i in 0..16 {
                    let filled = (st.pattern_filled >> i) & 1 == 1;
                    let active_local = (st.pattern_active as usize % 16 == i) && ((st.pattern_active as usize / 16) == st.pattern_bank as usize);
                    pad_colors[i] = if active_local { led::WHITE } else if filled { led::CYAN } else { led::DIM };
                }
            }
            Mk3Mode::Rack => {
                for i in 0..16 {
                    let vel = st.rack_steps[i];
                    pad_colors[i] = if st.playhead == i as u8 && st.playing { led::WHITE }
                    else if vel > 0 { if vel < 50 { led::WARM_YELLOW } else { led::GREEN } } else { led::DIM };
                }
            }
            Mk3Mode::Mixer => {
                for i in 0..16 {
                    let idx = self.cell_offset + i;
                    if idx >= caja_core::kit::NUM_PADS { pad_colors[i] = led::OFF; continue; }
                    if idx == self.selected_cell { pad_colors[i] = led::WHITE; }
                    else if st.pad_mutes[idx] { pad_colors[i] = led::RED; }
                    else { pad_colors[i] = led::DIM; }
                }
            }
            Mk3Mode::Samples => {
                for i in 0..16 {
                    let idx = self.cell_offset + i;
                    if idx >= caja_core::kit::NUM_PADS { pad_colors[i] = led::OFF; continue; }
                    if self.dice_pad == Some(idx) { pad_colors[i] = led::GREEN; }
                    else if idx == self.selected_cell { pad_colors[i] = led::WHITE; }
                    else { pad_colors[i] = led::DIM; }
                }
            }
            Mk3Mode::Library => { pad_colors.fill(led::DIM); }
        }

        if let Ok(mut q) = self.out.lock() {
            for i in 0..16 {
                if pad_colors[i] != self.last_pads[i] {
                    q.push_back(Mk3Out::PadLed { note: PAD_NOTES[i], color: pad_colors[i] });
                    self.last_pads[i] = pad_colors[i];
                }
            }
            let mode_leds: [(u8, u8); 7] = [
                (CC_PAD_MODE, if st.mode == Mk3Mode::Perform { 127 } else { 42 }),
                (CC_PATTERN, if st.mode == Mk3Mode::Pattern { 127 } else { 42 }),
                (CC_STEP, if st.mode == Mk3Mode::Rack { 127 } else { 42 }),
                (CC_PLAY, if st.playing { 127 } else { 42 }),
                (CC_KEYBOARD, if st.mode == Mk3Mode::Mixer { 127 } else { 42 }),
                (CC_CHORDS, if st.mode == Mk3Mode::Samples { 127 } else { 42 }),
                (CC_SAMPLING, if st.mode == Mk3Mode::Library { 127 } else { 42 }),
            ];
            for (j, &(cc, val)) in mode_leds.iter().enumerate() {
                if j < self.last_buttons.len() && self.last_buttons[j] != val {
                    q.push_back(Mk3Out::ButtonLed { cc, value: val });
                    self.last_buttons[j] = val;
                }
            }
        }
    }

    pub fn pulse_mode_leds(&mut self, playing: bool) {
        let dim = 42u8;
        let buttons: [(u8, u8); 12] = [
            (CC_PAD_MODE, if self.mode == Mk3Mode::Perform { 127 } else { dim }),
            (CC_PATTERN, if self.mode == Mk3Mode::Pattern { 127 } else { dim }),
            (CC_STEP, if self.mode == Mk3Mode::Rack { 127 } else { dim }),
            (CC_PLAY, if playing { 127 } else { dim }),
            (CC_SHIFT, if self.shift { 127 } else { 0 }),
            (CC_TEMPO, if self.enc_mode == 1 { 127 } else { 0 }),
            (CC_VOLUME, if self.enc_mode == 2 { 127 } else { 0 }),
            (CC_SWING, if self.swing_mode > 0 { 127 } else { 0 }),
            (CC_KEYBOARD, if self.mode == Mk3Mode::Mixer { 127 } else { dim }),
            (CC_CHORDS, if self.mode == Mk3Mode::Samples { 127 } else { dim }),
            (CC_SAMPLING, if self.mode == Mk3Mode::Library { 127 } else { dim }),
            (CC_FIXED_VEL, if self.fixed_vel { 127 } else { 0 }),
        ];
        if let Ok(mut q) = self.out.lock() {
            for (i, (cc, value)) in buttons.iter().enumerate() {
                if i < self.last_buttons.len() && self.last_buttons[i] != *value {
                    self.last_buttons[i] = *value;
                    q.push_back(Mk3Out::ButtonLed { cc: *cc, value: *value });
                }
            }
        }
    }

    pub fn handle_cc(&mut self, cc: u8, value: u8) {
        if !self.enabled { return; }
        let press = value > 0;
        match cc {
            CC_ENCODER => {
                let raw = if value >= 65 { value as i16 - 128 } else { value as i16 };
                let inc = (-raw).clamp(-8, 8) as i8;
                if inc == 0 { return; }
                
                if self.mode == Mk3Mode::Samples {
                    if self.enc_mode == 2 { self.push(Mk3Cmd::MasterDelta(inc)); }
                    else { self.push(Mk3Cmd::CellPanDelta(inc)); }
                } else if self.swing_mode > 0 {
                    match self.swing_mode {
                        1 => self.push(Mk3Cmd::SwingDelta(inc)),
                        2 => self.push(Mk3Cmd::AccentDelta(inc)),
                        3 => self.push(Mk3Cmd::HumanDelta(inc)),
                        _ => {}
                    }
                } else {
                    if self.shift || self.enc_mode != 1 { self.push(Mk3Cmd::MasterDelta(inc)); }
                    else { self.push(Mk3Cmd::BpmDelta(inc)); }
                }
            }
            CC_SHIFT => self.shift = press,
            CC_SELECT => self.select = press,
            CC_SWING if press => { self.swing_mode = (self.swing_mode + 1) % 4; }
            CC_KEYBOARD if press => { self.mode = Mk3Mode::Mixer; self.push(Mk3Cmd::SetMode(Mk3Mode::Mixer)); }
            CC_CHORDS if press => { self.mode = Mk3Mode::Samples; self.push(Mk3Cmd::SetMode(Mk3Mode::Samples)); }
            CC_SAMPLING if press => { self.mode = Mk3Mode::Library; self.push(Mk3Cmd::SetMode(Mk3Mode::Library)); }
            CC_PLUGIN if press => self.push(Mk3Cmd::OpenExplorer),
            CC_SLIDER if self.mode == Mk3Mode::Mixer => self.push(Mk3Cmd::CellVolAbsolute(value)),
            CC_FIXED_VEL if press => { self.fixed_vel = !self.fixed_vel; },
            CC_STAR if press => self.push(Mk3Cmd::ToggleSuggestions),
            CC_SCENE if press => self.push(Mk3Cmd::ToggleShowAll),
            CC_BROWSE if press => {
                self.genre_pad_mode = !self.genre_pad_mode;
                if self.genre_pad_mode { self.push(Mk3Cmd::FocusPhrase); }
            }
            // SWAP: Now Restart/Erase/Tap/Follow map to Muestra/Pitch/Decay/Filtro
            CC_RESTART if press => { if self.mode == Mk3Mode::Samples { self.push(Mk3Cmd::ToggleCellLockBit(self.selected_cell, 0)); } else { self.push(Mk3Cmd::GlobalAction(0)); } }
            CC_ERASE if press => { if self.mode == Mk3Mode::Samples { self.push(Mk3Cmd::ToggleCellLockBit(self.selected_cell, 1)); } else { self.push(Mk3Cmd::GlobalAction(1)); } }
            CC_TAP if press => { if self.mode == Mk3Mode::Samples { self.push(Mk3Cmd::ToggleCellLockBit(self.selected_cell, 2)); } else { self.push(Mk3Cmd::GlobalAction(2)); } }
            CC_FOLLOW if press => { if self.mode == Mk3Mode::Samples { self.push(Mk3Cmd::ToggleCellLockBit(self.selected_cell, 3)); } else { self.push(Mk3Cmd::GlobalAction(3)); } }
            // SWAP: Now Group/Auto/Lock/NoteRepeat map to All Roll/All Lock etc
            CC_GROUP if press => { if self.mode == Mk3Mode::Samples { self.push(Mk3Cmd::GlobalAction(4)); } }
            CC_AUTO if press => { if self.mode == Mk3Mode::Samples { self.push(Mk3Cmd::GlobalAction(5)); } }
            CC_LOCK if press => { if self.mode == Mk3Mode::Samples { self.push(Mk3Cmd::GlobalAction(6)); } }
            CC_NOTE_REPEAT if press => { if self.mode == Mk3Mode::Samples { self.push(Mk3Cmd::GlobalAction(7)); } }
            CC_MUTE if press && self.mode == Mk3Mode::Mixer => { self.push(Mk3Cmd::ToggleCellMute(self.selected_cell)); }
            CC_REC if press => self.push(Mk3Cmd::GlobalAction(8)),
            CC_PLAY if press => { self.genre_pad_mode = false; self.push(Mk3Cmd::GlobalAction(9)); }
            CC_STOP if press => { self.genre_pad_mode = false; self.push(Mk3Cmd::GlobalAction(10)); }
            CC_TEMPO if press => { self.enc_mode = if self.enc_mode == 1 { 0 } else { 1 }; }
            CC_VOLUME if press => { self.enc_mode = if self.enc_mode == 2 { 0 } else { 2 }; }
            CC_PATTERN if press => { self.mode = Mk3Mode::Pattern; self.push(Mk3Cmd::SetMode(Mk3Mode::Pattern)); }
            CC_STEP if press => { self.mode = Mk3Mode::Rack; self.push(Mk3Cmd::SetMode(Mk3Mode::Rack)); }
            CC_PAD_MODE if press => { self.mode = Mk3Mode::Perform; self.push(Mk3Cmd::SetMode(Mk3Mode::Perform)); }
            CC_LEFT if press => {
                if self.genre_pad_mode { self.genre_page = self.genre_page.saturating_sub(1); }
                else if self.mode == Mk3Mode::Mixer || self.mode == Mk3Mode::Samples { if self.cell_offset >= 16 { self.cell_offset -= 16; } else { self.cell_offset = 0; } }
                else if self.mode == Mk3Mode::Rack { self.push(Mk3Cmd::RackBarDelta(-1)); }
                else { self.pattern_bank = 0; self.push(Mk3Cmd::SetBank(0)); }
            }
            CC_RIGHT if press => {
                if self.genre_pad_mode { self.genre_page += 1; }
                else if self.mode == Mk3Mode::Mixer || self.mode == Mk3Mode::Samples { if self.cell_offset + 16 < 32 { self.cell_offset += 16; } }
                else if self.mode == Mk3Mode::Rack { self.push(Mk3Cmd::RackBarDelta(1)); }
                else { self.pattern_bank = 1; self.push(Mk3Cmd::SetBank(1)); }
            }
            _ => {}
        }
    }

    pub fn handle_pad_note_on(&mut self, note: u8, _vel: u8) -> bool {
        if !self.enabled { return false; }
        let Some(pad) = PAD_NOTES.iter().position(|&n| n == note) else { return false; };
        if self.genre_pad_mode {
            self.push(Mk3Cmd::SelectGenre(self.genre_page * 16 + pad));
            return true;
        }
        match self.mode {
            Mk3Mode::Perform => { if self.shift { self.push(Mk3Cmd::ShiftPadAction(pad)); true } else { false } }
            Mk3Mode::Mixer => { let idx = self.cell_offset + pad; self.selected_cell = idx; self.push(Mk3Cmd::SelectCell(idx)); true }
            Mk3Mode::Samples => {
                let idx = self.cell_offset + pad; self.selected_cell = idx; self.push(Mk3Cmd::SelectCell(idx));
                if self.dice_pad == Some(idx) { self.push(Mk3Cmd::SamplesSave(idx)); self.dice_pad = None; }
                else { self.push(Mk3Cmd::SamplesDice(idx)); self.dice_pad = Some(idx); }
                true
            }
            Mk3Mode::Library => { let row = pad / 4; let col = pad % 4; self.push(Mk3Cmd::LibrarySetting(row, col as u8)); true }
            Mk3Mode::Pattern => { let bank = if self.shift { 1u8 } else { self.pattern_bank }; let idx = bank * 16 + pad as u8; self.push(Mk3Cmd::SelectPattern(idx.min(31))); true }
            Mk3Mode::Rack => {
                if self.select { self.push(Mk3Cmd::SelectRow(pad as u8)); }
                else if self.shift { self.push(Mk3Cmd::ToggleGhost { step: pad as u8 }); }
                else { self.push(Mk3Cmd::ToggleStep { piece: 255, step: pad as u8 }); }
                true
            }
        }
    }

    pub fn handle_pad_note_off(&mut self, note: u8) -> bool {
        if !self.enabled { return false; }
        PAD_NOTES.iter().any(|&n| n == note) && (self.genre_pad_mode || matches!(self.mode, Mk3Mode::Pattern | Mk3Mode::Rack | Mk3Mode::Mixer | Mk3Mode::Samples | Mk3Mode::Library))
    }
}
