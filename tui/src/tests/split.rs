//! Editor en su propia ventana (fase L): la principal y el editor como dos
//! `App` unidas por el socket de verdad, en el mismo proceso. El motor de
//! la principal se mueve con render offline; el del editor no se mueve
//! nunca.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use engine::{Engine, new_engine, offline};

use super::{RATE, SONG, STEP, ctrl, draw, fixture, text, type_text};
use crate::app::App;
use crate::link::{self, Conn, Server, ToEditor, ToMain};
use crate::palette::Action;
use crate::ui;

/// Un compás a 120 BPM.
const BAR: usize = 16 * STEP;

/// La canción con las cajas en todos los pasos (corte 2).
const ALL_SNARES: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("2 2 2 2  2 2 2 2  2 2 2 2  2 2 2 2")
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
"#;

/// Carpeta de los sockets del test (privada, como la de verdad).
fn socket_dir(script: &Path) -> PathBuf {
    let dir = script.parent().unwrap().join("run");
    link::private_dir(&dir).unwrap();
    dir
}

/// La principal de un script, ya escuchando.
fn main_window(path: &Path, terminal: Option<Vec<String>>) -> (App, Engine) {
    let (controller, engine) = new_engine(RATE);
    let mut app = App::open(path, controller, RATE).unwrap();
    let socket = link::socket_path(&socket_dir(path), path);
    app.serve(Server::bind(&socket).unwrap(), terminal, "tui".into());
    (app, engine)
}

/// El editor de ese script (su motor se devuelve para que siga vivo, pero
/// no se mueve).
fn editor_window(path: &Path) -> (App, Engine) {
    let (controller, engine) = new_engine(RATE);
    let app = App::open_editor(path, controller, RATE, &socket_dir(path)).unwrap();
    (app, engine)
}

/// Unas vueltas del bucle de las dos ventanas.
fn pump(main: &mut App, editor: &mut App) {
    for _ in 0..4 {
        main.tick();
        editor.tick();
    }
}

/// Espera (con vueltas) a que se cumpla algo; lo bastante para que el
/// editor envíe lo escrito (lo manda como mucho cada 100 ms).
fn pump_until(main: &mut App, editor: &mut App, what: impl Fn(&App, &App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    while !what(main, editor) {
        assert!(Instant::now() < deadline, "no llegó a pasar");
        pump(main, editor);
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn link_line(app: &App) -> String {
    app.view().link.unwrap_or_default()
}

/// El corte que suena en la pista «drums» (desde 0).
fn drums_slice(app: &App) -> Option<usize> {
    app.view()
        .tracks
        .iter()
        .find(|t| t.name == "drums")
        .and_then(|t| t.playing_slice)
}

#[test]
fn the_editor_takes_the_text_and_follows_the_main_window() {
    let path = fixture("split_follow", Some(SONG));
    let (mut main, mut engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    assert_eq!(editor.text_for_test(), SONG);
    assert_eq!(link_line(&main), "⇄ editor en su ventana");

    offline::render(&mut engine, 3 * STEP + 100, 2);
    pump_until(&mut main, &mut editor, |_, e| {
        link_line(e).contains("compás 0 paso 03")
    });
    let line = link_line(&editor);
    assert!(line.starts_with("⇄ principal ▶ 120 BPM"), "{line}");
    assert!(line.contains("drums corte 1"), "{line}");

    // Las dos ventanas se dibujan: la del editor, con la línea del enlace;
    // la principal, sin editor que escribir.
    let screen = text(&ui_editor(&mut editor, 80, 20));
    assert!(screen.contains("⇄ principal ▶ 120 BPM"), "{screen}");
    assert!(screen.contains("track(\"drums\", amen)"), "{screen}");
    let screen = text(&draw(&mut main, 140, 44));
    assert!(screen.contains("⇄ editor en su ventana"), "{screen}");
    assert!(screen.contains("TOCAR · 1-0 q-i "), "{screen}");
    assert!(!screen.contains("q-i · Esc"), "{screen}");
}

fn ui_editor(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| ui::draw_editor_window(frame, app))
        .unwrap();
    terminal.backend().buffer().clone()
}

#[test]
fn a_load_from_the_editor_enters_at_the_next_bar() {
    let path = fixture("split_load", Some(SONG));
    let (mut main, mut engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);

    // A mitad del primer compás se carga otro patrón desde el editor.
    offline::render(&mut engine, 4 * STEP + 100, 2);
    editor.editor.select_all();
    editor.editor.cut();
    type_text(&mut editor, ALL_SNARES);
    editor.handle(ctrl('l'));
    pump_until(&mut main, &mut editor, |m, _| {
        m.view()
            .message
            .starts_with("cargado: entra en el siguiente compás")
    });
    assert_eq!(main.text_for_test(), editor.text_for_test());
    // El editor recibe el resultado de la carga.
    pump_until(&mut main, &mut editor, |_, e| {
        e.view()
            .message
            .starts_with("cargado: entra en el siguiente compás")
    });
    assert!(editor.view().error.is_none());

    // Hasta el final del compás, lo de antes (paso 13: corte 1)...
    offline::render(&mut engine, 8 * STEP, 2);
    main.tick();
    assert_eq!(drums_slice(&main), Some(0));
    // ...y desde el compás siguiente, lo nuevo (corte 2 en el paso 1).
    offline::render(&mut engine, BAR - 12 * STEP, 2);
    main.tick();
    assert_eq!(drums_slice(&main), Some(1));
}

#[test]
fn an_invalid_load_keeps_playing_and_the_error_shows_in_the_editor() {
    let path = fixture("split_error", Some(SONG));
    let (mut main, mut engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    offline::render(&mut engine, STEP, 2);

    editor
        .editor
        .move_cursor(ratatui_textarea::CursorMove::Bottom);
    editor.editor.move_cursor(ratatui_textarea::CursorMove::End);
    type_text(&mut editor, "\ntrack(\"x\", nada):steps(\"1\")");
    editor.handle(ctrl('l'));
    pump_until(&mut main, &mut editor, |_, e| e.view().error.is_some());
    let error = editor.view().error.unwrap();
    assert_eq!(error.line, Some(5), "{}", error.message);
    assert!(main.view().error.is_some());
    // Sigue sonando lo anterior.
    offline::render(&mut engine, BAR, 2);
    main.tick();
    assert!(main.view().transport.running);
    assert_eq!(main.view().tracks.len(), 2);
    let screen = text(&ui_editor(&mut editor, 80, 20));
    assert!(screen.contains("error en la línea 5"), "{screen}");

    // Arreglado, el error se va de las dos ventanas.
    editor.editor.select_all();
    editor.editor.cut();
    type_text(&mut editor, SONG);
    editor.handle(ctrl('l'));
    pump_until(&mut main, &mut editor, |m, e| {
        e.view().error.is_none() && m.view().error.is_none()
    });
}

#[test]
fn the_editor_can_close_and_come_back_and_recovers_the_text() {
    let path = fixture("split_reopen", Some(SONG));
    let (mut main, mut engine) = main_window(&path, None);
    let (mut editor, idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    // Algo escrito (sin guardar ni cargar) llega a la principal.
    type_text(&mut editor, "-- sin guardar\n");
    pump_until(&mut main, &mut editor, |m, e| {
        m.text_for_test() == e.text_for_test()
    });

    // Se cierra el editor: la principal sigue sonando.
    drop(editor);
    drop(idle);
    offline::render(&mut engine, STEP, 2);
    main.tick();
    assert!(
        main.view().message.starts_with("el editor se cerró"),
        "{}",
        main.view().message
    );
    assert!(link_line(&main).contains("sin conectar"));
    offline::render(&mut engine, BAR, 2);
    main.tick();
    assert!(main.view().transport.running);
    // (Sin editor, salir avisa: el texto de aquí no está guardado.)
    main.handle(ctrl('q'));
    assert!(
        main.view().message.contains("Ctrl-Q otra vez"),
        "{}",
        main.view().message
    );

    // Uno nuevo recupera el texto (lo escrito antes, sin guardar).
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    assert!(editor.text_for_test().starts_with("-- sin guardar\n"));
    assert!(editor.view().dirty);
    // Con el editor conectado, salir de la principal no pierde nada.
    assert_eq!(main.handle(ctrl('q')), crate::app::Flow::Quit);
}

#[test]
fn the_editor_survives_the_main_window_and_reconnects_by_itself() {
    let path = fixture("split_main_dies", Some(SONG));
    let (mut main, engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);

    drop(main);
    drop(engine);
    editor.tick();
    assert!(
        editor
            .view()
            .message
            .starts_with("se perdió la ventana principal"),
        "{}",
        editor.view().message
    );
    assert!(link_line(&editor).starts_with("✗ sin ventana principal"));
    // Se sigue editando y guardando.
    type_text(&mut editor, "-- a solas\n");
    editor.handle(ctrl('s'));
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("-- a solas\n")
    );
    assert!(
        editor.view().message.contains("no se cargó"),
        "{}",
        editor.view().message
    );
    type_text(&mut editor, "-- más\n");

    // Vuelve la principal (con lo guardado): el editor se reconecta solo y
    // se queda con su texto, que le manda.
    let (mut main, _engine) = main_window(&path, None);
    std::thread::sleep(link::RETRY);
    pump_until(&mut main, &mut editor, |m, e| {
        m.text_for_test() == e.text_for_test() && link_line(e).starts_with("⇄")
    });
    assert!(editor.text_for_test().starts_with("-- a solas\n-- más\n"));
    assert!(
        editor.view().message.starts_with("reconectado"),
        "{}",
        editor.view().message
    );
}

#[test]
fn text_changed_in_the_main_window_reaches_the_editor_and_undoes_at_once() {
    let path = fixture("split_text", Some(SONG));
    let (mut main, _engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);

    // La orden va del editor a la principal, que cambia el texto.
    editor.run_command("bpm 140");
    pump_until(&mut main, &mut editor, |_, e| {
        e.text_for_test().starts_with("bpm(140)")
    });
    assert!(main.text_for_test().starts_with("bpm(140)"));
    assert!(editor.view().dirty);
    editor.handle(ctrl('z'));
    assert_eq!(editor.text_for_test(), SONG);

    // Si la principal lo cambia mientras se escribe aquí, gana lo escrito.
    pump_until(&mut main, &mut editor, |m, _| m.text_for_test() == SONG);
    type_text(&mut editor, "--");
    main.run_command("bpm 150");
    pump(&mut main, &mut editor);
    let kept = editor.text_for_test();
    assert!(
        kept.starts_with("bpm(120)") && kept.ends_with("--\n"),
        "{kept}"
    );
    assert!(
        editor.view().message.contains("se queda el tuyo"),
        "{}",
        editor.view().message
    );
    pump_until(&mut main, &mut editor, |m, e| {
        m.text_for_test() == e.text_for_test()
    });
}

#[test]
fn engine_actions_go_to_the_main_window_and_text_ones_stay() {
    let path = fixture("split_actions", Some(SONG));
    let (mut main, mut engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    offline::render(&mut engine, STEP, 2);
    main.tick();
    assert!(main.view().transport.running);

    // Marcha/paro desde el editor para el motor de la principal.
    editor.handle(ctrl('t'));
    pump(&mut main, &mut editor);
    offline::render(&mut engine, STEP, 2);
    main.tick();
    assert!(!main.view().transport.running);
    pump_until(&mut main, &mut editor, |_, e| link_line(e).contains("■"));

    // Mute por la paleta, en la pista seleccionada de la principal.
    editor.run_action(Action::Mute);
    pump_until(&mut main, &mut editor, |m, _| m.view().tracks[0].mute);

    // Lo del modo tocar no está en el editor; lo del texto, no en la
    // principal.
    editor.run_action(Action::PlayMode);
    assert!(!editor.view().playing_mode);
    assert!(editor.view().message.contains("ventana principal"));
    let before = main.text_for_test();
    main.run_action(Action::DuplicateLines);
    assert_eq!(main.text_for_test(), before);
    assert_eq!(main.view().message, "eso se hace en la ventana del editor");
    main.paste("algo");
    assert_eq!(main.text_for_test(), before);

    // Etiquetar: la orden se escribe en el editor y se hace allí.
    editor.run_action(Action::LabelSlice);
    type_text(&mut editor, "golpe\n");
    pump_until(&mut main, &mut editor, |m, _| {
        m.view().message.contains("golpe") || m.view().tracks[0].labels.iter().any(|l| l.is_some())
    });

    // La principal no se cree que el archivo cambió fuera cuando lo
    // guarda el editor.
    type_text(&mut editor, "-- x\n");
    editor.handle(ctrl('s'));
    std::thread::sleep(Duration::from_millis(600));
    pump(&mut main, &mut editor);
    assert!(!main.view().external_change);
}

#[test]
fn pasting_in_the_editor_window_keeps_the_indent_and_undoes_at_once() {
    let path = fixture("split_paste", Some(SONG));
    let (mut main, _engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    editor
        .editor
        .move_cursor(ratatui_textarea::CursorMove::Bottom);
    editor.editor.move_cursor(ratatui_textarea::CursorMove::End);
    type_text(&mut editor, "\n");
    let before = editor.editor.lines().to_vec();
    editor.paste("track(\"hats\", amen)\r\n  :steps(\"x . x .\")\r\n    :gain(0.5)\n");
    let lines = editor.editor.lines();
    assert_eq!(
        lines[before.len() - 1..],
        [
            "track(\"hats\", amen)",
            "  :steps(\"x . x .\")",
            "    :gain(0.5)",
            ""
        ]
    );
    editor.handle(ctrl('z'));
    assert_eq!(editor.editor.lines(), before);
}

#[test]
fn a_second_editor_replaces_the_first_and_other_versions_are_refused() {
    let path = fixture("split_second", Some(SONG));
    let (mut main, _engine) = main_window(&path, None);
    let (mut first, _idle) = editor_window(&path);
    pump(&mut main, &mut first);
    let (mut second, _idle2) = editor_window(&path);
    pump(&mut main, &mut second);
    first.tick();
    assert!(
        first.view().message.contains("otro editor"),
        "{}",
        first.view().message
    );
    // El primero no se reconecta solo (le quitaría el sitio al segundo).
    std::thread::sleep(link::RETRY);
    first.tick();
    assert_eq!(link_line(&first), "✗ sin ventana principal");

    // Un extremo con otro protocolo recibe adiós.
    let socket = link::socket_path(&socket_dir(&path), &path);
    let mut raw = Conn::new(UnixStream::connect(&socket).unwrap()).unwrap();
    raw.send(&ToMain::Hello { version: 999 }).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let bye = loop {
        main.tick();
        if let Ok(messages) = raw.receive::<ToEditor>()
            && let Some(bye) = messages.into_iter().find(|m| matches!(m, ToEditor::Bye(_)))
        {
            break bye;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    };
    assert_eq!(
        bye,
        ToEditor::Bye(format!(
            "protocolo distinto (principal {}, editor 999)",
            link::VERSION
        ))
    );
    // Basura por el socket: se suelta ese extremo y nada más.
    let mut junk = UnixStream::connect(&socket).unwrap();
    junk.write_all(b"esto no es un mensaje\n").unwrap();
    main.tick();
    main.tick();
    assert!(
        link_line(&main).contains("sin conectar"),
        "{}",
        link_line(&main)
    );
}

#[test]
fn the_main_window_opens_the_editor_with_the_configured_terminal() {
    let path = fixture("split_launch", Some(SONG));
    let out = path.with_file_name("lanzado.txt");
    // Una «terminal» que apunta sus argumentos en un archivo.
    let terminal = vec![
        "sh".into(),
        "-c".into(),
        format!("printf '%s\\n' \"$@\" > '{}'", out.display()),
        "sh".into(),
    ];
    assert_eq!(
        crate::app::editor_command(&terminal, Path::new("/bin/tui"), &path),
        [
            "sh",
            "-c",
            &terminal[2],
            "sh",
            "/bin/tui",
            "--window-editor",
            &path.display().to_string()
        ]
    );
    let (mut main, _engine) = main_window(&path, Some(terminal));
    main.run_action(Action::EditorWindow);
    assert!(
        main.view().message.starts_with("abriendo el editor"),
        "{}",
        main.view().message
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::fs::read_to_string(&out).map_or(true, |t| t.lines().count() < 3) {
        assert!(Instant::now() < deadline, "no se lanzó");
        std::thread::sleep(Duration::from_millis(20));
    }
    let args = std::fs::read_to_string(&out).unwrap();
    assert_eq!(args, format!("tui\n--window-editor\n{}\n", path.display()));

    // Sin terminal configurada, dice cómo abrirlo a mano.
    let path = fixture("split_launch_none", Some(SONG));
    let (mut main, _engine) = main_window(&path, None);
    main.run_action(Action::EditorWindow);
    assert!(
        main.view().message.contains("tui --window-editor"),
        "{}",
        main.view().message
    );
    // Y en una sola ventana, que hace falta --split.
    let (_, mut single, _engine) = super::open("split_single", Some(SONG));
    single.run_action(Action::EditorWindow);
    assert!(single.view().message.contains("--split"));
}

/// Para ver las dos ventanas sin terminal:
/// `cargo test -p tui pantalla_ventanas -- --ignored --nocapture`.
#[test]
#[ignore]
fn pantalla_ventanas() {
    let path = fixture("split_pantalla", Some(SONG));
    let (mut main, mut engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    offline::render(&mut engine, 5 * STEP + 100, 2);
    pump_until(&mut main, &mut editor, |_, e| {
        link_line(e).contains("paso 05")
    });
    println!("{}", text(&draw(&mut main, 120, 40)));
    println!("{}", text(&ui_editor(&mut editor, 80, 16)));
}
