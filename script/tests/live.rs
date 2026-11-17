//! Qué parámetro del motor mueve un número del código: solo si se sabe
//! seguro (un sitio, una pista, el mismo valor, el último que lo escribe,
//! continuo y sin curva); si no, no hay destino y el número suena al compás
//! siguiente. Y que la carga que lo pide retira el gesto en el motor.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{KnobCurve, KnobTarget, new_engine, offline};
use script::lint::{NumberAt, number_at};
use script::{ScriptHost, api};

const RATE: u32 = 48_000;

#[path = "../../engine/tests/support/synth_break.rs"]
mod synth;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("live");
        std::fs::create_dir_all(&dir).unwrap();
        let (audio, _) = synth::synth_break();
        offline::write_wav(dir.join("amen.wav"), &audio, 1, RATE).unwrap();
        dir
    })
    .clone()
}

const SONG: &str = r#"bpm(170)
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("lowpass", { cutoff = 6000 }):fx("lowpass", { cutoff = 5000 })
  :fx("bitcrush", { bits = 8 }):fx("lowpass", { cutoff = 3000 })
  :gain(0.8):pan(-0.25):send("reverb", 0.3)
  -- (cuatro efectos como mucho)
for i = 1, 2 do
  track("t" .. i, amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . ."):gain(0.5)
end
track("bass", synth("acid", { cutoff = 400, res = 0.6 }))
  :melody("c2 . c3 .  . . . .  . . . .  . . . .")
track("kick", amen:slice_at(2)):steps("x . . .  x . . .  x . . .  x . . ."):gain(0.6)
track("kick"):gain(0.7)
"#;

/// El host con `SONG` cargada.
fn host(source: &str) -> ScriptHost {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(&mut controller, source, "song.lua", &dir())
        .unwrap();
    host
}

/// El destino del número que hay en `needle` (su primer carácter) en la
/// fila `row`.
fn target(
    host: &ScriptHost,
    source: &str,
    row: usize,
    needle: &str,
) -> Result<engine::KnobMap, &'static str> {
    let line = source.lines().nth(row).unwrap();
    let col = line[..line.find(needle).unwrap()].chars().count();
    let NumberAt::Number(n) = number_at(source, row, col) else {
        panic!("no hay número en {needle}");
    };
    let range = n
        .param
        .and_then(api::numeric)
        .map(|s| (s.min, s.max, s.log));
    host.live_target(&n.place, n.text.parse().unwrap(), range)
        .map(|t| t.map)
}

#[test]
fn certain_numbers_have_their_engine_parameter() {
    let host = host(SONG);
    let slot = host.track_slot("drums").unwrap() as u8;
    // El tercer lowpass (tras otro efecto en su línea): su ranura, con el
    // rango del registro en escala logarítmica.
    let second = target(&host, SONG, 5, "3000").unwrap();
    assert_eq!(
        second.target,
        KnobTarget::Fx {
            track: slot,
            slot: 3,
            param: 0
        }
    );
    assert_eq!(
        (second.min, second.max, second.curve),
        (20.0, 20000.0, KnobCurve::Log)
    );
    assert!(second.latch);
    let gain = target(&host, SONG, 6, "0.8").unwrap();
    assert_eq!(
        gain.target,
        KnobTarget::Mix {
            track: slot,
            index: engine::MIX_GAIN as u8
        }
    );
    assert_eq!((gain.min, gain.max), (0.0, 4.0), "el rango del registro");
    assert!(matches!(
        target(&host, SONG, 6, "-0.25").unwrap().target,
        KnobTarget::Mix { index, .. } if index as usize == engine::MIX_PAN
    ));
    assert!(matches!(
        target(&host, SONG, 6, "0.3").unwrap().target,
        KnobTarget::Mix { index, .. } if index as usize > engine::MIX_PAN
    ));
    let bass = host.track_slot("bass").unwrap() as u8;
    assert!(matches!(
        target(&host, SONG, 11, "400").unwrap().target,
        KnobTarget::Synth { track, .. } if track == bass
    ));
}

#[test]
fn uncertain_numbers_have_no_target_and_say_why() {
    let host = host(SONG);
    // Dos llamadas iguales en la misma fila (la misma línea para Lua): no
    // se sabe cuál es cuál.
    for n in ["6000", "5000"] {
        assert_eq!(
            target(&host, SONG, 4, n),
            Err("esa llamada se hace más de una vez")
        );
    }
    // En un bucle: más de una vez.
    assert_eq!(
        target(&host, SONG, 9, "0.5"),
        Err("esa llamada se hace más de una vez")
    );
    // Otra línea lo cambia después.
    assert_eq!(
        target(&host, SONG, 13, "0.6"),
        Err("otra línea lo cambia después")
    );
    assert!(target(&host, SONG, 14, "0.7").is_ok());
    // No es de un parámetro (bpm, slice).
    assert!(target(&host, SONG, 0, "170").is_err());
    assert!(target(&host, SONG, 1, "8").is_err());
    // El texto cambió y el número ya no es el que llegó al parámetro.
    let changed = SONG.replace("cutoff = 3000", "cutoff = 3100");
    assert_eq!(
        target(&host, &changed, 5, "3100"),
        Err("lo que llegó al parámetro no es este número")
    );
}

#[test]
fn with_sections_or_variants_nothing_is_certain() {
    let source = format!("{SONG}section(\"a\", function() end)\narrange({{ {{\"a\", 1}} }})\n");
    let host = host(&source);
    assert_eq!(
        target(&host, &source, 14, "0.7"),
        Err("no se sabe de qué parámetro es")
    );
}

#[test]
fn a_shared_synth_goes_to_more_than_one_track() {
    let source = "local s = synth(\"acid\", { cutoff = 400 })\ntrack(\"a\", s):melody(\"c2 . . .  . . . .  . . . .  . . . .\")\ntrack(\"b\", s):melody(\"c3 . . .  . . . .  . . . .  . . . .\")\n";
    let host = host(source);
    assert_eq!(
        target(&host, source, 0, "400"),
        Err("va a más de una pista")
    );
}

#[test]
fn the_load_that_asks_for_it_releases_the_gesture_at_the_bar() {
    // Con el gesto sobre el volumen de una pista, la carga que lo retira
    // deja el volumen escrito; mientras, manda el gesto.
    let source = "bpm(120)\nlocal amen = sample(\"amen.wav\"):slice(8)\ntrack(\"d\", amen):steps(\"1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1\"):gain(1)\n";
    let (mut controller, mut engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(&mut controller, source, "song.lua", &dir())
        .unwrap();
    controller.start().unwrap();
    let bar = (RATE as f64 * 2.0) as usize;
    let level = |engine: &mut engine::Engine, frames: usize| {
        let out = offline::render(engine, frames, 2);
        (out.iter().map(|s| s * s).sum::<f32>() / out.len() as f32).sqrt()
    };
    let loud = level(&mut engine, bar);
    let map = target(&host, source, 2, "1)").unwrap();
    let t = engine::KnobMap { ..map };
    controller.gesture(Some(t), 0.0).unwrap();
    level(&mut engine, bar / 8);
    let quiet = level(&mut engine, bar / 2);
    assert!(quiet < loud * 0.05, "el gesto manda ya: {quiet} {loud}");
    // La carga con el texto nuevo (gain 0) lo retira al compás: sigue en
    // silencio, ahora por el texto.
    let silent = source.replace("gain(1)", "gain(0)");
    host.release_gesture();
    host.load(&mut controller, &silent, "song.lua", &dir())
        .unwrap();
    let after = level(&mut engine, 2 * bar);
    assert!(after < loud * 0.05, "{after}");
    // Y otra carga sin gesto devuelve el volumen escrito (el gesto ya no
    // está).
    host.load(&mut controller, source, "song.lua", &dir())
        .unwrap();
    level(&mut engine, bar);
    let back = level(&mut engine, bar);
    assert!(back > loud * 0.5, "{back} {loud}");
}
