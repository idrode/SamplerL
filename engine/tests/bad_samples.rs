//! Samples rotos al cargar (N2 y N3 de `docs/AUDITORIA-2.md`): un WAV en
//! coma flotante con NaN, infinitos o valores enormes no mata el programa
//! (el resampleo de `rubato` entraba en pánico) ni deja muda la mezcla: se
//! sanea al cargarlo y [`Sample::repairs`] dice cuántas muestras.

use std::path::{Path, PathBuf};

use engine::{MAX_SAMPLE_LEVEL, Repairs, Sample};

fn dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("bad_samples");
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Escribe un WAV mono en coma flotante de 32 bits.
fn float_wav(name: &str, rate: u32, samples: &[f32]) -> PathBuf {
    let path = dir().join(name);
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    for &s in samples {
        writer.write_sample(s).unwrap();
    }
    writer.finalize().unwrap();
    path
}

/// Un seno con `bad` en medio.
fn with(bad: &[f32]) -> Vec<f32> {
    let mut samples: Vec<f32> = (0..4000).map(|i| 0.5 * (i as f32 * 0.05).sin()).collect();
    for (k, &b) in bad.iter().enumerate() {
        samples[2000 + 10 * k] = b;
    }
    samples
}

fn all_finite(sample: &Sample) -> bool {
    (0..sample.frames()).all(|i| {
        let (l, r) = sample.frame(i);
        l.is_finite() && r.is_finite()
    })
}

#[test]
fn nan_and_infinities_load_with_and_without_resampling() {
    let cases: [(&str, &[f32]); 3] = [
        ("nan", &[f32::NAN]),
        ("inf", &[f32::INFINITY, f32::NEG_INFINITY]),
        ("both", &[f32::NAN, f32::INFINITY, f32::NAN]),
    ];
    for (name, bad) in cases {
        for (rate, target) in [(44_100, 48_000), (48_000, 48_000), (96_000, 48_000)] {
            let path = float_wav(&format!("{name}-{rate}.wav"), rate, &with(bad));
            let sample =
                Sample::load_wav(&path, target).unwrap_or_else(|e| panic!("{name} a {rate}: {e}"));
            assert!(all_finite(&sample), "{name} a {rate}");
            assert_eq!(
                sample.repairs(),
                Repairs {
                    non_finite: bad.len(),
                    clamped: 0
                },
                "{name} a {rate}"
            );
        }
    }
}

#[test]
fn huge_values_are_clamped_and_subnormals_flushed() {
    let path = float_wav(
        "huge.wav",
        44_100,
        &with(&[1e30, -1e38, f32::MIN_POSITIVE / 4.0]),
    );
    let sample = Sample::load_wav(&path, 48_000).unwrap();
    assert!(all_finite(&sample));
    assert_eq!(sample.repairs().clamped, 2);
    assert_eq!(sample.repairs().non_finite, 0);
    // Sin resampleo: el valor exacto.
    let path = float_wav(
        "huge48.wav",
        48_000,
        &with(&[1e30, f32::MIN_POSITIVE / 4.0]),
    );
    let sample = Sample::load_wav(&path, 48_000).unwrap();
    assert_eq!(sample.frame(2000).0, MAX_SAMPLE_LEVEL);
    assert_eq!(sample.frame(2010).0, 0.0);
    let text = sample.repairs().to_string();
    assert!(text.contains("1 muestra fuera de rango"), "{text}");
}

#[test]
fn a_clean_file_is_untouched() {
    let samples = with(&[]);
    let path = float_wav("clean.wav", 48_000, &samples);
    let sample = Sample::load_wav(&path, 48_000).unwrap();
    assert!(!sample.repairs().any());
    for (i, &s) in samples.iter().enumerate() {
        assert_eq!(sample.frame(i), (s, s));
    }
}

/// Un WAV en coma flotante de 64 bits: `hound` no lo lee y la carga da
/// un error claro, sin pánico.
#[test]
fn a_64_bit_float_wav_is_a_clear_error() {
    let samples = [0.5f64, f64::NAN, -0.5, f64::INFINITY];
    let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
    let mut wav = Vec::new();
    wav.extend(b"RIFF");
    wav.extend((36 + data.len() as u32).to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(3u16.to_le_bytes()); // coma flotante
    wav.extend(1u16.to_le_bytes());
    wav.extend(48_000u32.to_le_bytes());
    wav.extend((48_000u32 * 8).to_le_bytes());
    wav.extend(8u16.to_le_bytes());
    wav.extend(64u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend((data.len() as u32).to_le_bytes());
    wav.extend(&data);
    let path = dir().join("f64.wav");
    std::fs::write(&path, wav).unwrap();
    let error = Sample::load_wav(&path, 44_100).err().expect("no se lee");
    assert!(error.to_string().contains("WAV"), "{error}");
}
