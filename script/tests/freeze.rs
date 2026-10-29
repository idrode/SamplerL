//! Congelar una pista (`Song::render_frozen` y `frozen()`): la canción con
//! la pista congelada suena bit a bit como la original desde la segunda
//! vuelta (en la primera, lo congelado ya lleva las colas de la vuelta
//! anterior), también con efectos y curvas, sidechain hacia ella y desde
//! ella, envíos, volumen y pan, y en un sintetizador; avisos y errores.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

#[path = "../../engine/tests/support/synth_break.rs"]
mod synth;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("freeze");
        std::fs::create_dir_all(dir.join("congelado")).unwrap();
        let (audio, _) = synth::synth_break();
        offline::write_wav(dir.join("amen.wav"), &audio, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.set_seed(Some(3));
    host.load(&mut controller, source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

/// Congela `name` de `source` en `wav` y devuelve el script con `frozen()`
/// al final, y la vuelta en compases.
fn freeze(source: &str, name: &str, wav: &str) -> (String, u64) {
    let original = song(source).unwrap();
    let frozen = original.render_frozen(RATE, name).unwrap();
    assert!(frozen.warnings.is_empty(), "{:?}", frozen.warnings);
    offline::write_wav_float(dir().join(wav), &frozen.audio, 2, RATE).unwrap();
    (
        format!(
            "{source}\nfrozen(\"{name}\", \"{wav}\", \"{}\")\n",
            frozen.fingerprint
        ),
        frozen.bars,
    )
}

/// Lo más que se separan dos audios.
fn max_diff(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .fold(0.0f32, |m, (x, y)| m.max((x - y).abs()))
}

/// Cuánto puede separarse la mezcla con la pista congelada de la original,
/// desde la segunda vuelta.
#[derive(Clone, Copy, Debug)]
enum Expect {
    /// Nada: la pista se repite igual cada vuelta.
    BitExact,
    /// Lo que la original se separa de sí misma entre su segunda vuelta y
    /// la tercera (la fase libre de un oscilador, el redondeo de una curva
    /// que un bitcrush convierte en un escalón, la cola de una reverb), con
    /// un margen del 50 %: congelar no cambia más de lo que la pista cambia
    /// sola.
    SelfVariation,
}

/// Congela `name` y comprueba: la salida de la pista congelada (tras sus
/// efectos, antes de su volumen) es bit a bit la de la original en su
/// segunda vuelta, vuelta tras vuelta; y la mezcla, desde la segunda
/// vuelta, como dice `expect`.
fn check(source: &str, name: &str, wav: &str, expect: Expect) -> Song {
    let (frozen_source, bars) = freeze(source, name, wav);
    let original = song(source).unwrap();
    let frozen = song(&frozen_source).unwrap();
    assert!(frozen.warnings.is_empty(), "{:?}", frozen.warnings);
    let track = frozen.tracks.iter().find(|t| t.name == name).unwrap();
    assert_eq!(track.frozen.as_ref().unwrap().bars, bars);
    assert!(track.fx.is_empty());
    let cycle = 2 * original.frames(RATE, bars) as usize;

    // La pista: lo congelado, bit a bit en cada vuelta.
    let tap = original.render_tap(RATE, 3 * bars, name).unwrap().unwrap();
    let frozen_tap = frozen.render_tap(RATE, 3 * bars, name).unwrap().unwrap();
    for lap in 0..3 {
        let laps = lap * cycle..(lap + 1) * cycle;
        assert!(
            frozen_tap[laps.clone()] == tap[cycle..2 * cycle],
            "{name}: la vuelta {lap} congelada no es la segunda original"
        );
    }
    assert!(tap[cycle..].iter().any(|&x| x.abs() > 0.01));

    // La mezcla, desde la segunda vuelta.
    let a = original.render(RATE, 3 * bars).unwrap();
    let b = frozen.render(RATE, 3 * bars).unwrap();
    let diff = max_diff(&a[cycle..], &b[cycle..]);
    let own = max_diff(&a[cycle..2 * cycle], &a[2 * cycle..]);
    eprintln!("{name}: mezcla, diferencia máxima {diff:e}; la original consigo misma, {own:e}");
    match expect {
        Expect::BitExact => assert!(a[cycle..] == b[cycle..], "{name}: {diff}"),
        Expect::SelfVariation => assert!(diff <= 1.5 * own, "{name}: {diff} > 1,5 · {own}"),
    }
    frozen
}

const DRUMS: &str = r#"bpm(170)
bus("reverb", { size = 0.5 })
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("lowpass", { cutoff = ramp(400, 6000, "2 compases"), q = 2 })
  :fx("bitcrush", { bits = 8 })
  :fx("dyneq", { freq = 100, threshold = -30, range = 6, sidechain = "kick" })
  :every(2, function(c) c:reverse() end)
  :gain(0.7):pan(-0.3):send("reverb", 0.3)
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
track("bass", amen:slice_at(3))
  :steps("1^-12 _ . .  . . 1^-12 .  . . . .  1^-7 . . .")
  :fx("dyneq", { freq = 200, threshold = -40, range = 9, sidechain = "drums" })
  :gain(0.5)
"#;

#[test]
fn a_frozen_track_sounds_like_the_original() {
    // Con efectos, una rampa de 2 compases, every, sidechain desde el bombo
    // y el bajo escuchándola a ella; volumen, pan y envío fuera del WAV. La
    // pista, bit a bit; la mezcla, lo que la original cambia sola.
    let frozen = check(DRUMS, "drums", "congelado/drums.wav", Expect::SelfVariation);
    let drums = frozen.tracks.iter().find(|t| t.name == "drums").unwrap();
    assert_eq!((drums.gain, drums.pan), (0.7, -0.3));
    assert_eq!(drums.sends[1], 0.3);
    assert!(!drums.frozen.as_ref().unwrap().stale);
}

/// Una pista que se repite igual cada vuelta (cortes con un filtro y un
/// bitcrush fijos, volumen y pan, y otra pista al lado): la mezcla
/// congelada es bit a bit la original. Con un EQ dinámico o un compresor no
/// (su reducción no se recalcula por menos de 0,005 dB, así que guarda algo
/// de toda su historia), ni con un envío (en la primera vuelta lo congelado
/// ya lleva su cola, y su eco llega a las siguientes): para eso,
/// [`a_frozen_track_sounds_like_the_original`].
#[test]
fn a_periodic_track_freezes_bit_for_bit() {
    let source = r#"bpm(170)
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("lowpass", { cutoff = 3000, q = 2 })
  :fx("bitcrush", { bits = 10 })
  :every(2, function(c) c:reverse() end)
  :gain(0.7):pan(-0.3)
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
"#;
    check(source, "drums", "congelado/periodic.wav", Expect::BitExact);
}

#[test]
fn a_synth_freezes_too() {
    let source = r#"bpm(130)
track("acid", synth("acid", { cutoff = lfo("2 compases", 300, 3000), res = 0.7 }))
  :melody("c2 c3 . c2  eb2/ . c2! .  g1 . c2 c3  . bb1 c2 .")
  :fx("distortion", { drive = 4 })
track("kick", sample("amen.wav"):slice(8):slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
"#;
    check(source, "acid", "congelado/acid.wav", Expect::SelfVariation);
}

#[test]
fn a_change_after_freezing_warns_but_keeps_the_frozen_audio() {
    let (frozen_source, _) = freeze(DRUMS, "drums", "congelado/drums_stale.wav");
    let changed = frozen_source.replace("bits = 8", "bits = 6");
    let s = song(&changed).unwrap();
    assert!(
        s.warnings
            .iter()
            .any(|w| w.starts_with("la pista drums ha cambiado desde que se congeló")),
        "{:?}",
        s.warnings
    );
    assert!(s.tracks[0].frozen.as_ref().unwrap().stale);
    // Sin huella, no hay con qué comparar: sin aviso.
    let bare = changed.replace(
        &format!(
            ", \"{}\")",
            song(DRUMS).unwrap().freeze_fingerprint("drums").unwrap()
        ),
        ")",
    );
    assert!(song(&bare).unwrap().warnings.is_empty());
}

#[test]
fn what_cannot_be_frozen_says_why() {
    let sections = "local amen = sample(\"amen.wav\"):slice(8)\n\
        section(\"a\", function() track(\"d\", amen):steps(\"1 . . .  . . . .  . . . .  . . . .\") end)\n";
    let e = song(sections)
        .unwrap()
        .render_frozen(RATE, "d")
        .unwrap_err();
    assert!(e.message.contains("con secciones no se congela"), "{e}");
    let ramp = "bpm_ramp(120, 140, \"4 compases\")\n\
        track(\"d\", sample(\"amen.wav\"):slice(8)):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n";
    let e = song(ramp).unwrap().render_frozen(RATE, "d").unwrap_err();
    assert!(e.message.contains("rampa de tempo"), "{e}");
    let choke = "track(\"d\", sample(\"amen.wav\"):slice(8)):steps(\"1 . . .  . . . .  . . . .  . . . .\"):choke(1)\n";
    let e = song(choke).unwrap().render_frozen(RATE, "d").unwrap_err();
    assert!(e.message.contains("tiene choke"), "{e}");
    let hz = "track(\"d\", sample(\"amen.wav\"):slice(8)):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n\
        :fx(\"lowpass\", { cutoff = lfo(0.3, 300, 3000) })\n";
    let w = song(hz).unwrap().render_frozen(RATE, "d").unwrap().warnings;
    assert!(w[0].contains("un lfo en Hz"), "{w:?}");
}

#[test]
fn frozen_errors_say_where() {
    std::fs::write(dir().join("congelado/vacio.wav"), b"").ok();
    let (ok, _) = freeze(DRUMS, "drums", "congelado/drums_err.wav");
    for (source, message) in [
        (
            format!("{DRUMS}frozen(\"nadie\", \"congelado/drums_err.wav\")"),
            "song.lua:16: frozen(\"nadie\", \"congelado/drums_err.wav\"): no hay ninguna pista \"nadie\"",
        ),
        (
            format!("{DRUMS}frozen(\"drums\", \"congelado/no.wav\")"),
            "song.lua:16: frozen(\"drums\", \"congelado/no.wav\"):",
        ),
        (
            format!("{ok}frozen(\"drums\", \"congelado/drums_err.wav\")"),
            "ya está congelada en song.lua:17",
        ),
        (
            "section(\"a\", function() frozen(\"d\", \"congelado/drums_err.wav\") end)".to_string(),
            "frozen",
        ),
    ] {
        let Err(error) = song(&source) else {
            panic!("{source}: sin error")
        };
        assert!(error.contains(message), "{error}");
    }
}

/// Coste: una pista cara (un subharm de 6 voces con sus capas y un filtro
/// con LFO) frente a la misma congelada. Ignorado: mide tiempo.
/// `cargo test -p script --release --test freeze cost -- --ignored --nocapture`
#[test]
#[ignore]
fn cost_of_a_frozen_track() {
    let source = r#"bpm(140)
track("pad", subharm{ voices = 6, cutoff = lfo("4 compases", 400, 4000) })
  :melody("c3+eb3+g3+bb3 _ _ _  _ _ _ _  ab2+c3+eb3+g3 _ _ _  _ _ _ _")
  :fx("distortion", { drive = 3 }):fx("lowpass", { cutoff = 6000 })
"#;
    let (frozen_source, bars) = freeze(source, "pad", "congelado/cost.wav");
    let time = |source: &str| {
        let song = song(source).unwrap();
        let start = std::time::Instant::now();
        let audio = song.render(RATE, 8 * bars).unwrap();
        let seconds = audio.len() as f64 / 2.0 / RATE as f64;
        start.elapsed().as_secs_f64() / seconds * 100.0
    };
    let (live, frozen) = (time(source), time(&frozen_source));
    println!("pista viva: {live:.2} % de un núcleo; congelada: {frozen:.2} %");
}
