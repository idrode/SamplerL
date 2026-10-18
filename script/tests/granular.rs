//! `pista:grains()` y `pista:freeze()` desde Lua: parámetros con números,
//! `ramp()` y `lfo()`, semillas, errores con la línea y que la suma de
//! ocho nubes no pase de ±1.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{Knob, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

/// `dc.wav`: un segundo de continua a 0,5; `tono.wav`: un seno de 440 Hz.
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("granular");
        std::fs::create_dir_all(&dir).unwrap();
        offline::write_wav(dir.join("dc.wav"), &vec![0.5; RATE as usize], 1, RATE).unwrap();
        let tone: Vec<f32> = (0..RATE as usize)
            .map(|i| (std::f32::consts::TAU * 440.0 * i as f32 / RATE as f32).sin())
            .collect();
        offline::write_wav(dir.join("tono.wav"), &tone, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!("bpm(120)\n{source}");
    host.load(&mut controller, &source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

const BAR: &str = "1 _ _ _  _ _ _ _  _ _ _ _  _ _ _ _";

#[test]
fn grains_and_freeze_reach_the_track() {
    let song = song(&format!(
        "local s = sample(\"tono.wav\"):slice(1)\n\
         track(\"a\", s):steps(\"{BAR}\"):grains({{ size = 40, density = ramp(5, 50, \"1 compás\"), pitch = lfo(1, -5, 5), seed = 3 }})\n\
         track(\"b\", s):steps(\"{BAR}\"):freeze(0.25, {{ jitter = 10 }})\n\
         track(\"c\", s):steps(\"{BAR}\"):freeze()"
    ))
    .unwrap();
    let a = song.tracks[0].granular.unwrap();
    assert_eq!(a.size_ms, Knob::Fixed(40.0));
    assert!(matches!(
        a.density,
        Knob::Moving(engine::Curve::Ramp { .. })
    ));
    assert!(matches!(a.pitch, Knob::Moving(engine::Curve::Lfo { .. })));
    assert_eq!(a.speed, Knob::Fixed(1.0));
    assert_eq!(a.seed, 3);
    let b = song.tracks[1].granular.unwrap();
    assert_eq!((b.position, b.speed), (Knob::Fixed(0.25), Knob::Fixed(0.0)));
    assert_eq!(b.jitter_ms, Knob::Fixed(10.0));
    let c = song.tracks[2].granular.unwrap();
    assert_eq!((c.position, c.speed), (Knob::Fixed(0.0), Knob::Fixed(0.0)));
    // Sin semilla, cada pista la suya (del nombre).
    assert_ne!(b.seed, c.seed);
}

/// Canal izquierdo del primer segundo.
fn first_second(song: &Song) -> Vec<f32> {
    let audio = song.render(RATE, 2).unwrap();
    audio
        .iter()
        .step_by(2)
        .take(RATE as usize)
        .copied()
        .collect()
}

#[test]
fn density_is_grains_per_second_from_lua() {
    let song = song(&format!(
        "track(\"g\", sample(\"dc.wav\"):slice(1)):steps(\"{BAR}\"):freeze(0.5, {{ size = 5, density = 12 }})"
    ))
    .unwrap();
    let out = first_second(&song);
    let bumps = (1..out.len())
        .filter(|&i| out[i - 1] <= 0.25 && out[i] > 0.25)
        .count();
    assert_eq!(bumps, 12);
}

#[test]
fn the_same_seed_sounds_the_same() {
    let cloud = |seed: &str| {
        song(&format!(
            "track(\"g\", sample(\"tono.wav\"):slice(1)):steps(\"{BAR}\")\n\
               :grains({{ size = 50, density = 40, jitter = 100, spread = 5{seed} }})"
        ))
        .unwrap()
    };
    let a = first_second(&cloud(", seed = 9"));
    let b = first_second(&cloud(", seed = 9"));
    let c = first_second(&cloud(", seed = 10"));
    // Sin semilla, también determinista (la del nombre).
    let d = first_second(&cloud(""));
    let e = first_second(&cloud(""));
    assert!(a == b && d == e);
    assert!(a != c && a != d);
}

#[test]
fn eight_clouds_never_pass_one() {
    let mut source = String::from("local s = sample(\"dc.wav\"):slice(1)\n");
    for t in 0..8 {
        source.push_str(&format!(
            "track(\"g{t}\", s):steps(\"1@4 _ _ _  _ _ _ _  _ _ _ _  _ _ _ _\"):gain(4)\n\
               :grains({{ size = 60, density = 100, jitter = 30, spread = 2 }})\n"
        ));
    }
    let song = song(&source).unwrap();
    let audio = song.render(RATE, 2).unwrap();
    let peak = audio.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak <= 1.0, "pico {peak}");
    assert!(peak > 0.9, "tenía que actuar el limitador: {peak}");
}

#[test]
fn errors_say_the_line() {
    let cases = [
        ("grains({ size = 2 })", "size fuera de rango"),
        ("grains({ density = 1000 })", "density fuera de rango"),
        ("grains({ size = 500, density = 100 })", "32 granos"),
        (
            "grains({ pitch = ramp(0, 30, \"1 compás\") })",
            "pitch fuera de rango",
        ),
        ("grains({ tamano = 3 })", "clave desconocida"),
        ("grains({ size = \"grande\" })", "un número, ramp() o lfo()"),
        ("grains({ seed = -1 })", "seed es un número entero"),
        ("freeze(2)", "pos fuera de rango"),
        ("freeze(0.5, { speed = 1 })", "congelado no avanza"),
    ];
    for (call, message) in cases {
        let source = format!(
            "local s = sample(\"tono.wav\"):slice(1)\n\
             track(\"g\", s):steps(\"{BAR}\"):{call}"
        );
        let Err(err) = song(&source) else {
            panic!("{call}: tenía que fallar")
        };
        assert!(
            err.contains(":3:") || err.contains("línea 3"),
            "{call}: {err}"
        );
        assert!(err.contains(message), "{call}: {err}");
    }
    let Err(err) = song(
        "local i = sample(\"tono.wav\"):instrument()\n\
         track(\"i\", i):grains()",
    ) else {
        panic!("un instrumento con granos tenía que fallar")
    };
    assert!(err.contains("es un instrumento"), "{err}");
}
