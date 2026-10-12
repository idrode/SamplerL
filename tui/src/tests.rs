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
    let screen = text(&draw(&mut app, 120, 40));

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
    let screen = text(&draw(&mut app, 100, 40));
    assert!(
        screen.contains("song.lua ●"),
        "debería marcar cambios:\n{screen}"
    );

    app.handle(ctrl('s'));
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(saved.starts_with("-- comentario\n"), "{saved}");
    let screen = text(&draw(&mut app, 100, 40));
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

    let buffer = draw(&mut app, 120, 40);
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
    let screen = text(&draw(&mut app, 120, 40));
    assert!(!screen.contains('✗'), "{screen}");
}

#[test]
fn missing_script_opens_a_template_and_quit_asks_if_unsaved() {
    let (path, mut app, _engine) = open("template", None);
    assert!(!path.exists());
    let screen = text(&draw(&mut app, 100, 40));
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
    let screen = text(&draw(&mut app, 100, 40));
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
            ..TrackView::default()
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
    println!("{}", text(&draw(&mut app, 130, 40)));
}

fn waveform_view(playing: Option<usize>) -> ViewModel {
    use crate::model::Waveform;
    use std::sync::Arc;
    // Dos bloques: 0.5 en el primer corte y -0.5 en el segundo.
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(-0.5);
    let sample = engine::Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap();
    ViewModel {
        tracks: vec![TrackView {
            name: "drums".into(),
            sample_name: "break.wav".into(),
            sample_frames: 6000,
            sample_rate: RATE,
            waveform: Arc::new(Waveform::of(&sample)),
            slices: vec![0..3000, 3000..6000],
            playing_slice: playing,
            ..TrackView::default()
        }],
        selected: Some(0),
        ..ViewModel::default()
    }
}

fn is_braille(symbol: &str) -> bool {
    symbol
        .chars()
        .next()
        .is_some_and(|c| ('\u{2801}'..='\u{28FF}').contains(&c))
}

#[test]
fn sample_panel_draws_the_waveform_in_braille_with_slices() {
    let view = waveform_view(Some(1));
    // 60 celdas útiles de ancho (62 con el borde).
    let buffer = draw_widget(ui::SamplePanel(&view), 62, 12);
    let screen = text(&buffer);
    assert!(
        screen.contains("sample · drums · break.wav · 0.12 s · 2 cortes"),
        "{screen}"
    );

    // Números de los cortes: el 1 al principio y el 2 en la mitad (3000/6000).
    assert_eq!(buffer[(1, 1)].symbol(), "1");
    assert_eq!(buffer[(1 + 30, 1)].symbol(), "2");
    assert_eq!(buffer[(31, 1)].fg, Color::Yellow);

    // Forma de onda en braille: el bloque positivo arriba en el corte 1 y el
    // negativo abajo en el corte 2.
    let braille = |x0: u16, x1: u16, y0: u16, y1: u16| {
        (x0..x1)
            .flat_map(|x| (y0..y1).map(move |y| (x, y)))
            .filter(|&p| is_braille(buffer[p].symbol()))
            .collect::<Vec<_>>()
    };
    assert!(
        !braille(3, 8, 2, 5).is_empty(),
        "sin bloque positivo:\n{screen}"
    );
    assert!(
        !braille(33, 38, 7, 11).is_empty(),
        "sin bloque negativo:\n{screen}"
    );
    // El corte que suena (el 2) va resaltado; el resto, en gris.
    assert!(
        braille(33, 38, 7, 11)
            .iter()
            .all(|&p| buffer[p].fg == Color::Cyan)
    );
    assert!(
        braille(3, 8, 2, 5)
            .iter()
            .all(|&p| buffer[p].fg == Color::Gray)
    );

    // Sin corte sonando, nada resaltado.
    let buffer = draw_widget(ui::SamplePanel(&waveform_view(None)), 62, 12);
    let area = buffer.area;
    assert!(
        (0..area.width)
            .flat_map(|x| (0..area.height).map(move |y| (x, y)))
            .all(|p| buffer[p].fg != Color::Cyan)
    );
}

#[test]
fn sample_panel_follows_the_selected_track() {
    let (_, mut app, _engine) = open("sample", Some(SONG));
    let screen = text(&draw(&mut app, 120, 40));
    assert!(
        screen.contains("sample · drums · break.wav · 0.12 s · 2 cortes"),
        "{screen}"
    );
    app.handle(ctrl('o'));
    let screen = text(&draw(&mut app, 120, 40));
    assert!(
        screen.contains("sample · kick · break.wav · 0.12 s · 1 cortes"),
        "{screen}"
    );
    assert!(screen.contains("›kick"), "{screen}");
    app.handle(ctrl('o'));
    assert!(text(&draw(&mut app, 120, 40)).contains("sample · drums"));
}

fn tracker_view(step: Option<usize>, running: bool) -> ViewModel {
    use engine::Step;
    let mut drums = [None; engine::STEPS];
    drums[0] = Some(Step::new(0));
    drums[5] = Some(Step::new(2).reversed().with_pitch(7.0));
    drums[12] = Some(Step::new(1).with_retrig(4));
    let mut kick = [None; engine::STEPS];
    kick[5] = Some(Step::new(0).with_gain(0.5));
    ViewModel {
        transport: TransportView {
            running,
            bpm: 120.0,
            bar: 3,
            step,
        },
        tracks: vec![
            TrackView {
                name: "drums".into(),
                steps: drums,
                ..TrackView::default()
            },
            TrackView {
                name: "kick".into(),
                steps: kick,
                ..TrackView::default()
            },
        ],
        selected: Some(1),
        ..ViewModel::default()
    }
}

/// Fila de la pantalla que empieza (tras el borde) por `prefix`.
fn row_starting(buffer: &Buffer, prefix: &str) -> Option<u16> {
    text(buffer)
        .lines()
        .position(|line| line.chars().skip(1).collect::<String>().starts_with(prefix))
        .map(|y| y as u16)
}

#[test]
fn tracker_shows_steps_and_highlights_the_current_row() {
    let view = tracker_view(Some(5), true);
    let buffer = draw_widget(ui::TrackerPanel(&view), 40, 20);
    let screen = text(&buffer);
    assert!(screen.contains("tracker · compás 3"), "{screen}");
    assert!(screen.contains("   drums      kick"), "{screen}");
    assert!(screen.contains("00 01         ---"), "{screen}");
    assert!(screen.contains("05 03'^7      01@0.5"), "{screen}");
    assert!(screen.contains("12 02r4       ---"), "{screen}");
    // Pista seleccionada en la cabecera.
    let (x, y) = find(&buffer, "kick").unwrap();
    assert_eq!(buffer[(x, y)].fg, Color::Cyan);

    // Solo la fila 05 resaltada, en todo su ancho.
    let row = row_starting(&buffer, "05").unwrap();
    for x in 1..39 {
        assert_eq!(buffer[(x, row)].bg, Color::Blue, "x = {x}");
    }
    let other = row_starting(&buffer, "06").unwrap();
    assert_ne!(buffer[(5, other)].bg, Color::Blue);

    // Parado: nada resaltado.
    let buffer = draw_widget(ui::TrackerPanel(&tracker_view(Some(5), false)), 40, 20);
    let row = row_starting(&buffer, "05").unwrap();
    assert_ne!(buffer[(5, row)].bg, Color::Blue);
}

#[test]
fn tracker_scrolls_to_keep_the_current_row_visible() {
    // 8 de alto: borde, cabecera y 5 filas.
    let buffer = draw_widget(ui::TrackerPanel(&tracker_view(Some(12), true)), 40, 8);
    let screen = text(&buffer);
    assert!(!screen.contains("│00 "), "{screen}");
    let row = row_starting(&buffer, "12").unwrap();
    assert_eq!(buffer[(5, row)].bg, Color::Blue, "{screen}");
    // Al final del compás no se pasa de la fila 15.
    let screen = text(&draw_widget(
        ui::TrackerPanel(&tracker_view(Some(15), true)),
        40,
        8,
    ));
    assert!(
        screen.contains("│15 ") && screen.contains("│11 "),
        "{screen}"
    );
}

#[test]
fn tracker_follows_the_engine_and_every() {
    let song = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen)
  :steps("1 . . .  1 . . .  1 . . .  1 . . .")
  :every(2, function(t) t:steps("2' . 2 .  . . . .  . . . .  . . . .") end)
"#;
    let (_, mut app, mut engine) = open("tracker", Some(song));
    // Compás 0, paso 2: variante de every.
    offline::render(&mut engine, 2 * STEP + 100, 2);
    app.tick();
    let buffer = draw(&mut app, 120, 40);
    let screen = text(&buffer);
    assert!(screen.contains("tracker · compás 0"), "{screen}");
    assert!(screen.contains("00 02'"), "{screen}");
    assert!(screen.contains("02 02 "), "{screen}");
    let (_, row) = find(&buffer, "02 02 ").unwrap();
    let (x, _) = find(&buffer, "02 02 ").unwrap();
    assert_eq!(buffer[(x, row)].bg, Color::Blue);

    // Compás 1, paso 4: patrón base.
    offline::render(&mut engine, 16 * STEP + 2 * STEP, 2);
    app.tick();
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("tracker · compás 1"), "{screen}");
    assert!(screen.contains("00 01 "), "{screen}");
    assert!(screen.contains("04 01 "), "{screen}");
}

#[test]
fn fx_panel_shows_gain_level_and_chain_per_track() {
    use engine::Fx;
    let view = ViewModel {
        tracks: vec![
            TrackView {
                name: "drums".into(),
                gain: 0.7,
                level: 1.0,
                fx: vec![Fx::lowpass(3000.0), Fx::bitcrush(6, 2)],
                ..TrackView::default()
            },
            TrackView {
                name: "kick".into(),
                gain: 2.0,
                level: 0.0,
                ..TrackView::default()
            },
        ],
        selected: Some(0),
        ..ViewModel::default()
    };
    let buffer = draw_widget(ui::FxPanel(&view), 60, ui::FX_PANEL_HEIGHT);
    let screen = text(&buffer);
    assert!(screen.contains("efectos por pista"), "{screen}");
    // Cada pista en su columna, con su cadena en orden.
    let (drums_x, drums_y) = find(&buffer, "drums").unwrap();
    let (kick_x, kick_y) = find(&buffer, "kick").unwrap();
    assert_eq!(drums_y, kick_y);
    assert!(kick_x > drums_x + 20);
    assert_eq!(buffer[(drums_x, drums_y)].fg, Color::Cyan);
    let (lp_x, lp_y) = find(&buffer, "› LP 3000 Hz q0.71").unwrap();
    let (crush_x, crush_y) = find(&buffer, "› crush 6b ÷2").unwrap();
    assert_eq!((lp_x, crush_x), (drums_x, drums_x));
    assert_eq!((lp_y, crush_y), (drums_y + 3, drums_y + 4));
    assert_eq!(find(&buffer, "sin efectos").unwrap().0, kick_x);
    assert!(screen.contains("vol 0.70"), "{screen}");
    assert!(screen.contains("vol 2.00"), "{screen}");
    // Nivel lleno en "drums" y vacío en "kick".
    let (_, level_y) = find(&buffer, "nivel").unwrap();
    let row: String = screen.lines().nth(level_y as usize).unwrap().into();
    assert!(row.contains("nivel ██████████"), "{row}");
    assert!(row.contains("nivel ░░░░░░░░░░"), "{row}");
}

#[test]
fn fx_panel_reads_the_script() {
    let song = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen)
  :steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
  :fx("filter", { type = "highpass", cutoff = 200 })
  :fx("bitcrush", { bits = 4, downsample = 3 })
  :gain(0.9)
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
"#;
    let (_, mut app, mut engine) = open("fx", Some(song));
    offline::render(&mut engine, STEP / 2, 2);
    app.tick();
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("› HP 200 Hz q0.71"), "{screen}");
    assert!(screen.contains("› crush 4b ÷3"), "{screen}");
    assert!(screen.contains("vol 0.90"), "{screen}");
    assert!(screen.contains("sin efectos"), "{screen}");
    // El paso 0 sonó en las dos pistas: hay nivel.
    assert!(!screen.contains("nivel ░░░░░░░░░░░░"), "{screen}");
}
