//! `pista:sidechain(origen, {...})`: el sidechain entre pistas desde Lua
//! suena igual que `Fx::Duck` en el motor, con el origen resuelto a su
//! ranura (aunque se defina después o esté en mute), con `ramp`, `lfo`,
//! macros y lanes en sus parámetros, y con errores claros (círculos,
//! pistas que no existen, congeladas, en un grupo, un corte o un paso).

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{DuckCurve, DuckParams, DuckTap, Fx, Pattern, Sample, Slices, new_engine, offline};
use script::ScriptHost;

const RATE: u32 = 48_000;
const KICK: &str = "x . . .  x . . .  x . . .  x . . .";
const ONE: &str = "1 . . .  . . . .  . . . .  . . . .";

/// `kick.wav` (100 Hz, 100 ms) y `bass.wav` (100 Hz, 2 s: un compás a
/// 120 BPM).
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("duck");
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
fn the_sidechain_sounds_like_the_engine() {
    let lua = render(&song(
        ":sidechain(\"kick\", { depth = -12, attack = 2, release = 90, curve = \"smooth\", band = 150 })",
    ));
    let params = DuckParams {
        depth_db: -12.0,
        attack_ms: 2.0,
        release_ms: 90.0,
        curve: DuckCurve::Smooth,
        band_hz: 150.0,
        ..DuckParams::new(0)
    };
    assert_eq!(lua, engine_render(&[Fx::Duck(params)]));
    assert_ne!(lua, engine_render(&[]));
    // Por audio, con sus valores por defecto; y `:fx("duck")` es lo mismo.
    let audio = render(&song(
        ":sidechain(\"kick\", { mode = \"audio\", tap = \"post\" })",
    ));
    let params = DuckParams {
        tap: DuckTap::Post,
        ..DuckParams::new(0).audio()
    };
    assert_eq!(audio, engine_render(&[Fx::Duck(params)]));
    assert_eq!(
        audio,
        render(&song(
            ":fx(\"duck\", { source = \"kick\", mode = \"audio\", tap = \"post\" })"
        ))
    );
}

#[test]
fn one_source_ducks_several_and_can_come_later_muted() {
    // Dos bajos por el mismo bombo, definido después y en mute.
    let song = |fx: &str| {
        format!(
            "bpm(120)\n\
             track(\"bass\", sample(\"bass.wav\"):slice_grid(1)):steps(\"{ONE}\"){fx}\n\
             track(\"pad\", sample(\"bass.wav\"):slice_grid(1)):steps(\"{ONE}\"){fx}\n\
             track(\"kick\", sample(\"kick.wav\"):slice_grid(1)):steps(\"{KICK}\"):mute()\n"
        )
    };
    let ducked = render(&song(
        ":sidechain(\"kick\", { depth = -12, release = 500 })",
    ));
    let plain = render(&song(""));
    let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
    // Durante el primer golpe (de 10 a 50 ms: a menos de una décima de la
    // relajación de 500 ms), los dos bajos unos 11,5 dB menos.
    let during = 20.0 * (rms(&ducked[480..2400]) / rms(&plain[480..2400])).log10();
    assert!((during + 11.5).abs() < 0.7, "{during}");
    // Con tap = "post", el bombo en mute no baja nada.
    let post = render(&song(
        ":sidechain(\"kick\", { depth = -12, release = 500, tap = \"post\" })",
    ));
    assert_eq!(post, plain);
}

#[test]
fn its_parameters_take_curves_macros_and_lanes() {
    let still = render(&song(":sidechain(\"kick\", { depth = -12 })"));
    for moving in [
        ":sidechain(\"kick\", { depth = ramp(-3, -18, \"1 compás\") })",
        ":sidechain(\"kick\", { release = lfo(\"4 pasos\", 40, 400) })",
        ":sidechain(\"kick\", { depth = -12 }):auto(\"depth\", { { 0, -2 }, { 1, -20 } })",
    ] {
        let out = render(&song(moving));
        assert_ne!(out, still, "{moving}");
    }
    // Una macro a 0 da su `desde`.
    let macro_ = render(&song(
        ":sidechain(\"kick\", { depth = macro(\"duck\", -12, -3) })",
    ));
    assert_eq!(macro_, still);
}

#[test]
fn its_errors_say_what_is_wrong() {
    let error = |fx: &str| load(&song(fx)).err().expect("debería fallar");
    let e = error(":sidechain(\"kik\")");
    assert!(
        e.contains("pista \"bass\": sidechain \"kik\" no es ninguna pista (hay: kick, bass)"),
        "{e}"
    );
    let e = error(":sidechain(\"bass\")");
    assert!(
        e.starts_with("song.lua:3: ") && e.contains("no puede dispararse a sí misma"),
        "{e}"
    );
    let e = error(":sidechain(1)");
    assert!(e.contains("el origen es el nombre de una pista"), "{e}");
    let e = error(":sidechain(\"kick\", { source = \"kick\" })");
    assert!(e.contains("source ya es el primer argumento"), "{e}");
    let e = error(":fx(\"duck\", { depth = -6 })");
    assert!(e.contains("falta source"), "{e}");
    let e = error(":sidechain(\"kick\", { depth = 3 })");
    assert!(e.contains("depth fuera de rango (-60 a 0 dB)"), "{e}");
    let e = error(":sidechain(\"kick\", { mode = \"loud\" })");
    assert!(
        e.contains("mode \"loud\" no existe (pattern o audio)"),
        "{e}"
    );
    let e = error(":sidechain(\"kick\", { filter = ramp(100, 200, \"1 compás\") })");
    assert!(e.contains("no admite ramp() ni lfo()"), "{e}");
    // En círculo: el bajo por el bombo y el bombo por el bajo (también
    // mezclado con un EQ dinámico).
    for back in [
        "track(\"kick\"):sidechain(\"bass\")\n",
        "track(\"kick\"):fx(\"dyneq\", { freq = 100, sidechain = \"bass\" })\n",
    ] {
        let e = load(&format!("{}{back}", song(":sidechain(\"kick\")")))
            .err()
            .expect("debería fallar");
        assert!(e.contains("en círculo"), "{e}");
    }
    // En un grupo, un corte o un paso, no.
    let e = load(&format!(
        "{}group(\"g\", {{ \"bass\" }}):fx(\"duck\", {{ source = \"kick\" }})\n",
        song("")
    ))
    .err()
    .expect("debería fallar");
    assert!(e.contains("no en los grupos"), "{e}");
    let e = load(
        "bpm(120)\n\
         track(\"kick\", sample(\"kick.wav\"):slice_grid(1)):steps(\"x{duck=-6} . . .  . . . .  . . . .  . . . .\")\n",
    )
    .err()
    .expect("debería fallar");
    assert!(e.contains("el sidechain no va en un paso"), "{e}");
}

#[test]
fn a_frozen_source_cannot_drive_a_pattern_sidechain() {
    // Congelar el bombo (su WAV) y pedir el sidechain por patrón: error;
    // por audio, escucha el WAV.
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let base = song("");
    let song_ = host
        .load(&mut controller, &base, "song.lua", &dir())
        .unwrap()
        .clone();
    let frozen = song_.render_frozen(RATE, "kick").unwrap();
    let wav = dir().join("kick.frozen.wav");
    offline::write_wav(&wav, &frozen.audio, 2, RATE).unwrap();
    let with = |fx: &str| {
        format!(
            "{}frozen(\"kick\", \"kick.frozen.wav\", \"{}\")\n",
            song(fx),
            frozen.fingerprint
        )
    };
    let e = load(&with(":sidechain(\"kick\")"))
        .err()
        .expect("debería fallar");
    assert!(e.contains("está congelada"), "{e}");
    let audio = load(&with(
        ":sidechain(\"kick\", { mode = \"audio\", threshold = -40 })",
    ));
    assert!(audio.is_ok(), "{:?}", audio.err());
}
