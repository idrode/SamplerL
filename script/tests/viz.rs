//! Las visualizaciones desde Lua: `pista:scope{...}` y compañía,
//! `master():spectrum{...}`, `viz_off()`; qué fuentes enciende el lote; y
//! los errores con línea, sin tocar las de antes.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{VIZ_MASTER, VIZ_SOURCES, new_engine, offline};
use script::{ScriptHost, Song, VizDef};

const RATE: u32 = 48_000;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("viz");
        std::fs::create_dir_all(&dir).unwrap();
        offline::write_wav(dir.join("b.wav"), &[0.3f32; 4800], 1, RATE).unwrap();
        dir
    })
    .clone()
}

const HEAD: &str = "bpm(120)\nlocal b = sample(\"b.wav\"):slice_grid(2)\n\
                    local t = track(\"t\", b):steps(\"1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .\")\n\
                    track(\"u\", b):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n";

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

fn def(kind: &'static str, track: Option<&str>, size: u16, bands: usize) -> VizDef {
    VizDef {
        kind,
        track: track.map(str::to_string),
        size,
        bands,
    }
}

#[test]
fn the_calls_end_up_in_the_model() {
    let song = song(
        "t:scope{ size = 8 }:punchcard()\n\
         master():spectrum{ bands = 48 }\n\
         t:scope{ size = 3 }",
    )
    .unwrap();
    assert_eq!(
        song.viz,
        [
            def("punchcard", Some("t"), 6, 32),
            def("spectrum", None, 6, 48),
            def("scope", Some("t"), 3, 32),
        ]
    );
}

#[test]
fn viz_off_takes_them_away() {
    let song = song("t:scope():spectrum()\nmaster():scope()\nt:viz_off()").unwrap();
    assert_eq!(song.viz, [def("scope", None, 6, 32)]);
    let song = self::song("t:scope()\nmaster():scope():viz_off()").unwrap();
    assert_eq!(song.viz, [def("scope", Some("t"), 6, 32)]);
    let song = self::song("t:scope()\nmaster():scope()\nviz_off()\nt:pianoroll()").unwrap();
    assert_eq!(song.viz, [def("pianoroll", Some("t"), 6, 32)]);
}

#[test]
fn the_batch_switches_on_the_sources_of_the_audio_ones() {
    let mut host = ScriptHost::new(RATE);
    let song = load(
        &mut host,
        "track(\"u\"):scope()\nt:punchcard()\nmaster():spectrum()",
    )
    .unwrap();
    let mut slots: [Option<String>; engine::MAX_TRACKS] = Default::default();
    for name in ["t", "u"] {
        slots[host.track_slot(name).unwrap()] = Some(name.to_string());
    }
    let on = script::viz_flags(&song, &slots);
    let u = host.track_slot("u").unwrap();
    let mut want = [false; VIZ_SOURCES];
    want[u] = true;
    want[VIZ_MASTER] = true;
    // `t` solo tiene una de rejilla: su cola no se enciende.
    assert_eq!(on, want);
}

#[test]
fn errors_point_at_the_line_and_keep_the_ones_before() {
    let error = |source: &str| song(source).err().expect("debería fallar");
    assert_eq!(
        error("t:scope{ size = 0 }"),
        "song.lua:5: scope(): size son filas, de 1 a 40"
    );
    assert!(error("t:spectrum{ bands = 2 }").contains("bands va de 4 a 128"));
    assert!(
        error("t:scope{ alto = 3 }").contains("opción desconocida \"alto\" (admite: size, bands)")
    );
    assert!(error("master():punchcard()").contains("el master no tiene: pista:punchcard()"));
    let mut host = ScriptHost::new(RATE);
    load(&mut host, "t:scope()").unwrap();
    assert!(load(&mut host, "t:scope{ size = 99 }").is_err());
    assert_eq!(host.song().unwrap().viz, [def("scope", Some("t"), 6, 32)]);
}
