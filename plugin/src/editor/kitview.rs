//! Vistas de la drum machine: pestañas, Biblioteca y Pads. Mismo estilo 8 bits (solo palabras).

use super::*;
use crate::params::mode_text;
use caja_core::kit::{is_lock, locked_index, ALL_LOCK, ALL_ROLL, NUM_PADS, PARAM_NAMES, P_SAMPLE, SEED_MAX, SLOTS};
use caja_core::library::{DEFAULT_BANK, MAX_BANK};
use caja_core::pieces::{PAD_ORDER, PIECES};
use caja_core::seedrand::splitmix64;

const BANK_CAPS: [usize; 5] = [8, 16, 32, 64, 128];
const MAX_LENS: [u32; 4] = [1000, 3000, 6000, 10_000];
const RAM_CAPS: [u32; 5] = [128, 256, 512, 1024, 2048];
const POOLS: [usize; 4] = [8, 16, 32, 64];

fn pad_name(pad: usize) -> &'static str {
    PIECES[PAD_ORDER[pad.min(NUM_PADS - 1)]].name
}

fn random_seed(salt: u64) -> u32 {
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
    (splitmix64(n ^ salt.wrapping_mul(0x9E37_79B9)) % (u64::from(SEED_MAX) + 1)) as u32
}

/// Control deslizante 0..=1000 para valores del estado (no parámetros del host).
fn kslider(ui: &egui::Ui, cv: &Cv, r: Rect, id: &str, label: &str, value: u16, text: &str) -> Option<u16> {
    let label_w = r.width() * 0.34;
    cv.text(pos2(r.min.x, r.center().y), Align2::LEFT_CENTER, label, cv.th.text_dim);
    let track = Rect::from_min_max(pos2(r.min.x + label_w, r.min.y), r.max);
    let resp = ui.interact(snap_rect(track), ui.id().with(id), Sense::click_and_drag());
    let mut out = None;
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let f = ((pos.x - track.min.x - 2.0) / (track.width() - 4.0)).clamp(0.0, 1.0);
            out = Some((f * 1000.0).round() as u16);
        }
    }
    let th = &cv.th;
    cv.boxed(track, th.panel, if resp.hovered() || resp.dragged() { th.accent } else { th.text_dim }, 2.0);
    let f = f32::from(value) / 1000.0;
    let inner = track.shrink(4.0);
    cv.fill(Rect::from_min_size(inner.min, vec2((inner.width() * f).round().max(0.0), inner.height())), th.accent);
    cv.text(track.center(), Align2::CENTER_CENTER, text, if f > 0.5 { th.bg } else { th.text });
    out
}


/// Fader vertical 0..=1000 (mesa de mezcla).

/// Barra de pan continua −1..=+1 (izq…centro…der). Clic/arrastre = posición exacta; derecho = centro.
fn panslider(ui: &egui::Ui, cv: &Cv, r: Rect, id: &str, value: f32) -> Option<f32> {
    let resp = ui.interact(snap_rect(r), ui.id().with(id), Sense::click_and_drag());
    let mut out = None;
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let f = ((pos.x - r.min.x) / r.width()).clamp(0.0, 1.0);
            out = Some(f * 2.0 - 1.0); // 0→-1, 0.5→0, 1→+1
        }
    }
    if resp.secondary_clicked() {
        out = Some(0.0);
    }
    let th = &cv.th;
    let v = out.unwrap_or(value).clamp(-1.0, 1.0);
    cv.boxed(r, th.panel, if resp.hovered() || resp.dragged() { th.accent } else { th.chassis_edge }, 2.0);
    // marca de centro
    let cx = r.center().x;
    cv.p.line_segment(
        [pos2(cx, r.min.y + 2.0), pos2(cx, r.max.y - 2.0)],
        egui::Stroke::new(1.0, th.text_dim),
    );
    // cursor
    let x = r.min.x + (v + 1.0) * 0.5 * r.width();
    let thumb = Rect::from_center_size(pos2(x, r.center().y), vec2(6.0, r.height() - 4.0));
    cv.fill(thumb, th.accent);
    out
}

fn vfader(ui: &egui::Ui, cv: &Cv, r: Rect, id: &str, value: u16) -> Option<u16> {
    let resp = ui.interact(snap_rect(r), ui.id().with(id), Sense::click_and_drag());
    let mut out = None;
    if resp.dragged() || resp.clicked() {
        if let Some(pos) = resp.interact_pointer_pos() {
            let f = 1.0 - ((pos.y - r.min.y) / r.height()).clamp(0.0, 1.0);
            out = Some((f * 1000.0).round() as u16);
        }
    }
    let th = &cv.th;
    cv.boxed(r, th.panel, if resp.hovered() || resp.dragged() { th.accent } else { th.chassis_edge }, 2.0);
    let f = f32::from(value) / 1000.0;
    let fill_h = ((r.height() - 6.0) * f).round().max(0.0);
    let fill = Rect::from_min_max(
        pos2(r.min.x + 3.0, r.max.y - 3.0 - fill_h),
        pos2(r.max.x - 3.0, r.max.y - 3.0),
    );
    cv.fill(fill, th.accent);
    out
}

impl Gui {
    pub(super) fn tabs(&mut self, ui: &egui::Ui, cv: &Cv, left: f32, right: f32, y: f32) -> f32 {
        let mut f = Flow::new(left, y, right);
        for (i, name) in ["Perform", "Library", "Pads", "Step"].iter().enumerate() {
            if button(ui, cv, &mut f, &format!("tab{i}"), name, self.tab == i) {
                self.tab = i;
                self.focus = Focus::None;
            }
        }
        f.bottom() + GAP * 2.0
    }

    /// Enter en un campo de texto de la drum machine.
    pub(super) fn apply_typed(&mut self, setter: &ParamSetter, p: &CajaParams) {
        match self.focus {
            Focus::Path => self.apply_path(),
            Focus::Seed => {
                if let Ok(n) = self.seed_text.parse::<u32>() {
                    self.set_seed(setter, p, self.sel_pad, n.min(SEED_MAX));
                }
                self.focus = Focus::None;
            }
            Focus::PatName => {
                let name = self.pat_name.clone();
                let snap = p.snap();
                self.model.with(snap, |s| s.set_active_name(&name));
                self.status = if name.trim().is_empty() {
                    "Nombre borrado".into()
                } else {
                    format!("Patron: {name}")
                };
                self.focus = Focus::None;
            }
            _ => {}
        }
    }

    fn apply_path(&mut self) {
        let folder = self.path.trim().to_string();
        self.kit.edit(|k| k.folder = folder.clone());
        self.kit.rescan.store(true, Ordering::Release);
        self.focus = Focus::None;
    }

    /// Abre el selector de carpeta del sistema en un hilo (zenity / kdialog en Linux, osascript en
    /// macOS). Sin dependencias nativas: si no hay ninguno, el campo de texto sigue valiendo.
    pub(super) fn pick_folder(&mut self) {
        let out = self.pick.clone();
        self.note = "Esperando el selector de carpeta...".into();
        std::thread::spawn(move || {
            let tries: [(&str, &[&str]); 3] = [
                ("zenity", &["--file-selection", "--directory", "--title=Carpeta de samples"]),
                ("kdialog", &["--getexistingdirectory", "."]),
                ("osascript", &["-e", "POSIX path of (choose folder)"]),
            ];
            for (cmd, args) in tries {
                if let Ok(o) = std::process::Command::new(cmd).args(args).output() {
                    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
                    *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(if o.status.success() && !s.is_empty() { Ok(s) } else { Err("selector cancelado".into()) });
                    return;
                }
            }
            *out.lock().unwrap_or_else(|e| e.into_inner()) = Some(Err("no hay zenity, kdialog ni osascript: pega la ruta en el campo".into()));
        });
    }

    // ───────────────────────────── Biblioteca ─────────────────────────────

    pub(super) fn library_view(&mut self, ui: &egui::Ui, cv: &Cv, _setter: &ParamSetter, _p: &CajaParams, left: f32, right: f32, y: f32) -> f32 {
        let now = ui.input(|i| i.time);
        let st = self.kit.snapshot();
        let lib = self.kit.library();
        let status = self.kit.status.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let picked = self.pick.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(r) = picked {
            match r {
                Ok(p) => {
                    self.path = p;
                    self.apply_path();
                    self.note.clear();
                }
                Err(e) => self.note = e,
            }
        }
        if self.focus != Focus::Path && self.path != st.folder && self.note.is_empty() {
            self.path = st.folder.clone();
        }
        let bh = button_h(cv);
        let mut y = y;

        // ruta + botones
        let pick_w = cv.text_w("Elegir carpeta") + 2.0 * PAD_X;
        let scan_w = cv.text_w("Reescanear") + 2.0 * PAD_X;
        let field = Rect::from_min_max(pos2(left, y), pos2(right - pick_w - scan_w - 2.0 * GAP, y + bh));
        self.text_field(ui, cv, field, "path", Focus::Path, "pega aqui la ruta de tu carpeta de samples", now);
        if button_at(ui, cv, Rect::from_min_size(pos2(right - pick_w - scan_w - GAP, y), vec2(pick_w, bh)), "pick", "Elegir carpeta", false) {
            self.pick_folder();
        }
        if button_at(ui, cv, Rect::from_min_size(pos2(right - scan_w, y), vec2(scan_w, bh)), "rescan", "Reescanear", false) {
            self.apply_path();
        }
        y += bh + GAP;
        cv.text(pos2(left, y), Align2::LEFT_TOP, "Los samples son tuyos: el plugin solo guarda la ruta, nunca los copia.", cv.th.text_dim);
        y += cv.fs + GAP * 2.0;

        // ajustes
        let mut f = Flow::new(left, y, right);
        row_label(cv, &mut f, "Banco por celda");
        for c in BANK_CAPS {
            if button(ui, cv, &mut f, &format!("cap{c}"), &c.to_string(), st.bank_cap == c) {
                self.kit.edit(|k| k.bank_cap = c);
            }
        }
        f.newline();
        row_label(cv, &mut f, "Maximo por muestra");
        for m in MAX_LENS {
            if button(ui, cv, &mut f, &format!("len{m}"), &format!("{} s", m / 1000), st.max_len_ms == m) {
                self.kit.edit(|k| k.max_len_ms = m);
                self.kit.rescan.store(true, Ordering::Release);
            }
        }
        f.newline();
        row_label(cv, &mut f, "Tope de RAM");
        for m in RAM_CAPS {
            if button(ui, cv, &mut f, &format!("ram{m}"), &format!("{m} MB"), st.ram_cap_mb == m) {
                self.kit.edit(|k| k.ram_cap_mb = m);
            }
        }
        f.newline();
        row_label(cv, &mut f, "Pool en ROLL");
        for m in POOLS {
            if button(ui, cv, &mut f, &format!("pool{m}"), &m.to_string(), st.pool == m) {
                self.kit.edit(|k| k.pool = m);
            }
        }
        f.newline();
        row_label(cv, &mut f, "ROLL reproducible");
        if button(ui, cv, &mut f, "rollrep", if st.roll_repro { "Si" } else { "No" }, st.roll_repro) {
            self.kit.edit(|k| k.roll_repro = !k.roll_repro);
        }
        y = f.bottom() + GAP * 2.0;

        // estado
        let ram = self.kit.store.ram_bytes.load(Ordering::Relaxed) as f64 / (1024.0 * 1024.0);
        let line = format!(
            "{}  Archivos: {}  Clasificados: {}  Sin clasificar: {}  RAM: {:.1} de {} MB",
            if status.scanning { "Escaneando..." } else { "Listo." },
            status.files_seen,
            lib.total_classified(),
            lib.unclassified.len(),
            ram,
            st.ram_cap_mb
        );
        y += cv.wrapped(pos2(left, y), right - left, &line, cv.th.text) + GAP;
        if status.ram_full {
            y += cv.wrapped(pos2(left, y), right - left, "Tope de RAM alcanzado: algunas muestras no se cargan. Sube el tope o baja el pool.", cv.th.accent) + GAP;
        }
        let kw = format!("Palabras clave: {}", status.keywords_origin);
        y += cv.wrapped(pos2(left, y), right - left, &kw, cv.th.text_dim) + GAP;
        for e in status.keywords_errors.iter().take(3) {
            y += cv.wrapped(pos2(left, y), right - left, e, cv.th.accent) + 2.0;
        }
        if !self.note.is_empty() {
            y += cv.wrapped(pos2(left, y), right - left, &self.note, cv.th.accent) + GAP;
        }
        if let Some(e) = self.kit.state_error.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            y += cv.wrapped(pos2(left, y), right - left, &e, cv.th.accent) + GAP;
        }
        y += GAP;

        // celdas
        cv.text(pos2(left, y), Align2::LEFT_TOP, "Celdas (orden de los pads)", cv.th.accent);
        y += cv.fs + GAP;
        for pad in 0..NUM_PADS {
            let bank = lib.bank(pad, st.bank_cap);
            let total = lib.cells[pad].len();
            let cur = self.kit.store.cur_idx[pad].load(Ordering::Relaxed);
            let cur_name = if cur >= 0 { bank.get(cur as usize).map_or("", String::as_str) } else { "" };
            let loaded = self.kit.store.loaded_in_pad(pad);
            let row = format!("{:<14} {:>3} de {:<4} RAM {:>3}  {}", pad_name(pad), bank.len(), total, loaded, cur_name);
            let h = cv.wrapped(pos2(left, y), right - left, &row, if total == 0 { cv.th.text_dim } else { cv.th.text });
            y += h + 3.0;
        }
        y += GAP;

        // bandeja "Sin clasificar"
        cv.text(pos2(left, y), Align2::LEFT_TOP, &format!("Sin clasificar ({})", lib.unclassified.len()), cv.th.accent);
        y += cv.fs + GAP;
        let mut f = Flow::new(left, y, right);
        for (i, rel) in lib.unclassified.iter().take(24).enumerate() {
            let on = self.sel_unclassified.as_deref() == Some(rel.as_str());
            if button(ui, cv, &mut f, &format!("unc{i}"), rel, on) {
                self.sel_unclassified = if on { None } else { Some(rel.clone()) };
            }
        }
        y = f.bottom() + GAP;
        if let Some(sel) = self.sel_unclassified.clone() {
            let mut f = Flow::new(left, y, right);
            row_label(cv, &mut f, "Asignar a");
            for pad in 0..NUM_PADS {
                if button(ui, cv, &mut f, &format!("asg{pad}"), pad_name(pad), false) {
                    self.kit.edit(|k| {
                        k.assign.retain(|(r, _)| *r != sel);
                        k.assign.push((sel.clone(), pad));
                    });
                    self.kit.rescan.store(true, Ordering::Release);
                    self.sel_unclassified = None;
                }
            }
            y = f.bottom() + GAP;
        }
        y += GAP;

        // errores de lectura
        cv.text(pos2(left, y), Align2::LEFT_TOP, &format!("Errores de lectura ({})", status.errors.len()), cv.th.accent);
        y += cv.fs + GAP;
        for e in status.errors.iter().take(12) {
            y += cv.wrapped(pos2(left, y), right - left, e, cv.th.text_dim) + 2.0;
        }
        let _ = (DEFAULT_BANK, MAX_BANK);
        y
    }

    // ───────────────────────────── Pads ─────────────────────────────

    /// Ruta de la muestra que suena ahora en el pad (o la que daría su semilla).
    fn current_sample(&self, p: &CajaParams, pad: usize) -> Option<String> {
        let st = self.kit.snapshot();
        let lib = self.kit.library();
        let bank = lib.bank(pad, st.bank_cap);
        let cur = self.kit.store.cur_idx[pad].load(Ordering::Relaxed);
        let i = if cur >= 0 && (cur as usize) < bank.len() { cur as usize } else { locked_index(&st.pads[pad], p.pads[pad].seed.value() as u32, bank.len())? };
        bank.get(i).cloned()
    }

    /// Fija el pin con la muestra actual y la semilla activa.
    pub(super) fn pin_pad(&self, p: &CajaParams, pad: usize) {
        let seed = p.pads[pad].seed.value() as u32;
        if let Some(rel) = self.current_sample(p, pad) {
            self.kit.edit(|k| {
                k.pads[pad].pin = Some(rel);
                k.pads[pad].pin_seed = seed;
            });
        }
    }

    /// Cambia la semilla desde la interfaz; con Muestra en LOCK, re-fija el pin a la muestra nueva.
    pub(super) fn set_seed(&mut self, setter: &ParamSetter, p: &CajaParams, pad: usize, seed: u32) {
        set(setter, &p.pads[pad].seed, seed as i32);
        if is_lock(p.pads[pad].mode.value() as u8, P_SAMPLE) {
            let st = self.kit.snapshot();
            let lib = self.kit.library();
            let bank = lib.bank(pad, st.bank_cap);
            if let Some(rel) = locked_index(&st.pads[pad], seed, bank.len()).and_then(|i| bank.get(i).cloned()) {
                self.kit.edit(|k| {
                    k.pads[pad].pin = Some(rel);
                    k.pads[pad].pin_seed = seed;
                });
            }
        }
    }


    pub(super) fn pads_view(&mut self, ui: &egui::Ui, cv: &Cv, setter: &ParamSetter, p: &CajaParams, left: f32, right: f32, mut y: f32) -> f32 {
        let now = ui.input(|i| i.time);
        let st = self.kit.snapshot();
        let lib = self.kit.library();
        let pad = self.sel_pad.min(NUM_PADS - 1);
        let bh = button_h(cv);
        let th = &cv.th;

        // ── globales ──
        let mut f = Flow::new(left, y, right);
        row_label(cv, &mut f, "Mixer");
        if button(ui, cv, &mut f, "allroll", "ALL ROLL", false) {
            for q in 0..NUM_PADS {
                set(setter, &p.pads[q].mode, i32::from(ALL_ROLL));
            }
        }
        if button(ui, cv, &mut f, "alllock", "ALL LOCK", false) {
            for q in 0..NUM_PADS {
                self.pin_pad(p, q);
                set(setter, &p.pads[q].mode, i32::from(ALL_LOCK));
            }
        }
        if button(ui, cv, &mut f, "rate4all", "Todos 4c", false) {
            for q in 0..NUM_PADS {
                set(setter, &p.pads[q].rate, 3);
            }
        }
        if button(ui, cv, &mut f, "rategolpe", "Todos golpe", false) {
            for q in 0..NUM_PADS {
                set(setter, &p.pads[q].rate, 0);
            }
        }
        let cp = p.clip_prio.value();
        if button(ui, cv, &mut f, "clipprio", if cp { "Clip prio si" } else { "Clip prio no" }, cp) {
            set(setter, &p.clip_prio, !cp);
        }
        y = f.bottom() + GAP;

        // ── MASTER ──
        let master_w = 72.0_f32;
        let strip_h = 168.0_f32;
        let master_r = Rect::from_min_size(pos2(left, y), vec2(master_w, strip_h));
        cv.boxed(master_r, th.face, th.accent, 2.0);
        cv.text(pos2(master_r.center().x, master_r.min.y + 10.0), Align2::CENTER_CENTER, "MASTER", th.accent);
        let mv = p.master.value(); // dB -24..6
        let mv_norm = ((mv + 24.0) / 30.0).clamp(0.0, 1.0);
        let fader = Rect::from_min_size(
            pos2(master_r.min.x + 18.0, master_r.min.y + 22.0),
            vec2(36.0, 110.0),
        );
        if let Some(n) = vfader(ui, cv, fader, "master_fader", (mv_norm * 1000.0) as u16) {
            let db = n as f32 / 1000.0 * 30.0 - 24.0;
            set(setter, &p.master, db);
        }
        cv.text(
            pos2(master_r.center().x, master_r.max.y - 14.0),
            Align2::CENTER_CENTER,
            &format!("{mv:.1} dB"),
            th.text,
        );

        // ── canales por pad (rejilla) ──
        let gap = 4.0_f32;
        let chan_w = 52.0_f32;
        let x0 = left + master_w + GAP * 2.0;
        let avail = (right - x0).max(chan_w);
        let cols = ((avail + gap) / (chan_w + gap)).floor().max(1.0) as usize;
        let mut y_mix = y;
        for q in 0..NUM_PADS {
            let col = q % cols;
            let row = q / cols;
            let x = x0 + col as f32 * (chan_w + gap);
            let yy = y + row as f32 * (strip_h + gap);
            y_mix = y_mix.max(yy + strip_h);
            let r = Rect::from_min_size(pos2(x, yy), vec2(chan_w, strip_h));
            let selected = q == pad;
            let muted = p.pads[q].mute.value();
            let solo = st.pads[q].solo;
            cv.boxed(
                r,
                if selected { th.rubber_on } else { th.face },
                if selected { th.accent } else { th.chassis_edge },
                if selected { 2.0 } else { 1.0 },
            );
            // nombre
            let name = pad_name(q);
            let short = if name.len() > 6 { &name[..6] } else { name };
            cv.text(pos2(r.center().x, r.min.y + 9.0), Align2::CENTER_CENTER, short, if selected { th.text } else { th.text_dim });
            // M / S
            let mb = Rect::from_min_size(pos2(r.min.x + 4.0, r.min.y + 18.0), vec2(20.0, 16.0));
            let sb = Rect::from_min_size(pos2(r.min.x + 28.0, r.min.y + 18.0), vec2(20.0, 16.0));
            if button_at(ui, cv, mb, &format!("mxM{q}"), "M", muted) {
                set(setter, &p.pads[q].mute, !muted);
                self.kit.edit(|k| k.pads[q].mute = !muted);
            }
            if button_at(ui, cv, sb, &format!("mxS{q}"), "S", solo) {
                self.kit.edit(|k| k.pads[q].solo = !k.pads[q].solo);
            }
            // fader de ganancia (0..2 → 0..1000)
            let gain = p.pads[q].gain.value().clamp(0.0, 2.0);
            let gf = Rect::from_min_size(pos2(r.min.x + 10.0, r.min.y + 40.0), vec2(32.0, 88.0));
            if let Some(n) = vfader(ui, cv, gf, &format!("mxG{q}"), (gain / 2.0 * 1000.0) as u16) {
                let g = n as f32 / 1000.0 * 2.0;
                set(setter, &p.pads[q].gain, g);
                self.kit.edit(|k| k.pads[q].gain = (g * 1000.0).round().clamp(0.0, 2000.0) as u16);
            }
            // pan continuo −100%…0…+100% (barra); clic derecho = centro
            let pan = p.pads[q].pan.value().clamp(-1.0, 1.0);
            let pb = Rect::from_min_size(pos2(r.min.x + 4.0, r.max.y - 22.0), vec2(chan_w - 8.0, 14.0));
            if let Some(v) = panslider(ui, cv, pb, &format!("mxP{q}"), pan) {
                set(setter, &p.pads[q].pan, v);
                self.kit.edit(|k| k.pads[q].pan = (v * 1000.0).round() as i16);
            }
            // clic en el canal (zona nombre) selecciona
            let hit = Rect::from_min_size(pos2(r.min.x, r.min.y), vec2(chan_w, 18.0));
            if ui.interact(snap_rect(hit), ui.id().with(("mxsel", q)), Sense::click()).clicked() {
                self.sel_pad = q;
                self.seed_text.clear();
            }
        }
        y = y_mix + GAP * 2.0;

        // ── detalle del pad seleccionado ──
        let _cfg = &st.pads[pad];
        let live = p.pads[pad].live();
        let bank = lib.bank(pad, st.bank_cap);
        cv.text(
            pos2(left, y),
            Align2::LEFT_TOP,
            &format!("{}   {} muestras   {}", pad_name(pad), bank.len(), mode_text(live.mode)),
            th.accent,
        );
        y += cv.fs + GAP;
        let cur = self.kit.store.cur_idx[pad].load(Ordering::Relaxed);
        let cur_name = if cur >= 0 {
            bank.get(cur as usize).map_or("(ninguna)", String::as_str)
        } else {
            "(ninguna)"
        };
        y += cv.wrapped(pos2(left, y), right - left, &format!("Muestra: {cur_name}"), th.text) + GAP;

        // semilla + slot
        let mut f = Flow::new(left, y, right);
        row_label(cv, &mut f, "Semilla");
        if button(ui, cv, &mut f, "seedminus", "-", false) {
            let s = if live.seed == 0 { SEED_MAX } else { live.seed - 1 };
            self.set_seed(setter, p, pad, s);
        }
        let fw = cv.text_w("00000") + 2.0 * PAD_X;
        let r = f.place(fw, bh);
        if self.focus != Focus::Seed {
            self.seed_text = live.seed.to_string();
        }
        self.text_field(ui, cv, r, "seedfield", Focus::Seed, "", now);
        if button(ui, cv, &mut f, "seedplus", "+", false) {
            let s = if live.seed >= SEED_MAX { 0 } else { live.seed + 1 };
            self.set_seed(setter, p, pad, s);
        }
        if button(ui, cv, &mut f, "dice", "Dado", false) {
            self.set_seed(setter, p, pad, random_seed(pad as u64 + 99));
        }
        row_label(cv, &mut f, "Slot");
        for s in 0..SLOTS {
            if button(ui, cv, &mut f, &format!("slot{s}"), &format!("{}", s + 1), live.slot as usize == s) {
                set(setter, &p.pads[pad].slot, s as i32);
            }
        }
        y = f.bottom() + GAP;

        // modos LOCK/ROLL
        let mut f = Flow::new(left, y, right);
        row_label(cv, &mut f, "Modos");
        for j in 0..4 {
            let lock = is_lock(live.mode, j);
            if button(ui, cv, &mut f, &format!("mode{j}"), &format!("{}:{}", PARAM_NAMES[j], if lock { "L" } else { "R" }), lock) {
                let bit = 1u8 << j;
                let new_mode = if lock { live.mode & !bit } else { live.mode | bit };
                set(setter, &p.pads[pad].mode, i32::from(new_mode));
            }
        }
        y = f.bottom() + GAP;

        // ritmo ROLL
        let rate = p.pads[pad].rate.value().clamp(0, 3);
        let mut f = Flow::new(left, y, right);
        row_label(cv, &mut f, "Ritmo ROLL");
        for (i, lab) in ["golpe", "1c", "2c", "4c"].iter().enumerate() {
            if button(ui, cv, &mut f, &format!("rate{i}"), lab, rate == i as i32) {
                set(setter, &p.pads[pad].rate, i as i32);
            }
        }
        y = f.bottom() + GAP * 2.0;
        y + cv.fs
    }
}
