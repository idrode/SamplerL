//! `macro()` y `macro_init()`: cada macro tiene su número por orden de
//! aparición y su valor inicial; vale donde valen `ramp` y `lfo`; suena
//! como el número que da con su valor; recargar conserva lo movido en vivo
//! salvo que el script cambie su `macro_init`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{Curve, new_engine, offline};
use script::{MacroDef, ScriptHost, Song};

const RATE: u32 = 48_000;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("macros");
        std::fs::create_dir_all(&dir).unwrap();
        let mut mono = vec![0.0f32; 6000];
        mono[..800].fill(0.3);
        mono[3000..3800].fill(0.2);
        offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

const PRELUDE: &str = "bpm(130)\nlocal b = sample(\"break.wav\"):slice(2)\n";

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(
        &mut controller,
        &format!("{PRELUDE}{source}"),
        "song.lua",
        &dir(),
    )
    .cloned()
    .map_err(|e| e.to_string())
}

fn defs(list: &[(&str, f32)]) -> Vec<MacroDef> {
    list.iter()
        .map(|&(name, value)| MacroDef {
            name: name.into(),
            value,
        })
        .collect()
}

const STEPS: &str = "1 . . .  2 . . .  . . 1 .  . . . .";

#[test]
fn macros_are_numbered_in_order_and_start_at_their_init() {
    let song = song(&format!(
        "macro_init(\"caos\", 0.25)\n\
         track(\"d\", b):steps(\"{STEPS}\")\n  \
         :fx(\"lowpass\", {{ cutoff = macro(\"brillo\", 300, 8000) }})\n  \
         :gain(macro(\"caos\", 1, 0.5))\n  \
         :send(\"reverb\", macro(\"brillo\", 0, 0.4))\n\
         bus(\"delay\", {{ feedback = macro(\"eco\", 0.2, 0.8) }})\n"
    ))
    .unwrap();
    assert_eq!(
        song.macros,
        defs(&[("caos", 0.25), ("brillo", 0.0), ("eco", 0.0)])
    );
    let t = &song.tracks[0];
    assert_eq!(
        t.automation[0].curve,
        Curve::Macro {
            index: 1,
            from: 300.0,
            to: 8000.0
        }
    );
    assert_eq!(
        t.mix_curves[engine::MIX_GAIN],
        Some(Curve::Macro {
            index: 0,
            from: 1.0,
            to: 0.5
        })
    );
    assert!(song.buses.curves[engine::bus::DELAY_FEEDBACK].is_some());
}

#[test]
fn a_macro_sounds_like_the_number_it_gives() {
    let at = |init: f32| {
        song(&format!(
            "macro_init(\"v\", {init})\n\
             track(\"d\", b):steps(\"{STEPS}\"):gain(macro(\"v\", 0, 2))\n"
        ))
        .unwrap()
        .render(RATE, 2)
        .unwrap()
    };
    let fixed = |gain: f32| {
        song(&format!(
            "track(\"d\", b):steps(\"{STEPS}\"):gain({gain})\n"
        ))
        .unwrap()
        .render(RATE, 2)
        .unwrap()
    };
    for (init, gain) in [(0.25, 0.5), (0.5, 1.0)] {
        let a = at(init);
        let b = fixed(gain);
        assert!(a.iter().any(|&v| v != 0.0));
        let diff = a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        assert!(diff < 1e-6, "{init}: {diff}");
    }
}

#[test]
fn reloading_keeps_what_was_moved_live() {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = |init: f32| {
        format!(
            "{PRELUDE}macro_init(\"brillo\", {init})\n\
             track(\"d\", b):steps(\"{STEPS}\"):fx(\"lowpass\", {{ cutoff = macro(\"brillo\", 300, 8000) }})\n\
             track(\"e\", b):steps(\"{STEPS}\"):gain(macro(\"nivel\", 0, 1))\n"
        )
    };
    host.load(&mut controller, &source(0.5), "song.lua", &dir())
        .unwrap();
    assert_eq!(host.macros(), defs(&[("brillo", 0.5), ("nivel", 0.0)]));
    host.set_macro(&mut controller, 1, 0.8).unwrap();
    host.set_macro(&mut controller, 0, 0.1).unwrap();
    assert_eq!(host.macros(), defs(&[("brillo", 0.1), ("nivel", 0.8)]));
    // Recargar igual: siguen donde se pusieron.
    host.load(&mut controller, &source(0.5), "song.lua", &dir())
        .unwrap();
    assert_eq!(host.macros(), defs(&[("brillo", 0.1), ("nivel", 0.8)]));
    // El script cambia el valor inicial de una: manda el script.
    host.load(&mut controller, &source(0.7), "song.lua", &dir())
        .unwrap();
    assert_eq!(host.macros(), defs(&[("brillo", 0.7), ("nivel", 0.8)]));
    // Y lo que se exporta suena con los valores de ahora.
    assert_eq!(host.song().unwrap().macros[1].value, 0.8);
    assert!(host.set_macro(&mut controller, 2, 0.5).is_err());
    assert!(host.set_macro(&mut controller, 0, 1.5).is_err());
}

#[test]
fn macros_work_wherever_curves_do() {
    song(&format!(
        "track(\"d\", b):steps(\"{STEPS}\")\n  \
         :pan(macro(\"m\", -1, 1)):pitch(macro(\"m\", -12, 12))\n  \
         :fx(\"eq\", {{ gain1 = macro(\"m\", -6, 6) }})\n\
         b:set(1, {{ attack = macro(\"m\", 0, 50) }})\n\
         track(\"s\", synth(\"acid\", {{ cutoff = macro(\"m\", 200, 2000) }})):melody(\"c . . .  . . . .  . . . .  . . . .\")\n\
         track(\"g\", b):steps(\"{STEPS}\"):grains({{ size = macro(\"m\", 20, 200) }})\n\
         bus(\"reverb\", {{ size = macro(\"m\", 0.2, 0.9) }})\n\
         group(\"todo\", {{ \"d\", \"s\" }}):fx(\"distortion\", {{ mix = macro(\"m\", 0, 1) }}):gain(macro(\"m\", 0.5, 1))\n"
    ))
    .unwrap();
}

#[test]
fn macro_errors_say_the_line() {
    let cases = [
        (
            "macro_init(\"m\", 1.5)",
            "song.lua:3: macro_init(\"m\", 1.5): el valor va de 0 a 1",
        ),
        (
            "for i = 1, 9 do local _ = macro(\"m\" .. i, 0, 1) end",
            "macro(\"m9\", ...): hay 8 macros como mucho",
        ),
        (
            "track(\"d\", b):steps(\"1 . . .  . . . .  . . . .  . . . .\"):gain(macro(\"m\", 0, 9))",
            "song.lua:3: gain(macro(...)): la curva se sale del rango (0 a 4)",
        ),
        (
            "track(\"d\", b):steps(\"1 . . .  . . . .  . . . .  . . . .\"):mode(macro(\"m\", 0, 1))",
            "song.lua:3:",
        ),
        (
            "section(\"a\", function() macro_init(\"m\", 0.5) end)",
            "macro_init() es de toda la canción",
        ),
    ];
    for (code, expected) in cases {
        let Err(err) = song(&format!("{code}\n")) else {
            panic!("debería fallar: {code}");
        };
        assert!(err.contains(expected), "{code}\n→ {err}");
    }
}
