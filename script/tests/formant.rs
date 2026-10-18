//! Formantes desde Lua: `pista:formant(n)` y `formant` en el instrumento,
//! que el script suena como el motor con lo mismo, el aviso si no hay
//! pasos con `%` y los errores con la línea.

use std::f64::consts::TAU;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{Pattern, Sample, Slices, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

/// `voz.wav`: medio segundo de una nota de 150 Hz con armónicos.
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("formant");
        std::fs::create_dir_all(&dir).unwrap();
        let mono: Vec<f32> = (0..RATE as usize / 2)
            .map(|i| {
                let t = i as f64 / RATE as f64;
                (1..12)
                    .map(|k| (TAU * 150.0 * k as f64 * t).sin() / k as f64)
                    .sum::<f64>() as f32
                    * 0.3
            })
            .collect();
        offline::write_wav(dir.join("voz.wav"), &mono, 1, RATE).unwrap();
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

const STEPS: &str = "1%7 . 1%-5 .  1 . . .  1%0 . . .  . . . .";

#[test]
fn formant_reaches_the_track_and_sounds_like_the_engine() {
    let song = song(&format!(
        "track(\"v\", sample(\"voz.wav\"):slice(1)):steps(\"{STEPS}\"):formant(0)"
    ))
    .unwrap();
    assert_eq!(song.tracks[0].formant, Some(0.0));
    assert!(song.warnings.is_empty(), "{:?}", song.warnings);
    let sample = Arc::new(Sample::load_wav(dir().join("voz.wav"), RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    let steps = engine::notation::parse_steps(STEPS).unwrap();
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_formant(Some(0.0))
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_track_pattern(0, Some(pattern)).unwrap();
    controller.start().unwrap();
    let direct = offline::render(&mut engine, engine::bar_frames(120.0, RATE) as usize, 2);
    let scripted = song.render(RATE, 2).unwrap();
    assert!(
        scripted[..direct.len()] == direct[..],
        "el script no suena como el motor"
    );
    // Y sin formant, otra cosa.
    let plain = self::song(&format!(
        "track(\"v\", sample(\"voz.wav\"):slice(1)):steps(\"{STEPS}\")"
    ))
    .unwrap();
    assert!(plain.render(RATE, 2).unwrap() != scripted);
}

#[test]
fn an_instrument_takes_formant_with_keep() {
    let song = song(
        "local v = sample(\"voz.wav\"):instrument({ keep = true, formant = -3 })\n\
         track(\"v\", v):melody(\"c . g .  . . . .  . . . .  . . . .\")",
    )
    .unwrap();
    let instrument = song.tracks[0].instrument.unwrap();
    assert_eq!(instrument.engine.formant, Some(-3.0));
    assert!(instrument.engine.keep);
}

#[test]
fn formant_without_percent_steps_warns() {
    let song = song("track(\"v\", sample(\"voz.wav\"):slice(1)):steps(\"1 . 1^7 .  . . . .  . . . .  . . . .\"):formant(0)")
        .unwrap();
    assert!(
        song.warnings
            .iter()
            .any(|w| w.contains("pista \"v\"") && w.contains("solo actúa en los pasos con %")),
        "{:?}",
        song.warnings
    );
}

#[test]
fn errors_say_the_line() {
    for (source, message) in [
        (
            "\ntrack(\"v\", sample(\"voz.wav\"):slice(1)):formant(30)",
            "fuera de rango",
        ),
        (
            "\nsample(\"voz.wav\"):instrument({ formant = 0 })",
            "formant necesita keep = true",
        ),
        (
            "\nsample(\"voz.wav\"):instrument({ keep = true, formant = \"alto\" })",
            "formant son semitonos",
        ),
        (
            "\ntrack(\"i\", sample(\"voz.wav\"):instrument({ keep = true })):formant(0)",
            "es un instrumento",
        ),
    ] {
        let Err(err) = song(source) else {
            panic!("{source}: tenía que fallar")
        };
        assert!(
            err.contains(":3:") || err.contains("línea 3"),
            "{source}: {err}"
        );
        assert!(err.contains(message), "{source}: {err}");
    }
}
