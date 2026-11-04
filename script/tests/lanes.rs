//! Lanes de automatización desde Lua: `pista:auto()`, `auto_clear()`,
//! `auto_list()` y `lane()`. Cada destino recibe su curva, los nombres
//! ambiguos van al primero con aviso, las opciones se leen bien y los
//! errores dicen la línea sin tocar lo que suena.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{Curve, LaneCurve, LaneMode, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("lanes");
        std::fs::create_dir_all(&dir).unwrap();
        let mut mono = vec![0.0f32; 24_000];
        for (i, x) in mono.iter_mut().enumerate() {
            *x = 0.4 * (i as f32 * 0.05).sin();
        }
        offline::write_wav(dir.join("b.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

const HEAD: &str = "bpm(120)\nlocal b = sample(\"b.wav\"):slice_grid(2)\n\
                    local t = track(\"t\", b):steps(\"1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .\")\n";

fn load(host: &mut ScriptHost, source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    host.load(
        &mut controller,
        &format!("{HEAD}{source}"),
        "song.lua",
        &dir(),
    )
    .cloned()
    .map_err(|e| e.to_string())
}

fn song(source: &str) -> Result<Song, String> {
    load(&mut ScriptHost::new(RATE), source)
}

fn lane_curve(curve: Option<&Curve>) -> (u8, f32, f32) {
    match curve {
        Some(Curve::Lane { index, min, max }) => (*index, *min, *max),
        other => panic!("no es una lane: {other:?}"),
    }
}

#[test]
fn auto_puts_a_lane_on_an_effect_parameter() {
    let song = song(
        "t:fx(\"lowpass\", { cutoff = 3000 })\n\
         t:auto(\"cutoff\", { {0, 200, \"exp\"}, {2, 8000, \"exp\"}, {4, 200} }, { len = \"4 compases\" })",
    )
    .unwrap();
    let track = &song.tracks[0];
    assert_eq!(track.lanes, [("lowpass.cutoff".to_string(), 0)]);
    let automation = track.automation.iter().find(|a| a.fx == 0).unwrap();
    assert_eq!(automation.param, 0);
    assert_eq!(lane_curve(Some(&automation.curve)), (0, 200.0, 8000.0));
    let def = song.lanes[0].as_ref().unwrap();
    assert_eq!(def.param, "lowpass.cutoff");
    assert_eq!(def.lane.length(), 64.0);
    assert_eq!(def.lane.points()[1].at, 32.0);
    assert_eq!(def.lane.points()[1].curve, LaneCurve::Exp);
    assert_eq!(def.lane.mode(), LaneMode::Loop);
    // `once` vuelve al valor fijo del efecto.
    assert!(song.warnings.is_empty(), "{:?}", song.warnings);
    // Suena distinto que sin la lane.
    let plain = self::song("t:fx(\"lowpass\", { cutoff = 3000 })").unwrap();
    assert_ne!(
        song.render(RATE, 2).unwrap(),
        plain.render(RATE, 2).unwrap()
    );
}

#[test]
fn an_ambiguous_name_goes_to_the_first_and_says_so() {
    let source = "t:fx(\"lowpass\", { cutoff = 3000 }):fx(\"highpass\", { cutoff = 40 })\n\
                  local _, aviso = t:auto(\"cutoff\", { {0, 500}, {1, 2000} })\n\
                  assert(aviso:find(\"va a \\\"lowpass.cutoff\\\"\"), aviso)\n\
                  t:auto(\"highpass.cutoff\", { {0, 40}, {1, 200} })\n\
                  t:auto(\"2.q\", { {0, 0.7}, {1, 4} })";
    let song = song(source).unwrap();
    let track = &song.tracks[0];
    let params: Vec<&str> = track.lanes.iter().map(|(p, _)| p.as_str()).collect();
    assert_eq!(params, ["lowpass.cutoff", "highpass.cutoff", "highpass.q"]);
    assert_eq!(song.warnings.len(), 1, "{:?}", song.warnings);
    assert!(song.warnings[0].starts_with("song.lua:5: auto(\"cutoff\"):"));
    assert!(song.warnings[0].contains("para los otros: highpass.cutoff"));
    // "pan" es de la pista y de los cortes: la pista, con aviso.
    let song = self::song("t:auto(\"pan\", { {0, -1}, {1, 1} })").unwrap();
    assert!(song.tracks[0].mix_curves[engine::MIX_PAN].is_some());
    assert!(
        song.warnings[0].contains("slice.pan"),
        "{:?}",
        song.warnings
    );
}

#[test]
fn every_kind_of_target() {
    let song = song(
        "t:auto(\"gain\", { {0, 0.2}, {1, 1} })\n\
         t:auto(\"send.reverb\", { {0, 0}, {1, 0.5} })\n\
         t:auto(\"pitch\", { {0, -12}, {1, 12} })\n\
         t:auto(\"bend\", { {0, 0}, {1, -12, \"step\"} }, { mode = \"hold\" })\n\
         local s = track(\"s\", synth(\"acid\", { cutoff = 800 })):melody(\"c2 . . .  . . . .  . . . .  . . . .\")\n\
         s:auto(\"cutoff\", { {0, 300}, {1, 3000} })",
    )
    .unwrap();
    let t = &song.tracks[0];
    lane_curve(t.mix_curves[engine::MIX_GAIN].as_ref());
    lane_curve(t.mix_curves[engine::MIX_SEND + engine::bus::REVERB].as_ref());
    lane_curve(t.pitch_curve.as_ref());
    // La forma de cada corte de la pista lleva la curva de bend.
    for shape in &t.shapes {
        lane_curve(shape.curves[engine::SHAPE_BEND].as_ref());
    }
    let synth = song.tracks[1].instrument.unwrap().synth.unwrap();
    assert!(
        synth
            .curves
            .iter()
            .flatten()
            .any(|c| matches!(c, Curve::Lane { .. }))
    );
    assert_eq!(song.lanes.iter().flatten().count(), 5);
    assert!(song.render(RATE, 2).is_ok());
}

#[test]
fn options_len_offset_range_and_scale() {
    let song = song(
        "t:fx(\"lowpass\", { cutoff = 3000 })\n\
         t:auto(\"cutoff\", { {0, 0}, {0.5, 1} }, { len = \"3/4\", offset = \"2 pasos\", range = {100, 6400}, scale = \"log\", mode = \"once\" })\n\
         t:auto(\"gain\", { {0, 0.5} }, { len = \"500 ms\" })",
    )
    .unwrap();
    let cutoff = song.lanes[0].as_ref().unwrap();
    assert_eq!(cutoff.lane.length(), 12.0);
    assert_eq!(cutoff.lane.offset(), 2.0);
    assert!(cutoff.lane.is_log());
    assert_eq!(cutoff.lane.points()[0].value, 100.0);
    assert!((cutoff.lane.points()[1].value - 6400.0).abs() < 0.01);
    // Once: fuera, el valor fijo del efecto.
    assert_eq!(cutoff.lane.rest(), 3000.0);
    assert_eq!(cutoff.lane.value(100.0), 3000.0);
    // 500 ms a 120 BPM son 4 pasos.
    assert_eq!(song.lanes[1].as_ref().unwrap().lane.length(), 4.0);
}

#[test]
fn lane_goes_wherever_a_curve_goes() {
    let song = song(
        "t:fx(\"lowpass\", { cutoff = lane({ {0, 300}, {1, 5000, \"smooth\"} }, { len = 2 }) })\n\
         t:gain(lane({ {0, 0.4}, {0.5, 1, \"step\"} }))",
    )
    .unwrap();
    let t = &song.tracks[0];
    assert_eq!(lane_curve(Some(&t.automation[0].curve)), (0, 300.0, 5000.0));
    assert_eq!(lane_curve(t.mix_curves[engine::MIX_GAIN].as_ref()).0, 1);
    assert!(song.lanes[0].as_ref().unwrap().track.is_none());
    assert!(song.render(RATE, 1).is_ok());
}

#[test]
fn auto_clear_and_auto_list() {
    let song = song(
        "t:fx(\"lowpass\", { cutoff = 3000 })\n\
         t:auto(\"cutoff\", { {0, 300}, {1, 900} }):auto(\"gain\", { {0, 0.2}, {1, 1} })\n\
         local list = t:auto_list()\n\
         assert(#list == 2 and list[1].param == \"lowpass.cutoff\" and list[1].len == 1)\n\
         assert(list[2].points[2][2] == 1 and list[2].points[2][3] == \"linear\")\n\
         t:auto_clear(\"cutoff\")\n\
         assert(#t:auto_list() == 1)\n\
         -- El hueco se vuelve a usar.\n\
         t:auto(\"pan\", { {0, 0}, {1, 1} })\n\
         t:auto_clear()",
    )
    .unwrap();
    let t = &song.tracks[0];
    assert!(t.lanes.is_empty());
    assert!(t.automation.is_empty());
    assert!(t.mix_curves.iter().all(Option::is_none));
    assert!(song.lanes.iter().all(Option::is_none));
    // Volver a poner el mismo parámetro reusa su hueco.
    let song = self::song(
        "t:auto(\"gain\", { {0, 0.2}, {1, 1} })\nt:auto(\"gain\", { {0, 0.5}, {1, 1} })",
    )
    .unwrap();
    assert_eq!(song.lanes.len(), 1);
}

#[test]
fn errors_point_at_the_line_and_keep_what_sounds() {
    let error = |source: &str| song(source).err().expect("debería fallar");
    let e = error("t:auto(\"zzz\", { {0, 1} })");
    assert!(
        e.starts_with("song.lua:4: auto(\"zzz\"): la pista no tiene el parámetro \"zzz\""),
        "{e}"
    );
    assert!(e.contains("gain, pan, send.delay"), "{e}");
    let e =
        error("t:fx(\"lowpass\", { cutoff = 3000 }):auto(\"cutoff\", { {1, 300}, {0.5, 900} })");
    assert!(e.contains("en orden"), "{e}");
    let e = error("t:fx(\"lowpass\", { cutoff = 3000 }):auto(\"cutoff\", { {0, 5}, {1, 900} })");
    assert!(
        e.contains("el punto 1 vale 5, fuera del rango del parámetro"),
        "{e}"
    );
    assert!(
        error("t:auto(\"gain\", { {0, 1, \"curva\"} })").contains("la curva \"curva\" no existe")
    );
    assert!(
        error("t:auto(\"gain\", { {0, 1} }, { mode = \"pingpong\" })")
            .contains("mode = \"pingpong\" no existe")
    );
    assert!(error("t:auto(\"gain\", { {0, 1} }, { len = 0 })").contains("len va de más de 0"));
    assert!(
        error("t:auto(\"gain\", { {0, 1} }, { lenght = 2 })")
            .contains("opción desconocida \"lenght\"")
    );
    assert!(error("t:auto(\"gain\", { })").contains("faltan los puntos"));
    assert!(error("t:auto(\"gain\", { 3 })").contains("el punto 1 es {posición"));
    assert!(error("t:auto(\"dur\", { {0, 1} })").contains("no tiene el parámetro \"dur\""));
    // Más de 32 lanes: error claro.
    let many: String = (0..33)
        .map(|i| format!("lane({{ {{0, {i}}} }})\n"))
        .collect();
    let e = error(&many);
    assert!(
        e.contains("song.lua:36: lane: no caben más lanes: hay 32 como mucho"),
        "{e}"
    );
    // Lo que sonaba sigue: el modelo del host es el de antes.
    let mut host = ScriptHost::new(RATE);
    load(&mut host, "t:auto(\"gain\", { {0, 0.2}, {1, 1} })").unwrap();
    assert!(load(&mut host, "t:auto(\"gain\", { {0, 9} })").is_err());
    let kept = host.song().unwrap();
    assert_eq!(kept.lanes.len(), 1);
    assert_eq!(kept.lanes[0].as_ref().unwrap().lane.points()[0].value, 0.2);
}

#[test]
fn freezing_knows_how_long_a_lane_takes_to_repeat() {
    // Una lane en loop de 3 compases en un efecto (lo que se congela va
    // antes del volumen): la vuelta congelada es de 3 (contra la de 1 del
    // patrón) y no avisa.
    let song = song(
        "t:fx(\"lowpass\", { cutoff = 3000 })\n\
         t:auto(\"cutoff\", { {0, 300}, {1.5, 3000}, {3, 300} }, { len = 3 })",
    )
    .unwrap();
    let frozen = song.render_frozen(RATE, "t").unwrap();
    assert_eq!(frozen.bars, 3);
    assert!(frozen.warnings.is_empty(), "{:?}", frozen.warnings);
    // Una once no se repite: avisa.
    let song = self::song(
        "t:fx(\"lowpass\", { cutoff = 3000 })\n\
         t:auto(\"cutoff\", { {0, 300}, {1, 3000} }, { mode = \"once\" })",
    )
    .unwrap();
    let frozen = song.render_frozen(RATE, "t").unwrap();
    assert!(
        frozen.warnings[0].contains("una lane que no se repite"),
        "{:?}",
        frozen.warnings
    );
}

#[test]
fn where_the_points_are_written_or_why_not() {
    // En una línea, y en una cadena de métodos con la tabla en varias.
    let song = song(
        "t:fx(\"lowpass\", { cutoff = 3000 })\n\
         t:auto(\"cutoff\", { {0, 200, \"exp\"}, {2, 8000} }, { len = 4 })\n\
         track(\"u\", b):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n  \
           :auto(\"gain\", {\n    {0, 0.2},\n    {1, 1, \"step\"},\n  })\n\
         local p = { {0, 0}, {1, 1} }\n\
         t:auto(\"pan\", p)\n\
         t:auto(\"pitch\", { {0, 1 + 1} })",
    )
    .unwrap();
    let span = |i: usize| song.lanes[i].as_ref().unwrap().written.clone();
    let cutoff = span(0).unwrap();
    // Fila 4 (desde 0) del texto, con las tres líneas de cabecera.
    assert_eq!(cutoff.start.0, 4);
    let gain = span(1).unwrap();
    assert_eq!((gain.start.0, gain.end.0), (6, 9));
    let pan = span(2).unwrap_err();
    assert!(
        pan.contains("no se encuentra la llamada escrita") || pan.contains("no es literal"),
        "{pan}"
    );
    assert!(
        pan.contains("escríbela tal cual dentro de la llamada"),
        "{pan}"
    );
    let pitch = span(3).unwrap_err();
    assert!(
        pitch.contains("no es literal (lleva variables, cuentas o comentarios)"),
        "{pitch}"
    );
}

#[test]
fn reading_and_writing_a_points_table() {
    let text = "{ {0, 200, \"exp\"}, {2.5, 8000}, {4, 0.125, 'step'}, }";
    let points = script::parse_points(text).unwrap();
    assert_eq!(
        points,
        [
            (0.0, 200.0, LaneCurve::Exp),
            (2.5, 8000.0, LaneCurve::Linear),
            (4.0, 0.125, LaneCurve::Step)
        ]
    );
    assert_eq!(
        script::points_text(&points, text),
        "{ {0, 200, \"exp\"}, {2.5, 8000}, {4, 0.125, \"step\"} }"
    );
    // En varias líneas, con su sangría.
    let multi = "{\n      {0, 1},\n      {1, 2, \"smooth\"},\n    }";
    let points = script::parse_points(multi).unwrap();
    assert_eq!(
        script::points_text(&points, multi),
        "{\n      {0, 1},\n      {1, 2, \"smooth\"},\n    }"
    );
    assert!(script::parse_points("{ {0, x} }").is_none());
    assert!(script::parse_points("{ {0, 1} } -- y más").is_none());
    assert!(script::parse_points("{ }").is_none());
    assert_eq!(script::number_text(0.0625), "0.0625");
    assert_eq!(script::number_text(2.0), "2");
    assert_eq!(script::number_text(-0.00001), "0");
}
