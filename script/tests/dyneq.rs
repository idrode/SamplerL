//! `:fx("dyneq", {...})`: el EQ dinámico desde Lua suena igual que en el
//! motor, con `sidechain = "pista"` resuelto a su ranura (aunque la pista se
//! defina después o esté en mute), con `ramp`/`lfo` y con errores claros.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{DynEqParams, Fx, Pattern, Sample, Slices, new_engine, offline};
use script::ScriptHost;

const RATE: u32 = 48_000;
const KICK: &str = "x . . .  x . . .  x . . .  x . . .";
const ONE: &str = "1 . . .  . . . .  . . . .  . . . .";

/// `kick.wav` (100 Hz, 100 ms) y `bass.wav` (100 Hz, 2 s: un compás a
/// 120 BPM).
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("dyneq");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, amp, frames) in [("kick", 0.8, 4800), ("bass", 0.3, 96_000)] {
            let mono: Vec<f32> = (0..frames)
                .map(|i| {
                    let env = (i as f32 / 240.0).min(1.0).min((frames - i) as f32 / 240.0);
                    env * amp * (std::f32::consts::TAU * 100.0 * i as f32 / RATE as f32).sin()
                })
                .collect();
            offline::write_wav(dir.join(format!("{name}.wav")), &mono, 1, RATE).unwrap();
        }
        dir
    })
    .clone()
}

fn load(source: &str) -> Result<script::Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(&mut controller, source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

fn render(source: &str) -> Vec<f32> {
    load(source).unwrap().render(RATE, 1).unwrap()
}

fn song(bass_fx: &str) -> String {
    format!(
        "bpm(120)\n\
         track(\"kick\", sample(\"kick.wav\"):slice_grid(1)):steps(\"{KICK}\")\n\
         track(\"bass\", sample(\"bass.wav\"):slice_grid(1)):steps(\"{ONE}\"){bass_fx}\n"
    )
}

/// Lo mismo directamente en el motor: el bombo en la ranura 0 y el bajo en
/// la 1.
fn engine_render(bass_fx: &[Fx]) -> Vec<f32> {
    let load = |name: &str| {
        let sample = Arc::new(Sample::load_wav(dir().join(format!("{name}.wav")), RATE).unwrap());
        let slices = Slices::grid(&sample, 1);
        (sample, slices)
    };
    let (kick, kick_slices) = load("kick");
    let (bass, bass_slices) = load("bass");
    let steps = |text: &str| engine::notation::parse_steps(text).unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller
        .set_track_pattern(
            0,
            Some(Pattern::from_steps(kick, &kick_slices, steps(KICK)).unwrap()),
        )
        .unwrap();
    controller
        .set_track_pattern(
            1,
            Some(
                Pattern::from_steps(bass, &bass_slices, steps(ONE))
                    .unwrap()
                    .with_fx(bass_fx)
                    .unwrap(),
            ),
        )
        .unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, engine::bar_frames(120.0, RATE) as usize, 2)
}

#[test]
fn the_dynamic_eq_sounds_like_the_engine() {
    let lua = render(&song(
        ":fx(\"dyneq\", { freq = 100, threshold = -30, ratio = 20, range = 6, attack = 2, release = 60, sidechain = \"kick\" })",
    ));
    let params = DynEqParams {
        threshold_db: -30.0,
        ratio: 20.0,
        range_db: 6.0,
        attack_ms: 2.0,
        release_ms: 60.0,
        ..DynEqParams::new(100.0)
    };
    assert_eq!(lua, engine_render(&[Fx::DynEq(params.with_sidechain(0))]));
    assert_ne!(lua, engine_render(&[]));
    // Sin sidechain escucha la propia pista; valores por defecto.
    assert_eq!(
        render(&song(":fx(\"dyneq\", { freq = 100 })")),
        engine_render(&[Fx::DynEq(DynEqParams::new(100.0))])
    );
}

#[test]
fn the_key_can_come_later_and_be_muted() {
    // El bajo escucha al bombo, definido después y en mute (el «ghost
    // kick»): no se oye, pero sigue mandando.
    let ghost = |fx: &str| {
        format!(
            "bpm(120)\n\
             track(\"bass\", sample(\"bass.wav\"):slice_grid(1)):steps(\"{ONE}\"){fx}\n\
             track(\"kick\", sample(\"kick.wav\"):slice_grid(1)):steps(\"{KICK}\"):mute()\n"
        )
    };
    let ducked = render(&ghost(
        ":fx(\"dyneq\", { freq = 100, range = 6, sidechain = \"kick\" })",
    ));
    let plain = render(&ghost(""));
    let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
    // Durante el primer golpe (de 20 a 70 ms), más bajo.
    let during = 20.0 * (rms(&ducked[1920..6720]) / rms(&plain[1920..6720])).log10();
    assert!(during < -3.0, "{during}");
}

#[test]
fn its_parameters_take_ramp_and_lfo() {
    let moving = render(&song(
        ":fx(\"dyneq\", { freq = 100, threshold = ramp(-40, -10, \"1 compás\"), sidechain = \"kick\" })",
    ));
    let still = render(&song(
        ":fx(\"dyneq\", { freq = 100, threshold = -40, sidechain = \"kick\" })",
    ));
    assert_ne!(moving, still);
}

#[test]
fn its_errors_say_what_is_wrong() {
    let error = |fx: &str| load(&song(fx)).err().expect("debería fallar");
    let e = error(":fx(\"dyneq\", { freq = 100, sidechain = \"kik\" })");
    assert!(
        e.contains(
            "pista \"bass\": fx(\"dyneq\"): sidechain \"kik\" no es ninguna pista (hay: kick, bass)"
        ),
        "{e}"
    );
    let e = error(":fx(\"dyneq\", { freq = 100, sidechain = \"bass\" })");
    assert!(
        e.starts_with("song.lua:3: ") && e.contains("no puede escucharse a sí misma"),
        "{e}"
    );
    let e = error(":fx(\"dyneq\", { freq = 100, sidechain = 1 })");
    assert!(e.contains("sidechain es el nombre de una pista"), "{e}");
    let e = error(":fx(\"dyneq\", { range = 6 })");
    assert!(e.contains("falta freq"), "{e}");
    let e = error(":fx(\"dyneq\", { freq = 100, range = 30 })");
    assert!(e.contains("range fuera de rango (0 a 24 dB)"), "{e}");
    // En círculo: el bajo escucha al bombo y el bombo al bajo.
    let e = load(&format!(
        "{}track(\"kick\"):fx(\"dyneq\", {{ freq = 100, sidechain = \"bass\" }})\n",
        song(":fx(\"dyneq\", { freq = 100, sidechain = \"kick\" })")
    ))
    .err()
    .expect("debería fallar");
    assert!(e.contains("sidechain en círculo"), "{e}");
}
