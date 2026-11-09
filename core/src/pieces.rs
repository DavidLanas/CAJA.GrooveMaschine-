//! Las 25 piezas. El índice no es el orden del piano roll: las notas del preset
//! "Bitwig Drum Machine" salen de `PAD_ORDER` (C1 hacia arriba). Las 5 últimas
//! siguen al final del arreglo para no mover ningún índice de patrón.

pub const NUM_PIECES: usize = 25;
/// Piezas de la appweb original. El motor las procesa con el flujo aleatorio original.
pub const LEGACY_PIECES: usize = 20;

pub const KICK: usize = 0;
pub const SNARE: usize = 1;
pub const CH: usize = 2;
pub const OH: usize = 3;
pub const KICK2: usize = 4;
pub const SN2: usize = 5;
pub const CH2: usize = 6;
pub const OH2: usize = 7;
pub const EGG: usize = 8;
pub const CLAP: usize = 9;
pub const RIM: usize = 10;
pub const RIDE: usize = 11;
pub const SHAKER: usize = 12;
pub const TOM1: usize = 13;
pub const TOM2: usize = 14;
pub const TOM3: usize = 15;
pub const CRASH: usize = 16;
pub const COWBELL: usize = 17;
pub const CONGA_H: usize = 18;
pub const CONGA_L: usize = 19;
pub const CLAVE: usize = 20;
pub const TIMBAL_H: usize = 21;
pub const TIMBAL_L: usize = 22;
pub const GUIRO: usize = 23;
pub const PANDERETA: usize = 24;

/// Pads de la Drum Machine, de la nota más grave a la más aguda, empezando en C1 (36).
/// Los 16 primeros son el kit pedido; si el género usa más piezas, van después.
pub const PAD_ORDER: [usize; NUM_PIECES] = [
    KICK, SNARE, CH, OH, KICK2, SN2, CH2, OH2, SHAKER, CLAP, RIM, RIDE, EGG, TOM1, TOM2, TOM3, CRASH, COWBELL, CONGA_H, CONGA_L, CLAVE, TIMBAL_H, TIMBAL_L, GUIRO, PANDERETA,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Kick,
    Snare,
    Hat,
    Perc,
    Cym,
    Tom,
}

#[derive(Clone, Copy, Debug)]
pub struct PieceDef {
    /// Id estable (entra en el hash de humanización y en genres.json).
    pub id: &'static str,
    /// Nombre en pantalla (solo palabras).
    pub name: &'static str,
    /// Nombre ASCII para pistas del .mid.
    pub midi_name: &'static str,
    /// Nota MIDI por defecto (Drum Machine de Bitwig: C1 = 36, cromático en `PAD_ORDER`).
    pub note: u8,
    /// Duración base en ticks (480 PPQ).
    pub dur: u16,
    pub group: Group,
    /// Fuera de los 16 pads del kit base.
    pub extra: bool,
}

const fn pd(id: &'static str, name: &'static str, midi_name: &'static str, note: u8, dur: u16, group: Group, extra: bool) -> PieceDef {
    PieceDef { id, name, midi_name, note, dur, group, extra }
}

pub const PIECES: [PieceDef; NUM_PIECES] = [
    pd("kick", "Kick", "Kick", 36, 80, Group::Kick, false),
    pd("snare", "Snare", "Snare", 37, 60, Group::Snare, false),
    pd("ch", "Hat cerrado", "Hat cerrado", 38, 24, Group::Hat, false),
    pd("oh", "Hat abierto", "Hat abierto", 39, 180, Group::Hat, false),
    pd("kick2", "Kick sec.", "Kick sec.", 40, 100, Group::Kick, false),
    pd("sn2", "Snare sec.", "Snare sec.", 41, 50, Group::Snare, false),
    pd("ch2", "Hat cer. sec.", "Hat cer. sec.", 42, 20, Group::Hat, false),
    pd("oh2", "Hat ab. sec.", "Hat ab. sec.", 43, 200, Group::Hat, false),
    pd("egg", "Egg", "Egg", 48, 18, Group::Perc, false),
    pd("clap", "Clap", "Clap", 45, 48, Group::Snare, false),
    pd("rim", "Rim", "Rim", 46, 28, Group::Snare, false),
    pd("ride", "Ride", "Ride", 47, 110, Group::Cym, false),
    pd("shaker", "Shaker", "Shaker", 44, 22, Group::Perc, false),
    pd("tom1", "Tom 1", "Tom 1", 49, 100, Group::Tom, false),
    pd("tom2", "Tom 2", "Tom 2", 50, 90, Group::Tom, false),
    pd("tom3", "Tom 3", "Tom 3", 51, 80, Group::Tom, false),
    pd("crash", "Crash", "Crash", 52, 360, Group::Cym, true),
    pd("cowbell", "Cowbell", "Cowbell", 53, 70, Group::Perc, true),
    pd("congaH", "Conga alta", "Conga alta", 54, 70, Group::Perc, true),
    pd("congaL", "Conga baja", "Conga baja", 55, 90, Group::Perc, true),
    pd("clave", "Clave", "Clave", 56, 40, Group::Perc, true),
    pd("timbalH", "Timbal alto", "Timbal alto", 57, 70, Group::Perc, true),
    pd("timbalL", "Timbal bajo", "Timbal bajo", 58, 90, Group::Perc, true),
    pd("guiro", "Güiro corto", "Guiro corto", 59, 40, Group::Perc, true),
    pd("pandereta", "Pandereta", "Pandereta", 60, 60, Group::Perc, true),
];

pub fn piece_index(id: &str) -> Option<usize> {
    PIECES.iter().position(|p| p.id == id)
}

/// Notas por defecto de las 25 piezas (preset "Bitwig Drum Machine").
pub fn default_notes() -> [u8; NUM_PIECES] {
    std::array::from_fn(|i| PIECES[i].note)
}

/// Mapa anterior (snare en 38 y hat cerrado en 37, más el resto no cromático).
/// Lo usan la paridad de los .mid históricos y la migración de proyectos guardados.
pub fn legacy_notes() -> [u8; NUM_PIECES] {
    [36, 38, 37, 39, 35, 40, 44, 55, 69, 46, 42, 51, 70, 45, 47, 50, 49, 56, 63, 64, 75, 65, 66, 73, 54]
}

/// Si el proyecto se guardó con el mapa viejo intacto, lo pasa al orden de pads.
pub fn migrate_notes(notes: [u8; NUM_PIECES]) -> [u8; NUM_PIECES] {
    if notes == legacy_notes() { default_notes() } else { notes }
}

/// Siguiente pieza en el orden de pads (grave → agudo). `None` si ya es la última.
pub fn pad_after(piece: usize) -> Option<usize> {
    let pos = PAD_ORDER.iter().position(|&p| p == piece)?;
    PAD_ORDER.get(pos + 1).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_notes_do_not_clash() {
        for i in 0..NUM_PIECES {
            for j in (i + 1)..NUM_PIECES {
                assert_ne!(PIECES[i].id, PIECES[j].id);
                assert_ne!(PIECES[i].note, PIECES[j].note, "{} y {} comparten nota", PIECES[i].id, PIECES[j].id);
            }
        }
    }

    #[test]
    fn bitwig_pads_run_chromatic_from_c1_in_the_requested_order() {
        for (i, &pi) in PAD_ORDER.iter().enumerate() {
            assert_eq!(PIECES[pi].note, 36 + i as u8, "{}", PIECES[pi].id);
        }
        assert_eq!(PIECES[SNARE].note, 37, "snare = C#1, el pad 2");
        assert_eq!(PIECES[CH].note, 38, "hat cerrado = D1, el pad 3");
        assert_eq!(PIECES[SHAKER].note, 44);
        assert_eq!(PIECES[RIDE].note, 47);
        assert_eq!(PIECES[EGG].note, 48);
        assert_eq!(PIECES[TOM3].note, 51);
        assert_eq!(PIECES[CRASH].note, 52, "lo que sobre va después de los 16 pads");
        assert!(PIECES[CRASH].extra && !PIECES[RIDE].extra);
    }

    #[test]
    fn legacy_map_is_the_old_swap_and_migration_only_touches_it() {
        assert_eq!(legacy_notes()[SNARE], 38);
        assert_eq!(legacy_notes()[CH], 37);
        assert_eq!(migrate_notes(legacy_notes())[SNARE], 37);
        assert_eq!(migrate_notes(legacy_notes())[CH], 38);
        let mut custom = legacy_notes();
        custom[KICK] = 40;
        assert_eq!(migrate_notes(custom), custom);
    }
}