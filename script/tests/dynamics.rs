//! `:fx("compressor")` y `:fx("distortion")` suenan igual que en el motor.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{Fx, Pattern, Sample, Slices, new_engine, offline};
use script::ScriptHost;

const RATE: u32 = 48_000;
const STEPS: &str = "1 . 2 .  1 . 2 2  . 1 . 2  1 . . 2";

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("dynamics");
        std::fs::create_dir_all(&dir).unwrap();
        let mut mono = vec![0.0f32; 6000];
        mono[..800].fill(0.6);
        mono[3000..3800].fill(0.3);
        offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn script(fx: &str) -> Result<Vec<f32>, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!(
        "bpm(120)\nlocal b = sample(\"break.wav\"):slice(2)\ntrack(\"d\", b):steps(\"{STEPS}\"){fx}"
    );
    host.load(&mut controller, &source, "song.lua", &dir())
        .map(|s| s.render(RATE, 2).unwrap())
        .map_err(|e| e.to_string())
}

fn engine_with(chain: &[Fx]) -> Vec<f32> {
    let sample = Arc::new(Sample::load_wav(dir().join("break.wav"), RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    let steps = engine::notation::parse_steps(STEPS).unwrap();
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_fx(chain)
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, 2 * engine::bar_frames(120.0, RATE) as usize, 2)
}

#[test]
fn compressor_and_distortion_sound_like_the_engine() {
    let a = script(
        ":fx(\"compressor\", { threshold = -20, ratio = 6, attack = 2, release = 80, makeup = 4 })\n\
         :fx(\"drive\", { drive = 8, mix = 0.6 })",
    )
    .unwrap();
    let b = engine_with(&[
        Fx::Compressor {
            threshold_db: -20.0,
            ratio: 6.0,
            attack_ms: 2.0,
            release_ms: 80.0,
            makeup_db: 4.0,
        },
        Fx::distortion(8.0, 0.6),
    ]);
    assert_eq!(a, b);
    // Valores por defecto.
    let a = script(":fx(\"comp\"):fx(\"distortion\")").unwrap();
    assert_eq!(
        a,
        engine_with(&[Fx::compressor(-18.0, 4.0), Fx::distortion(4.0, 1.0)])
    );
}

#[test]
fn their_errors_point_at_the_line() {
    let error = |fx: &str| script(fx).unwrap_err();
    assert_eq!(
        error(":fx(\"compressor\", { ratio = 40 })"),
        "song.lua:3: fx(\"compressor\"): ratio fuera de rango (1 a 20)"
    );
    assert!(error(":fx(\"distortion\", { drive = 0 })").contains("drive fuera de rango (1 a 50)"));
    assert!(
        error(":fx(\"comp\", { knee = 2 })").contains("parámetro desconocido \"knee\""),
        "{}",
        error(":fx(\"comp\", { knee = 2 })")
    );
}
