//! Huellas del audio: canciones que usan casi toda la API, renderizadas
//! offline con semilla fija. Las huellas se tomaron antes de la fase G
//! (longitud variable, envíos, automatización): sin usar lo nuevo, la
//! salida tiene que seguir siendo idéntica bit a bit. Las funciones de
//! `libm` (sin, exp, powf...) pueden diferir en el último bit entre
//! arquitecturas, así que las huellas solo se comprueban en x86_64; en las
//! demás, solo que el render es finito y determinista.

#[path = "../../engine/tests/support/synth_break.rs"]
mod synth;

use std::path::{Path, PathBuf};

use engine::{new_engine, offline};
use script::ScriptHost;

const RATE: u32 = 48_000;

fn samples() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("golden_samples");
    std::fs::create_dir_all(&dir).unwrap();
    let (audio, _) = synth::synth_break();
    offline::write_wav(dir.join("amen.wav"), &audio, 1, synth::RATE).unwrap();
    dir
}

/// FNV-1a de los bits de cada muestra.
fn fingerprint(audio: &[f32]) -> u64 {
    audio.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, s| {
        s.to_bits()
            .to_le_bytes()
            .iter()
            .fold(h, |h, &b| (h ^ b as u64).wrapping_mul(0x0100_0000_01b3))
    })
}

fn render(source: &str, bars: u64) -> Vec<f32> {
    let dir = samples();
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.set_seed(Some(7));
    let song = host
        .load(&mut controller, source, "golden.lua", &dir)
        .unwrap_or_else(|e| panic!("{e}"))
        .clone();
    song.render(RATE, bars).unwrap()
}

/// Canciones y sus huellas (tomadas con el código anterior a la fase G).
pub const SONGS: &[(&str, &str, u64)] = &[
    (
        "pasos",
        r#"bpm(170)
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 3' .  1^7 . 5@0.5 .  . 3'^-12@0.8 . x  2r4 . 6r2 8")
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . ."):gain(0.9)
"#,
        0xdc4a_c52d_6d92_6159,
    ),
    (
        "efectos",
        r#"bpm(140)
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("filter", { type = "lowpass", cutoff = 2500, q = 2 })
  :fx("bitcrush", { bits = 6, downsample = 2 })
  :gain(1.5)
track("hats", amen:slice_at(3))
  :steps(". x . x  . x . x  . x . x  . x . x")
  :fx("highpass", { cutoff = 3000 })
"#,
        0xa098_e6ab_432d_eb45,
    ),
    (
        "transformaciones",
        r#"bpm(160)
local amen = sample("amen.wav"):slice(8)
amen:name(1, "kick"):name(2, "snare")
track("drums", amen)
  :steps("kick . snare .  3 . kick .  5 6 snare .  7 . 8 .")
  :every(4, function(c) c:reverse() end)
  :every(3, function(c) c:steps(rotate("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .", 2)) end)
  :chance(0.3, "retrig", 4)
  :fill("roll", 4)
  :swing(62)
  :humanize(8, 0.2)
local escala = scale("minor", "D")
track("melodia", amen:slice_at(4))
  :steps(escala:melody("1 . 3 .  5 . 3 .  8 . 7 .  5 . 3 .", "x"))
  :gain(0.5)
track("euclid", amen):steps(euclid(5, 16, "2@0.6")):mute()
"#,
        0x4246_9dfa_0c66_db21,
    ),
];

#[test]
fn songs_render_like_before_phase_g() {
    let print = std::env::var_os("HUELLAS").is_some();
    for &(name, source, expected) in SONGS {
        let audio = render(source, 8);
        assert!(audio.iter().all(|s| s.is_finite()), "{name}");
        assert_eq!(
            fingerprint(&audio),
            fingerprint(&render(source, 8)),
            "{name}: no es determinista"
        );
        let got = fingerprint(&audio);
        if print {
            println!("{name}: {got:#x}");
            continue;
        }
        if cfg!(target_arch = "x86_64") {
            assert_eq!(got, expected, "{name}: el audio ha cambiado");
        }
    }
}
