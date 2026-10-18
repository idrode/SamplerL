//! Variantes A/B: la A es lo escrito y la B lo que cambia `variant()`,
//! de una pista suelta o de una sección; alternar entra al compás siguiente
//! y, con secciones, sigue por la misma sección y el mismo compás.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;
const BAR: usize = 96_000;

/// `a.wav`: un golpe de continua a 0,5 (1000 frames).
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("variants");
        std::fs::create_dir_all(&dir).unwrap();
        offline::write_wav(dir.join("a.wav"), &[0.5; 1000], 1, RATE).unwrap();
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

/// Evaluado sin cargar: la A con la B dentro (`alternate`).
fn evaluated(source: &str) -> Song {
    let mut host = ScriptHost::new(RATE);
    host.set_seed(Some(3));
    host.eval(source, "song.lua", &dir()).unwrap()
}

fn levels(audio: &[f32], bars: usize) -> Vec<f32> {
    (0..bars).map(|b| audio[2 * (b * BAR + 500)]).collect()
}

const BASE: &str = r#"bpm(120)
local a = sample("a.wav"):slice(1)
track("x", a):steps("1 . . .  . . . .  . . . .  . . . .")
"#;

const TRACK_B: &str = r#"variant("x", "B", function(t) t:gain(0.5) end)
"#;

#[test]
fn a_is_what_is_written_and_b_what_variant_changes() {
    let with = evaluated(&format!("{BASE}{TRACK_B}"));
    assert_eq!(with.variant, Some('A'));
    let b = with.alternate.as_deref().unwrap();
    assert_eq!(b.variant, Some('B'));
    // La A, bit a bit lo de sin variantes; la B, como escribir el cambio.
    let plain = song(BASE).unwrap().render(RATE, 2).unwrap();
    assert!(with.render(RATE, 2).unwrap() == plain);
    let written = song(&format!("{BASE}track(\"x\"):gain(0.5)\n")).unwrap();
    assert!(b.render(RATE, 2).unwrap() == written.render(RATE, 2).unwrap());
    assert_eq!(levels(&b.render(RATE, 1).unwrap(), 1), [0.25]);
}

struct Live {
    host: ScriptHost,
    controller: engine::Controller,
    engine: engine::Engine,
}

impl Live {
    fn new(source: &str) -> Self {
        let (mut controller, engine) = new_engine(RATE);
        let mut host = ScriptHost::new(RATE);
        host.set_seed(Some(3));
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

    fn toggle(&mut self) -> char {
        self.host
            .toggle_variant(&mut self.controller)
            .unwrap()
            .variant
            .unwrap()
    }
}

#[test]
fn toggling_enters_on_the_next_bar() {
    let mut live = Live::new(&format!("{BASE}{TRACK_B}"));
    let mut audio = live.render(BAR + BAR / 2);
    assert_eq!(live.toggle(), 'B');
    audio.extend(live.render(BAR / 2 + 2 * BAR));
    assert_eq!(levels(&audio, 4), [0.5, 0.5, 0.25, 0.25]);
    // El cambio, en el frame del compás.
    assert_eq!(audio[2 * (2 * BAR - 1)], 0.0);
    // Otra vez, la A; y recargar conserva la que suena.
    assert_eq!(live.toggle(), 'A');
    live.toggle();
    let source = format!("{BASE}{TRACK_B}");
    live.host
        .load(&mut live.controller, &source, "song.lua", &dir())
        .unwrap();
    assert_eq!(live.host.variant(), Some('B'));
}

const SECTIONS: &str = r#"
section("intro", function() end)
section("drop", function() track("x"):steps("1 . . .  . . . .  1 . . .  . . . .") end)
variant("drop", "B", function() track("x"):gain(0.25) end)
arrange({ {"intro", 1}, {"drop", 4} })
"#;

#[test]
fn with_sections_it_keeps_the_section_and_the_bar() {
    let source = format!("{BASE}{SECTIONS}");
    let mut live = Live::new(&source);
    let mut audio = live.render(2 * BAR + BAR / 2);
    // En el segundo compás del drop: la B entra en el tercero.
    assert_eq!(live.toggle(), 'B');
    audio.extend(live.render(BAR / 2 + 2 * BAR));
    let place = live.controller.status().section.unwrap();
    assert_eq!(
        (place.entry, place.bar),
        (1, 3),
        "sigue por el compás del drop"
    );
    assert_eq!(levels(&audio, 5), [0.5, 0.5, 0.5, 0.125, 0.125]);
    // La intro no tiene variante B: igual en las dos.
    let song = evaluated(&source);
    let a = song.render(RATE, 1).unwrap();
    let b = song.alternate.unwrap().render(RATE, 1).unwrap();
    assert!(a == b);
}

#[test]
fn without_variants_there_is_nothing_to_toggle() {
    let mut live = Live::new(BASE);
    assert_eq!(live.host.variant(), None);
    let err = live
        .host
        .toggle_variant(&mut live.controller)
        .err()
        .unwrap();
    assert!(err.to_string().contains("no hay variantes"));
}

#[test]
fn errors_say_the_line() {
    let nine: String = (0..8)
        .map(|i| format!("  track(\"n{i}\", a):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n"))
        .collect();
    let cases = [
        (
            "variant(\"x\", \"C\", function() end)".to_string(),
            "la variante es \"A\" o \"B\"",
            4,
        ),
        (
            "variant(\"nada\", \"B\", function() end)".to_string(),
            "no hay ninguna sección ni pista \"nada\"",
            4,
        ),
        (
            "variant(\"x\", \"B\", function() end)\nvariant(\"x\", \"B\", function() end)"
                .to_string(),
            "ya está definida",
            5,
        ),
        (
            "section(\"s\", function()\n  variant(\"x\", \"B\", function() end)\nend)".to_string(),
            "variant() es de toda la canción",
            5,
        ),
        (
            format!("variant(\"x\", \"B\", function(t)\n{nine}end)"),
            "más de 8 pistas",
            12,
        ),
    ];
    for (code, message, line) in cases {
        let err = song(&format!("{BASE}{code}\n")).err().unwrap_or_default();
        assert!(err.contains(message), "{code}: {err}");
        assert!(err.contains(&format!("song.lua:{line}:")), "{code}: {err}");
    }
}
