//! `pista:mpe{ ... }` (fase Q): los ejes de la expresión por nota de un
//! sintetizador, con sus rangos en las unidades del motor; las notas del
//! patrón suenan igual con o sin ella; los errores dicen la línea.

use std::path::Path;

use engine::new_engine;
use engine::synth::{BEND, CUTOFF, ExprMap, ExprTarget, PRESSURE, PW, RESONANCE, SLIDE, sh101};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!("bpm(120)\n{source}");
    host.load(&mut controller, &source, "song.lua", Path::new("."))
        .cloned()
        .map_err(|e| e.to_string())
}

fn expr(source: &str) -> [Option<ExprMap>; 3] {
    let song = song(source).unwrap_or_else(|e| panic!("{source}: {e}"));
    song.tracks[0].instrument.unwrap().synth.unwrap().patch.expr
}

fn map(target: ExprTarget, range: f32, curve: f32) -> Option<ExprMap> {
    Some(ExprMap {
        target,
        range,
        curve,
    })
}

#[test]
fn the_axes_go_where_the_table_says_with_their_defaults() {
    let got = expr(
        "track(\"t\", subharm{ voices = 4 })\n\
           :mpe{ bend = \"pitch\", pressure = \"cutoff\", slide = \"res\" }",
    );
    assert_eq!(got[BEND], map(ExprTarget::Pitch, 12.0, 1.0));
    assert_eq!(got[PRESSURE], map(ExprTarget::Param(CUTOFF), 2.0, 1.0));
    // La resonancia del subharm va de 0 a 1: medio rango.
    assert_eq!(got[SLIDE], map(ExprTarget::Param(RESONANCE), 0.5, 1.0));

    let got = expr(
        "track(\"t\", sh101{})\n\
           :mpe{ pressure = { \"gain\", range = 0.5, curve = 2 }, slide = { \"pw\", range = 0.2 }, bend = false }",
    );
    assert_eq!(got[BEND], None);
    assert_eq!(got[PRESSURE], map(ExprTarget::Gain, 0.5, 2.0));
    // El ancho de pulso del sh101, en las unidades del motor.
    let scale = sh101::param("pw").unwrap().scale;
    assert_eq!(got[SLIDE], map(ExprTarget::Param(PW), 0.2 * scale, 1.0));

    // Sin tabla, nada; y lo que no se dice, nada.
    assert_eq!(expr("track(\"t\", synth(\"acid\", {})):mpe()"), [None; 3]);
    let got = expr("track(\"t\", synth(\"acid\", {})):mpe{ pressure = { \"pitch\" } }");
    assert_eq!(got[PRESSURE], map(ExprTarget::Pitch, 2.0, 1.0));
    assert_eq!(got[BEND], None);
}

#[test]
fn pattern_notes_sound_the_same_with_or_without_mpe() {
    let source = |mpe: &str| {
        format!(
            "track(\"t\", subharm{{ voices = 4, cutoff = 400 }}){mpe}\n\
               :melody(\"c . e .  g . c5 .  g . e .  c . . .\")"
        )
    };
    let plain = song(&source("")).unwrap();
    let with = song(&source(
        ":mpe{ bend = \"pitch\", pressure = { \"gain\", range = 1 }, slide = \"cutoff\" }",
    ))
    .unwrap();
    let a = plain.render(RATE, 2).unwrap();
    let b = with.render(RATE, 2).unwrap();
    assert!(a.iter().any(|&x| x != 0.0));
    assert_eq!(a, b);
}

#[test]
fn a_track_of_slices_has_no_expression() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mpe");
    std::fs::create_dir_all(&dir).unwrap();
    engine::offline::write_wav(dir.join("x.wav"), &[0.1; 4800], 1, RATE).unwrap();
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = "bpm(120)\ntrack(\"t\", sample(\"x.wav\"):slice(1))\n  :mpe{}";
    let error = host
        .load(&mut controller, source, "song.lua", &dir)
        .map(|_| ())
        .unwrap_err()
        .to_string();
    assert!(error.contains("no es un sintetizador"), "{error}");
    assert!(error.contains("song.lua:3"), "{error}");
}

#[test]
fn bad_tables_are_errors_with_the_line() {
    let cases = [
        (
            "track(\"t\", sh101{})\n  :mpe{ tilt = \"pitch\" }",
            "el eje \"tilt\" no existe",
        ),
        (
            "track(\"t\", sh101{})\n  :mpe{ pressure = \"glide\" }",
            "\"glide\" no se mueve por nota en sh101",
        ),
        (
            "track(\"t\", sh101{})\n  :mpe{ pressure = \"nada\" }",
            "hay: pitch, gain, saw",
        ),
        (
            "track(\"t\", sh101{})\n  :mpe{ pressure = { range = 2 } }",
            "la tabla empieza por adónde va",
        ),
        (
            "track(\"t\", sh101{})\n  :mpe{ pressure = { \"gain\", range = 2 } }",
            "el volumen va de 0 a 1",
        ),
        (
            "track(\"t\", sh101{})\n  :mpe{ pressure = { \"cutoff\", curve = 0 } }",
            "curve de 0.1 a 8",
        ),
        (
            "track(\"t\", sh101{})\n  :mpe{ pressure = { \"cutoff\", rango = 2 } }",
            "clave desconocida",
        ),
        (
            "track(\"t\", sh101{})\n  :mpe{ pressure = 3 }",
            "va a un texto",
        ),
    ];
    for (source, expected) in cases {
        let Err(error) = song(source) else {
            panic!("{source}: tenía que fallar");
        };
        assert!(error.contains(expected), "{source}\n→ {error}");
        assert!(error.contains("song.lua:3"), "{source}\n→ {error}");
    }
}
