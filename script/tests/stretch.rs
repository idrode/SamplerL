//! `sample:stretch()`: el sample se estira al tempo de la canción sin
//! cambiar el tono; los cortes se escalan y caen en la rejilla.

#[path = "../../engine/tests/support/synth_break.rs"]
mod synth;

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

/// Se escribe una sola vez: reescribir el WAV cambiaría su fecha y la caché
/// de samples lo volvería a cargar.
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("stretch");
        std::fs::create_dir_all(&dir).unwrap();
        let (audio, _) = synth::synth_break();
        offline::write_wav(dir.join("amen.wav"), &audio, 1, synth::RATE).unwrap();
        // Un golpe suelto: no es un loop, no hay tempo que detectar.
        let mut one = vec![0.0f32; 4000];
        one[..300].fill(0.5);
        offline::write_wav(dir.join("golpe.wav"), &one, 1, RATE).unwrap();
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

fn song(source: &str) -> Result<Song, String> {
    load(&mut ScriptHost::new(RATE), source)
}

const PATTERN: &str = "1 . 2 .  3 . 4 .  5 . 6 .  7 . 8 .";

#[test]
fn a_140_bpm_break_lasts_a_bar_at_170_and_its_hits_land_on_the_grid() {
    let plain = song(&format!(
        "bpm(170)\nlocal a = sample(\"amen.wav\"):slice(8)\ntrack(\"d\", a):steps(\"{PATTERN}\")"
    ))
    .unwrap();
    let stretched = song(&format!(
        "bpm(170)\nlocal a = sample(\"amen.wav\"):stretch(140):slice(8)\n\
         track(\"d\", a):steps(\"{PATTERN}\")"
    ))
    .unwrap();
    let (before, after) = (&plain.tracks[0], &stretched.tracks[0]);
    let ratio = 140.0 / 170.0;
    assert_eq!(after.stretch.unwrap().ratio, ratio);
    assert_eq!(
        after.sample.frames(),
        (before.sample.frames() as f64 * ratio).round() as usize
    );
    // Cada corte, escalado; y el n-ésimo golpe (corcheas) en el paso 2n.
    let step = 48_000.0 * 60.0 / (170.0 * 4.0);
    for (i, (a, b)) in before.slices.iter().zip(after.slices.iter()).enumerate() {
        assert_eq!(b.start, (a.start as f64 * ratio).round() as usize);
        let grid = (2 * i) as f64 * step;
        assert!(
            (b.start as f64 - grid).abs() < 0.003 * 48_000.0,
            "golpe {i} en {} y la rejilla en {grid}",
            b.start
        );
    }
    // El sample estirado dura un compás a 170 (con la cola del último golpe).
    let bar = engine::bar_frames(170.0, RATE) as f64;
    let frames = after.sample.frames() as f64;
    assert!((frames / bar - before.sample.frames() as f64 / (bar / ratio)).abs() < 1e-3);
    // Y detect_bpm ve 170.
    let guess = engine::detect_bpm(&after.sample).unwrap();
    assert_eq!(guess.rounded(), 170.0);
}

#[test]
fn without_a_tempo_it_uses_the_detected_one() {
    let auto = song("bpm(170)\nlocal a = sample(\"amen.wav\"):stretch():slice(8)\ntrack(\"d\", a)")
        .unwrap();
    assert_eq!(auto.tracks[0].stretch.unwrap().source_bpm, 140.0);
    let error = song("local a = sample(\"golpe.wav\"):stretch():slice(1)\ntrack(\"d\", a)")
        .err()
        .unwrap();
    assert_eq!(
        error,
        "song.lua:1: stretch(): no se detecta el tempo de golpe.wav; pásalo: stretch(bpm)"
    );
}

#[test]
fn at_its_own_tempo_nothing_changes() {
    let render = |stretch: &str| {
        song(&format!(
            "bpm(140)\nlocal a = sample(\"amen.wav\"){stretch}:slice(8)\n\
             track(\"d\", a):steps(\"{PATTERN}\")"
        ))
        .unwrap()
        .render(RATE, 2)
        .unwrap()
    };
    assert_eq!(render(":stretch(140)"), render(""));
}

#[test]
fn tracks_of_the_same_sample_share_it_and_reloads_reuse_it() {
    let source = "bpm(160)\nlocal a = sample(\"amen.wav\"):stretch(140):slice(8)\n\
                  track(\"d\", a)\ntrack(\"k\", a:slice_at(1))";
    let mut host = ScriptHost::new(RATE);
    let first = load(&mut host, source).unwrap();
    assert!(Arc::ptr_eq(
        &first.tracks[0].sample,
        &first.tracks[1].sample
    ));
    assert_eq!(first.tracks[1].slices.get(0), first.tracks[0].slices.get(0));
    let again = load(&mut host, source).unwrap();
    assert!(Arc::ptr_eq(
        &first.tracks[0].sample,
        &again.tracks[0].sample
    ));
    // Otro tempo: otro sample.
    let faster = load(&mut host, &source.replace("bpm(160)", "bpm(170)")).unwrap();
    assert!(!Arc::ptr_eq(
        &first.tracks[0].sample,
        &faster.tracks[0].sample
    ));
}

#[test]
fn manual_positions_are_in_ms_of_the_original() {
    let moved = song(
        "bpm(170)\nlocal a = sample(\"amen.wav\"):stretch(140):slice(8):move(2, 200)\ntrack(\"d\", a)",
    )
    .unwrap();
    let expected = (0.2 * 48_000.0 * 140.0 / 170.0f64).round() as usize;
    assert_eq!(moved.tracks[0].slices.get(1).unwrap().start, expected);
}

#[test]
fn stretch_errors_point_at_the_line() {
    assert_eq!(
        song("local a = sample(\"amen.wav\"):stretch(10)")
            .err()
            .unwrap(),
        "song.lua:1: stretch(10): el tempo de origen va de 20 a 400"
    );
    let error =
        song("bpm(400)\nlocal a = sample(\"amen.wav\"):stretch(60):slice(8)\ntrack(\"d\", a)")
            .err()
            .unwrap();
    assert!(
        error.contains("estirar de 60 a 400 BPM es ×0.15"),
        "{error}"
    );
}
