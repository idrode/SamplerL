//! `cortes:set(i, { play = "loop", crossfade = ms })` desde Lua: por defecto
//! (10 ms) el salto del bucle no tiene discontinuidad; con 0, el salto seco
//! de siempre; y se avisa si el corte no tiene nada fuera para el fundido.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

/// `seno.wav`: 4000 frames de un seno de 444 Hz a 0,5 (cada cuarto son
/// 9,25 periodos: el salto de un bucle cae entre un cero y una cresta).
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("loop_crossfade");
        std::fs::create_dir_all(&dir).unwrap();
        let sine: Vec<f32> = (0..4000)
            .map(|i| 0.5 * (std::f32::consts::TAU * 444.0 * i as f32 / RATE as f32).sin())
            .collect();
        offline::write_wav(dir.join("seno.wav"), &sine, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Song {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(
        &mut controller,
        &format!("bpm(120)\n{source}"),
        "song.lua",
        &dir(),
    )
    .cloned()
    .unwrap()
}

/// El mayor salto de una muestra a la siguiente en el primer paso (canal
/// izquierdo), pasado el ataque.
fn max_jump(song: &Song) -> f32 {
    let left: Vec<f32> = song
        .render(RATE, 1)
        .unwrap()
        .iter()
        .step_by(2)
        .copied()
        .collect();
    left[100..6000]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

const STEP: f32 = 0.5 * std::f32::consts::TAU * 444.0 / RATE as f32;

#[test]
fn the_default_crossfade_removes_the_jump_and_zero_keeps_it() {
    let looped = |extra: &str| {
        song(&format!(
            "local s = sample(\"seno.wav\"):slice_grid(4)\n\
             s:set(2, {{ play = \"loop\"{extra} }})\n\
             track(\"l\", s):steps(\"2 . . .  . . . .  . . . .  . . . .\")\n"
        ))
    };
    let smooth = looped("");
    assert!(smooth.warnings.is_empty(), "{:?}", smooth.warnings);
    assert_eq!(smooth.tracks[0].shapes[1].crossfade_ms, 10.0);
    assert!(max_jump(&smooth) < 1.2 * STEP, "{}", max_jump(&smooth));
    assert!(max_jump(&looped(", crossfade = 0")) > 10.0 * STEP);
    // Más largo, también sin salto.
    assert!(max_jump(&looped(", crossfade = 20")) < 1.2 * STEP);
}

#[test]
fn a_loop_without_room_outside_the_slice_warns() {
    let song = song(
        "local s = sample(\"seno.wav\"):slice_grid(1)\n\
         s:set(1, { play = \"loop\" })\n\
         track(\"l\", s):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n",
    );
    assert!(
        song.warnings.iter().any(|w| w.contains("\"l\"")
            && w.contains("el bucle del corte 1")
            && w.contains("seco")),
        "{:?}",
        song.warnings
    );
    // Sin fundido pedido, o sin bucle, no se avisa.
    for set in [
        "{ play = \"loop\", crossfade = 0 }",
        "{ play = \"pingpong\" }",
    ] {
        let quiet = self::song(&format!(
            "local s = sample(\"seno.wav\"):slice_grid(1)\n\
             s:set(1, {set})\n\
             track(\"l\", s):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n"
        ));
        assert!(quiet.warnings.is_empty(), "{set}: {:?}", quiet.warnings);
    }
}
