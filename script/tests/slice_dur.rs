//! `dur`, `fit`, `loop_start` y `loop_end` en `cortes:set()` y en las llaves
//! de un paso: llegan al modelo, suenan como el motor con lo mismo y los
//! errores dicen la línea.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{Dur, Fit, HitShape, Pattern, Sample, SliceShape, Slices, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;
const STEPS: &str = "1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .";

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("slice_dur");
        std::fs::create_dir_all(&dir).unwrap();
        // Dos notas largas: 220 Hz y 330 Hz, medio segundo cada una.
        let mono: Vec<f32> = (0..48_000)
            .map(|i| {
                let hz = if i < 24_000 { 220.0 } else { 330.0 };
                0.4 * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin()
            })
            .collect();
        offline::write_wav(dir.join("notas.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!("bpm(120)\nlocal n = sample(\"notas.wav\"):slice_grid(2)\n{source}");
    host.load(&mut controller, &source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

fn base() -> (Arc<Sample>, Slices) {
    let sample = Arc::new(Sample::load_wav(dir().join("notas.wav"), RATE).unwrap());
    let slices = Slices::grid(&sample, 2);
    (sample, slices)
}

fn engine_render(pattern: Pattern) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_track_pattern(0, Some(pattern)).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, 2 * engine::bar_frames(120.0, RATE) as usize, 2)
}

#[test]
fn set_takes_ms_figures_and_bars() {
    let song = song(
        "n:set(1, { dur = 120 }):set(2, { dur = \"1/8\", fit = \"stretch\" })\n\
         track(\"a\", n):steps(\"1 2 . .  . . . .  . . . .  . . . .\")\n\
         n:set(1, { dur = \"2 compases\", fit = \"loop\", loop_start = 50, loop_end = 200 })\n\
         track(\"b\", n:slice_at(1)):steps(\"x . . .  . . . .  . . . .  . . . .\")\n\
         n:set(2, { dur = \"1 compás\" }):set(1, { dur = \"3bars\" })",
    )
    .unwrap();
    let shapes = &song.tracks[0].shapes;
    assert_eq!(shapes[0].dur, Some(Dur::Bars(3)));
    assert_eq!(shapes[0].fit, Fit::Loop);
    assert_eq!(shapes[0].loop_start_ms, Some(50.0));
    assert_eq!(shapes[0].loop_end_ms, Some(200.0));
    assert_eq!(shapes[1].dur, Some(Dur::Bars(1)));
    assert_eq!(shapes[1].fit, Fit::Stretch);
    // `slice_at` copia la forma de cuando se llamó.
    assert_eq!(song.tracks[1].shapes[0].dur, Some(Dur::Bars(2)));
}

#[test]
fn variants_sound_like_the_engine() {
    let source = format!(
        "n:set(1, {{ dur = \"1/16\" }})\n\
         n:set(2, {{ dur = 400, fit = \"stretch\" }})\n\
         track(\"a\", n):steps(\"{STEPS}\")"
    );
    let song = song(&source).unwrap();
    let (sample, slices) = base();
    let shapes = [
        SliceShape {
            dur: Some(Dur::Note { num: 1, den: 16 }),
            ..script::DEFAULT_SHAPE
        },
        SliceShape {
            dur: Some(Dur::Ms(400.0)),
            fit: Fit::Stretch,
            ..script::DEFAULT_SHAPE
        },
    ];
    let pattern = Pattern::from_steps(
        sample,
        &slices,
        engine::notation::parse_steps(STEPS).unwrap(),
    )
    .unwrap()
    .with_shapes(&shapes)
    .unwrap();
    assert_eq!(song.render(RATE, 2).unwrap(), engine_render(pattern));
}

#[test]
fn braces_of_a_step_override_the_slice() {
    let song = song(
        "n:set(1, { dur = 300 })\n\
         track(\"a\", n):steps(\"1{dur=1/8,fit=loop,loop_start=20,loop_end=90} . 1{dur=2bars}{lp=800} 1  . . . .  . . . .  . . . .\")",
    )
    .unwrap();
    let bar = &song.tracks[0].bars[0];
    let first = bar[0].unwrap();
    assert_eq!(
        first.hit,
        HitShape {
            dur: Some(Dur::Note { num: 1, den: 8 }),
            fit: Some(Fit::Loop),
            loop_start_ms: Some(20.0),
            loop_end_ms: Some(90.0),
            ..HitShape::default()
        }
    );
    let third = bar[2].unwrap();
    assert_eq!(third.hit.dur, Some(Dur::Bars(2)));
    assert!(third.fx.is_some());
    assert!(bar[3].unwrap().hit.is_empty());
    // Suena: dos compases sin errores.
    assert!(song.render(RATE, 2).is_ok());
}

#[test]
fn errors_point_at_the_line() {
    let error = |source: &str| song(source).err().expect("debería fallar");
    assert_eq!(
        error("n:set(1, { dur = \"mucho\" })"),
        "song.lua:3: set(1): dur = \"mucho\": una duración es ms (un número), una figura (\"1/8\") o compases (\"2 compases\")"
    );
    assert!(error("n:set(1, { dur = 0 })").contains("dur fuera de rango"));
    assert!(error("n:set(1, { dur = \"40 compases\" })").contains("1 a 16 compases"));
    assert!(error("n:set(1, { fit = \"estira\" })").contains("fit = \"estira\" no existe"));
    assert!(
        error("n:set(1, { dur = 100, play = \"loop\" })").contains("dur va con play = \"normal\"")
    );
    assert!(
        error("n:set(1, { loop_start = 90, loop_end = 20 })")
            .contains("loop_end tiene que ir después de loop_start")
    );
    let e = error("n:set(1, { dur = ramp(100, 400, \"4 compases\") })");
    assert!(e.contains("dur"), "{e}");
    let e = error("track(\"a\", n):length(1):steps(\"1{dur=0/8}\")");
    assert!(e.contains("song.lua:3") && e.contains("dur"), "{e}");
    let e = error("track(\"a\", n):length(1):steps(\"1{fit=estira}\")");
    assert!(e.contains("fit es trim, stretch o loop"), "{e}");
    let e = error("track(\"a\", n):length(1):steps(\"1{dur=1/8,lp=300}\")");
    assert!(
        e.contains("la forma del corte va en sus propias llaves"),
        "{e}"
    );
    let e =
        error("track(\"a\", sample(\"notas.wav\"):instrument()):length(1):steps(\"1{dur=1/8}\")");
    assert!(e.contains("un instrumento no tiene forma por paso"), "{e}");
}
