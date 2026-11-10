//! Huellas del audio: canciones que usan casi toda la API, renderizadas
//! offline con semilla fija. Las huellas se tomaron antes de la fase G
//! (longitud variable, envíos, automatización): sin usar lo nuevo, la
//! salida tiene que seguir siendo idéntica bit a bit. La fase K cambió la
//! interpolación del pitch (sinc en vez de lineal): las canciones con pasos
//! con pitch tienen huellas nuevas, y sus versiones sin pitch conservan las
//! de antes (lo único que cambia es el pitch). Las funciones de
//! `libm` (sin, exp, powf...) pueden diferir en el último bit entre
//! arquitecturas, así que las huellas solo se comprueban en x86_64; en las
//! demás, solo que el render es finito y determinista.

#[path = "../../engine/tests/support/synth_break.rs"]
mod synth;

use std::path::{Path, PathBuf};

use engine::{new_engine, offline};
use script::ScriptHost;

const RATE: u32 = 48_000;

/// La carpeta con `amen.wav`, escrito una sola vez (los dos tests lo leen
/// a la vez).
fn samples() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("golden_samples");
        std::fs::create_dir_all(&dir).unwrap();
        let (audio, _) = synth::synth_break();
        offline::write_wav(dir.join("amen.wav"), &audio, 1, synth::RATE).unwrap();
        dir
    })
    .clone()
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

/// Canciones y sus huellas (tomadas con el código anterior a la fase G;
/// las de «pasos» y «transformaciones», que usan pitch, con la
/// interpolación sinc de la fase K).
pub const SONGS: &[(&str, &str, u64)] = &[
    (
        "pasos",
        r#"bpm(170)
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 3' .  1^7 . 5@0.5 .  . 3'^-12@0.8 . x  2r4 . 6r2 8")
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . ."):gain(0.9)
"#,
        0x25c2_e62a_8333_85f9,
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
        0x60b1_05f0_5d98_5c71,
    ),
    // Las dos canciones con pitch, sin él: huellas tomadas con la
    // interpolación lineal de antes de la fase K.
    (
        "pasos sin pitch",
        r#"bpm(170)
local amen = sample("amen.wav"):slice(8)
track("drums", amen):steps("1 . 3' .  1 . 5@0.5 .  . 3'@0.8 . x  2r4 . 6r2 8")
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . ."):gain(0.9)
"#,
        0x4484_3a52_5f51_1679,
    ),
    (
        "transformaciones sin pitch",
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
  :steps("x . x .  x . x .  x . x .  x . x .")
  :gain(0.5)
track("euclid", amen):steps(euclid(5, 16, "2@0.6")):mute()
"#,
        0x994c_ee47_2e81_06e1,
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

/// Huellas de lo que las cinco de [`SONGS`] no tocan (N10 de
/// `docs/AUDITORIA-2.md`): los sintetizadores (sub con un EQ dinámico en
/// sidechain, subharm y sh101), los buses con sus efectos, las lanes y los
/// grupos con macros. Las canciones son copias fijas (basadas en las
/// recetas 10 a 22, que pueden cambiar). Como las de antes, las huellas
/// solo se comprueban en x86_64 (ver arriba); en ARM64,
/// `scripts/comparar-audio.sh`. Se imprimen con `HUELLAS=1 cargo test -p
/// script --test golden -- --nocapture`.
pub const NEWER: &[(&str, &str, u64)] = &[
    (
        "sintetizadores",
        r#"bpm(124)
local amen = sample("amen.wav"):slice(8)
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . ."):gain(0.5)
track("sub", synth("sub", { tone = 200, drive = 0.2, release = 120 }))
  :melody("c _ _ _  _ _ _ _  ab0 _ _ _  bb0! _ g0/ _")
  :fx("dyneq", { freq = 60, q = 0.7, range = 8, threshold = -30,
                 attack = 2, release = 90, sidechain = "kick" })
  :gain(0.6)
track("sh", subharm{
  o1 = { div = 1, rdiv = 4, pan = -0.5, level = 0.8 },
  s1 = { div = 3, rdiv = 3, pan = 0.5, level = 0.7 },
  s2 = { div = 4, rdiv = 1, level = 0.5 },
  cutoff = 1100, res = 0.45, envmod = 0.3, decay = 350,
}):melody("c2 _ _ _  _ _ _ _  _ _ _ _  _ _ _ _"):gain(0.5)
track("bass", sh101{
  saw = 0.8, pulse = 0.3, pw = 0.4, sub = 0.6, sub_div = 2,
  cutoff = 500, res = 0.7, env_amt = 0.5,
  a = 2, d = 250, s = 0, r = 100,
  glide = 80, glide_mode = "auto",
}):melody("c . c c/  eb . c . g1 . bb1 c/  . c eb f"):gain(0.6)
"#,
        0x04d1_a737_7134_de20,
    ),
    (
        "buses",
        r#"bpm(170)
bus("delay", { time = "3/16", feedback = 0.45 })
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("compressor", { threshold = -20, ratio = 4, attack = 5, makeup = 3 })
  :fx("distortion", { drive = 3, mix = 0.4 })
  :send("reverb", 0.3)
  :send("delay", 0.25)
track("caja", amen:slice_at(3))
  :steps(". . . .  x . . .  . . . .  x . . x")
  :fx("eq", { type1 = "lowshelf", freq1 = 120, gain1 = -6, freq2 = 1800, gain2 = 4, q2 = 1.2 })
  :send("reverb", 0.6)
"#,
        0xe261_1714_4fc6_0672,
    ),
    (
        "lanes",
        r#"bpm(170)
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("lowpass", { cutoff = 3000, q = 2 })
  :auto("cutoff", { {0, 200, "exp"}, {3, 9000, "exp"}, {4, 200} }, { len = "4 compases" })
  :auto("pan", { {0, -0.7, "smooth"}, {1, 0.7, "smooth"}, {2, -0.7} }, { len = 2 })
  :auto("gain", { {0, 0.3, "step"}, {0.25, 1, "step"}, {0.5, 0.5, "step"}, {0.75, 1, "step"} }, { len = 1 })
"#,
        0xc763_c06b_aed8_3843,
    ),
    (
        "grupos y macros",
        r#"bpm(170)
macro_init("tension", 0.4)
bus("delay", { time = "3/16", feedback = macro("tension", 0.2, 0.7) })
local amen = sample("amen.wav"):slice(8)
track("kick", amen:slice_at(1)):steps("x . . .  . . x .  . . . .  . x . .")
track("caja", amen:slice_at(3)):steps(". . . .  x . . .  . . . .  x . . x")
track("hats", amen:slice_at(6)):steps("x x x x  x x x x  x x x x  x x x x"):gain(0.4)
group("bateria", { "kick", "caja", "hats" })
  :fx("compressor", { threshold = -22, ratio = 4, attack = 10, makeup = 4 })
  :fx("distortion", { drive = 2, mix = 0.3 })
  :send("reverb", 0.1)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("lowpass", { cutoff = macro("tension", 600, 12000), q = 1.5 })
  :send("reverb", macro("tension", 0, 0.35))
  :send("delay", 0.2)
"#,
        0x5af8_f6c5_0fae_ba33,
    ),
];

#[test]
fn newer_features_render_like_when_fingerprinted() {
    let print = std::env::var_os("HUELLAS").is_some();
    for &(name, source, expected) in NEWER {
        let audio = render(source, 8);
        assert!(audio.iter().all(|s| s.is_finite()), "{name}");
        assert!(audio.iter().any(|s| s.abs() > 0.01), "{name}: no suena");
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
