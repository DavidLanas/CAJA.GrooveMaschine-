//! Archivos del usuario: géneros, mapas de kit y exportación .mid. Todo fuera del hilo de audio.
//!
//! Rutas: `CAJA_GENRES` (archivo) o `<config>/genres.json`; `CAJA_DRUM_MAPS` (archivo) o
//! `<config>/drum-maps.json`; mapas guardados en `<config>/maps/<nombre>.json`; exportación en
//! `~/Documents/CAJA/`. `<config>` = `CAJA_CONFIG_DIR`, o `%APPDATA%\caja` en Windows, o
//! `$XDG_CONFIG_HOME/caja`, o `~/.config/caja`.

use crate::drum_map::{DrumMap, MapSet};
use crate::genres::GenreSet;
use std::fs;
use std::path::{Path, PathBuf};

pub struct Loaded<T> {
    pub value: T,
    /// De dónde salió ("integrado", o la ruta del archivo).
    pub origin: String,
    /// Errores visibles en la interfaz. Con errores en un archivo de usuario se usa el integrado.
    pub errors: Vec<String>,
}

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn home_dir() -> Option<PathBuf> {
    env_path("HOME").or_else(|| env_path("USERPROFILE"))
}

pub fn config_dir() -> Option<PathBuf> {
    if let Some(p) = env_path("CAJA_CONFIG_DIR") {
        return Some(p);
    }
    if cfg!(windows) {
        if let Some(a) = env_path("APPDATA") {
            return Some(a.join("caja"));
        }
    }
    if let Some(x) = env_path("XDG_CONFIG_HOME") {
        return Some(x.join("caja"));
    }
    home_dir().map(|h| h.join(".config").join("caja"))
}

/// `~/Documents/CAJA` (o `CAJA_EXPORT_DIR`).
pub fn export_dir() -> Option<PathBuf> {
    env_path("CAJA_EXPORT_DIR").or_else(|| home_dir().map(|h| h.join("Documents").join("CAJA")))
}

pub fn genres_file() -> Option<PathBuf> {
    env_path("CAJA_GENRES").or_else(|| config_dir().map(|d| d.join("genres.json")))
}

pub fn drum_maps_file() -> Option<PathBuf> {
    env_path("CAJA_DRUM_MAPS").or_else(|| config_dir().map(|d| d.join("drum-maps.json")))
}

/// `sample-keywords.json` (o `CAJA_SAMPLE_KEYWORDS`): tabla editable para clasificar samples.
pub fn sample_keywords_file() -> Option<PathBuf> {
    env_path("CAJA_SAMPLE_KEYWORDS").or_else(|| config_dir().map(|d| d.join("sample-keywords.json")))
}

/// Tabla de palabras clave: archivo de usuario si existe y es válido; si no, la integrada.
/// Devuelve también el JSON usado (para guardarlo en el estado del proyecto).
pub fn load_keywords() -> Loaded<(crate::library::KeywordTable, String)> {
    use crate::library::{KeywordTable, KEYWORDS_JSON};
    if let Some(f) = sample_keywords_file().filter(|f| f.is_file()) {
        let shown = f.display().to_string();
        match fs::read_to_string(&f) {
            Err(e) => {
                return Loaded { value: (KeywordTable::builtin(), KEYWORDS_JSON.to_string()), origin: "integrada".into(), errors: vec![format!("{shown}: no se pudo leer ({e})")] };
            }
            Ok(src) => match KeywordTable::from_json(&src) {
                Ok(t) => return Loaded { value: (t, src), origin: shown, errors: vec![] },
                Err(e) => {
                    return Loaded { value: (KeywordTable::builtin(), KEYWORDS_JSON.to_string()), origin: "integrada".into(), errors: vec![format!("{shown}: no es válido, se usa la tabla integrada"), e] };
                }
            },
        }
    }
    Loaded { value: (KeywordTable::builtin(), KEYWORDS_JSON.to_string()), origin: "integrada".into(), errors: vec![] }
}

pub fn maps_dir() -> Option<PathBuf> {
    config_dir().map(|d| d.join("maps"))
}

/// Carga géneros: archivo de usuario si existe y es válido; si no, el integrado (con los errores a la vista).
pub fn load_genres_from(file: Option<&Path>) -> Loaded<GenreSet> {
    if let Some(f) = file.filter(|f| f.is_file()) {
        let shown = f.display().to_string();
        match fs::read_to_string(f) {
            Err(e) => {
                return Loaded { value: GenreSet::builtin(), origin: "integrado".into(), errors: vec![format!("{shown}: no se pudo leer ({e})")] };
            }
            Ok(src) => match GenreSet::from_json(&src) {
                Ok(g) => return Loaded { value: g, origin: shown, errors: vec![] },
                Err(errs) => {
                    let mut errors = vec![format!("{shown}: no es válido, se usan los géneros integrados")];
                    errors.extend(errs);
                    return Loaded { value: GenreSet::builtin(), origin: "integrado".into(), errors };
                }
            },
        }
    }
    Loaded { value: GenreSet::builtin(), origin: "integrado".into(), errors: vec![] }
}

pub fn load_genres() -> Loaded<GenreSet> {
    load_genres_from(genres_file().as_deref())
}

/// Mapas: integrados + los del archivo `drum-maps.json` + los guardados en `maps/`.
pub fn load_maps_from(file: Option<&Path>, dir: Option<&Path>) -> Loaded<MapSet> {
    let mut set = MapSet::builtin();
    let mut errors: Vec<String> = Vec::new();
    let mut origin = "integrados".to_string();
    if let Some(f) = file.filter(|f| f.is_file()) {
        let shown = f.display().to_string();
        match fs::read_to_string(f).map_err(|e| vec![e.to_string()]).and_then(|s| MapSet::from_json(&s)) {
            Ok(extra) => {
                for m in extra.maps {
                    set.upsert(m);
                }
                origin = format!("integrados + {shown}");
            }
            Err(errs) => {
                errors.push(format!("{shown}: no es válido, se ignora"));
                errors.extend(errs);
            }
        }
    }
    if let Some(d) = dir.filter(|d| d.is_dir()) {
        let mut files: Vec<PathBuf> = fs::read_dir(d).map(|rd| rd.filter_map(|e| e.ok().map(|e| e.path())).collect()).unwrap_or_default();
        files.sort();
        for f in files.into_iter().filter(|f| f.extension().is_some_and(|e| e == "json")) {
            let shown = f.display().to_string();
            match fs::read_to_string(&f).map_err(|e| vec![e.to_string()]).and_then(|s| DrumMap::parse_one(&s)) {
                Ok(m) => {
                    set.upsert(m);
                }
                Err(errs) => {
                    errors.push(format!("{shown}: mapa inválido, se ignora"));
                    errors.extend(errs);
                }
            }
        }
    }
    Loaded { value: set, origin, errors }
}

pub fn load_maps() -> Loaded<MapSet> {
    load_maps_from(drum_maps_file().as_deref(), maps_dir().as_deref())
}

fn safe_file_name(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' }).collect();
    let s = s.trim().to_string();
    if s.is_empty() {
        "mapa".to_string()
    } else {
        s
    }
}

pub fn save_map_to(dir: &Path, map: &DrumMap) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("no se pudo crear {}: {e}", dir.display()))?;
    let path = dir.join(format!("{}.json", safe_file_name(&map.name)));
    fs::write(&path, map.to_json()).map_err(|e| format!("no se pudo escribir {}: {e}", path.display()))?;
    Ok(path)
}

pub fn save_map(map: &DrumMap) -> Result<PathBuf, String> {
    let dir = maps_dir().ok_or("no encuentro la carpeta de configuración")?;
    save_map_to(&dir, map)
}

/// Guarda como `genero-bpm-semilla.mid` (si existe, añade -2, -3...).
pub fn export_to(dir: &Path, genre_id: &str, bpm: i32, seed: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("no se pudo crear {}: {e}", dir.display()))?;
    let base = safe_file_name(&format!("{genre_id}-{bpm}-{seed}"));
    let mut path = dir.join(format!("{base}.mid"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base}-{n}.mid"));
        n += 1;
    }
    fs::write(&path, bytes).map_err(|e| format!("no se pudo escribir {}: {e}", path.display()))?;
    Ok(path)
}

pub fn export_midi(genre_id: &str, bpm: i32, seed: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let dir = export_dir().ok_or("no encuentro la carpeta Documentos")?;
    export_to(&dir, genre_id, bpm, seed, bytes)
}

/// Guarda como `genero-bpm-semilla.wav` (si existe, añade -2, -3...).
pub fn export_wav_to(dir: &Path, genre_id: &str, bpm: i32, seed: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("no se pudo crear {}: {e}", dir.display()))?;
    let base = safe_file_name(&format!("{genre_id}-{bpm}-{seed}"));
    let mut path = dir.join(format!("{base}.wav"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{base}-{n}.wav"));
        n += 1;
    }
    fs::write(&path, bytes).map_err(|e| format!("no se pudo escribir {}: {e}", path.display()))?;
    Ok(path)
}

pub fn export_wav(genre_id: &str, bpm: i32, seed: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    let dir = export_dir().ok_or("no encuentro la carpeta Documentos")?;
    export_wav_to(&dir, genre_id, bpm, seed, bytes)
}

/// Abre una carpeta con el explorador del sistema.
pub fn open_folder(path: &Path) -> Result<(), String> {
    let _ = fs::create_dir_all(path);
    let (cmd, args): (&str, Vec<&std::ffi::OsStr>) = if cfg!(target_os = "windows") {
        ("explorer", vec![path.as_os_str()])
    } else if cfg!(target_os = "macos") {
        ("open", vec![path.as_os_str()])
    } else {
        ("xdg-open", vec![path.as_os_str()])
    };
    std::process::Command::new(cmd).args(args).spawn().map(|_| ()).map_err(|e| format!("no se pudo abrir la carpeta ({cmd}): {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("caja-test-{}-{}", name, std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn missing_user_genres_file_falls_back_silently() {
        let l = load_genres_from(Some(Path::new("/no/existe/genres.json")));
        assert!(l.errors.is_empty());
        assert_eq!(l.origin, "integrado");
        assert_eq!(l.value.len(), 53);
    }

    #[test]
    fn invalid_user_genres_file_reports_errors_and_falls_back() {
        let d = tmp("g");
        let f = d.join("genres.json");
        fs::write(&f, r#"{"genres":[{"id":"x","name":"X","bpm":100,"pieces":{"bombo":{"v":9,"p":[1]}}}]}"#).unwrap();
        let l = load_genres_from(Some(&f));
        assert!(l.errors.len() >= 2 && l.errors.iter().any(|e| e.contains("desconocida")), "{:?}", l.errors);
        assert_eq!(l.value.len(), 53);
        // y uno válido se carga
        let good = crate::genres::BUILTIN_JSON.replace("\"bpm\": 118", "\"bpm\": 119");
        fs::write(&f, good).unwrap();
        let l = load_genres_from(Some(&f));
        assert!(l.errors.is_empty());
        assert_eq!(l.value.get(l.value.index_of("motown").unwrap()).bpm, 119);
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn maps_save_and_reload_by_name() {
        let d = tmp("m");
        let mut m = MapSet::builtin().maps[1].clone();
        m.name = "Mi kit: 1/2".into();
        m.notes[0] = 60;
        let p = save_map_to(&d, &m).unwrap();
        assert!(p.file_name().unwrap().to_string_lossy().starts_with("Mi kit"));
        let l = load_maps_from(None, Some(&d));
        assert!(l.errors.is_empty(), "{:?}", l.errors);
        let i = l.value.index_of("Mi kit: 1/2").unwrap();
        assert_eq!(l.value.maps[i].notes[0], 60);
        fs::write(d.join("roto.json"), "{").unwrap();
        let l = load_maps_from(None, Some(&d));
        assert!(!l.errors.is_empty());
        assert!(l.value.index_of("GM").is_some());
        let _ = fs::remove_dir_all(d);
    }

    #[test]
    fn export_names_do_not_collide() {
        let d = tmp("e");
        let a = export_to(&d, "techno", 132, "4242", b"x").unwrap();
        let b = export_to(&d, "techno", 132, "4242", b"y").unwrap();
        assert_eq!(a.file_name().unwrap(), "techno-132-4242.mid");
        assert_eq!(b.file_name().unwrap(), "techno-132-4242-2.mid");
        let _ = fs::remove_dir_all(d);
    }
}
