//! `bpm_ramp()` y `meter()` en Lua: igual que el motor, la duración de la
//! canción, y los errores con línea.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{Batch, Meter, Pattern, Sample, Slices, TempoRamp, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("tempo");
        std::fs::create_dir_all(&dir).unwrap();
        let mut mono = vec![0.0f32; 6000];
        mono[..800].fill(0.5);
        mono[3000..3800].fill(0.25);
        offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!("local b = sample(\"break.wav\"):slice(2)\n{source}");
    host.load(&mut controller, &source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

#[test]
fn ramps_and_meters_sound_like_the_engine() {
    let steps = "1 . 2 .  1 2 . 2  . 1 . 2";
    let s = song(&format!(
        "bpm_ramp(110, 150, \"4 compases\")\nmeter(3, 4)\n\
         track(\"d\", b):length(12):steps(\"{steps}\")"
    ))
    .unwrap();
    let ramp = TempoRamp {
        from: 110.0,
        to: 150.0,
        bars: 4,
    };
    assert_eq!(s.tempo_ramp, Some(ramp));
    assert_eq!(s.meter, Meter::new(3, 4));
    assert_eq!(s.bar_steps(), 12);
    assert_eq!(s.bpm, Some(110.0));
    // Cinco compases: 110, 120, 130, 140 y 150, de 12 pasos.
    let frames: u64 = (0..5)
        .map(|k| engine::bar_frames_steps(12, ramp.bpm_at(k), RATE))
        .sum();
    assert_eq!(s.frames(RATE, 5), frames);

    let sample = Arc::new(Sample::load_wav(dir().join("break.wav"), RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    let pattern = Pattern::from_steps(
        sample,
        &slices,
        engine::notation::parse_pattern_labeled(steps, 12, &[]).unwrap(),
    )
    .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_tempo_ramp(ramp)
        .set_meter(Meter::new(3, 4).unwrap())
        .set_pattern(0, Some(pattern));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    assert_eq!(
        s.render(RATE, 5).unwrap(),
        offline::render(&mut engine, frames as usize, 2)
    );
    // La canción, en compases de 12: una pista de 12 dura un compás.
    assert_eq!(s.length_bars(1), 1);
}

#[test]
fn tempo_errors_point_at_the_line() {
    let error = |source: &str| song(source).err().expect("debería fallar");
    assert!(error("bpm_ramp(10, 150, \"4 compases\")").contains("song.lua:2: bpm_ramp"));
    assert!(error("bpm_ramp(120, 150, \"6 pasos\")").contains("la duración es \"N compases\""));
    assert_eq!(
        error("meter(3, 5)"),
        "song.lua:2: meter(3, 5): el denominador es 4, 8 o 16 y el compás, de 1 a 64 semicorcheas"
    );
}
