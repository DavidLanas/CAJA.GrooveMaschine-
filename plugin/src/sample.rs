//! Muestras decodificadas: lectura (WAV/FLAC/OGG con symphonia), remuestreo a la frecuencia del
//! host y recorte. Todo esto corre en el hilo de carga: **nunca** en `process`.

use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::DecoderOptions;
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;

/// Audio en `f32` a la frecuencia del host. Mono = solo `left`.
pub struct Sample {
    pub left: Vec<f32>,
    pub right: Option<Vec<f32>>,
}

impl Sample {
    #[allow(dead_code)] // lo usan las pruebas
    pub fn len(&self) -> usize {
        self.left.len()
    }
    pub fn is_empty(&self) -> bool {
        self.left.is_empty()
    }
    /// Bytes de RAM que ocupa (para el tope).
    pub fn bytes(&self) -> usize {
        (self.left.len() + self.right.as_ref().map_or(0, Vec::len)) * 4
    }
}

/// Decodifica a planar `f32` en la frecuencia original.
fn decode(path: &Path) -> Result<(Vec<Vec<f32>>, u32), String> {
    let file = File::open(path).map_err(|e| format!("no se pudo abrir: {e}"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("formato no reconocido: {e}"))?;
    let mut format = probed.format;
    let track = format.default_track().ok_or("sin pista de audio")?;
    let track_id = track.id;
    let sr = track.codec_params.sample_rate.ok_or("sin frecuencia de muestreo")?;
    let mut decoder = symphonia::default::get_codecs().make(&track.codec_params, &DecoderOptions::default()).map_err(|e| format!("códec no soportado: {e}"))?;
    let mut planar: Vec<Vec<f32>> = Vec::new();
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(SymError::ResetRequired) => break,
            Err(e) => {
                if planar.is_empty() {
                    return Err(format!("lectura fallida: {e}"));
                }
                break;
            }
        };
        if packet.track_id() != track_id {
            continue;
        }
        match decoder.decode(&packet) {
            Ok(buf) => {
                let spec = *buf.spec();
                let ch = spec.channels.count().max(1);
                if planar.is_empty() {
                    planar = vec![Vec::new(); ch.min(2)];
                }
                let mut sb = SampleBuffer::<f32>::new(buf.capacity() as u64, spec);
                sb.copy_interleaved_ref(buf);
                for frame in sb.samples().chunks_exact(ch) {
                    for (c, v) in planar.iter_mut().enumerate() {
                        v.push(frame[c]);
                    }
                }
            }
            Err(SymError::DecodeError(_)) => continue, // un paquete roto no tira el archivo
            Err(e) => return Err(format!("decodificación fallida: {e}")),
        }
    }
    if planar.is_empty() || planar[0].is_empty() {
        return Err("archivo vacío".into());
    }
    Ok((planar, sr))
}

/// Remuestreo con sinc ventaneada (Hann), de banda limitada; offline.
pub fn resample(src: &[f32], from: u32, to: u32) -> Vec<f32> {
    if from == to || src.is_empty() {
        return src.to_vec();
    }
    const HALF: isize = 16; // cruces por cero a cada lado
    let ratio = f64::from(to) / f64::from(from);
    let cutoff = ratio.min(1.0); // al bajar la frecuencia, filtra primero
    let n_out = ((src.len() as f64) * ratio).round().max(1.0) as usize;
    let width = HALF as f64 / cutoff;
    let mut out = Vec::with_capacity(n_out);
    for i in 0..n_out {
        let t = i as f64 / ratio;
        let c = t.floor() as isize;
        let lo = c - width.ceil() as isize;
        let hi = c + width.ceil() as isize + 1;
        let mut acc = 0.0f64;
        let mut norm = 0.0f64;
        for k in lo..=hi {
            let d = k as f64 - t;
            if d.abs() >= width {
                continue;
            }
            let x = std::f64::consts::PI * d * cutoff;
            let sinc = if x.abs() < 1e-9 { 1.0 } else { x.sin() / x };
            let w = 0.5 * (1.0 + (std::f64::consts::PI * d / width).cos());
            let h = sinc * w * cutoff;
            norm += h;
            if k >= 0 && (k as usize) < src.len() {
                acc += h * f64::from(src[k as usize]);
            }
        }
        out.push(if norm.abs() > 1e-12 { (acc / norm) as f32 } else { 0.0 });
    }
    out
}

/// Lee un archivo y lo deja listo: a la frecuencia del host, recortado a `max_len_ms`
/// (con un fundido corto para no hacer clic).
pub fn load(path: &Path, host_sr: u32, max_len_ms: u32) -> Result<Sample, String> {
    let (planar, sr) = decode(path)?;
    let max_frames = ((u64::from(max_len_ms) * u64::from(sr) / 1000) as usize).max(1);
    let trimmed = planar.iter().any(|c| c.len() > max_frames);
    let mut chans: Vec<Vec<f32>> = planar
        .into_iter()
        .map(|mut c| {
            c.truncate(max_frames);
            c
        })
        .collect();
    if sr != host_sr {
        chans = chans.iter().map(|c| resample(c, sr, host_sr)).collect();
    }
    // Fundido de 3 ms al final si se recortó, para que no haga clic.
    if trimmed {
        for c in &mut chans {
            let fade = ((host_sr as usize) * 3 / 1000).max(1).min(c.len());
            let n = c.len();
            for i in 0..fade {
                c[n - fade + i] *= 1.0 - (i as f32 + 1.0) / fade as f32;
            }
        }
    }
    let mut it = chans.into_iter();
    let left = it.next().ok_or("sin canales")?;
    let mut right = it.next();
    // Estéreo falso (L == R): guardar solo uno ahorra la mitad de la RAM.
    if right.as_ref().is_some_and(|r| *r == left) {
        right = None;
    }
    Ok(Sample { left, right })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_wav(path: &Path, sr: u32, ch: u16, frames: usize, f: impl Fn(usize, u16) -> f32) {
        let mut data = Vec::new();
        for i in 0..frames {
            for c in 0..ch {
                data.extend_from_slice(&((f(i, c) * 32767.0) as i16).to_le_bytes());
            }
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&ch.to_le_bytes());
        out.extend_from_slice(&sr.to_le_bytes());
        out.extend_from_slice(&(sr * u32::from(ch) * 2).to_le_bytes());
        out.extend_from_slice(&(ch * 2).to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        std::fs::write(path, out).unwrap();
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("caja-test-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d.join(name)
    }

    #[test]
    fn loads_mono_and_stereo_wav() {
        let p = tmp("mono.wav");
        write_wav(&p, 48000, 1, 4800, |i, _| (i as f32 * 0.05).sin() * 0.5);
        let s = load(&p, 48000, 3000).unwrap();
        assert_eq!(s.len(), 4800);
        assert!(s.right.is_none());
        let p = tmp("stereo.wav");
        write_wav(&p, 48000, 2, 4800, |i, c| (i as f32 * 0.05).sin() * if c == 0 { 0.5 } else { 0.25 });
        let s = load(&p, 48000, 3000).unwrap();
        assert!(s.right.is_some() && s.right.as_ref().unwrap().len() == 4800);
    }

    #[test]
    fn resamples_to_host_rate_keeping_pitch() {
        // 1 kHz a 44.1 k -> 48 k: misma duración en segundos y la frecuencia sigue en 1 kHz.
        let p = tmp("sine44.wav");
        write_wav(&p, 44100, 1, 44100, |i, _| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44100.0).sin() * 0.8);
        let s = load(&p, 48000, 3000).unwrap();
        assert!((s.len() as i64 - 48000).abs() <= 1);
        let crossings = s.left.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
        assert!((995..=1005).contains(&crossings), "{crossings}");
        let peak = s.left[1000..40000].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!((peak - 0.8).abs() < 0.02, "{peak}");
    }

    #[test]
    fn trims_long_samples_and_rejects_garbage() {
        let p = tmp("long.wav");
        write_wav(&p, 48000, 1, 48000 * 5, |i, _| ((i % 100) as f32 / 100.0) - 0.5);
        let s = load(&p, 48000, 1000).unwrap();
        assert_eq!(s.len(), 48000);
        assert!(s.left[s.len() - 1].abs() < 0.05, "fundido final");
        let bad = tmp("bad.wav");
        std::fs::write(&bad, b"esto no es audio").unwrap();
        assert!(load(&bad, 48000, 3000).is_err());
        assert!(load(&tmp("missing.wav"), 48000, 3000).is_err());
    }
}

/// Comprobación barata de la cabecera (sin decodificar todo): el escaneo ignora lo que falle aquí.
pub fn probe(path: &Path) -> Result<(), String> {
    let file = File::open(path).map_err(|e| format!("no se pudo abrir ({e})"))?;
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
        hint.with_extension(ext);
    }
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("formato no reconocido ({e})"))?;
    let track = probed.format.default_track().ok_or("sin pista de audio")?;
    if track.codec_params.sample_rate.is_none() {
        return Err("sin frecuencia de muestreo".into());
    }
    Ok(())
}
