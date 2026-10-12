//! Interfaz con el backend de test de ratatui: se dibuja en un buffer y se
//! comprueba el texto. El motor se mueve con render offline.

use std::path::{Path, PathBuf};

use engine::{Engine, new_engine, offline};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Color;
use ratatui_textarea::{Input, Key};

use crate::app::{App, Flow};
use crate::model::{TrackView, TransportView, ViewModel};
use crate::ui;

pub const RATE: u32 = 48_000;
/// A 120 BPM: 6000 frames por paso.
pub const STEP: usize = 6000;

pub const SONG: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
"#;

/// Carpeta propia del test con `break.wav` (dos cortes) y `song.lua`.
pub fn fixture(test: &str, song: Option<&str>) -> PathBuf {
    // `CARGO_TARGET_TMPDIR` solo existe en tests de integración.
    let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../target/tmp"))
        .join(format!("tui_{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
    let path = dir.join("song.lua");
    if let Some(song) = song {
        std::fs::write(&path, song).unwrap();
    }
    path
}

pub fn open(test: &str, song: Option<&str>) -> (PathBuf, App, Engine) {
    let path = fixture(test, song);
    let (controller, engine) = new_engine(RATE);
    let app = App::open(&path, controller, RATE).unwrap();
    (path, app, engine)
}

/// Dibuja la interfaz entera y devuelve el buffer.
pub fn draw(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    terminal.backend().buffer().clone()
}

/// Dibuja un widget suelto.
pub fn draw_widget(widget: impl ratatui::widgets::Widget, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| frame.render_widget(widget, frame.area()))
        .unwrap();
    terminal.backend().buffer().clone()
}

pub fn text(buffer: &Buffer) -> String {
    let area = buffer.area;
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Posición (x, y) de la primera aparición de `needle`.
pub fn find(buffer: &Buffer, needle: &str) -> Option<(u16, u16)> {
    text(buffer).lines().enumerate().find_map(|(y, line)| {
        line.find(needle)
            .map(|byte| (line[..byte].chars().count() as u16, y as u16))
    })
}

pub fn key(c: char) -> Input {
    Input {
        key: Key::Char(c),
        ..Input::default()
    }
}

pub fn ctrl(c: char) -> Input {
    Input {
        key: Key::Char(c),
        ctrl: true,
        ..Input::default()
    }
}

fn type_text(app: &mut App, text: &str) {
    for c in text.chars() {
        let input = if c == '\n' {
            Input {
                key: Key::Enter,
                ..Input::default()
            }
        } else {
            key(c)
        };
        app.handle(input);
    }
}

/// Selecciona todo el texto del editor y lo sustituye.
fn replace_all(app: &mut App, text: &str) {
    app.editor.select_all();
    app.editor.cut();
    type_text(app, text);
}

#[test]
fn shows_the_script_and_the_engine_state() {
    let (_, mut app, mut engine) = open("state", Some(SONG));
    // Tres pasos de audio: el último disparado es el 2.
    offline::render(&mut engine, 2 * STEP + 100, 2);
    app.tick();
    let screen = text(&draw(&mut app, 120, 20));

    assert!(screen.contains("song.lua"), "{screen}");
    assert!(screen.contains("track(\"drums\", amen)"), "{screen}");
    assert!(screen.contains("▶ en marcha"), "{screen}");
    assert!(screen.contains("120.0 BPM"), "{screen}");
    assert!(screen.contains("compás 0   paso 02"), "{screen}");
    // En el paso 2, "drums" tocó el corte 2 y "kick" sigue con el 1.
    assert!(screen.contains("›drums"), "{screen}");
    assert!(
        screen.contains("▶02  2 cortes  ciclo 1  pista 1"),
        "{screen}"
    );
    assert!(
        screen.contains("▶01  1 cortes  ciclo 1  pista 2"),
        "{screen}"
    );
    assert!(
        screen.contains("cargado: entra en el siguiente compás"),
        "{screen}"
    );
}

#[test]
fn ctrl_s_saves_and_loads() {
    let (path, mut app, mut engine) = open("save", Some(SONG));
    type_text(&mut app, "-- comentario\n");
    let screen = text(&draw(&mut app, 100, 16));
    assert!(
        screen.contains("song.lua ●"),
        "debería marcar cambios:\n{screen}"
    );

    app.handle(ctrl('s'));
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.starts_with("-- comentario\n"), "{saved}");
    let screen = text(&draw(&mut app, 100, 16));
    assert!(!screen.contains("song.lua ●"), "{screen}");

    // El cambio guardado entra en el motor: sigue sonando bien.
    offline::render(&mut engine, STEP + 100, 2);
    app.tick();
    assert!(app.view().transport.running);
}

#[test]
fn lua_errors_show_in_the_editor_and_keep_playing() {
    let (path, mut app, mut engine) = open("error", Some(SONG));
    let bad = "bpm(120)\nlocal amen = sample(\"break.wav\"):slice(2)\ntrack(\"drums\", amen):steps(\"1 . 9 .  . . . .  . . . .  . . . .\")\n";
    replace_all(&mut app, bad);
    app.handle(ctrl('s'));

    let buffer = draw(&mut app, 120, 16);
    let screen = text(&buffer);
    assert!(
        screen.contains("✗ song.lua:3: steps: el corte 9 no existe"),
        "{screen}"
    );
    assert!(
        screen.contains("✗ error en la línea 3 · Ctrl-G"),
        "{screen}"
    );
    assert!(
        screen.contains("error: sigue sonando lo anterior"),
        "{screen}"
    );
    // En rojo, en el borde del editor.
    let (x, y) = find(&buffer, "✗").unwrap();
    assert_eq!(buffer[(x, y)].fg, Color::Red);
    assert_eq!(buffer[(0, 0)].fg, Color::Red);
    // El archivo se guardó aunque sea inválido.
    assert_eq!(
        std::fs::read_to_string(&path).unwrap().trim_end(),
        bad.trim_end()
    );

    // Sigue sonando lo anterior: las dos pistas.
    offline::render(&mut engine, STEP + 100, 2);
    app.tick();
    let view = app.view();
    assert_eq!(view.tracks.len(), 2);
    assert!(view.transport.running);

    // Ctrl-G lleva el cursor a la línea del error.
    app.handle(ctrl('g'));
    assert_eq!(app.editor.cursor().0, 2);

    // Al corregirlo, el error desaparece.
    replace_all(&mut app, SONG);
    app.handle(ctrl('s'));
    let screen = text(&draw(&mut app, 120, 16));
    assert!(!screen.contains('✗'), "{screen}");
}

#[test]
fn missing_script_opens_a_template_and_quit_asks_if_unsaved() {
    let (path, mut app, _engine) = open("template", None);
    assert!(!path.exists());
    let screen = text(&draw(&mut app, 100, 16));
    assert!(screen.contains("song.lua ●"), "{screen}");
    // La plantilla usa amen.wav, que no existe: error en el editor.
    assert!(screen.contains("amen.wav"), "{screen}");

    assert_eq!(app.handle(ctrl('q')), Flow::Continue);
    assert!(app.view().message.contains("Ctrl-Q otra vez"));
    assert_eq!(app.handle(ctrl('q')), Flow::Quit);

    // Sin cambios, sale a la primera.
    let (_, mut app, _engine) = open("quit", Some(SONG));
    assert_eq!(app.handle(ctrl('q')), Flow::Quit);
}

#[test]
fn ctrl_t_stops_and_starts_the_transport() {
    let (_, mut app, mut engine) = open("transport", Some(SONG));
    offline::render(&mut engine, 512, 2);
    app.tick();
    app.handle(ctrl('t'));
    offline::render(&mut engine, 512, 2);
    app.tick();
    let screen = text(&draw(&mut app, 100, 16));
    assert!(screen.contains("■ parado"), "{screen}");
    app.handle(ctrl('t'));
    offline::render(&mut engine, 512, 2);
    app.tick();
    assert!(app.view().transport.running);
}

#[test]
fn engine_panel_only_reads_the_view_model() {
    let view = ViewModel {
        transport: TransportView {
            running: true,
            bpm: 174.0,
            bar: 7,
            step: Some(13),
        },
        tracks: vec![TrackView {
            name: "amen".into(),
            slot: Some(2),
            slices: vec![0..10, 10..20, 20..30],
            cycle: 4,
            playing_slice: Some(1),
            level: 0.5,
        }],
        selected: Some(0),
        ..ViewModel::default()
    };
    let screen = text(&draw_widget(ui::EnginePanel(&view), 80, 6));
    assert!(
        screen.contains("174.0 BPM   compás 7   paso 13"),
        "{screen}"
    );
    assert!(screen.contains("›amen"), "{screen}");
    assert!(
        screen.contains("›amen      ████░░░░ ▶02  3 cortes  ciclo 4  pista 3"),
        "{screen}"
    );
}

#[test]
#[ignore = "solo para ver la pantalla: cargo test -p tui pantalla -- --ignored --nocapture"]
fn pantalla() {
    let (_, mut app, mut engine) = open("pantalla", Some(SONG));
    offline::render(&mut engine, 5 * STEP + 100, 2);
    app.tick();
    println!("{}", text(&draw(&mut app, 120, 24)));
    replace_all(
        &mut app,
        "bpm(120)\nlocal a = sample(\"break.wav\"):slice(2)\ntrack(\"d\", a):steps(\"1 . 9\")\n",
    );
    app.handle(ctrl('s'));
    println!("{}", text(&draw(&mut app, 120, 24)));
}
