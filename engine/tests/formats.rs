//! Samples en FLAC, MP3, OGG Vorbis y AIFF (M4 de la primera auditoría):
//! se leen con `symphonia`; los WAV siguen con `hound`. Las muestras de
//! `tests/data/` son el mismo seno estéreo de 0,25 s (440 Hz a la
//! izquierda, 660 a la derecha, a 44,1 kHz), hecho con ffmpeg y lame.

use std::path::{Path, PathBuf};

use engine::Sample;

const RATE: u32 = 48_000;

fn data(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn channel(sample: &Sample, right: bool) -> Vec<f32> {
    (0..sample.frames())
        .map(|i| {
            let (l, r) = sample.frame(i);
            if right { r } else { l }
        })
        .collect()
}

/// La frecuencia de un seno por sus cruces por cero, en 100 ms desde que
/// suena (un resto de silencio con ruido cruzaría el cero muchas veces).
fn frequency(x: &[f32]) -> f32 {
    let start = x.iter().position(|v| v.abs() > 0.05).unwrap_or(0) + 200;
    let x = &x[start..(start + RATE as usize / 10).min(x.len())];
    let crossings = x.windows(2).filter(|w| w[0] <= 0.0 && w[1] > 0.0).count();
    crossings as f32 * RATE as f32 / x.len() as f32
}

#[test]
fn flac_and_aiff_are_the_same_as_the_wav() {
    let wav = Sample::load_wav(data("seno.wav"), RATE).unwrap();
    for name in ["seno.flac", "seno.aiff"] {
        let other = Sample::load_wav(data(name), RATE).unwrap();
        assert_eq!(other.frames(), wav.frames(), "{name}");
        for i in 0..wav.frames() {
            assert_eq!(other.frame(i), wav.frame(i), "{name}, frame {i}");
        }
    }
}

#[test]
fn mp3_and_ogg_sound_like_the_wav() {
    let wav = Sample::load_wav(data("seno.wav"), RATE).unwrap();
    for name in ["seno.mp3", "seno.ogg"] {
        let other = Sample::load_wav(data(name), RATE).unwrap();
        // Con pérdidas y el relleno del códec: la misma duración más o
        // menos un par de bloques, y la misma frecuencia en cada canal.
        assert!(
            other.frames().abs_diff(wav.frames()) < 4096,
            "{name}: {} frames frente a {}",
            other.frames(),
            wav.frames()
        );
        for (right, hz) in [(false, 440.0), (true, 660.0)] {
            let f = frequency(&channel(&other, right));
            assert!((f - hz).abs() < hz * 0.08, "{name}: {f} Hz en vez de {hz}");
        }
    }
}

#[test]
fn a_broken_file_is_an_error_without_panicking() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("formats");
    std::fs::create_dir_all(&dir).unwrap();
    // La cabecera de un FLAC y luego basura; un MP3 vacío; un OGG cortado.
    let flac = std::fs::read(data("seno.flac")).unwrap();
    let mut broken = flac[..64].to_vec();
    broken.extend((0..4000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8));
    std::fs::write(dir.join("roto.flac"), broken).unwrap();
    std::fs::write(dir.join("vacio.mp3"), b"").unwrap();
    let ogg = std::fs::read(data("seno.ogg")).unwrap();
    std::fs::write(dir.join("cortado.ogg"), &ogg[..ogg.len() / 3]).unwrap();
    for name in ["roto.flac", "vacio.mp3", "cortado.ogg"] {
        // Puede leer algo (lo que hay antes de lo roto) o dar error; nunca
        // un pánico, y lo que lee es finito.
        if let Ok(sample) = Sample::load_wav(dir.join(name), RATE) {
            assert!(
                (0..sample.frames()).all(|i| sample.frame(i).0.is_finite()),
                "{name}"
            );
        }
    }
    assert!(engine::SAMPLE_EXTENSIONS.contains(&"flac"));
}
