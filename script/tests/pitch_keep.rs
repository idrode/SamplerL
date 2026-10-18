//! Pitch sin cambiar la duración desde Lua: `%` en los pasos y `keep` en
//! los instrumentos, con su aviso más allá de ±12 semitonos.

use std::f64::consts::TAU;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

/// `c4.wav`: un seno de medio segundo a 261,63 Hz.
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("pitch_keep");
        std::fs::create_dir_all(&dir).unwrap();
        let mono: Vec<f32> = (0..RATE as usize / 2)
            .map(|i| (0.5 * (TAU * 261.625_565 * i as f64 / RATE as f64).sin()) as f32)
            .collect();
        offline::write_wav(dir.join("c4.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!("bpm(60)\n{source}");
    host.load(&mut controller, &source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
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

fn duration(x: &[f32]) -> usize {
    x.iter().rposition(|v| v.abs() > 1e-3).map_or(0, |i| i + 1)
}

fn frequency(x: &[f32]) -> f64 {
    let crossings: Vec<f64> = (1..x.len())
        .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
        .map(|i| (i - 1) as f64 + (-x[i - 1] / (x[i] - x[i - 1])) as f64)
        .collect();
    RATE as f64 * (crossings.len() - 1) as f64 / (crossings[crossings.len() - 1] - crossings[0])
}

/// Duración (frames) y frecuencia de lo que suena.
fn measure(song: &Song) -> (usize, f64) {
    let out = first_second(song);
    let length = duration(&out);
    (length, frequency(&out[length / 5..length * 4 / 5]))
}

const HALF_SECOND: f64 = RATE as f64 / 2.0;
const G4: f64 = 391.995_436;

#[test]
fn percent_keeps_the_duration_of_a_slice() {
    let (length, hz) = measure(
        &song("track(\"t\", sample(\"c4.wav\"):slice(1)):steps(\"1%7 . . .  . . . .  . . . .  . . . .\")")
            .unwrap(),
    );
    assert!((length as f64 - HALF_SECOND).abs() < 48.0, "{length}");
    assert!((hz / G4 - 1.0).abs() < 0.005, "{hz}");
    // Con ^ dura 1/1,5 de lo que duraba.
    let (length, _) = measure(
        &song("track(\"t\", sample(\"c4.wav\"):slice(1)):steps(\"1^7 . . .  . . . .  . . . .  . . . .\")")
            .unwrap(),
    );
    assert!((length as f64 - HALF_SECOND / 2f64.powf(7.0 / 12.0)).abs() < 48.0);
}

#[test]
fn keep_keeps_the_duration_of_every_note_of_an_instrument() {
    let lead = |keep: bool| {
        song(&format!(
            "local i = sample(\"c4.wav\"):instrument({{ keep = {keep} }})\n\
             track(\"lead\", i):melody(\"g4 _ _ _  . . . .  . . . .  . . . .\")"
        ))
        .unwrap()
    };
    let (length, hz) = measure(&lead(true));
    assert!((length as f64 - HALF_SECOND).abs() < 48.0, "{length}");
    assert!((hz / G4 - 1.0).abs() < 0.005, "{hz}");
    let (length, hz) = measure(&lead(false));
    assert!((length as f64 - HALF_SECOND / 2f64.powf(7.0 / 12.0)).abs() < 48.0);
    assert!((hz / G4 - 1.0).abs() < 0.005, "{hz}");
}

#[test]
fn percent_zero_sounds_like_no_pitch() {
    let a = song("track(\"t\", sample(\"c4.wav\"):slice(1)):steps(\"1 . 1' .  . . . .  1@0.5 . . .  . . . .\")")
        .unwrap();
    let b = song("track(\"t\", sample(\"c4.wav\"):slice(1)):steps(\"1%0 . 1'%0 .  . . . .  1%0@0.5 . . .  . . . .\")")
        .unwrap();
    assert!(a.render(RATE, 2).unwrap() == b.render(RATE, 2).unwrap());
}

#[test]
fn transformations_keep_the_percent() {
    let song = song(
        "track(\"t\", sample(\"c4.wav\"):slice(1))\n\
           :steps(\"1%7 . . .  . . . .  . . . .  . . . .\")\n\
           :every(2, function(c) c:pitch(-5) end)",
    )
    .unwrap();
    let steps: Vec<_> = song.tracks[0].bars.iter().map(|b| b[0].unwrap()).collect();
    assert!(steps.iter().all(|s| s.keep));
    assert_eq!(
        steps.iter().map(|s| s.semitones).collect::<Vec<_>>(),
        [-5.0, 7.0]
    );
}

#[test]
fn far_transpositions_warn() {
    let far = song("track(\"t\", sample(\"c4.wav\"):slice(1)):steps(\"1%14 . . .  . . . .  . . . .  . . . .\")")
        .unwrap();
    assert!(
        far.warnings
            .iter()
            .any(|w| w.contains("pista \"t\"") && w.contains("14 semitonos")),
        "{:?}",
        far.warnings
    );
    // Con ^ no hay aviso: cambia la duración, sin granos.
    let classic = song("track(\"t\", sample(\"c4.wav\"):slice(1)):steps(\"1^14 . . .  . . . .  . . . .  . . . .\")")
        .unwrap();
    assert!(classic.warnings.is_empty(), "{:?}", classic.warnings);
    let lead = song(
        "local i = sample(\"c4.wav\"):instrument({ keep = true })\n\
         track(\"lead\", i):melody(\"c4 c6 . .  . . . .  . . . .  . . . .\")",
    )
    .unwrap();
    assert!(
        lead.warnings.iter().any(|w| w.contains("24 semitonos")),
        "{:?}",
        lead.warnings
    );
}

#[test]
fn errors_say_the_line() {
    for (source, message) in [
        (
            "\nsample(\"c4.wav\"):instrument({ keep = 1 })",
            "keep es true o false",
        ),
        (
            "\ntrack(\"t\", sample(\"c4.wav\"):slice(1)):steps(\"1^2%7 . . .  . . . .  . . . .  . . . .\")",
            "en vez del primer ^",
        ),
        (
            "\ntrack(\"t\", sample(\"c4.wav\"):slice(1)):steps(\"1%40 . . .  . . . .  . . . .  . . . .\")",
            "fuera de rango",
        ),
    ] {
        let Err(err) = song(source) else {
            panic!("{source}: tenía que fallar")
        };
        assert!(
            err.contains("línea 3") || err.contains(":3:"),
            "{source}: {err}"
        );
        assert!(err.contains(message), "{source}: {err}");
    }
}
