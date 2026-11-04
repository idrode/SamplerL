//! `bend`, `bend_time` y `rev` en `cortes:set()` y en las llaves de un paso:
//! llegan al modelo, suenan como el motor con lo mismo, `bend` admite curvas
//! por golpe y los errores dicen la línea.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{HitShape, Pattern, Sample, SliceShape, Slices, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;
const STEPS: &str = "1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .";

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("slice_bend");
        std::fs::create_dir_all(&dir).unwrap();
        // Un «bombo»: un seno de 120 Hz que se apaga, y una rampa.
        let mono: Vec<f32> = (0..24_000)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                if i < 12_000 {
                    0.6 * (-t * 8.0).exp() * (std::f32::consts::TAU * 120.0 * t).sin()
                } else {
                    0.1 + 0.3 * (i - 12_000) as f32 / 12_000.0
                }
            })
            .collect();
        offline::write_wav(dir.join("golpes.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!("bpm(120)\nlocal g = sample(\"golpes.wav\"):slice_grid(2)\n{source}");
    host.load(&mut controller, &source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

fn engine_render(pattern: Pattern) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_track_pattern(0, Some(pattern)).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, 2 * engine::bar_frames(120.0, RATE) as usize, 2)
}

#[test]
fn bend_and_rev_sound_like_the_engine() {
    let song = song(&format!(
        "g:set(1, {{ bend = -12, bend_time = 150 }}):set(2, {{ rev = true }})\n\
         track(\"a\", g):steps(\"{STEPS}\")"
    ))
    .unwrap();
    let shapes = [
        SliceShape {
            bend: -12.0,
            bend_ms: 150.0,
            ..script::DEFAULT_SHAPE
        },
        SliceShape {
            reverse: true,
            ..script::DEFAULT_SHAPE
        },
    ];
    assert_eq!(song.tracks[0].shapes, shapes);
    let sample = Arc::new(Sample::load_wav(dir().join("golpes.wav"), RATE).unwrap());
    let slices = Slices::grid(&sample, 2);
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
fn bend_takes_curves_and_braces() {
    let song = song(
        "g:set(1, { bend = ramp(0, -24, \"2 compases\"), bend_time = lfo(\"1 compás\", 50, 300) })\n\
         track(\"a\", g):steps(\"1{bend=7,bend_time=20} . 1{rev=1} .  1 . . .  . . . .  . . . .\")",
    )
    .unwrap();
    let shape = song.tracks[0].shapes[0];
    assert!(shape.curves[engine::SHAPE_BEND].is_some());
    assert!(shape.curves[engine::SHAPE_BEND_TIME].is_some());
    let bar = &song.tracks[0].bars[0];
    assert_eq!(
        bar[0].unwrap().hit,
        HitShape {
            bend: Some(7.0),
            bend_ms: Some(20.0),
            ..HitShape::default()
        }
    );
    assert_eq!(bar[2].unwrap().hit.reverse, Some(true));
    assert!(song.render(RATE, 2).is_ok());
}

#[test]
fn errors_point_at_the_line() {
    let error = |source: &str| song(source).err().expect("debería fallar");
    assert_eq!(
        error("g:set(1, { bend = 40 })"),
        "song.lua:3: set(1): bend fuera de rango (-36 a 36 semitonos)"
    );
    assert!(error("g:set(1, { bend_time = 0 })").contains("bend_time fuera de rango"));
    assert!(error("g:set(1, { rev = \"sí\" })").contains("rev es true o false"));
    let e = error("g:set(1, { bend = -5 })\ntrack(\"a\", g):length(1):steps(\"1%3\")");
    assert!(e.contains("bend no va con %"), "{e}");
    let e = error("track(\"a\", g):length(1):steps(\"1{rev=2}\")");
    assert!(e.contains("rev es 1 o 0"), "{e}");
    let e = error("track(\"a\", g):length(1):steps(\"1{bend=1>3}\")");
    assert!(e.contains("bend no admite rampas"), "{e}");
}
