//! Paridad byte a byte con la appweb original (Node) y con el motor C++ de referencia.
//!
//! Fixtures en `tests/golden/` (generados por `tests/run_parity.sh`):
//!  - `cases15.txt` / `expected15.txt`: 622 casos (15 géneros × 40 + 22 frases). Referencia: la appweb
//!    original ejecutada en Node (y el motor C++ coincide con ella en los 622).
//!  - `cases7.txt` / `expected7.txt`: 280 casos de los 7 géneros añadidos en C++ + 12 frases.
//!    Referencia: el motor C++ (la appweb no conoce esos géneros).
//!  - `mid_node/`, `mid_cpp/`: archivos .mid completos (formato 0 y 1) para comparar byte a byte.

use caja_core::genres::GenreSet;
use caja_core::parser::parse_phrase;
use caja_core::pattern::*;
use caja_core::pieces::*;
use caja_core::rng::fnv64;
use caja_core::smf::build_midi_file;
use caja_core::timing::TimingOpts;
use std::fs;
use std::path::PathBuf;

fn golden(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests").join("golden").join(name)
}

fn lines(name: &str) -> Vec<String> {
    fs::read_to_string(golden(name)).unwrap_or_else(|e| panic!("{name}: {e}")).lines().filter(|l| !l.is_empty()).map(String::from).collect()
}

/// Misma huella que parity_node.js / parity_cpp.cpp: solo las 20 piezas originales.
fn run_case(line: &str, genres: &GenreSet) -> String {
    if let Some(phrase) = line.strip_prefix('P') {
        let p = parse_phrase(phrase, genres);
        return format!("{}|{}|{}|{}|{}|{}|{}|{}", genres.get(p.genre).id, p.bpm, p.swing, p.density, p.bars, p.human, p.mods, p.warning);
    }
    let f: Vec<&str> = line[1..].split('|').collect();
    let gi = genres.index_of(f[0]).unwrap_or_else(|| panic!("género {}", f[0]));
    let mods: u32 = f[1].parse().unwrap();
    let bars: usize = f[2].parse().unwrap();
    let dens: i32 = f[3].parse().unwrap();
    let seed = f[4].to_string();
    let swing: i32 = f[5].parse().unwrap();
    let human: i32 = f[6].parse().unwrap();
    let sep = f[7] != "0";
    let spec = Spec { genre: gi, mods, bars, density: dens, seed: seed.clone(), accent: 50 };
    let pat = generate_pattern(&spec, genres);
    let mut cells = String::new();
    for p in 0..LEGACY_PIECES {
        for i in 0..pat.steps() {
            let c = pat.cell[p][i];
            cells.push_str(&format!("{},", u32::from(c.vel) * 2 + u32::from(c.ghost)));
        }
    }
    for p in LEGACY_PIECES..NUM_PIECES {
        assert!((0..pat.steps()).all(|i| !pat.cell[p][i].on()), "las piezas nuevas no deben sonar en {}", f[0]);
    }
    let t = TimingOpts { genre: gi, mods, swing, human, seed };
    let mid = build_midi_file(&pat, &t, 100, sep, genres, &legacy_notes());
    format!("{:016x}|{:016x}|{}", fnv64(cells.as_bytes()), fnv64(&mid), mid.len())
}

fn check(cases: &str, expected: &str) -> usize {
    let genres = GenreSet::builtin();
    let (c, e) = (lines(cases), lines(expected));
    assert_eq!(c.len(), e.len());
    let mut bad = Vec::new();
    for (i, (case, exp)) in c.iter().zip(&e).enumerate() {
        let got = run_case(case, &genres);
        if &got != exp {
            bad.push(format!("caso {} `{}`\n  esperado {}\n  obtenido {}", i + 1, case, exp, got));
        }
    }
    assert!(bad.is_empty(), "{} diferencias:\n{}", bad.len(), bad.iter().take(10).cloned().collect::<Vec<_>>().join("\n"));
    c.len()
}

#[test]
fn parity_622_cases_vs_original_webapp() {
    assert_eq!(check("cases15.txt", "expected15.txt"), 622);
}

#[test]
fn parity_7_added_genres_vs_cpp_reference() {
    assert_eq!(check("cases7.txt", "expected7.txt"), 292);
}

fn mid_dir(dir: &str, count: usize) {
    let genres = GenreSet::builtin();
    let mut n = 0;
    for entry in fs::read_dir(golden(dir)).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_string_lossy().to_string(); // p. ej. "funk-f1"
        let (gid, fmt) = name.rsplit_once("-f").unwrap();
        let gi = genres.index_of(gid).unwrap();
        let spec = Spec { genre: gi, mods: 0, bars: 2, density: 50, seed: "2026".into(), accent: 50 };
        let pat = generate_pattern(&spec, &genres);
        let t = TimingOpts { genre: gi, mods: 0, swing: 20, human: 14, seed: "2026".into() };
        let mid = build_midi_file(&pat, &t, 100, fmt == "1", &genres, &legacy_notes());
        assert_eq!(mid, fs::read(&path).unwrap(), "{name}.mid difiere");
        n += 1;
    }
    assert_eq!(n, count);
}

#[test]
fn mid_files_identical_to_webapp_node() {
    mid_dir("mid_node", 30);
}

#[test]
fn mid_files_identical_to_cpp_reference() {
    mid_dir("mid_cpp", 44);
}
