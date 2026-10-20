//! Interfaz con el backend de test de ratatui: se dibuja en un buffer y se
//! comprueba el texto. El motor se mueve con render offline.

use std::path::{Path, PathBuf};

use engine::{Engine, new_engine, offline};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use ratatui_textarea::{Input, Key};

use crate::app::{App, Flow};

#[path = "../../engine/tests/support/synth_break.rs"]
mod synth;
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

/// Un bloque de la forma de onda en barras (`█▇…▁`, `▀`, `▔`).
fn is_bar(symbol: &str) -> bool {
    symbol
        .chars()
        .next()
        .is_some_and(|c| ('\u{2580}'..='\u{2594}').contains(&c))
}

#[test]
fn wave_bars_fill_from_low_to_high_in_eighths() {
    // 4 filas = 32 octavos; de 0 a 1 llena las dos de arriba.
    assert_eq!(
        ui::wave_bar(0.0, 1.0, 4),
        [Some('█'), Some('█'), None, None]
    );
    // De −1 a 1, todo.
    assert_eq!(ui::wave_bar(-1.0, 1.0, 2), [Some('█'), Some('█')]);
    // Hasta 0,5 (8 octavos sobre el centro): justo la fila de encima.
    assert_eq!(ui::wave_bar(0.0, 0.5, 4), [None, Some('█'), None, None]);
    // Hasta 0,75 (12 octavos): la de arriba, medio bloque por abajo.
    assert_eq!(
        ui::wave_bar(0.0, 0.75, 4),
        [Some('▄'), Some('█'), None, None]
    );
    // Por debajo del centro, el borde de abajo con bloques de arriba.
    assert_eq!(
        ui::wave_bar(-0.75, 0.0, 4),
        [None, None, Some('█'), Some('▀')]
    );
    // Silencio: una raya en el centro.
    assert_eq!(ui::wave_bar(0.0, 0.0, 3), [None, Some('─'), None]);
    // Un borde que cae a un octavo del fondo de la celda.
    assert_eq!(ui::wave_bar(0.0, 0.125, 2)[0], Some('▁'));
}

#[test]
fn sample_panel_draws_the_waveform_in_bars_with_slices() {
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

    // Forma de onda en barras: el bloque positivo arriba en el corte 1 y el
    // negativo abajo en el corte 2.
    let bars = |x0: u16, x1: u16, y0: u16, y1: u16| {
        (x0..x1)
            .flat_map(|x| (y0..y1).map(move |y| (x, y)))
            .filter(|&p| is_bar(buffer[p].symbol()))
            .collect::<Vec<_>>()
    };
    // Ni un punto de braille.
    assert!(
        !screen
            .chars()
            .any(|c| ('\u{2800}'..='\u{28FF}').contains(&c)),
        "{screen}"
    );
    // Las marcas de los cortes cruzan el panel.
    assert_eq!(buffer[(31, 6)].fg, Color::Yellow, "{screen}");
    assert!(
        !bars(3, 8, 2, 5).is_empty(),
        "sin bloque positivo:\n{screen}"
    );
    assert!(
        !bars(33, 38, 7, 11).is_empty(),
        "sin bloque negativo:\n{screen}"
    );
    // El corte que suena (el 2) va resaltado; el resto, en gris.
    assert!(
        bars(33, 38, 7, 11)
            .iter()
            .all(|&p| buffer[p].fg == Color::Cyan)
    );
    assert!(
        bars(3, 8, 2, 5)
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
    let mut drums = vec![None; engine::STEPS];
    drums[0] = Some(Step::new(0));
    drums[5] = Some(Step::new(2).reversed().with_pitch(7.0));
    drums[12] = Some(Step::new(1).with_retrig(4));
    let mut kick = vec![None; engine::STEPS];
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
                length: drums.len(),
                steps: drums,
                position: step.filter(|_| running),
                ..TrackView::default()
            },
            TrackView {
                name: "kick".into(),
                length: kick.len(),
                steps: kick,
                position: step.filter(|_| running),
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

fn esc() -> Input {
    Input {
        key: Key::Esc,
        ..Input::default()
    }
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

#[test]
fn play_mode_triggers_slices_from_the_keyboard() {
    let (_, mut app, mut engine) = open("pads", Some(SONG));
    // Parado: solo suena lo que se dispara con el teclado.
    app.handle(ctrl('t'));
    offline::render(&mut engine, 4096, 2);
    app.tick();
    let before = app.editor.lines().to_vec();

    app.handle(esc());
    assert!(app.view().playing_mode);
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("TOCAR · 1-0 q-i · Esc"), "{screen}");

    // "2": corte 2 de "drums" (bloque a 0.25), ya, sin esperar al compás.
    app.handle(key('2'));
    assert!(app.view().message.contains("▶ drums: corte 2"));
    let audio = left(&offline::render(&mut engine, 1000, 2));
    assert!((audio[200] - 0.25).abs() < 1e-3, "{}", audio[200]);
    app.tick();
    let view = app.view();
    assert_eq!(view.tracks[0].playing_slice, Some(1));

    // "1": corte 1 (0.5), cortando el anterior.
    app.handle(key('1'));
    let audio = left(&offline::render(&mut engine, 1000, 2));
    assert!((audio[200] - 0.5).abs() < 1e-3, "{}", audio[200]);

    // "w" sería el corte 12: "drums" solo tiene 2.
    app.handle(key('w'));
    assert!(
        app.view()
            .message
            .contains("drums no tiene corte 12 (tiene 2)")
    );

    // Las teclas no escribieron en el editor.
    assert_eq!(app.editor.lines(), before.as_slice());

    // Ctrl-O: ahora suena en "kick" (su único corte, el 1 del sample).
    app.handle(ctrl('o'));
    app.handle(key('1'));
    offline::render(&mut engine, 1000, 2);
    app.tick();
    let view = app.view();
    assert_eq!(view.tracks[1].playing_slice, Some(0));
    assert!(view.message.contains("▶ kick: corte 1"));

    // Esc vuelve al editor: las teclas escriben otra vez.
    app.handle(esc());
    app.handle(key('1'));
    assert!(!app.view().playing_mode);
    assert_eq!(app.editor.lines()[0], "1bpm(120)");
}

#[test]
fn pads_cover_the_number_row_and_q_to_i() {
    use crate::app::PADS;
    assert_eq!(PADS.len(), 18);
    assert_eq!(
        &PADS[..10],
        &['1', '2', '3', '4', '5', '6', '7', '8', '9', '0']
    );
    assert_eq!(&PADS[10..], &['q', 'w', 'e', 'r', 't', 'y', 'u', 'i']);
}

#[test]
fn live_triggers_use_the_track_effects_and_gain() {
    let song = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps(". . . .  . . . .  . . . .  . . . ."):gain(0.5)
"#;
    let (_, mut app, mut engine) = open("pads_gain", Some(song));
    offline::render(&mut engine, 512, 2);
    app.handle(esc());
    app.handle(key('1'));
    let audio = left(&offline::render(&mut engine, 1000, 2));
    // Corte 1 (0.5) por el volumen de la pista (0.5).
    assert!((audio[200] - 0.25).abs() < 1e-3, "{}", audio[200]);
}

/// Filas del buffer con el fondo de línea de error en la columna `x`.
fn error_rows(buffer: &Buffer, x: u16) -> Vec<u16> {
    (0..buffer.area.height)
        .filter(|&y| buffer[(x, y)].bg == ui::error_line())
        .collect()
}

#[test]
fn the_error_line_is_highlighted_in_the_editor() {
    let (_, mut app, _engine) = open("error_line", Some(SONG));
    let bad = "bpm(120)\nlocal amen = sample(\"break.wav\"):slice(2)\ntrack(\"drums\", amen):steps(\"1 . 9 .  . . . .  . . . .  . . . .\")";
    replace_all(&mut app, bad);
    app.handle(ctrl('s'));
    // Cursor al principio: si no, la línea larga se ve desplazada.
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(0, 0));
    let buffer = draw(&mut app, 120, 40);
    // Línea 3 en la fila 3 de la pantalla (borde + 2), en todo su ancho
    // dentro del editor; ninguna otra.
    assert_eq!(error_rows(&buffer, 5), vec![3]);
    let (x, _) = find(&buffer, "track(\"drums\"").unwrap();
    assert_eq!(buffer[(x, 3)].bg, ui::error_line());
    assert_eq!(buffer[(x, 3)].symbol(), "t");
    assert_ne!(buffer[(70, 3)].bg, ui::error_line(), "fuera del editor");

    // Sin error, sin resaltado.
    replace_all(&mut app, SONG);
    app.handle(ctrl('s'));
    assert!(error_rows(&draw(&mut app, 120, 40), 5).is_empty());
}

#[test]
fn the_error_line_follows_the_scroll() {
    let (_, mut app, _engine) = open("error_scroll", Some(SONG));
    // 60 líneas de comentarios y el error al final (línea 63).
    let mut long = String::new();
    for i in 0..60 {
        long.push_str(&format!("-- línea {}\n", i + 1));
    }
    long.push_str("bpm(120)\nlocal amen = sample(\"break.wav\"):slice(2)\nbpm(9999)");
    replace_all(&mut app, &long);
    app.handle(ctrl('s'));
    // El cursor está al final: el editor se desplaza y el error se ve.
    let buffer = draw(&mut app, 120, 40);
    let screen = text(&buffer);
    let rows = error_rows(&buffer, 5);
    assert_eq!(rows.len(), 1, "{screen}");
    let line: String = screen.lines().nth(rows[0] as usize).unwrap().into();
    assert!(line.contains("63 bpm(9999)"), "{line}");

    // Con el cursor arriba, la línea 63 queda fuera de la vista: nada.
    app.editor.move_cursor(ratatui_textarea::CursorMove::Top);
    let buffer = draw(&mut app, 120, 40);
    assert!(error_rows(&buffer, 5).is_empty());
    // Ctrl-G la trae a la vista otra vez.
    app.handle(ctrl('g'));
    let buffer = draw(&mut app, 120, 40);
    assert_eq!(error_rows(&buffer, 5).len(), 1);
}

fn f(n: u8) -> Input {
    Input {
        key: Key::F(n),
        ..Input::default()
    }
}

const SONG_KICK_ONLY: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
"#;

#[test]
fn external_change_is_offered_and_reloaded_with_f3() {
    let (path, mut app, _engine) = open("external", Some(SONG));
    app.check_external();
    assert!(!app.view().external_change, "sin cambios fuera");

    std::fs::write(&path, SONG_KICK_ONLY).unwrap();
    app.check_external();
    let view = app.view();
    assert!(view.external_change);
    assert!(view.message.contains("F3 recargar"), "{}", view.message);
    let screen = text(&draw(&mut app, 120, 40));
    assert!(
        screen.contains("⚠ cambiado fuera · F3 recargar"),
        "{screen}"
    );
    // Aún no se ha tocado nada: el editor y el motor siguen con lo anterior.
    assert_eq!(app.view().tracks.len(), 2);
    assert!(app.editor.lines()[2].contains("drums"));

    app.handle(f(3));
    let view = app.view();
    assert!(!view.external_change);
    assert_eq!(view.message, "recargado del disco");
    assert_eq!(
        app.editor.lines()[2],
        SONG_KICK_ONLY.lines().nth(2).unwrap()
    );
    assert!(!view.dirty);
    // Se cargó: ahora solo hay "kick".
    assert_eq!(view.tracks.len(), 1);
    assert_eq!(view.tracks[0].name, "kick");
    // Sin texto sin guardar, no hay copia.
    assert!(!app.unsaved_backup_path().exists());
}

#[test]
fn reloading_keeps_unsaved_text_in_a_backup() {
    let (path, mut app, _engine) = open("external_dirty", Some(SONG));
    type_text(&mut app, "-- sin guardar\n");
    let unsaved = format!("{}\n", app.editor.lines().join("\n"));

    std::fs::write(&path, SONG_KICK_ONLY).unwrap();
    app.check_external();
    app.handle(f(3));

    let backup = app.unsaved_backup_path();
    assert_eq!(backup.file_name().unwrap(), "song.sin-guardar.lua");
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), unsaved);
    assert!(
        app.view().message.contains("song.sin-guardar.lua"),
        "{}",
        app.view().message
    );
    assert_eq!(
        app.editor.lines()[2],
        SONG_KICK_ONLY.lines().nth(2).unwrap()
    );
}

#[test]
fn saving_over_an_external_change_asks_first() {
    let (path, mut app, _engine) = open("external_save", Some(SONG));
    type_text(&mut app, "-- mío\n");
    std::fs::write(&path, SONG_KICK_ONLY).unwrap();
    app.check_external();

    // Primer Ctrl-S: avisa y no escribe.
    app.handle(ctrl('s'));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SONG_KICK_ONLY);
    assert!(app.view().message.contains("Ctrl-S otra vez"));
    // Otra tecla en medio desarma la confirmación.
    app.handle(Input {
        key: Key::Down,
        ..Input::default()
    });
    app.handle(ctrl('s'));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SONG_KICK_ONLY);
    // Dos seguidas: sobrescribe.
    app.handle(ctrl('s'));
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("-- mío\n")
    );
    assert!(!app.view().external_change);
    // Nuestro propio guardado no cuenta como cambio externo.
    app.check_external();
    assert!(!app.view().external_change);
}

#[test]
fn reverting_the_external_change_clears_the_warning() {
    let (path, mut app, _engine) = open("external_revert", Some(SONG));
    std::fs::write(&path, SONG_KICK_ONLY).unwrap();
    app.check_external();
    assert!(app.view().external_change);
    std::fs::write(&path, SONG).unwrap();
    app.check_external();
    assert!(!app.view().external_change);
}

fn command(app: &mut App, line: &str) {
    app.handle(f(2));
    for c in line.chars() {
        app.handle(key(c));
    }
    app.handle(Input {
        key: Key::Enter,
        ..Input::default()
    });
}

fn read_wav(path: &Path) -> (hound::WavSpec, Vec<f32>) {
    let mut reader = hound::WavReader::open(path).unwrap();
    let samples = reader
        .samples::<i16>()
        .map(|s| s.unwrap() as f32 / i16::MAX as f32)
        .collect();
    (reader.spec(), samples)
}

#[test]
fn f2_opens_a_command_line() {
    let (_, mut app, _engine) = open("prompt", Some(SONG));
    let before = app.editor.lines().to_vec();
    app.handle(f(2));
    for c in "exportarx".chars() {
        app.handle(key(c));
    }
    app.handle(Input {
        key: Key::Backspace,
        ..Input::default()
    });
    let screen = text(&draw(&mut app, 130, 40));
    assert!(screen.contains("orden> exportar█"), "{screen}");
    // Esc cancela; el editor no recibió nada.
    app.handle(esc());
    assert!(app.view().prompt.is_none());
    assert!(
        !app.view().playing_mode,
        "Esc en la orden no cambia de modo"
    );
    assert_eq!(app.editor.lines(), before.as_slice());

    command(&mut app, "bailar");
    assert!(app.view().message.contains("orden desconocida «bailar»"));
}

#[test]
fn export_renders_the_mix_to_wav() {
    let (path, mut app, _engine) = open("export", Some(SONG));
    command(&mut app, "exportar mezcla.wav 2");
    let out = path.parent().unwrap().join("mezcla.wav");
    assert!(
        app.view().message.contains("2 compases a 120 BPM (4.0 s)"),
        "{}",
        app.view().message
    );
    let (spec, samples) = read_wav(&out);
    assert_eq!((spec.channels, spec.sample_rate), (2, RATE));
    assert_eq!(samples.len() / 2, 2 * 16 * STEP);

    // Lo mismo que renderizar la canción por separado (a 16 bits).
    let mut host = script::ScriptHost::new(RATE);
    let (mut c2, _e2) = new_engine(RATE);
    let song = host
        .load(&mut c2, SONG, "song.lua", path.parent().unwrap())
        .unwrap();
    let reference = song.render(RATE, 2).unwrap();
    assert!(
        samples
            .iter()
            .zip(&reference)
            .all(|(a, b)| (a - b.clamp(-1.0, 1.0)).abs() <= 1.0 / i16::MAX as f32)
    );
    assert!(samples.iter().any(|&s| s > 0.4));

    // Sin argumentos: <script>-mezcla.wav y la canción entera (al menos 4).
    command(&mut app, "exportar");
    let (_, samples) = read_wav(&path.parent().unwrap().join("song-mezcla.wav"));
    assert_eq!(samples.len() / 2, 4 * 16 * STEP);
    // Sin extensión se añade .wav; compases inválidos, error.
    command(&mut app, "exportar corto 1");
    assert!(path.parent().unwrap().join("corto.wav").exists());
    command(&mut app, "exportar x.wav 0");
    assert!(app.view().message.contains("compases no válidos"));
}

#[test]
fn export_uses_the_last_good_load() {
    let (path, mut app, _engine) = open("export_dirty", Some(SONG));
    replace_all(&mut app, "bpm(\n");
    app.handle(ctrl('l'));
    command(&mut app, "exportar ok.wav 1");
    let message = app.view().message;
    assert!(message.contains("lo último cargado bien"), "{message}");
    let (_, samples) = read_wav(&path.parent().unwrap().join("ok.wav"));
    assert_eq!(samples.len() / 2, 16 * STEP);
}

#[test]
fn projects_save_and_open_from_the_command_line() {
    let (path, mut app, _engine) = open("project", Some(SONG));
    let dir = path.parent().unwrap().to_path_buf();
    type_text(&mut app, "-- versión 2\n");
    command(&mut app, "guardar mi-proyecto");
    let project = dir.join("mi-proyecto.sampler");
    let text = std::fs::read_to_string(&project).unwrap();
    assert_eq!(
        text,
        "# proyecto del sampler\nscript = song.lua\nbpm = 120\n"
    );
    // Guardar el proyecto guarda también el script.
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("-- versión 2\n")
    );
    let screen = text_of(&mut app);
    assert!(
        screen.contains("song.lua · mi-proyecto.sampler"),
        "{screen}"
    );

    // Otro script en otra carpeta de samples, sin bpm(): toma el del proyecto.
    std::fs::create_dir_all(dir.join("samples")).unwrap();
    std::fs::rename(dir.join("break.wav"), dir.join("samples").join("break.wav")).unwrap();
    let other = "local amen = sample(\"break.wav\"):slice(2)\ntrack(\"solo\", amen):steps(\"2 . . .  . . . .  . . . .  . . . .\")\n";
    std::fs::write(dir.join("otro.lua"), other).unwrap();
    std::fs::write(
        dir.join("otro.sampler"),
        "script = otro.lua\nsamples = samples\nbpm = 96\n",
    )
    .unwrap();

    // Abrir con cambios sin guardar: se niega; con abrir!, sí.
    type_text(&mut app, "x");
    command(&mut app, "abrir otro.sampler");
    assert!(app.view().message.contains("abrir! ..."));
    command(&mut app, "abrir! otro.sampler");
    let view = app.view();
    assert!(
        view.message.contains("otro.sampler abierto"),
        "{}",
        view.message
    );
    assert_eq!(view.file, "otro.lua");
    assert_eq!(view.tracks.len(), 1);
    assert_eq!(view.tracks[0].name, "solo");
    assert_eq!(app.editor.lines()[1], other.lines().nth(1).unwrap());
    assert!(!view.dirty);

    // Exportar usa su tempo (96) y su carpeta.
    command(&mut app, "exportar");
    assert!(
        app.view().message.contains("a 96 BPM"),
        "{}",
        app.view().message
    );
    assert!(dir.join("otro-mezcla.wav").exists());

    // `guardar` sin nombre reescribe el proyecto abierto.
    command(&mut app, "guardar");
    let text = std::fs::read_to_string(dir.join("otro.sampler")).unwrap();
    assert_eq!(
        text,
        "# proyecto del sampler\nscript = otro.lua\nsamples = samples\nbpm = 96\n"
    );
}

#[test]
fn edit_opens_a_project_file_directly() {
    let path = fixture("project_cli", Some(SONG));
    let dir = path.parent().unwrap();
    std::fs::write(dir.join("p.sampler"), "script = song.lua\nbpm = 100\n").unwrap();
    let (controller, _engine) = new_engine(RATE);
    let app = App::open_project(&dir.join("p.sampler"), controller, RATE).unwrap();
    let view = app.view();
    assert_eq!(view.project.as_deref(), Some("p.sampler"));
    assert_eq!(view.tracks.len(), 2);
    let (controller, _engine) = new_engine(RATE);
    assert!(App::open_project(&dir.join("no.sampler"), controller, RATE).is_err());
}

#[test]
fn samples_command_changes_the_sample_folder() {
    let (path, mut app, _engine) = open("samples_cmd", Some(SONG));
    let dir = path.parent().unwrap();
    command(&mut app, "muestras");
    assert_eq!(app.view().message, "samples junto al script");
    command(&mut app, "muestras vacía");
    // En esa carpeta no está break.wav: error en el editor, sigue lo anterior.
    let view = app.view();
    assert!(view.error.is_some());
    assert_eq!(view.tracks.len(), 2);
    command(&mut app, "muestras .");
    assert!(app.view().error.is_none());
    command(&mut app, "muestras");
    assert!(app.view().message.contains(&dir.display().to_string()));
}

fn text_of(app: &mut App) -> String {
    text(&draw(app, 130, 40))
}

// --- Autocompletado ---

fn special(key: Key) -> Input {
    Input {
        key,
        ..Input::default()
    }
}

/// Abre la canción con el cursor en una línea nueva al final.
fn open_at_end(test: &str) -> (App, Engine) {
    let (_, mut app, engine) = open(test, Some(SONG));
    app.editor.move_cursor(ratatui_textarea::CursorMove::Bottom);
    app.editor.move_cursor(ratatui_textarea::CursorMove::End);
    type_text(&mut app, "\n");
    (app, engine)
}

fn current_line(app: &App) -> String {
    app.editor.lines()[app.editor.cursor().0].clone()
}

fn completion_items(app: &App) -> Vec<String> {
    app.view()
        .completion
        .map(|c| c.items.into_iter().map(|(label, _)| label).collect())
        .unwrap_or_default()
}

#[test]
fn completion_filters_by_prefix_while_typing() {
    let (mut app, _engine) = open_at_end("complete_filter");
    type_text(&mut app, "t");
    assert_eq!(completion_items(&app), ["track", "then", "true"]);
    type_text(&mut app, "r");
    assert_eq!(completion_items(&app), ["track", "true"]);
    type_text(&mut app, "a");
    assert_eq!(completion_items(&app), ["track"]);

    // El popup sale junto al cursor, con la firma y la descripción.
    let buffer = draw(&mut app, 120, 40);
    let screen = text(&buffer);
    let (x, y) = find(&buffer, "completar").unwrap();
    let (_, cursor_row) = find(&buffer, "5 tra").unwrap();
    assert_eq!(y, cursor_row + 1, "{screen}");
    assert!(x < 20, "{screen}");
    assert!(
        screen.contains("track(nombre, cortes) / track(nombre) → pista"),
        "{screen}"
    );
    assert!(
        screen.contains("Define una pista que toca esos cortes"),
        "{screen}"
    );
    assert!(screen.contains("función"), "{screen}");
    assert!(screen.contains("COMPLETAR: Tab aceptar"), "{screen}");

    // Borrar vuelve a ampliar la lista; sin coincidencias se cierra.
    app.handle(special(Key::Backspace));
    assert_eq!(completion_items(&app), ["track", "true"]);
    type_text(&mut app, "x");
    assert!(app.view().completion.is_none());
}

#[test]
fn completion_follows_the_context() {
    let (mut app, _engine) = open_at_end("complete_context");
    type_text(&mut app, "track(\"drums\"):");
    let track: Vec<String> = script::api::functions(script::api::Receiver::Track)
        .map(|f| f.name.to_string())
        .collect();
    assert_eq!(completion_items(&app), track);
    assert!(text_of(&mut app).contains("método de pista"));

    type_text(&mut app, "fx(\"");
    assert_eq!(
        completion_items(&app),
        [
            "filter",
            "lowpass",
            "highpass",
            "bitcrush",
            "compressor",
            "distortion",
            "gain",
            "eq",
            "dyneq"
        ]
    );
    type_text(&mut app, "filter\", { ");
    app.handle(f(4));
    assert_eq!(completion_items(&app), ["type", "cutoff", "q"]);
    app.handle(esc());

    // WAV de la carpeta de muestras y pistas ya definidas.
    type_text(&mut app, "\nlocal b = sample(\"");
    assert_eq!(completion_items(&app), ["break.wav"]);
    app.handle(esc());
    type_text(&mut app, "\ntrack(\"");
    assert_eq!(completion_items(&app), ["drums", "kick"]);
    app.handle(esc());

    // Al inicio de línea, las funciones globales primero.
    type_text(&mut app, "\n");
    app.handle(f(4));
    assert_eq!(&completion_items(&app)[..3], ["bpm", "sample", "track"]);
    app.handle(esc());

    // Dentro de steps("..."), nada (ni con F4).
    type_text(&mut app, "track(\"drums\"):steps(\"1 . x");
    assert!(app.view().completion.is_none());
    app.handle(f(4));
    assert!(app.view().completion.is_none());
    assert_eq!(app.view().message, "nada que completar aquí");
}

#[test]
fn arrows_move_the_selection() {
    let (mut app, _engine) = open_at_end("complete_arrows");
    type_text(&mut app, "t");
    let selected = |app: &App| app.view().completion.unwrap().selected;
    app.handle(special(Key::Down));
    assert_eq!(selected(&app), 1);
    assert!(text_of(&mut app).contains("palabra clave de Lua"));
    app.handle(special(Key::Up));
    app.handle(special(Key::Up));
    assert_eq!(selected(&app), 2, "da la vuelta");
    // Las flechas no mueven el cursor del editor mientras tanto.
    assert_eq!(current_line(&app), "t");
    // Escribir después cambia la lista: Enter vuelve a ser salto de línea.
    type_text(&mut app, "r");
    assert_eq!(completion_items(&app), ["track", "true"]);
    app.handle(special(Key::Enter));
    assert_eq!(current_line(&app), "");
    assert_eq!(app.editor.lines()[app.editor.cursor().0 - 1], "tr");
}

#[test]
fn accepting_inserts_the_template_with_marked_parameters() {
    let (mut app, _engine) = open_at_end("complete_accept");
    // Enter sin haber elegido con las flechas: salto de línea normal.
    type_text(&mut app, "bp");
    assert_eq!(completion_items(&app), ["bpm", "bpm_ramp"]);
    app.handle(special(Key::Enter));
    assert!(app.view().completion.is_none());
    assert_eq!(current_line(&app), "");
    assert_eq!(app.editor.lines()[app.editor.cursor().0 - 1], "bp");
    app.handle(special(Key::Backspace));
    // Tras moverse con las flechas, Enter acepta: queda marcado el
    // parámetro y lo escrito lo sustituye.
    type_text(&mut app, "m");
    assert_eq!(
        completion_items(&app),
        ["bpm", "bpm_ramp"],
        "con plantilla, sigue abierto"
    );
    app.handle(special(Key::Down));
    app.handle(special(Key::Up));
    app.handle(special(Key::Enter));
    assert_eq!(current_line(&app), "bpm(170)");
    assert!(app.view().completion.is_none());
    type_text(&mut app, "140");
    assert_eq!(current_line(&app), "bpm(140)");

    // Tab acepta; Tab salta de un parámetro al siguiente.
    app.editor.move_cursor(ratatui_textarea::CursorMove::End);
    type_text(&mut app, "\ntrack(\"drums\"):fx");
    app.handle(special(Key::Tab));
    assert_eq!(
        current_line(&app),
        "track(\"drums\"):fx(\"filter\", { type = \"lowpass\", cutoff = 3000 })"
    );
    assert_eq!(
        app.editor.selection_range(),
        Some(((5, 19), (5, 25))),
        "filter"
    );
    app.handle(special(Key::Tab));
    type_text(&mut app, "highpass");
    app.handle(special(Key::Tab));
    type_text(&mut app, "800");
    // Tras el último parámetro, Tab lleva al final de la plantilla.
    app.handle(special(Key::Tab));
    let line = current_line(&app);
    assert_eq!(
        line,
        "track(\"drums\"):fx(\"filter\", { type = \"highpass\", cutoff = 800 })"
    );
    assert_eq!(app.editor.cursor().1, line.chars().count());
    assert_eq!(app.editor.selection_range(), None);
    // Ya sin parámetros pendientes, se encadena otro método.
    type_text(&mut app, ":ga");
    assert_eq!(completion_items(&app), ["gain"]);
    app.handle(special(Key::Tab));
    assert!(current_line(&app).ends_with(":gain(0.8)"));

    // El resultado es código válido: se carga y el panel lo muestra.
    app.handle(ctrl('s'));
    let view = app.view();
    assert!(view.error.is_none(), "{:?}", view.error);
    assert_eq!(engine::Fx::highpass(800.0), view.tracks[0].fx[0]);
}

#[test]
fn changing_the_effect_regenerates_its_parameters() {
    let (mut app, _engine) = open_at_end("complete_name");
    type_text(&mut app, "track(\"drums\"):fx");
    app.handle(special(Key::Tab));
    // Sobre «filter» marcado se escribe «b»: el popup ofrece bitcrush, y al
    // aceptarlo la tabla pasa a ser la de bitcrush.
    type_text(&mut app, "b");
    assert_eq!(completion_items(&app), ["bitcrush"]);
    app.handle(special(Key::Tab));
    assert_eq!(
        current_line(&app),
        "track(\"drums\"):fx(\"bitcrush\", { bits = 6, downsample = 2 })"
    );
    assert_eq!(selection_text(&app), "6");
    // Un solo Ctrl-Z deshace el cambio de efecto entero; Ctrl-R lo rehace.
    app.handle(ctrl('z'));
    assert_eq!(
        current_line(&app),
        "track(\"drums\"):fx(\"b\", { type = \"lowpass\", cutoff = 3000 })"
    );
    app.handle(ctrl('r'));
    assert_eq!(
        current_line(&app),
        "track(\"drums\"):fx(\"bitcrush\", { bits = 6, downsample = 2 })"
    );
    // Tab recorre los parámetros nuevos y acaba tras el `)`.
    let (mut app, _engine) = open_at_end("complete_name_tabs");
    type_text(&mut app, "track(\"drums\"):fx");
    app.handle(special(Key::Tab));
    type_text(&mut app, "b");
    app.handle(special(Key::Tab));
    type_text(&mut app, "4");
    app.handle(special(Key::Tab));
    assert_eq!(selection_text(&app), "2");
    app.handle(special(Key::Tab));
    assert_eq!(app.editor.cursor().1, current_line(&app).chars().count());
    assert_eq!(
        current_line(&app),
        "track(\"drums\"):fx(\"bitcrush\", { bits = 4, downsample = 2 })"
    );

    // Escrito entero, el popup sigue abierto para poder regenerar.
    let (mut app, _engine) = open_at_end("complete_name_full");
    type_text(&mut app, "track(\"drums\"):fx");
    app.handle(special(Key::Tab));
    type_text(&mut app, "lowpass");
    assert_eq!(completion_items(&app), ["lowpass"]);
    app.handle(special(Key::Tab));
    assert_eq!(
        current_line(&app),
        "track(\"drums\"):fx(\"lowpass\", { cutoff = 3000 })"
    );
    assert_eq!(selection_text(&app), "3000");
    app.handle(ctrl('s'));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
}

#[test]
fn choosing_another_effect_with_the_arrows_replaces_the_rest_of_the_name() {
    let (_, mut app, _engine) = open("complete_fx_line", Some(SONG));
    let text = format!("{SONG}track(\"drums\"):fx(\"lowpass\", {{ cutoff = 800, q = 2 }})");
    replace_all(&mut app, &text);
    // Cursor justo tras las comillas de fx(".
    let row = app.editor.lines().len() - 1;
    let col = current_line(&app).find("lowpass").unwrap();
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(row as u16, col as u16));
    app.handle(f(4));
    assert_eq!(
        completion_items(&app),
        [
            "filter",
            "lowpass",
            "highpass",
            "bitcrush",
            "compressor",
            "distortion",
            "gain",
            "eq",
            "dyneq"
        ]
    );
    app.handle(special(Key::Down));
    app.handle(special(Key::Down));
    app.handle(special(Key::Enter));
    // Los parámetros que tienen los dos efectos conservan su valor.
    assert_eq!(
        current_line(&app),
        "track(\"drums\"):fx(\"highpass\", { cutoff = 800, q = 2 })"
    );
    assert_eq!(selection_text(&app), "800");
    app.handle(ctrl('s'));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
}

#[test]
fn esc_closes_the_popup_without_entering_play_mode() {
    let (mut app, _engine) = open_at_end("complete_esc");
    type_text(&mut app, "tr");
    assert!(app.view().completion.is_some());
    app.handle(esc());
    let view = app.view();
    assert!(view.completion.is_none());
    assert!(!view.playing_mode, "Esc solo cierra el popup");
    let screen = text_of(&mut app);
    assert!(!screen.contains("completar"), "{screen}");
    assert!(!screen.contains("TOCAR"), "{screen}");
    // Se sigue escribiendo en el editor (y el popup vuelve: «track» tiene
    // plantilla).
    type_text(&mut app, "ack");
    assert_eq!(current_line(&app), "track");
    assert_eq!(completion_items(&app), ["track"]);
    app.handle(esc());
    assert!(!app.view().playing_mode);

    // Sin popup, Esc sí entra en modo tocar, y ahí no se abre.
    app.handle(esc());
    assert!(app.view().playing_mode);
    type_text(&mut app, "tr");
    app.handle(f(4));
    let view = app.view();
    assert!(view.completion.is_none());
    assert!(view.playing_mode);
    assert_eq!(current_line(&app), "track");
}

#[test]
fn entering_play_mode_never_happens_while_completing() {
    // Abierto con F4 (sin prefijo), Esc también solo cierra.
    let (mut app, _engine) = open_at_end("complete_f4_esc");
    app.handle(f(4));
    assert!(app.view().completion.is_some());
    app.handle(esc());
    assert!(!app.view().playing_mode);
    // Los pads no disparan nada: «1» se escribe.
    type_text(&mut app, "1");
    assert_eq!(current_line(&app), "1");
}

#[test]
#[ignore = "solo para ver la pantalla: cargo test -p tui pantalla_autocompletado -- --ignored --nocapture"]
fn pantalla_autocompletado() {
    let (mut app, _engine) = open_at_end("pantalla_completar");
    type_text(&mut app, "track(\"drums\"):fx(\"filter\", { ");
    app.handle(f(4));
    app.handle(special(Key::Down));
    println!("{}", text(&draw(&mut app, 120, 40)));
}

#[test]
fn the_popup_goes_below_the_cursor_or_above_if_it_does_not_fit() {
    let view = crate::model::CompletionView {
        items: vec![("track".into(), "función".into()); 3],
        selected: 0,
        signature: "track(nombre, cortes)".into(),
        description: "Define una pista.".into(),
        prefix_len: 2,
    };
    let screen = ratatui::layout::Rect::new(0, 0, 100, 40);
    // 3 opciones + separador + 2 de detalle + bordes = 8 filas.
    let below = ui::completion_area(&view, 10, 5, screen);
    assert_eq!((below.x, below.y, below.height), (10, 6, 8));
    let above = ui::completion_area(&view, 10, 36, screen);
    assert_eq!((above.y, above.bottom()), (28, 36));
    // Pegado al borde derecho, se desplaza para caber.
    let right = ui::completion_area(&view, 95, 5, screen);
    assert_eq!(right.right(), 100);
}

// --- Paleta de comandos y edición de líneas ---

fn type_query(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle(key(c));
    }
}

#[test]
fn f1_opens_a_filterable_command_palette() {
    let (mut app, _engine) = open_at_end("palette_filter");
    app.handle(f(1));
    let screen = text_of(&mut app);
    assert!(screen.contains("paleta de comandos"), "{screen}");
    assert!(screen.contains("Deshacer"), "{screen}");
    assert!(screen.contains("Ctrl-Z o Ctrl-U"), "{screen}");
    assert!(screen.contains("PALETA: escribe para filtrar"), "{screen}");

    // Escribir filtra (sin tildes) y no toca el texto.
    type_query(&mut app, "lineas");
    let view = app.view().palette.unwrap();
    assert_eq!(view.items.len(), 4);
    assert_eq!(
        view.description,
        "Pone o quita `-- ` en la línea del cursor o en las seleccionadas"
    );
    type_query(&mut app, " baj");
    let names: Vec<String> = app
        .view()
        .palette
        .unwrap()
        .items
        .into_iter()
        .map(|(n, _)| n)
        .collect();
    assert_eq!(names, ["Bajar líneas"]);
    assert_eq!(current_line(&app), "");
    // Sin coincidencias lo dice.
    type_query(&mut app, "zzz");
    assert!(text_of(&mut app).contains("ninguna acción coincide"));
}

#[test]
fn the_palette_runs_line_actions_and_remembers_the_last() {
    let (_, mut app, _engine) = open("palette_run", Some(SONG));
    // Cursor en la línea 1 ("bpm(120)").
    app.handle(f(1));
    type_query(&mut app, "duplicar");
    app.handle(special(Key::Enter));
    assert!(app.view().palette.is_none());
    assert_eq!(&app.editor.lines()[..2], ["bpm(120)", "bpm(120)"]);

    // Comentar, bajar y subir.
    app.handle(f(1));
    type_query(&mut app, "comentar");
    app.handle(special(Key::Enter));
    assert_eq!(app.editor.lines()[1], "-- bpm(120)");
    app.handle(f(1));
    type_query(&mut app, "bajar");
    app.handle(special(Key::Enter));
    assert_eq!(app.editor.lines()[2], "-- bpm(120)");
    // La última acción sale la primera: F1 y Enter la repite.
    app.handle(f(1));
    assert_eq!(app.view().palette.unwrap().items[0].0, "Bajar líneas");
    app.handle(special(Key::Enter));
    assert_eq!(app.editor.lines()[3], "-- bpm(120)");
    app.handle(f(1));
    type_query(&mut app, "subir");
    app.handle(special(Key::Down));
    app.handle(special(Key::Up));
    app.handle(special(Key::Enter));
    assert_eq!(app.editor.lines()[2], "-- bpm(120)");

    // Y todo se deshace con Ctrl-Z (y se rehace con la paleta).
    for _ in 0..5 {
        app.handle(ctrl('z'));
    }
    assert_eq!(app.text_for_test(), SONG);
    app.handle(ctrl('z'));
    assert_eq!(app.view().message, "nada que deshacer");
    app.handle(f(1));
    type_query(&mut app, "rehacer");
    app.handle(special(Key::Enter));
    assert_eq!(&app.editor.lines()[..2], ["bpm(120)", "bpm(120)"]);
}

#[test]
fn esc_closes_the_palette_without_entering_play_mode() {
    let (mut app, _engine) = open_at_end("palette_esc");
    app.handle(f(1));
    app.handle(esc());
    let view = app.view();
    assert!(view.palette.is_none());
    assert!(!view.playing_mode);
    // En modo tocar también se abre, y las teclas filtran (no disparan).
    app.handle(esc());
    assert!(app.view().playing_mode);
    app.handle(f(1));
    type_query(&mut app, "1q");
    assert_eq!(app.view().palette.unwrap().query, "1q");
    app.handle(esc());
    assert!(app.view().playing_mode, "sigue en modo tocar");
}

#[test]
fn the_palette_can_quit_and_run_editor_actions() {
    let (_, mut app, _engine) = open("palette_quit", Some(SONG));
    let run = |app: &mut App, query: &str| {
        app.handle(f(1));
        type_query(app, query);
        app.handle(special(Key::Enter))
    };
    assert_eq!(run(&mut app, "marcha"), Flow::Continue);
    assert_eq!(app.view().message, "parado");
    assert_eq!(run(&mut app, "salir"), Flow::Quit, "sin cambios sale");
    type_text(&mut app, "x");
    assert_eq!(run(&mut app, "salir"), Flow::Continue);
    assert!(app.view().message.contains("cambios sin guardar"));
}

#[test]
fn compound_edits_undo_and_redo_in_one_step() {
    let (mut app, _engine) = open_at_end("undo_groups");
    type_text(&mut app, "track(\"drums\"):fx");
    app.handle(special(Key::Tab));
    assert!(current_line(&app).contains("fx(\"filter\""));
    // Un solo Ctrl-Z quita la plantilla entera y deja lo escrito.
    app.handle(ctrl('z'));
    assert_eq!(current_line(&app), "track(\"drums\"):fx");
    app.handle(ctrl('r'));
    assert!(current_line(&app).contains("fx(\"filter\""));
    // Ctrl-U (el deshacer del editor) hace lo mismo.
    app.handle(ctrl('u'));
    assert_eq!(current_line(&app), "track(\"drums\"):fx");
    // Lo escrito a mano se deshace como siempre, letra a letra.
    app.handle(ctrl('u'));
    assert_eq!(current_line(&app), "track(\"drums\"):f");
}

// --- Snippets ---

fn selection_text(app: &App) -> String {
    let ((r1, c1), (r2, c2)) = app.editor.selection_range().expect("sin selección");
    assert_eq!(r1, r2);
    app.editor.lines()[r1]
        .chars()
        .skip(c1)
        .take(c2 - c1)
        .collect()
}

#[test]
fn snippets_from_the_completion_jump_between_fields_with_tab() {
    let (mut app, _engine) = open_at_end("snippet_ghost");
    type_text(&mut app, "gh");
    assert_eq!(completion_items(&app), ["ghost"]);
    assert!(text_of(&mut app).contains("snippet"));
    app.handle(special(Key::Tab));
    let lines = app.editor.lines().to_vec();
    assert_eq!(lines[4], "track(\"ghost\", amen:slice_at(3))");
    assert_eq!(lines[7], "  :fx(\"lowpass\", { cutoff = 4000 })");
    assert_eq!(selection_text(&app), "ghost");

    type_text(&mut app, "fantasma");
    app.handle(special(Key::Tab));
    assert_eq!(selection_text(&app), "3");
    type_text(&mut app, "2");
    app.handle(special(Key::Tab));
    assert!(selection_text(&app).starts_with(". x@0.2"));
    app.handle(special(Key::Tab));
    assert_eq!(selection_text(&app), "0.4");
    app.handle(special(Key::Tab));
    assert_eq!(selection_text(&app), "4000");
    type_text(&mut app, "3000");
    // Tras el último campo, al final del snippet.
    app.handle(special(Key::Tab));
    assert_eq!(app.editor.selection_range(), None);
    assert_eq!(app.editor.cursor(), (8, 0));

    let lines = app.editor.lines().to_vec();
    assert_eq!(lines[4], "track(\"fantasma\", amen:slice_at(2))");
    assert_eq!(lines[7], "  :fx(\"lowpass\", { cutoff = 3000 })");
    app.handle(ctrl('s'));
    let view = app.view();
    assert!(view.error.is_none(), "{:?}", view.error);
    assert_eq!(view.tracks.len(), 3);
    assert_eq!(view.tracks[2].name, "fantasma");
}

#[test]
fn snippets_from_the_palette_are_indented_and_undone_at_once() {
    let (mut app, _engine) = open_at_end("snippet_palette");
    type_text(&mut app, "  ");
    app.handle(f(1));
    type_query(&mut app, "snippet intro");
    app.handle(special(Key::Enter));
    let lines = app.editor.lines().to_vec();
    assert_eq!(lines[4], "  track(\"intro\", amen)");
    assert_eq!(
        lines[5],
        "    :steps(\"1 . . .  . . . .  1 . . .  . . . .\")"
    );
    assert_eq!(selection_text(&app), "intro");
    assert!(app.view().message.starts_with("snippet intro"));
    app.handle(ctrl('z'));
    assert_eq!(current_line(&app), "  ");
    assert_eq!(app.editor.lines().len(), 5);
}

#[test]
fn the_amen_snippet_is_a_whole_song() {
    let (_, mut app, _engine) = open("snippet_amen", Some(""));
    app.handle(f(1));
    type_query(&mut app, "snippet amen");
    app.handle(special(Key::Enter));
    // Se rellenan los campos: tempo, archivo, cortes, pista y patrón.
    for value in [
        "120",
        "break.wav",
        "2",
        "d",
        "1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .",
    ] {
        type_text(&mut app, value);
        app.handle(special(Key::Tab));
    }
    app.handle(ctrl('s'));
    let view = app.view();
    assert!(
        view.error.is_none(),
        "{:?}\n{}",
        view.error,
        app.text_for_test()
    );
    assert_eq!(view.tracks[0].name, "d");
    assert_eq!(view.tracks[0].slices.len(), 2);
}

// --- Diagnóstico en vivo de steps ---

#[test]
fn steps_errors_are_underlined_while_typing() {
    let (mut app, _engine) = open_at_end("diagnose_live");
    type_text(&mut app, "track(\"drums\"):steps(\"1 . 5^ .  . . . .");
    // Sin guardar ni cargar: el aviso sale ya.
    let buffer = draw(&mut app, 120, 40);
    let screen = text(&buffer);
    // Al pie del editor, ajustado al ancho.
    assert!(
        screen.contains("⚠ línea 5: steps: paso 3 (\"5^\"):"),
        "{screen}"
    );
    assert!(screen.contains("falta el número después"), "{screen}");
    assert!(app.view().error.is_none(), "no se ha cargado nada");
    // Solo el paso erróneo va subrayado.
    let (x, y) = find(&buffer, "5^ .").unwrap();
    for dx in 0..2 {
        let cell = &buffer[(x + dx, y)];
        assert!(cell.modifier.contains(Modifier::UNDERLINED), "{dx}");
        assert_eq!(cell.fg, ui::diagnostic());
    }
    // (La línea del cursor va subrayada entera; lo que distingue el paso es
    // el color.)
    assert_ne!(buffer[(x + 2, y)].fg, ui::diagnostic());
    assert_ne!(buffer[(x - 2, y)].fg, ui::diagnostic());

    // Al corregirlo, queda el aviso de los pasos que faltan, sobre todo el patrón.
    for _ in 0..13 {
        app.handle(special(Key::Backspace));
    }
    type_text(&mut app, "5 .  . . . .");
    let view = app.view();
    assert_eq!(view.diagnostics.len(), 1);
    assert_eq!(
        view.diagnostics[0].message,
        "steps: el patrón necesita 16 pasos, tiene 8"
    );
    // Completo y bien: nada.
    type_text(&mut app, "  1 . . .  1 . . .\")");
    assert_eq!(
        current_line(&app),
        "track(\"drums\"):steps(\"1 . 5 .  . . . .  1 . . .  1 . . .\")"
    );
    assert!(app.view().diagnostics.is_empty());
    assert!(!text_of(&mut app).contains("⚠ línea"));
}

// --- Ayuda contextual ---

#[test]
fn the_help_line_explains_the_function_under_the_cursor() {
    let (mut app, _engine) = open_at_end("help_line");
    type_text(
        &mut app,
        "track(\"drums\"):fx(\"bitcrush\", { bits = 4, downsample = ",
    );
    assert!(app.view().completion.is_none(), "tras «= » no se abre solo");
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("ⓘ fx(\"bitcrush\", { ... })"), "{screen}");
    assert!(
        screen.contains("downsample: repite cada muestra n veces"),
        "{screen}"
    );
    assert_eq!(
        app.view().help.unwrap().description,
        "downsample: repite cada muestra n veces (1–64, por defecto 1)"
    );

    // Sobre el nombre de una función, su firma y su resumen.
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(0, 1));
    let screen = text(&draw(&mut app, 140, 40));
    assert!(
        screen.contains("ⓘ bpm(n)  Tempo en pulsos por minuto, de 20 a 400."),
        "{screen}"
    );
    // Sobre algo que no es de la API, nada.
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(1, 2));
    assert!(app.view().help.is_none());
    assert!(!text(&draw(&mut app, 140, 40)).contains("ⓘ"));

    // En modo tocar no hay ayuda.
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(0, 1));
    assert!(app.view().help.is_some());
    app.handle(esc());
    assert!(app.view().playing_mode);
    assert!(app.view().help.is_none());
}

#[test]
#[ignore = "solo para ver la pantalla: cargo test -p tui pantalla_paleta -- --ignored --nocapture"]
fn pantalla_paleta() {
    let (mut app, _engine) = open_at_end("pantalla_paleta");
    type_text(&mut app, "track(\"drums\"):steps(\"1 . 5^ .\")");
    app.handle(f(1));
    type_query(&mut app, "lin");
    println!("{}", text(&draw(&mut app, 120, 40)));
}

// --- Etiquetas de corte ---

const LABELED: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2):name(1, "kick"):name(2, "snare")
track("drums", amen):steps("kick . snare .  kick . snare .  kick . snare .  kick^7 . snare' .")
"#;

#[test]
fn the_tracker_and_the_sample_view_show_labels() {
    let (_, mut app, mut engine) = open("labels_views", Some(LABELED));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    offline::render(&mut engine, STEP + 100, 2);
    app.tick();
    let screen = text(&draw(&mut app, 120, 40));
    // Tracker: el nombre en vez del número, con sus sufijos.
    assert!(screen.contains("00 kick"), "{screen}");
    assert!(screen.contains("02 snare"), "{screen}");
    // Vista del sample: número y etiqueta en cada marca.
    assert!(screen.contains("1 kick"), "{screen}");
    assert!(screen.contains("2 snare"), "{screen}");
    let view = app.view();
    assert_eq!(view.tracks[0].label(0), Some("kick"));
    assert_eq!(
        crate::model::step_text(&view.tracks[0].steps[12], &view.tracks[0].labels),
        "kick^7"
    );
}

#[test]
fn labels_complete_inside_steps_and_unknown_ones_are_underlined() {
    let (_, mut app, _engine) = open("labels_complete", Some(LABELED));
    app.editor.move_cursor(ratatui_textarea::CursorMove::Bottom);
    app.editor.move_cursor(ratatui_textarea::CursorMove::End);
    type_text(&mut app, "\ntrack(\"b\", amen):steps(\"kick . sn");
    assert_eq!(completion_items(&app), ["snare"]);
    app.handle(special(Key::Tab));
    assert_eq!(
        current_line(&app),
        "track(\"b\", amen):steps(\"kick . snare"
    );

    // Una etiqueta que no existe se ve al momento, con las que hay.
    type_text(&mut app, " snre");
    let view = app.view();
    assert_eq!(view.diagnostics.len(), 1);
    assert_eq!(
        view.diagnostics[0].message,
        "steps: paso 4 (\"snre\"): la etiqueta \"snre\" no existe (hay: kick, snare)"
    );
    // Una etiqueta recién escrita con :name() (sin cargar) ya cuenta.
    app.handle(special(Key::Esc));
    let (_, mut app, _engine) = open("labels_typed", Some(LABELED));
    app.editor.move_cursor(ratatui_textarea::CursorMove::Bottom);
    app.editor.move_cursor(ratatui_textarea::CursorMove::End);
    type_text(
        &mut app,
        "\namen:name(1, \"bombo\")\ntrack(\"c\", amen):steps(\"bombo . . .  . . . .  . . . .  . . . .\")",
    );
    assert!(
        app.view().diagnostics.is_empty(),
        "{:?}",
        app.view().diagnostics
    );
    app.handle(ctrl('s'));
    let view = app.view();
    assert!(view.error.is_none(), "{:?}", view.error);
    assert_eq!(view.tracks[0].label(0), Some("bombo"));
}

// --- Etiquetar desde la paleta ---

fn palette(app: &mut App, query: &str) {
    app.handle(f(1));
    type_query(app, query);
    app.handle(special(Key::Enter));
}

#[test]
fn the_palette_labels_the_slice_played_last() {
    let (_, mut app, _engine) = open("label_palette", Some(SONG));
    // Tocar el corte 2 lo selecciona.
    app.handle(esc());
    app.handle(key('2'));
    assert_eq!(app.view().selected_slice, Some(1));
    app.handle(esc());

    palette(&mut app, "etiquetar");
    assert_eq!(app.view().prompt.as_deref(), Some("etiquetar 2 "));
    type_query(&mut app, "snare");
    app.handle(special(Key::Enter));
    let view = app.view();
    assert_eq!(
        view.message,
        "corte 2 de drums: «snare» (sin proyecto: se guardará con «guardar»)"
    );
    assert_eq!(view.tracks[0].label(1), Some("snare"));
    // El tracker y la vista del sample lo enseñan ya.
    let buffer = draw(&mut app, 160, 40);
    let screen = text(&buffer);
    assert!(screen.contains("02 snare"), "{screen}");
    assert!(screen.contains("cortes · corte 2 snare"), "{screen}");
    // Fila de las marcas: la de debajo del título del panel del sample.
    let (_, title) = find(&buffer, "cortes · corte 2 snare").unwrap();
    let row = screen.lines().nth(title as usize + 1).unwrap();
    let x = row[..row.find("2 snare").unwrap()].chars().count() as u16;
    assert_eq!(
        buffer[(x, title + 1)].bg,
        ui::slice_color(),
        "el seleccionado, resaltado"
    );
    // Y se puede usar en steps.
    let text = app
        .text_for_test()
        .replace("\"1 . 2 .  1 . 2 .", "\"1 . snare .  1 . 2 .");
    replace_all(&mut app, &text);
    app.handle(ctrl('s'));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);

    // Tocar suena con su etiqueta en el mensaje.
    app.handle(esc());
    app.handle(key('2'));
    assert_eq!(app.view().message, "▶ drums: corte 2 (snare)");
}

#[test]
fn labels_are_saved_in_the_project_and_reopened() {
    let path = fixture("label_project", Some(SONG));
    let dir = path.parent().unwrap().to_path_buf();
    let project = dir.join("p.sampler");
    std::fs::write(&project, "script = song.lua\nbpm = 100\n").unwrap();
    let (controller, _engine) = new_engine(RATE);
    let mut app = App::open_project(&project, controller, RATE).unwrap();

    command(&mut app, "etiquetar 1 kick");
    assert!(
        app.view().message.contains("(guardado en"),
        "{}",
        app.view().message
    );
    let saved = std::fs::read_to_string(&project).unwrap();
    assert!(saved.contains("etiqueta = break.wav 2 1 kick\n"), "{saved}");
    assert!(
        saved.contains("bpm = 100\n"),
        "el tempo del proyecto no cambia: {saved}"
    );

    // La pista "kick" es slice_at(1): su corte 1 es el 1 del sample, y la
    // etiqueta vale para las dos pistas.
    app.next_track();
    command(&mut app, "etiquetar 1 bombo");
    let view = app.view();
    assert_eq!(view.tracks[1].label(0), Some("bombo"));
    assert_eq!(view.tracks[0].label(0), Some("bombo"));

    let (controller, _engine) = new_engine(RATE);
    let reopened = App::open_project(&project, controller, RATE).unwrap();
    assert_eq!(reopened.view().tracks[0].label(0), Some("bombo"));
}

#[test]
fn labeling_errors_are_clear() {
    let (_, mut app, _engine) = open("label_errors", Some(SONG));
    let mut check = |line: &str, expected: &str| {
        command(&mut app, line);
        let message = app.view().message;
        assert!(message.contains(expected), "{line}: {message}");
    };
    check("etiquetar", "uso: etiquetar corte nombre");
    check(
        "etiquetar 2 12",
        "no se pudo etiquetar: \"12\" es un número",
    );
    check("etiquetar 2 x", "ya es el corte 1");
    check("etiquetar 2 kickr4", "retrigger");
    check("etiquetar 9 hat", "drums no tiene corte 9 (tiene 2)");
    check("etiquetar 1 kick", "corte 1 de drums: «kick»");
    check("etiquetar 2 kick", "la etiqueta \"kick\" ya es del corte 1");
    check("etiquetar 1", "corte 1 de drums: sin etiqueta");
    assert_eq!(app.view().tracks[0].label(0), None);
}

#[test]
fn slices_are_selected_from_the_palette_and_the_script_wins() {
    let song = SONG.replace(
        "local amen = sample(\"break.wav\"):slice(2)",
        "local amen = sample(\"break.wav\"):slice(2):name(2, \"caja\")",
    );
    let (_, mut app, _engine) = open("label_select", Some(&song));
    palette(&mut app, "corte siguiente");
    assert_eq!(app.view().selected_slice, Some(1));
    assert_eq!(app.view().message, "drums: corte 2 (caja) seleccionado");
    palette(&mut app, "corte siguiente");
    assert_eq!(app.view().selected_slice, Some(0), "da la vuelta");
    palette(&mut app, "corte anterior");
    assert_eq!(app.view().selected_slice, Some(1));
    // El script nombra ese corte: la etiqueta se guarda, pero manda :name().
    palette(&mut app, "etiquetar");
    type_query(&mut app, "snare");
    app.handle(special(Key::Enter));
    let message = app.view().message;
    assert!(
        message.ends_with("el script lo nombra «caja» con :name() y manda"),
        "{message}"
    );
    assert_eq!(app.view().tracks[0].label(1), Some("caja"));
}

#[test]
fn loading_warns_about_project_labels_for_another_slice_count() {
    let path = fixture("label_stale", Some(SONG));
    let dir = path.parent().unwrap().to_path_buf();
    let project = dir.join("p.sampler");
    // Etiquetada con slice(4); el script usa slice(2).
    std::fs::write(
        &project,
        "script = song.lua\netiqueta = break.wav 4 1 kick\n",
    )
    .unwrap();
    let (controller, _engine) = new_engine(RATE);
    let mut app = App::open_project(&project, controller, RATE).unwrap();
    let view = app.view();
    assert!(view.error.is_none());
    assert_eq!(
        view.message,
        "cargado: entra en el siguiente compás · ⚠ la etiqueta «kick» de break.wav \
         es de slice(4) y este script usa slice(2): no se aplica"
    );
    let screen = text(&draw(&mut app, 200, 40));
    assert!(
        screen.contains("⚠ la etiqueta «kick» de break.wav es de slice(4)"),
        "{screen}"
    );

    // Usarla da un error que lo explica.
    let text = app
        .text_for_test()
        .replace("\"1 . 2 .  1 . 2 .", "\"kick . 2 .  1 . 2 .");
    replace_all(&mut app, &text);
    app.handle(ctrl('s'));
    let error = app.view().error.unwrap().message;
    assert!(
        error
            .ends_with("la etiqueta «kick» del proyecto es de slice(4) y este script usa slice(2)"),
        "{error}"
    );
}

#[test]
fn the_fx_panel_shows_the_swing() {
    let song = SONG.replace(
        "track(\"kick\", amen:slice_at(1)):steps(\"x . . .  x . . .  x . . .  x . . .\")",
        "track(\"kick\", amen:slice_at(1)):steps(\"x . . .  x . . .  x . . .  x . . .\"):swing(62)",
    );
    let (_, mut app, _engine) = open("swing_panel", Some(&song));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("kick · swing 62"), "{screen}");
    assert!(!screen.contains("drums · swing"), "{screen}");
}

// --- Mute y solo ---

#[test]
fn mute_and_solo_from_the_palette_and_keys_show_and_silence() {
    let (_, mut app, mut engine) = open("mute_solo", Some(SONG));
    // Mute de "drums" desde la paleta.
    palette(&mut app, "mute de la pista");
    assert_eq!(app.view().message, "drums: mute sí");
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("drums M"), "{screen}");
    assert!(screen.contains("drums MUTE"), "{screen}");
    // El motor la calla ya: la pista 1 no da nivel, la 2 sí.
    offline::render(&mut engine, 2 * STEP, 2);
    let status = app.controller_status_for_test();
    assert_eq!(status.tracks[0].peak, 0.0);
    assert!(status.tracks[1].peak > 0.1);

    // Sobrevive a recargar.
    app.handle(ctrl('l'));
    assert!(app.view().tracks[0].mute);

    // Solo de "kick" con la tecla s del modo tocar: drums sigue muda y
    // kick suena; quitar todo deja las dos.
    app.handle(esc());
    app.handle(ctrl('o'));
    app.handle(key('s'));
    assert_eq!(app.view().message, "kick: solo sí");
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("kick S"), "{screen}");
    assert!(screen.contains("kick SOLO"), "{screen}");
    app.handle(key('m'));
    assert_eq!(app.view().message, "kick: mute sí");
    assert!(!app.view().audible(1));
    palette(&mut app, "quitar mute");
    let view = app.view();
    assert!(view.audible(0) && view.audible(1));
    app.controller_status_for_test();
    offline::render(&mut engine, 16 * STEP, 2);
    let status = app.controller_status_for_test();
    assert!(status.tracks[0].peak > 0.1 && status.tracks[1].peak > 0.1);
}

#[test]
fn script_mute_and_solo_show_and_dim_the_others() {
    let song = SONG.replace(
        "track(\"kick\", amen:slice_at(1)):steps(\"x . . .  x . . .  x . . .  x . . .\")",
        "track(\"kick\", amen:slice_at(1)):steps(\"x . . .  x . . .  x . . .  x . . .\"):solo()",
    );
    let (_, mut app, _engine) = open("script_solo", Some(&song));
    let view = app.view();
    assert!(view.tracks[1].solo && !view.audible(0));
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("drums (solo de otra)"), "{screen}");
}

// --- Medidores ---

#[test]
fn master_meter_warns_when_the_limiter_acts_and_when_it_saturates() {
    // SONG: en el paso 0 suenan las dos pistas a 0,5 (suma 1,0): el
    // limitador actúa (pasa de 0,9) pero no satura (no pasa de 1).
    let (_, mut app, mut engine) = open("meters_limit", Some(SONG));
    offline::render(&mut engine, 1000, 2);
    app.tick();
    let view = app.view();
    assert!(
        view.master.limiting && !view.master.clipping,
        "{:?}",
        view.master
    );
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("master"), "{screen}");
    assert!(screen.contains("LIMITADOR"), "{screen}");
    assert!(!screen.contains("SATURA"), "{screen}");
    assert!(screen.contains(" +0.0 dB"), "{screen}");

    // Con la pista a volumen 4, pasa de 1: SATURA en el master y SAT en la
    // pista.
    let loud = SONG.replace(
        "track(\"drums\", amen):steps(\"1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .\")",
        "track(\"drums\", amen):steps(\"1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .\"):gain(4)",
    );
    let (_, mut app, mut engine) = open("meters_clip", Some(&loud));
    offline::render(&mut engine, 1000, 2);
    app.tick();
    let view = app.view();
    assert!(view.master.clipping && view.tracks[0].clipping && !view.tracks[1].clipping);
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("SATURA"), "{screen}");
    assert!(screen.contains(" SAT"), "{screen}");
    // La paleta los reinicia.
    palette(&mut app, "reiniciar medidores");
    let view = app.view();
    assert!(!view.master.limiting && !view.master.clipping && !view.tracks[0].clipping);
    assert_eq!(view.master.db(), None);

    // Una pista sola a 0,5: ni limitador ni saturación.
    let (_, mut app, mut engine) = open("meters_quiet", Some(SONG_KICK_ONLY));
    offline::render(&mut engine, 1000, 2);
    app.tick();
    let view = app.view();
    assert!(!view.master.limiting && !view.master.clipping);
    assert!((view.master.db().unwrap() + 6.02).abs() < 0.01, "-6 dB");
}

// --- Metrónomo ---

#[test]
fn the_metronome_toggles_from_the_palette_and_stays_out_of_exports() {
    let (path, mut app, mut engine) = open("metronome", Some(SONG_KICK_ONLY));
    palette(&mut app, "metrónomo");
    assert!(app.view().metronome);
    assert!(text_of(&mut app).contains("♩ clic"));
    // El motor da el clic. Entre golpe y golpe (pasos 2 y 3: ni kick ni
    // pulso) hay silencio; en el pulso 2 (paso 4) suenan el kick y el clic.
    let audio = offline::render(&mut engine, 2 * 24_000, 2);
    let left: Vec<f32> = audio.iter().step_by(2).copied().collect();
    assert!(left[13_000..23_000].iter().all(|s| *s == 0.0));
    assert!(left[24_001..24_100].iter().any(|s| s.abs() > 0.0));

    // exportar no lo lleva; «exportar ... metronomo», sí.
    let dir = path.parent().unwrap().to_path_buf();
    command(&mut app, "exportar sin.wav 1");
    command(&mut app, "exportar con.wav 1 metronomo");
    assert!(
        app.view().message.ends_with("con metrónomo"),
        "{}",
        app.view().message
    );
    let (_, sin) = read_wav(&dir.join("sin.wav"));
    let (_, con) = read_wav(&dir.join("con.wav"));
    assert_eq!(sin.len(), con.len());
    assert_ne!(sin, con);
    // Sin la pista, lo que queda es el clic: en el silencio entre golpes.
    assert!(sin[2 * 13_000..2 * 23_000].iter().all(|s| *s == 0.0));

    // metronome(false) en el script lo apaga al cargar.
    let text = format!("metronome(false)\n{}", app.text_for_test());
    replace_all(&mut app, &text);
    app.handle(ctrl('s'));
    assert!(!app.view().metronome);
}

// --- Detección de BPM ---

#[test]
fn the_palette_proposes_the_detected_bpm_and_writes_it() {
    let path = fixture("detect_bpm", None);
    let dir = path.parent().unwrap();
    let (audio, _) = synth::synth_break();
    offline::write_wav(dir.join("synth.wav"), &audio, 1, synth::RATE).unwrap();
    let song = "bpm(120)\nlocal amen = sample(\"synth.wav\"):slice(8)\n\
                track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n";
    std::fs::write(&path, song).unwrap();
    let (controller, _engine) = new_engine(RATE);
    let mut app = App::open(&path, controller, RATE).unwrap();
    palette(&mut app, "detectar bpm");
    let view = app.view();
    assert!(
        view.message
            .starts_with("drums: 140 BPM, el sample dura 1 compás"),
        "{}",
        view.message
    );
    assert_eq!(view.prompt.as_deref(), Some("bpm 140"));
    // Enter cambia la línea bpm() del script (sin guardar).
    app.handle(special(Key::Enter));
    assert_eq!(app.editor.lines()[0], "bpm(140)");
    assert!(app.dirty());
    assert!(app.view().message.starts_with("bpm(140) en el script"));
    app.handle(ctrl('z'));
    assert_eq!(app.editor.lines()[0], "bpm(120)");
    // Sin línea bpm(), la añade al principio.
    let text = app.text_for_test().replacen("bpm(120)\n", "", 1);
    replace_all(&mut app, &text);
    command(&mut app, "bpm 172.5");
    assert_eq!(app.editor.lines()[0], "bpm(172.5)");
    command(&mut app, "bpm 9000");
    assert_eq!(app.view().message, "uso: bpm N (de 20 a 400)");
}

#[test]
fn detecting_the_bpm_of_a_one_shot_says_so() {
    // break.wav dura 0,125 s: ningún número de compases da un tempo en rango.
    let (_, mut app, _engine) = open("detect_none", Some(SONG));
    palette(&mut app, "detectar bpm");
    assert!(
        app.view().message.contains("no se encontró un tempo"),
        "{}",
        app.view().message
    );
    assert!(app.view().prompt.is_none());
}

// --- Preescucha ---

#[test]
fn arrows_in_play_mode_browse_and_preview_slices() {
    let (_, mut app, mut engine) = open("preview", Some(SONG));
    // Parado, para oír solo la preescucha.
    app.handle(ctrl('t'));
    offline::render(&mut engine, 2000, 2);
    let text_before = app.text_for_test();
    app.handle(esc());
    app.handle(special(Key::Right));
    let view = app.view();
    assert_eq!(view.selected_slice, Some(1));
    assert_eq!(view.message, "preescucha drums: corte 2");
    let audio = offline::render(&mut engine, 1000, 2);
    assert!(
        audio.iter().any(|s| (s.abs() - 0.25).abs() < 1e-6),
        "suena el corte 2"
    );
    assert!(audio.iter().all(|s| s.abs() < 0.3), "y no el 1");
    // Ni el texto ni las pistas cambian.
    assert_eq!(app.text_for_test(), text_before);
    let status = app.controller_status_for_test();
    assert!(
        status.tracks.iter().all(|t| t.peak == 0.0),
        "no suena por ninguna pista"
    );
    // ← da la vuelta; espacio repite.
    app.handle(special(Key::Left));
    app.handle(special(Key::Left));
    assert_eq!(app.view().selected_slice, Some(1));
    app.handle(key(' '));
    assert_eq!(app.view().message, "preescucha drums: corte 2");
    palette(&mut app, "preescuchar");
    assert_eq!(app.view().message, "preescucha drums: corte 2");
}

// --- Ajuste manual de cortes ---

#[test]
fn slices_are_nudged_with_keys_and_saved_in_the_project() {
    let path = fixture("nudge", Some(SONG));
    let dir = path.parent().unwrap().to_path_buf();
    let project = dir.join("p.sampler");
    std::fs::write(&project, "script = song.lua\n").unwrap();
    let (controller, mut engine) = new_engine(RATE);
    let mut app = App::open_project(&project, controller, RATE).unwrap();
    // break.wav: el corte 2 empieza en 3000 (62,5 ms). Se selecciona y se
    // retrasa 5 ms con «.», y 1 ms con «>».
    app.handle(esc());
    app.handle(special(Key::Right));
    app.handle(key('.'));
    let view = app.view();
    assert!(
        view.message
            .starts_with("corte 2 de drums: empieza en 67.5 ms (guardado en"),
        "{}",
        view.message
    );
    assert_eq!(view.tracks[0].slices[1], 3240..6000);
    assert_eq!(
        view.tracks[0].slices[0],
        0..3240,
        "el anterior acaba donde empieza"
    );
    app.handle(key('>'));
    assert_eq!(app.view().tracks[0].slices[1].start, 3288);
    let saved = std::fs::read_to_string(&project).unwrap();
    assert!(saved.contains("corte = break.wav 2 2 68.5\n"), "{saved}");
    // La vista del sample lo marca con * y da su posición.
    let screen = text(&draw(&mut app, 160, 40));
    assert!(screen.contains("corte 2* 68.5 ms"), "{screen}");
    // Se preescucha al moverlo.
    let audio = offline::render(&mut engine, 200, 2);
    assert!(!audio.is_empty());

    // «,» lo adelanta; no puede pasar de su vecino (1 ms tras el corte 1).
    // (El motor avanza entre teclas, como en tiempo real: cada ajuste es
    // una recarga y una preescucha en la cola de comandos.)
    for _ in 0..20 {
        app.handle(key(','));
        offline::render(&mut engine, 100, 2);
    }
    let view = app.view();
    // De 5 en 5 ms llega a 3,5 ms; otros 5 se saldrían (el tope es 1 ms
    // tras el corte 1): se queda donde estaba.
    assert_eq!(view.tracks[0].slices[1].start, 168);
    assert!(
        view.message
            .starts_with("no se mueve: el ajuste del corte 2"),
        "{}",
        view.message
    );

    // Reabierto, sigue igual; «Quitar el ajuste» lo devuelve al detector.
    let (controller, _engine) = new_engine(RATE);
    let mut app = App::open_project(&project, controller, RATE).unwrap();
    assert_eq!(app.view().tracks[0].slices[1].start, 168);
    assert_eq!(app.view().tracks[0].adjusted, vec![false, true]);
    app.select_slice(true);
    palette(&mut app, "quitar el ajuste");
    assert_eq!(app.view().tracks[0].slices[1].start, 3000);
    assert!(
        !std::fs::read_to_string(&project)
            .unwrap()
            .contains("corte =")
    );
}

mod automation;
mod blocks;
mod browser;
mod clipboard;
mod config;
mod formant;
mod grains;
mod grains_live;
mod grid;
mod history;
mod import;
mod instrument;
mod length;
mod main_window;
mod masking;
mod midi_in;
mod midi_out;
mod record;
mod resample;
mod sections;
mod sends;
mod shapes;
mod slice_fx;
mod split;
mod stretch;
mod swap;
mod tempo;
mod tracker_rows;
mod variants;

const PASTED: &str = "track(\"hats\", amen)\r\n  :steps(\"x . x .\") -- abiertos\r\n  :gain(0.5)";

#[test]
fn pasting_keeps_the_lines_and_the_indent_and_undoes_at_once() {
    let (mut app, _engine) = open_at_end("paste_lines");
    let before = app.editor.lines().to_vec();
    app.paste(PASTED);
    let lines = app.editor.lines();
    assert_eq!(lines.len(), before.len() + 2);
    assert_eq!(
        lines[before.len() - 1..],
        [
            "track(\"hats\", amen)",
            "  :steps(\"x . x .\") -- abiertos",
            "  :gain(0.5)"
        ]
    );
    assert!(app.view().completion.is_none());
    let screen = text(&draw(&mut app, 120, 40));
    assert!(
        screen.contains("  :steps(\"x . x .\") -- abiertos"),
        "{screen}"
    );
    assert!(screen.contains("  :gain(0.5)"), "{screen}");

    // Un solo Ctrl-Z deshace el pegado entero.
    app.handle(ctrl('z'));
    assert_eq!(app.editor.lines(), before);
}

#[test]
fn pasting_with_the_completion_open_accepts_nothing() {
    let (mut app, _engine) = open_at_end("paste_completion");
    type_text(&mut app, "tr");
    assert_eq!(completion_items(&app), ["track", "true"]);
    // Tras moverse, Enter aceptaría la opción marcada.
    app.handle(special(Key::Down));
    app.paste("ue\rbpm(90)\n");
    assert!(app.view().completion.is_none());
    let n = app.editor.lines().len();
    assert_eq!(app.editor.lines()[n - 3..], ["true", "bpm(90)", ""]);
}

#[test]
fn a_burst_of_keys_with_line_breaks_is_a_paste() {
    let (mut app, _engine) = open_at_end("paste_burst");
    let before = app.editor.lines().len();
    type_text(&mut app, "tr");
    app.handle(special(Key::Down));
    // Sin bracketed paste: el terminal manda `\r` (Enter) y a veces `\n`
    // (Ctrl-J), todo en la misma ráfaga.
    let mut burst: Vec<Input> = "ue".chars().map(key).collect();
    burst.push(special(Key::Enter));
    burst.extend("  -- uno".chars().map(key));
    burst.push(ctrl('j'));
    burst.extend("  x".chars().map(key));
    assert_eq!(app.handle_keys(burst), Flow::Continue);
    assert!(app.view().completion.is_none());
    assert_eq!(
        app.editor.lines()[before - 1..],
        ["true", "  -- uno", "  x"]
    );
    // Una tecla sola sigue siendo una tecla.
    app.handle_keys(vec![special(Key::Enter)]);
    assert_eq!(app.editor.lines().len(), before + 3);
}

#[test]
fn a_burst_in_play_mode_plays_pads() {
    let (mut app, _engine) = open_at_end("paste_play");
    let before = app.editor.lines().to_vec();
    app.handle(esc());
    app.handle_keys(vec![key('1'), special(Key::Enter), key('2')]);
    app.paste("x\ny");
    assert_eq!(app.editor.lines(), before);
}
