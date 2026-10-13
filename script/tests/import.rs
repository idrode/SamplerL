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
