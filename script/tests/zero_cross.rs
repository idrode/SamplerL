//! `zero_cross` en `slice` y `slice_grid`: cada inicio de corte en el cruce
//! por cero más cercano, también los de `add`, `move` y los ajustes del
//! proyecto; sin la opción, los cortes de siempre.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{Sample, new_engine};
use script::{Cut, ScriptHost, SlicePosition, Song};

const RATE: u32 = 48_000;
/// La amplitud y la frecuencia del seno del sample.
const AMP: f32 = 0.8;
const HZ: f32 = 113.0;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("zero_cross");
        std::fs::create_dir_all(&dir).unwrap();
        // 1 s de un seno de 113 Hz: un cruce cada ~4,4 ms.
        let mono: Vec<f32> = (0..RATE as usize)
            .map(|i| AMP * (std::f32::consts::TAU * HZ * i as f32 / RATE as f32).sin())
            .collect();
        engine::offline::write_wav(dir.join("seno.wav"), &mono, 1, RATE).unwrap();
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

fn starts(song: &Song) -> Vec<usize> {
    song.tracks[0].slices.iter().map(|r| r.start).collect()
}

fn sample() -> Sample {
    Sample::load_wav(dir().join("seno.wav"), RATE).unwrap()
}

/// Lo más que cambia el seno de una muestra a la siguiente: un inicio en
/// el cruce vale menos que esto.
fn one_step() -> f32 {
    AMP * (std::f32::consts::TAU * HZ / RATE as f32)
}

fn value_at(sample: &Sample, frame: usize) -> f32 {
    sample.frame(frame).0
}

#[test]
fn without_the_option_the_slices_are_the_same_as_always() {
    for options in [
        "",
        ", {}",
        ", { zero_cross = false }",
        ", { zero_cross = 0 }",
    ] {
        let with = song(&format!(
            "local s = sample(\"seno.wav\"):slice_grid(8{options})\ntrack(\"d\", s):steps(\"1 2 3 4  5 6 7 8  1 . . .  . . . .\")"
        ))
        .unwrap();
        assert_eq!(starts(&with), (0..8).map(|k| k * 6000).collect::<Vec<_>>());
        assert_eq!(with.tracks[0].origin.zero_cross, None);
    }
    let plain = song(
        "track(\"d\", sample(\"seno.wav\"):slice(4)):steps(\"1 . . .  1 . . .  1 . . .  1 . . .\")",
    )
    .unwrap();
    let off = song(
        "track(\"d\", sample(\"seno.wav\"):slice(4, { zero_cross = false })):steps(\"1 . . .  1 . . .  1 . . .  1 . . .\")",
    )
    .unwrap();
    assert_eq!(plain.render(RATE, 2).unwrap(), off.render(RATE, 2).unwrap());
}

#[test]
fn with_the_option_every_start_is_at_a_crossing() {
    let s = sample();
    let grid = song("track(\"d\", sample(\"seno.wav\"):slice_grid(8))").unwrap();
    let zero =
        song("track(\"d\", sample(\"seno.wav\"):slice_grid(8, { zero_cross = 3 }))").unwrap();
    assert_eq!(zero.tracks[0].origin.zero_cross, Some(3.0));
    // Las etiquetas y ajustes siguen siendo los de la rejilla de 8.
    assert_eq!(zero.tracks[0].origin.cut(), Cut::grid(8));
    let mut far = 0;
    for (a, b) in starts(&grid).into_iter().zip(starts(&zero)) {
        assert!(a.abs_diff(b) <= 144, "{a} → {b}: más de 3 ms");
        assert!(
            value_at(&s, b).abs() <= one_step(),
            "{b}: {}",
            value_at(&s, b)
        );
        far += usize::from(value_at(&s, a).abs() > 10.0 * one_step());
    }
    // Sin la opción, casi todos empezaban lejos del cero.
    assert!(far >= 5, "{far}");
    // `true` es 2 ms.
    let two =
        song("track(\"d\", sample(\"seno.wav\"):slice_grid(8, { zero_cross = true }))").unwrap();
    assert_eq!(two.tracks[0].origin.zero_cross, Some(2.0));
}

/// El golpe empieza sin salto: lo primero que suena del corte sale de cero
/// (sin la opción, el fundido de 1,5 ms sube desde la mitad del seno).
#[test]
fn the_hit_starts_without_a_jump() {
    let pattern = ". . . .  3 . . .  . . . .  . . . .";
    let first = |options: &str| -> f32 {
        let song = song(&format!(
            "bpm(120)\ntrack(\"d\", sample(\"seno.wav\"):slice_grid(8{options})):steps(\"{pattern}\")"
        ))
        .unwrap();
        let audio = song.render(RATE, 1).unwrap();
        // El paso 4 a 120 BPM: frame 24000. Lo más alto del primer
        // cuarto del fundido (18 frames).
        let at = 24_000;
        assert!(audio[..2 * at].iter().all(|&x| x == 0.0));
        audio[2 * at..2 * (at + 18)]
            .iter()
            .fold(0.0f32, |m, x| m.max(x.abs()))
    };
    let jump = first("");
    let clean = first(", { zero_cross = 3 }");
    // El corte 3 de la rejilla empieza en 12000: el seno ahí está en su pico.
    assert!(jump > 0.1, "{jump}");
    // Desde el cero, 18 frames de seno de 113 Hz no pasan de 0,21, y el
    // fundido los baja aún más.
    assert!(clean < 0.05, "{clean}");
}

#[test]
fn add_and_move_land_on_a_crossing_too() {
    let s = sample();
    let song = song(
        "local s = sample(\"seno.wav\"):slice(1, { zero_cross = 3 })\n\
         s:add(301)\n\
         s:move(2, 512)\n\
         s:add(733)\n\
         track(\"d\", s)",
    )
    .unwrap();
    let starts = starts(&song);
    assert_eq!(starts.len(), 3, "{starts:?}");
    for (&frame, ms) in starts[1..].iter().zip([512.0, 733.0]) {
        let target = (ms / 1000.0 * RATE as f64) as usize;
        assert!(frame.abs_diff(target) <= 144, "{frame} lejos de {target}");
        assert!(value_at(&s, frame).abs() <= one_step(), "{frame}");
    }
}

/// Los ajustes en ms del proyecto son el objetivo: el inicio real es el
/// cruce más cercano a él (y el ms guardado no cambia).
#[test]
fn a_manual_adjustment_is_the_target_of_the_snap() {
    let s = sample();
    let mut host = ScriptHost::new(RATE);
    host.set_positions(vec![SlicePosition {
        sample: "seno.wav".into(),
        slices: Cut::grid(4),
        slice: 2,
        ms: 260.0,
    }]);
    let plain = load(
        &mut host,
        "track(\"d\", sample(\"seno.wav\"):slice_grid(4))",
    )
    .unwrap();
    assert_eq!(starts(&plain)[1], 12_480);
    let zero = load(
        &mut host,
        "track(\"d\", sample(\"seno.wav\"):slice_grid(4, { zero_cross = 3 }))",
    )
    .unwrap();
    let start = starts(&zero)[1];
    assert!(start.abs_diff(12_480) <= 144);
    assert!(value_at(&s, start).abs() <= one_step());
    assert_eq!(host.positions()[0].ms, 260.0);
}

#[test]
fn a_wrong_option_is_an_error_with_its_line() {
    for (source, message) in [
        (
            "local s = sample(\"seno.wav\")\nlocal c = s:slice(4, { zero_cross = 80 })",
            "song.lua:2: slice(4): zero_cross es true, false o los ms de 0 a 50 (no 80)",
        ),
        (
            "local c = sample(\"seno.wav\"):slice_grid(4, { zero = 2 })",
            "song.lua:1: slice_grid(4): opción desconocida «zero» (valen: zero_cross)",
        ),
        (
            "local c = sample(\"seno.wav\"):slice(4, { zero_cross = ramp(1, 2, \"4 pasos\") })",
            "zero_cross no admite ramp() ni lfo()",
        ),
    ] {
        let Err(error) = song(source) else {
            panic!("{source}: sin error")
        };
        assert!(error.contains(message), "{error}");
    }
}
