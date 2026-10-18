//! Secciones y línea de tiempo desde Lua: cada sección suena en su compás,
//! una sección sin `arrange` es lo de siempre, recargar conserva la
//! posición, el render offline es lo que suena en vivo y los errores dicen
//! la línea.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;
/// A 120 BPM, un compás.
const BAR: usize = 96_000;

/// `a.wav` y `b.wav`: golpes de continua a 0,5 y 0,25 (1000 frames).
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sections");
        std::fs::create_dir_all(&dir).unwrap();
        offline::write_wav(dir.join("a.wav"), &[0.5; 1000], 1, RATE).unwrap();
        offline::write_wav(dir.join("b.wav"), &[0.25; 1000], 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.set_seed(Some(7));
    host.load(&mut controller, source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

/// Nivel a mitad del golpe de cada compás.
fn levels(audio: &[f32], bars: usize) -> Vec<f32> {
    (0..bars).map(|b| audio[2 * (b * BAR + 500)]).collect()
}

const BASE: &str = r#"bpm(120)
local a = sample("a.wav"):slice(1)
local b = sample("b.wav"):slice(1)
track("x", a):steps("1 . . .  . . . .  . . . .  . . . .")
"#;

const SONG: &str = r#"
section("intro", function() end)
section("drop", function()
  track("x"):mute()
  track("y", b):steps("1 . . .  . . . .  . . . .  . . . .")
end)
arrange({ {"intro", 2}, {"drop", 1}, {"intro", 1} })
"#;

#[test]
fn each_section_sounds_in_its_bar() {
    let song = song(&format!("{BASE}{SONG}")).unwrap();
    assert_eq!(song.timeline, [(0, 2), (1, 1), (0, 1)]);
    assert_eq!(song.length_bars(1), 4);
    let audio = song.render(RATE, 5).unwrap();
    assert_eq!(levels(&audio, 5), [0.5, 0.5, 0.25, 0.5, 0.5]);
    let first = (2 * BAR..3 * BAR).find(|&f| audio[2 * f] != 0.0).unwrap();
    assert_eq!(first, 2 * BAR + 1, "la sección entra justo en su compás");
}

#[test]
fn a_single_section_without_arrange_is_what_it_always_was() {
    let plain = song(BASE).unwrap().render(RATE, 3).unwrap();
    let sectioned = song(&format!("{BASE}section(\"todo\", function() end)\n")).unwrap();
    assert!(sectioned.warnings.is_empty(), "{:?}", sectioned.warnings);
    assert!(sectioned.render(RATE, 3).unwrap() == plain);
    // Con cambios dentro de la sección, como si se escribieran fuera.
    let inside = song(&format!(
        "{BASE}section(\"s\", function() track(\"x\"):steps(\"1 . 1 .  . . . .  . . . .  . . . .\") end)\n"
    ))
    .unwrap();
    let outside = song(&format!(
        "{BASE}track(\"x\"):steps(\"1 . 1 .  . . . .  . . . .  . . . .\")\n"
    ))
    .unwrap();
    assert!(inside.render(RATE, 2).unwrap() == outside.render(RATE, 2).unwrap());
}

#[test]
fn every_counts_from_the_start_of_its_section() {
    // `every(2)` en una sección que empieza en el compás 1: sus compases 0
    // y 2 (los 1 y 3 de la canción) llevan el cambio.
    let song = song(&format!(
        r#"{BASE}
section("uno", function() track("x"):steps(". . . .  . . . .  . . . .  . . . .") end)
section("dos", function()
  track("x"):every(2, function(c) c:gain(0.5) end)
end)
arrange({{ {{"uno", 1}}, {{"dos", 4}} }})
"#
    ))
    .unwrap();
    let audio = song.render(RATE, 5).unwrap();
    assert_eq!(levels(&audio, 5), [0.0, 0.25, 0.5, 0.25, 0.5]);
}

/// La canción en vivo: el anfitrión carga en un motor que va renderizando.
struct Live {
    host: ScriptHost,
    controller: engine::Controller,
    engine: engine::Engine,
}

impl Live {
    fn new(source: &str) -> Self {
        let (mut controller, engine) = new_engine(RATE);
        let mut host = ScriptHost::new(RATE);
        host.set_seed(Some(7));
        host.load(&mut controller, source, "song.lua", &dir())
            .unwrap();
        controller.start().unwrap();
        Live {
            host,
            controller,
            engine,
        }
    }

    fn render(&mut self, frames: usize) -> Vec<f32> {
        offline::render(&mut self.engine, frames, 2)
    }

    fn reload(&mut self, source: &str) -> Vec<String> {
        self.host
            .load(&mut self.controller, source, "song.lua", &dir())
            .unwrap()
            .warnings
            .clone()
    }
}

#[test]
fn the_offline_render_is_what_plays_live() {
    let source = format!("{BASE}{SONG}");
    let offline = song(&source).unwrap().render(RATE, 6).unwrap();
    let mut live = Live::new(&source);
    let played = live.render(offline.len() / 2);
    assert!(played == offline, "el render no coincide con lo que suena");
}

#[test]
fn reloading_mid_song_keeps_the_section_and_the_bar() {
    let source = format!("{BASE}{SONG}");
    let mut live = Live::new(&source);
    let mut audio = live.render(2 * BAR + BAR / 3);
    // En el compás 2 (la sección «drop»): recargar lo mismo no mueve nada.
    assert!(live.reload(&source).is_empty());
    audio.extend(live.render(3 * BAR - BAR / 3));
    let reference = Live::new(&source).render(5 * BAR);
    assert!(audio == reference, "recargar ha movido la canción");
    let place = live.controller.status().section.unwrap();
    assert_eq!((place.entry, place.bar), (0, 0), "dio la vuelta");
}

#[test]
fn reloading_without_the_current_section_goes_back_to_the_start() {
    // «drop» dura dos compases: a mitad del primero, el siguiente también
    // es suyo.
    let long_drop = SONG.replace("{\"drop\", 1}", "{\"drop\", 2}");
    let mut live = Live::new(&format!("{BASE}{long_drop}"));
    live.render(2 * BAR + BAR / 2);
    // Suena «drop»; la versión nueva ya no la tiene.
    let warnings = live.reload(&format!(
        "{BASE}section(\"intro\", function() end)\nsection(\"otra\", function() track(\"x\"):mute() end)\narrange({{ {{\"otra\", 1}}, {{\"intro\", 2}} }})\n"
    ));
    assert!(
        warnings.iter().any(|w| w.contains("\"drop\" ya no existe")),
        "{warnings:?}"
    );
    let audio = live.render(BAR / 2 + 2 * BAR);
    let at = |b: usize| audio[2 * (BAR / 2 + b * BAR + 500)];
    assert_eq!(
        [at(0), at(1)],
        [0.0, 0.5],
        "vuelve al principio (otra, intro)"
    );
    assert!(live.controller.status().section_lost);
}

#[test]
fn notices() {
    let several = song(&format!(
        "{BASE}section(\"a\", function() end)\nsection(\"b\", function() end)\n"
    ))
    .unwrap();
    assert!(
        several
            .warnings
            .iter()
            .any(|w| w.contains("suena solo \"a\"")),
        "{:?}",
        several.warnings
    );
    let long = song(&format!(
        "{BASE}section(\"a\", function() end)\narrange({{ {{\"a\", 200}}, {{\"a\", 100}} }})\n"
    ))
    .unwrap();
    assert!(
        long.warnings.iter().any(|w| w.contains("300 compases")),
        "{:?}",
        long.warnings
    );
    assert_eq!(long.length_bars(1), 256);
}

#[test]
fn errors_say_the_line() {
    let eight: String = (0..8)
        .map(|i| format!("  track(\"t{i}\", a):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n"))
        .collect();
    let cases = [
        (
            "section(\"a\", function() end)\narrange({ {\"b\", 2} })".to_string(),
            "la sección \"b\" no existe (hay: a)",
            6,
        ),
        (
            "section(\"a\", function() end)\narrange({ {\"a\", 0} })".to_string(),
            "van de 1 a 256",
            6,
        ),
        (
            "section(\"a\", function() end)\narrange({ {\"a\", 1.5} })".to_string(),
            "van de 1 a 256",
            6,
        ),
        ("arrange({ {\"a\", 2} })".to_string(), "no hay secciones", 5),
        (
            "section(\"a\", function()\n  bpm(100)\nend)".to_string(),
            "bpm() es de toda la canción",
            6,
        ),
        (
            "section(\"a\", function() end)\nsection(\"a\", function() end)".to_string(),
            "ya hay una sección",
            6,
        ),
        (
            format!("section(\"a\", function()\n{eight}end)"),
            "más de 8 pistas",
            13,
        ),
        (
            "arrange({ \"a\" })".to_string(),
            "es {\"sección\", compases}",
            5,
        ),
    ];
    for (code, message, line) in cases {
        let err = song(&format!("{BASE}{code}\n")).err().unwrap_or_default();
        assert!(err.contains(message), "{code}: {err}");
        assert!(err.contains(&format!("song.lua:{line}:")), "{code}: {err}");
    }
}
