//! `group()`: el modelo guarda cada grupo con sus pistas, efectos,
//! volumen, pan y envíos; cambia solo la mezcla de sus pistas;
//! los errores dicen la línea.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{Fx, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("groups");
        std::fs::create_dir_all(&dir).unwrap();
        let mut mono = vec![0.0f32; 6000];
        mono[..800].fill(0.3);
        mono[3000..3800].fill(0.2);
        offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!("bpm(130)\nlocal b = sample(\"break.wav\"):slice(2)\n{source}");
    host.load(&mut controller, &source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

const TRACKS: &str = "track(\"k\", b):steps(\"1 . . .  1 . . .  1 . . .  1 . . .\")\n\
                      track(\"s\", b):steps(\". . 2 .  . . 2 .  . . 2 .  . . 2 .\")\n\
                      track(\"h\", b):steps(\"2 2 2 2  2 2 2 2  2 2 2 2  2 2 2 2\"):gain(0.3)\n";

#[test]
fn the_model_keeps_each_group() {
    let song = song(&format!(
        "{TRACKS}group(\"drums\", {{ \"k\", \"s\" }})\n  \
         :fx(\"compressor\", {{ threshold = -20, ratio = 4 }})\n  \
         :fx(\"lowpass\", {{ cutoff = ramp(400, 4000, \"4 compases\") }})\n  \
         :gain(0.8):pan(-0.2):send(\"reverb\", 0.3)\n\
         group(\"drums\"):send(\"delay\", lfo(\"1 compás\", 0, 0.5))\n"
    ))
    .unwrap();
    assert_eq!(song.groups.len(), 1);
    let g = &song.groups[0];
    assert_eq!(g.name, "drums");
    assert_eq!(g.tracks, ["k", "s"]);
    assert_eq!(g.fx[0], Fx::compressor(-20.0, 4.0));
    assert!(matches!(g.fx[1], Fx::Filter { .. }));
    assert_eq!(g.automation.len(), 1);
    assert_eq!(g.automation[0].fx, 1);
    assert_eq!((g.gain, g.pan), (0.8, -0.2));
    assert_eq!(g.sends, [0.0, 0.3]);
    assert!(g.mix_curves[engine::MIX_SEND].is_some());
}

#[test]
fn a_group_changes_only_the_mix_of_its_tracks() {
    let plain = song(TRACKS).unwrap().render(RATE, 2).unwrap();
    // Sin efectos ni volumen, nada cambia.
    let same = song(&format!(
        "{TRACKS}group(\"g\", {{ \"k\", \"s\", \"h\" }})\n"
    ))
    .unwrap()
    .render(RATE, 2)
    .unwrap();
    assert_eq!(plain, same);
    // Callado, solo suena la pista de fuera.
    let only_h = song(&format!(
        "{TRACKS}group(\"g\", {{ \"k\", \"s\" }}):gain(0)\n"
    ))
    .unwrap()
    .render(RATE, 2)
    .unwrap();
    let h = song(&format!(
        "{TRACKS}track(\"k\"):mute()\ntrack(\"s\"):mute()\n"
    ))
    .unwrap()
    .render(RATE, 2)
    .unwrap();
    assert_eq!(only_h, h);
    // Con un compresor fuerte, la suma suena distinta que sin él.
    let squashed = song(&format!(
        "{TRACKS}group(\"g\", {{ \"k\", \"s\", \"h\" }}):fx(\"compressor\", {{ threshold = -30, ratio = 10 }})\n"
    ))
    .unwrap()
    .render(RATE, 2)
    .unwrap();
    let energy = |a: &[f32]| a.iter().map(|v| v * v).sum::<f32>();
    assert!(energy(&squashed) < 0.5 * energy(&plain));
}

#[test]
fn group_errors_say_the_line() {
    let cases = [
        (
            "group(\"g\", { \"k\", \"nadie\" })",
            "grupo \"g\": group \"nadie\" no es ninguna pista",
        ),
        (
            "group(\"g\", { \"k\" })\ngroup(\"h\", { \"k\" })",
            "song.lua:7: group(\"h\"): \"k\" ya va al grupo \"g\"",
        ),
        (
            "group(\"g\", { \"k\", \"k\" })",
            "song.lua:6: group(\"g\"): \"k\" está dos veces",
        ),
        ("group(\"g\")", "song.lua:6: group(\"g\"): no existe"),
        (
            "group(\"g\", { \"k\" })\ngroup(\"g\", { \"s\" })",
            "song.lua:7: group(\"g\"): ya existe",
        ),
        (
            "group(\"g\", {})",
            "song.lua:6: group(\"g\"): falta la lista",
        ),
        (
            "for i = 1, 5 do group(\"g\" .. i, { ({ \"k\", \"s\", \"h\", \"k\", \"s\" })[i] }) end",
            "\"k\" ya va al grupo \"g1\"",
        ),
        (
            "group(\"g\", { \"k\" }):fx(\"dyneq\", { freq = 200, sidechain = \"bajo\" })",
            "grupo \"g\": fx(\"dyneq\"): sidechain \"bajo\" no es ninguna pista",
        ),
        (
            "section(\"a\", function() group(\"g\", { \"k\" }) end)",
            "group() es de toda la canción",
        ),
        (
            "group(\"g\", { \"k\" }):gain(5)",
            "song.lua:6: volumen de pista 5 fuera de rango",
        ),
    ];
    for (code, expected) in cases {
        let Err(err) = song(&format!("{TRACKS}{code}\n")) else {
            panic!("debería fallar: {code}");
        };
        assert!(err.contains(expected), "{code}\n→ {err}");
    }
}

#[test]
fn at_most_four_groups() {
    let mut code = String::new();
    for i in 0..5 {
        code.push_str(&format!(
            "track(\"t{i}\", b):steps(\"1 . . .  . . . .  . . . .  . . . .\")\ngroup(\"g{i}\", {{ \"t{i}\" }})\n"
        ));
    }
    let Err(err) = song(&code) else {
        panic!("debería fallar");
    };
    assert!(
        err.contains("group(\"g4\"): hay 4 grupos como mucho"),
        "{err}"
    );
}

#[test]
fn a_group_can_hold_tracks_of_any_section() {
    let song = song(
        "section(\"a\", function() track(\"k\", b):steps(\"1 . . .  . . . .  . . . .  . . . .\") end)\n\
         section(\"b\", function() track(\"s\", b):steps(\"2 . . .  . . . .  . . . .  . . . .\") end)\n\
         arrange({ { \"a\", 1 }, { \"b\", 1 } })\n\
         group(\"g\", { \"k\", \"s\" }):gain(0.5)\n",
    )
    .unwrap();
    assert_eq!(song.groups[0].tracks, ["k", "s"]);
    assert!(song.render(RATE, 2).unwrap().iter().any(|&v| v != 0.0));
}
