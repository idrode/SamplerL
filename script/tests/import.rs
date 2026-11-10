//! `sample(ruta, { trim, normalize, fade_in, fade_out })`.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{ImportOptions, Sample, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("import");
        std::fs::create_dir_all(&dir).unwrap();
        let mut mono = vec![0.0f32; 12_000];
        mono[2000..4000].fill(0.2);
        mono[6000..8000].fill(0.1);
        offline::write_wav(dir.join("suelto.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn load(host: &mut ScriptHost, source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    host.load(&mut controller, source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

#[test]
fn options_prepare_the_sample_before_slicing() {
    let source = "local s = sample(\"suelto.wav\", { trim = true, normalize = true, fade_out = 5 })\n\
                  track(\"d\", s:slice_grid(2))";
    let mut host = ScriptHost::new(RATE);
    let song = load(&mut host, source).unwrap();
    let sample = &song.tracks[0].sample;
    let original = Sample::load_wav(dir().join("suelto.wav"), RATE).unwrap();
    let expected = original.processed(&ImportOptions {
        trim_db: Some(-60.0),
        normalize_db: Some(0.0),
        fade_in_ms: 0.0,
        fade_out_ms: 5.0,
    });
    assert_eq!(sample.frames(), 6000);
    assert!((0..6000).all(|i| sample.frame(i) == expected.frame(i)));
    // La rejilla, sobre el sample recortado.
    assert_eq!(song.tracks[0].slices.get(1), Some(3000..6000));
    // Recargar reutiliza el preparado.
    let again = load(&mut host, source).unwrap();
    assert!(Arc::ptr_eq(sample, &again.tracks[0].sample));
    // Con `false`, nada.
    let plain = load(
        &mut host,
        "track(\"d\", sample(\"suelto.wav\", { trim = false }):slice(1))",
    )
    .unwrap();
    assert_eq!(plain.tracks[0].sample.frames(), 12_000);
}

#[test]
fn import_errors_point_at_the_line() {
    let error = |source: &str| load(&mut ScriptHost::new(RATE), source).err().unwrap();
    assert_eq!(
        error("local s = sample(\"suelto.wav\", { recortar = true })"),
        "song.lua:1: sample(\"suelto.wav\"): opción desconocida \"recortar\" \
         (admite: trim, normalize, fade_in, fade_out)"
    );
    assert!(
        error("local s = sample(\"suelto.wav\", { normalize = 3 })")
            .contains("normalize fuera de rango (-60 a 0 dB)")
    );
    assert!(
        error("local s = sample(\"suelto.wav\", { trim = \"mucho\" })")
            .contains("trim es true, false o un nivel en dB")
    );
    assert!(
        error("local s = sample(\"suelto.wav\", { fade_in = true })").contains("fade_in son ms")
    );
}

/// Un WAV en coma flotante con NaN e infinitos carga (saneado: esas
/// muestras a 0) y la carga lo avisa con el nombre y cuántas eran. A otra
/// frecuencia que el motor mataba el programa; a la misma, dejaba la mezcla
/// muda (N2 y N3 de `docs/AUDITORIA-2.md`).
#[test]
fn a_float_wav_with_nan_loads_with_a_warning_and_the_mix_keeps_sounding() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("import-nan");
    std::fs::create_dir_all(&dir).unwrap();
    // WAV mono en coma flotante de 32 bits, escrito a mano.
    for (file, rate) in [("nan44.wav", 44_100u32), ("nan48.wav", 48_000)] {
        let samples: Vec<f32> = (0..4001)
            .map(|i| match i {
                2000 => f32::NAN,
                2001 => f32::INFINITY,
                _ if i < 2000 => (i as f32 * 0.05).sin(),
                _ => 0.1,
            })
            .collect();
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut wav = Vec::new();
        wav.extend(b"RIFF");
        wav.extend((36 + data.len() as u32).to_le_bytes());
        wav.extend(b"WAVEfmt ");
        wav.extend(16u32.to_le_bytes());
        wav.extend(3u16.to_le_bytes());
        wav.extend(1u16.to_le_bytes());
        wav.extend(rate.to_le_bytes());
        wav.extend((rate * 4).to_le_bytes());
        wav.extend(4u16.to_le_bytes());
        wav.extend(32u16.to_le_bytes());
        wav.extend(b"data");
        wav.extend((data.len() as u32).to_le_bytes());
        wav.extend(&data);
        std::fs::write(dir.join(file), wav).unwrap();
    }
    engine::offline::write_wav(dir.join("a.wav"), &[0.5; 1000], 1, 48_000).unwrap();
    for file in ["nan44.wav", "nan48.wav"] {
        let source = format!(
            r#"bpm(120)
local malo = sample("{file}")
local a = sample("a.wav"):slice(1)
track("malo", malo:slice(1)):steps("1 . . .  . . . .  . . . .  . . . .")
  :fx("filter", {{ type = "lowpass", cutoff = 3000 }}):send("reverb", 0.3)
track("x", a):steps("1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1")
"#
        );
        let (mut controller, mut engine) = engine::new_engine(48_000);
        let mut host = script::ScriptHost::new(48_000);
        let song = host
            .load(&mut controller, &source, "song.lua", &dir)
            .map_err(|e| e.message)
            .unwrap();
        assert!(
            song.warnings
                .iter()
                .any(|w| w.starts_with(&format!("{file}: 2 muestras no válidas"))),
            "{:?}",
            song.warnings
        );
        controller.start().unwrap();
        let bar = 96_000;
        let audio = engine::offline::render(&mut engine, 4 * bar, 2);
        for b in 0..4 {
            let silent = audio[2 * b * bar..2 * (b + 1) * bar]
                .chunks(2)
                .filter(|f| f[0] == 0.0 && f[1] == 0.0)
                .count();
            assert!(
                silent < bar / 2,
                "{file}, compás {b}: {silent} frames mudos"
            );
        }
    }
}
