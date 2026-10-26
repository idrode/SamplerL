//! Arrancar una canción de Lua en un compás (`Controller::start_at`, el
//! cue y `--from`): las curvas (`ramp`, `lfo`) y las vueltas (`every`)
//! siguen por donde irían si la canción hubiera llegado hasta él.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{new_engine, offline};
use script::ScriptHost;

const RATE: u32 = 48_000;
/// A 120 BPM, un compás.
const BAR: usize = 96_000;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("start_at");
        std::fs::create_dir_all(&dir).unwrap();
        // Continua larga: el nivel que se oye es el volumen de la pista.
        offline::write_wav(dir.join("dc.wav"), &vec![0.25f32; 40_000], 1, RATE).unwrap();
        dir
    })
    .clone()
}

/// Renderiza `bars` compases de `source` arrancando en el compás `from`.
fn render(source: &str, from: Option<u64>, bars: usize) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(&mut controller, source, "song.lua", &dir())
        .unwrap_or_else(|e| panic!("{e}"));
    match from {
        Some(bar) => controller.start_at(bar).unwrap(),
        None => controller.start().unwrap(),
    }
    offline::render(&mut engine, bars * BAR, 2)
}

/// Nivel (canal izquierdo) a `offset` frames del compás `bar`.
fn level(audio: &[f32], bar: usize, offset: usize) -> f32 {
    audio[2 * (bar * BAR + offset)]
}

const RAMP: &str = r#"bpm(120)
track("dc", sample("dc.wav"):slice_grid(1))
  :steps("1 . . .  . . . .  . . . .  . . . .")
  :gain(ramp(0.1, 1, "8 compases"))
"#;

#[test]
fn a_ramp_goes_on_from_the_bar_where_it_starts() {
    let whole = render(RAMP, None, 7);
    let cued = render(RAMP, Some(5), 2);
    // A media nota (el volumen ya se ha asentado), lo mismo que llegando.
    for (bar, from_cue) in [(5, 0), (6, 1)] {
        let (a, b) = (level(&whole, bar, 20_000), level(&cued, from_cue, 20_000));
        assert!((a - b).abs() < 1e-3, "compás {bar}: {a} y {b}");
    }
    assert!(level(&cued, 0, 20_000) > level(&whole, 1, 20_000));
}

#[test]
fn a_tempo_ramp_and_every_count_the_bars_before() {
    let source = r#"bpm_ramp(100, 180, "8 compases")
track("dc", sample("dc.wav"):slice_grid(1))
  :steps("1 . . .  . . . .  . . . .  . . . .")
  :every(3, function(t) t:gain(0.5) end)
"#;
    let (mut controller, mut engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let song = host
        .load(&mut controller, source, "song.lua", &dir())
        .unwrap_or_else(|e| panic!("{e}"))
        .clone();
    controller.start_at(4).unwrap();
    offline::render(&mut engine, 512, 2);
    assert_eq!(controller.status().bpm, song.bpm_at(4));

    // La vuelta de `every`: la misma que en el compás 4 llegando a él.
    let whole = {
        let (mut controller, mut engine) = new_engine(RATE);
        let mut host = ScriptHost::new(RATE);
        host.load(&mut controller, source, "song.lua", &dir())
            .unwrap();
        controller.start().unwrap();
        let frames = song.frames(RATE, 5) as usize;
        offline::render(&mut engine, frames, 2)
    };
    let bar4 = song.frames(RATE, 4) as usize;
    let mut audio = vec![0.0f32; 2 * 20_000];
    engine.process(&mut audio, 2);
    let (a, b) = (whole[2 * (bar4 + 10_000)], audio[2 * (10_000 - 512)]);
    assert!((a - b).abs() < 1e-3, "{a} y {b}");
}
