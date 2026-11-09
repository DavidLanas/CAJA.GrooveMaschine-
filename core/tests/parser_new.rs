//! Parser con los géneros nuevos ("propuesta") y regresiones de casos límite.

use caja_core::genres::GenreSet;
use caja_core::parser::*;

fn id_of(phrase: &str) -> (String, ParsedPhrase) {
    let g = GenreSet::builtin();
    let p = parse_phrase(phrase, &g);
    (g.get(p.genre).id.clone(), p)
}

#[test]
fn new_genres_are_recognised() {
    let cases = [
        ("motown", "motown"), ("soul memphis", "soulmemphis"), ("gospel shuffle", "gospelshuffle"), ("slow jam", "slowjam"),
        ("disco 118", "disco"), ("trap soul", "trapsoul"), ("uk drill", "drill"), ("jersey club", "jerseyclub"), ("baile funk", "bailefunk"),
        ("funk carioca", "bailefunk"), ("uk garage 2 step", "ukgarage"), ("jungle", "jungle"), ("breakbeat", "breakbeat"),
        ("afro house", "afrohouse"), ("amapiano", "amapiano"), ("gqom", "gqom"), ("electro", "electro"), ("trance 138", "trance"),
        ("dancehall", "dancehall"), ("reggae one drop", "reggae"), ("steppers", "steppers"), ("ska", "ska"), ("cumbia", "cumbia"),
        ("salsa", "salsa32"), ("salsa clave 3-2", "salsa32"), ("salsa clave 2-3", "salsa23"), ("merengue", "merengue"), ("bachata", "bachata"),
        ("punk", "punk"), ("punk rock", "punk"), ("blues shuffle", "bluesshuffle"), ("metal doble bombo", "metal"), ("country train beat", "country"), ("pop 4/4", "pop"),
    ];
    for (phrase, want) in cases {
        assert_eq!(id_of(phrase).0, want, "frase: {phrase}");
    }
}

#[test]
fn longest_alias_wins_and_ties_keep_original_priority() {
    assert_eq!(id_of("baile funk").0, "bailefunk"); // más largo que "funk"
    assert_eq!(id_of("afro house").0, "afrohouse"); // más largo que "house"
    assert_eq!(id_of("trap soul").0, "trapsoul");
    assert_eq!(id_of("funk trap").0, "trap"); // empate de longitud: gana la prioridad original (trap antes que funk)
    assert_eq!(id_of("reggaeton").0, "reggaeton"); // contiene "reggae" pero es más largo
}

#[test]
fn unknown_text_falls_back_with_warning_and_empty_is_silent() {
    let (id, p) = id_of("algo raro xyz");
    assert_eq!(id, "funk");
    assert_eq!(p.warning, "No reconocí el género, así que armé un funk.");
    assert!(id_of("").1.warning.is_empty());
    assert_eq!(id_of("shuffle sin genero").0, "jazz");
}

#[test]
fn numbers_bars_and_modifiers() {
    let (_, p) = id_of("techno 132 bpm con fill 4 compases ghost notes");
    assert_eq!((p.bpm, p.bars), (132, 4));
    assert!(p.mods & MOD_FILL != 0 && p.mods & MOD_GHOST != 0);
    assert_eq!(id_of("funk 300 bpm").1.bpm, 210);
    assert_eq!(id_of("funk 12 compases").1.bars, 8);
    assert!(id_of("funk 12 compases").1.warning.contains("Máximo 8"));
    // desbordamiento: la versión C++ lanzaba una excepción; aquí satura
    let (_, p) = id_of("funk 99999999999999999999 compases");
    assert_eq!(p.bars, 8);
    assert_eq!(id_of("funk dos compases").1.bars, 2);
    assert_eq!(id_of("funk 3 compases").1.bars, 2);
    assert_eq!(id_of("funk lento").1.bpm, (f64::from(GenreSet::builtin().get(0).bpm) * 0.86 + 0.5).floor() as i32);
}

#[test]
fn hostile_input_does_not_panic() {
    let g = GenreSet::builtin();
    let long = "a ".repeat(50_000);
    for s in ["\u{0}\u{1}\u{ff}", "�����", "r & b", "R&B 90bpm", "1".repeat(5000).as_str(), long.as_str(), "😀 funk 😀", "\u{e9}\u{e8}"] {
        let _ = parse_phrase(s, &g);
    }
    let bytes = [0xC3u8, 0x28, 0xF0, 0x28, 0x8C, 0xBC, 0xE2, 0x82];
    let s = String::from_utf8_lossy(&bytes).to_string();
    let _ = parse_phrase(&s, &g);
}
