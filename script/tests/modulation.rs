//! Flanger, phaser y chorus desde Lua: `:fx("flanger", {...})` hace el
//! efecto del motor con sus valores por defecto y los escritos (`sync` en
//! figuras, compases, pasos o ciclos por compás; `wave`, `invert`), con
//! `ramp()`/`lfo()`/`macro()` en lo continuo; el phaser va también en un
//! corte y en las llaves de un paso, el flanger y el chorus no; y cada
//! error dice su línea mientras sigue sonando lo anterior.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{Controller, Engine, Fx, LfoWave, Sync, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;
const BAR: usize = 16 * 6000;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("modulation");
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = 0x1234_5678u32;
        let mono: Vec<f32> = (0..4 * 6000)
            .map(|i| {
                state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                if i % 6000 < 4000 {
                    0.4 * ((state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0)
                } else {
                    0.0
                }
            })
            .collect();
        offline::write_wav(dir.join("ruido.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

const PRE: &str = "bpm(120)\nlocal a = sample(\"ruido.wav\"):slice_grid(4)\n";
const STEPS: &str = "1 . 2 .  3 . . .  4 . 1 .  . . 2 .";

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(
        &mut controller,
        &format!("{PRE}{source}"),
        "song.lua",
        &dir(),
    )
    .cloned()
    .map_err(|e| e.to_string())
}

fn fx_of(source: &str) -> Fx {
    song(&format!(
        "track(\"d\", a):steps(\"{STEPS}\"):fx({source})\n"
    ))
    .unwrap_or_else(|e| panic!("{e}"))
    .tracks[0]
        .fx[0]
}

#[test]
fn the_tables_make_the_engine_effects() {
    let Fx::Flanger(p) = fx_of("\"flanger\", {}") else {
        panic!()
    };
    assert_eq!(p, engine::FlangerParams::DEFAULT);
    let Fx::Flanger(p) = fx_of(
        "\"flanger\", { rate = 3, depth = 4, delay = 0.5, feedback = -0.9, mix = 1, wave = \"tri\", phase = 0, invert = true }",
    ) else {
        panic!()
    };
    assert_eq!(
        (p.lfo.rate_hz, p.depth_ms, p.delay_ms, p.feedback, p.mix),
        (3.0, 4.0, 0.5, -0.9, 1.0)
    );
    assert_eq!(
        (p.lfo.wave, p.lfo.phase_deg, p.invert),
        (LfoWave::Triangle, 0.0, true)
    );
    let Fx::Phaser(p) = fx_of("\"phaser\", { stages = 12, center = 2000, depth = 3 }") else {
        panic!()
    };
    assert_eq!((p.stages, p.center_hz, p.depth_oct), (12, 2000.0, 3.0));
    let Fx::Chorus(p) = fx_of("\"chorus\", { voices = 4, delay = 25 }") else {
        panic!()
    };
    assert_eq!((p.voices, p.delay_ms), (4, 25.0));
}

#[test]
fn sync_takes_figures_bars_steps_and_cycles_per_bar() {
    let sync = |text: &str| {
        let Fx::Flanger(p) = fx_of(&format!("\"flanger\", {{ sync = {text} }}")) else {
            panic!()
        };
        p.lfo.sync.unwrap()
    };
    assert_eq!(sync("\"1/4\""), Sync { num: 1, den: 4 });
    assert_eq!(sync("\"3/16\""), Sync { num: 3, den: 16 });
    assert_eq!(sync("\"2 compases\""), Sync { num: 2, den: 1 });
    assert_eq!(sync("\"8 pasos\""), Sync { num: 8, den: 16 });
    assert_eq!(sync("4"), Sync { num: 1, den: 4 });
    assert_eq!(sync("4").steps(), 4.0);
}

#[test]
fn every_continuous_parameter_takes_curves() {
    let src = format!(
        "macro_init(\"m\", 0.5)\ntrack(\"d\", a):steps(\"{STEPS}\")\n\
         :fx(\"flanger\", {{ rate = lfo(1, 0.1, 4), depth = ramp(0, 5, \"1 compás\"), delay = macro(\"m\", 1, 5), feedback = lfo(2, -0.5, 0.5), mix = ramp(0, 1, \"2 compases\"), phase = lfo(0.5, 0, 180) }})\n\
         :fx(\"phaser\", {{ center = lfo(\"1 compás\", 200, 4000) }})\n\
         :fx(\"chorus\", {{ mix = macro(\"m\", 0, 1) }})\n"
    );
    let song = song(&src).unwrap();
    // Las 8 curvas que admite una pista.
    assert_eq!(song.tracks[0].automation.len(), 6 + 1 + 1);
    let audio = song.render(RATE, 2).unwrap();
    assert!(audio.iter().all(|x| x.is_finite()));
    assert!(audio.iter().any(|x| x.abs() > 0.01));
}

#[test]
fn the_phaser_goes_in_a_slice_and_a_step_and_mix_zero_changes_nothing() {
    let plain = song(&format!("track(\"d\", a):steps(\"{STEPS}\")\n"))
        .unwrap()
        .render(RATE, 1)
        .unwrap();
    // Por corte y por paso, con la mezcla a cero: bit a bit lo de antes.
    let zero = song(
        "a:fx(2, \"phaser\", { mix = 0 })\ntrack(\"d\", a):steps(\"1 . 2 .  3{phaser=0} . . .  4 . 1 .  . . 2 .\")\n",
    )
    .unwrap()
    .render(RATE, 1)
    .unwrap();
    assert!(
        plain
            .iter()
            .zip(&zero)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
    // Con mezcla, solo cambian los golpes del corte 2 y el del paso 3.
    let wet = song(
        "a:fx(2, \"phaser\", { mix = 0.5, depth = 0, center = 1000 })\ntrack(\"d\", a):steps(\"1 . 2 .  3{phaser=0.5,stages=2} . . .  4 . 1 .  . . 2 .\")\n",
    )
    .unwrap()
    .render(RATE, 1)
    .unwrap();
    let step = |k: usize| 2 * 6000 * k..2 * 6000 * (k + 1);
    for k in [0, 8, 10] {
        assert_eq!(plain[step(k)], wet[step(k)], "paso {k}");
    }
    for k in [2, 4, 14] {
        assert_ne!(plain[step(k)], wet[step(k)], "paso {k}");
    }
}

fn load(host: &mut ScriptHost, c: &mut Controller, src: &str) -> Result<(), String> {
    host.load(c, src, "song.lua", &dir())
        .map(|_| ())
        .map_err(|e| e.message)
}

fn render_left(engine: &mut Engine, frames: usize) -> Vec<f32> {
    offline::render(engine, frames, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

#[test]
fn errors_say_the_line_and_the_old_song_keeps_playing() {
    let good = format!(
        "{PRE}track(\"d\", a):steps(\"{STEPS}\"):fx(\"flanger\", {{ sync = \"1/4\", feedback = 0.7 }})\n"
    );
    let (mut controller, mut engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    load(&mut host, &mut controller, &good).unwrap();
    controller.start().unwrap();
    let mut heard = render_left(&mut engine, BAR + 777);

    let cases: &[(&str, &[&str])] = &[
        (
            "t:fx(\"flanger\", { feedback = 0.99 })",
            &["song.lua:4:", "feedback fuera de rango (-0.95 a 0.95)"],
        ),
        (
            "t:fx(\"flanger\", { mix = 0/0 })",
            &["song.lua:4:", "mix fuera de rango"],
        ),
        (
            "t:fx(\"phaser\", { rate = math.huge })",
            &["song.lua:4:", "rate fuera de rango (0.01 a 20 Hz)"],
        ),
        (
            "t:fx(\"phaser\", { stages = 5 })",
            &["song.lua:4:", "stages: 2, 4, 6, 8 o 12"],
        ),
        (
            "t:fx(\"chorus\", { voices = 2.5 })",
            &["song.lua:4:", "voices es un número entero"],
        ),
        (
            "t:fx(\"flanger\", { speed = 2 })",
            &["song.lua:4:", "\"speed\""],
        ),
        (
            "t:fx(\"flanger\", { sync = \"1/0\" })",
            &["song.lua:4:", "sync fuera de rango"],
        ),
        (
            "t:fx(\"flanger\", { sync = \"tres\" })",
            &["song.lua:4:", "sync es un número de ciclos por compás"],
        ),
        (
            "t:fx(\"flanger\", { sync = lfo(1, 1, 4) })",
            &["song.lua:4:", "sync no admite ramp() ni lfo()"],
        ),
        (
            "t:fx(\"flanger\", { wave = \"square\" })",
            &["song.lua:4:", "wave \"square\" no existe (sine o tri)"],
        ),
        (
            "t:fx(\"flanger\", { invert = \"sí\" })",
            &["song.lua:4:", "invert es true o false"],
        ),
        (
            "t:fx(\"phaser\", { stages = lfo(1, 2, 12) })",
            &["song.lua:4:", "stages no admite ramp() ni lfo()"],
        ),
        (
            "t:fx(\"chorus\", { delay = ramp(5, 20, \"1 compás\") })",
            &["song.lua:4:", "delay fuera de rango (10 a 30 ms)"],
        ),
        (
            "t:steps(\"1{flanger=0.5} . . .  . . . .  . . . .  . . . .\")",
            &["song.lua:4:", "{flanger}: el flanger no va en un paso"],
        ),
        (
            "a:fx(2, \"chorus\", {})",
            &["song.lua:4:", "van en la pista o el grupo"],
        ),
    ];
    for (line, fragments) in cases {
        let src = format!("{PRE}local t = track(\"d\", a):steps(\"{STEPS}\")\n{line}\n");
        let err = load(&mut host, &mut controller, &src).unwrap_err();
        for fragment in *fragments {
            assert!(
                err.contains(fragment),
                "error \"{err}\" sin \"{fragment}\"\n{src}"
            );
        }
    }
    heard.extend(render_left(&mut engine, BAR - 777));

    // Igual que si no se hubiera intentado ninguna recarga.
    let (mut c2, mut e2) = new_engine(RATE);
    let mut h2 = ScriptHost::new(RATE);
    load(&mut h2, &mut c2, &good).unwrap();
    c2.start().unwrap();
    assert_eq!(heard, render_left(&mut e2, 2 * BAR));
}
