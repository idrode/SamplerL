//! Cambiar el editor de ventana en caliente (fase L, parte 2, punto 6):
//! sacarlo de una sola ventana a la suya y volver a meterlo, sin perder el
//! texto, el cursor ni el historial de deshacer.

use ratatui_textarea::CursorMove;

use super::split::{editor_window, main_window, pump, pump_until, socket_dir};
use super::{SONG, ctrl, draw, open, text, type_text};
use crate::app::{App, Flow};
use crate::link;
use crate::palette::Action;

/// Una sola ventana con tres cambios en el editor (dos tecleados y uno
/// compuesto) y el último deshecho, para que haya algo que rehacer.
/// Devuelve los textos: el de antes de todo, tras cada cambio y el que
/// queda (el del segundo cambio).
fn edited(test: &str) -> (std::path::PathBuf, App, engine::Engine, Vec<String>) {
    let (path, mut app, engine) = open(test, Some(SONG));
    app.set_socket_dir(socket_dir(&path));
    let mut texts = vec![app.text_for_test()];
    app.editor.move_cursor(CursorMove::Bottom);
    type_text(&mut app, "-- uno");
    texts.push(app.text_for_test());
    app.editor.move_cursor(CursorMove::Top);
    type_text(&mut app, "x");
    texts.push(app.text_for_test());
    // Duplicar la línea: un cambio compuesto.
    app.run_action(Action::DuplicateLines);
    texts.push(app.text_for_test());
    app.handle(ctrl('z'));
    assert_eq!(app.text_for_test(), texts[2]);
    (path, app, engine, texts)
}

/// Deshace hasta que no cambie nada y devuelve los textos por los que pasa.
fn undo_all(app: &mut App) -> Vec<String> {
    let mut seen = vec![app.text_for_test()];
    loop {
        app.handle(ctrl('z'));
        let now = app.text_for_test();
        if &now == seen.last().unwrap() {
            return seen;
        }
        seen.push(now);
    }
}

#[test]
fn the_editor_goes_out_to_its_window_with_its_cursor_and_history() {
    let (path, mut single, _engine, texts) = edited("swap_out");
    let cursor = single.editor.cursor();
    // Sin terminal configurada: se queda escuchando y dice cómo abrirlo.
    single.run_action(Action::EditorWindow);
    assert!(
        single.view().message.contains("tui --window-editor"),
        "{}",
        single.view().message
    );
    assert!(single.view().editor_outside);
    assert!(single.view().playing_mode);
    let (mut editor, _idle) = editor_window(&path);
    pump_until(&mut single, &mut editor, |_, e| {
        e.view().message.contains("historial")
    });
    // El texto sin guardar, el cursor y lo que rehace Ctrl-R.
    assert_eq!(editor.text_for_test(), texts[2]);
    assert_eq!(editor.editor.cursor(), cursor);
    editor.handle(ctrl('r'));
    assert_eq!(editor.text_for_test(), texts[3]);
    editor.handle(ctrl('z'));
    // Y lo que deshace Ctrl-Z, paso a paso hasta el principio (lo tecleado
    // va letra a letra; el duplicado, de una vez).
    let seen = undo_all(&mut editor);
    assert_eq!(seen.first(), Some(&texts[2]));
    assert_eq!(seen.last(), Some(&texts[0]));
    assert!(seen.contains(&texts[1]), "{seen:?}");
    assert_eq!(seen.len(), "-- uno".len() + "x".len() + 1);
    // Con el texto de la principal, la principal no lo tiene por cambiado.
    let screen = text(&draw(&mut single, 120, 40));
    assert!(screen.contains("⇄ editor en su ventana"), "{screen}");
}

#[test]
fn the_editor_comes_back_from_its_window_with_its_cursor_and_history() {
    let (path, mut main, _engine, _) = edited("swap_back");
    main.run_action(Action::EditorWindow);
    let (mut editor, _idle) = editor_window(&path);
    pump_until(&mut main, &mut editor, |_, e| {
        e.view().message.contains("historial")
    });
    // Se escribe algo más en el editor y se vuelve desde allí.
    editor.editor.move_cursor(CursorMove::Bottom);
    type_text(&mut editor, "-- dos");
    let text_out = editor.text_for_test();
    let cursor = editor.editor.cursor();
    let history = undo_all(&mut editor);
    for _ in 1..history.len() {
        editor.handle(ctrl('r'));
    }
    assert_eq!(editor.text_for_test(), text_out);
    editor
        .editor
        .move_cursor(CursorMove::Jump(cursor.0 as u16, cursor.1 as u16));
    assert_eq!(editor.run_action(Action::EditorBack), Flow::Quit);
    assert!(editor.leaving());
    drop(editor);
    for _ in 0..4 {
        main.tick();
    }
    // La principal vuelve a ser una sola ventana, en el editor, con todo.
    let view = main.view();
    assert!(!view.editor_outside);
    assert!(!view.playing_mode);
    assert!(view.link.is_none());
    assert!(view.dirty, "sin guardar");
    assert_eq!(main.text_for_test(), text_out);
    assert_eq!(main.editor.cursor(), cursor);
    assert_eq!(undo_all(&mut main), history);
    // El socket ya no está: otra principal del mismo script puede escuchar.
    let socket = link::socket_path(&socket_dir(&path), &path);
    assert!(!socket.exists());
    let screen = text(&draw(&mut main, 120, 40));
    assert!(screen.starts_with("┌ song.lua"), "{screen}");
    // Y se puede volver a sacar.
    main.run_action(Action::EditorWindow);
    assert!(main.view().editor_outside, "{}", main.view().message);
}

#[test]
fn the_main_window_can_ask_for_the_editor_back() {
    let (path, mut main, _engine, texts) = edited("swap_ask");
    main.run_action(Action::EditorWindow);
    let (mut editor, _idle) = editor_window(&path);
    pump_until(&mut main, &mut editor, |_, e| {
        e.view().message.contains("historial")
    });
    main.run_action(Action::EditorBack);
    assert!(main.view().message.contains("pidiendo"));
    pump_until(&mut main, &mut editor, |m, e| {
        e.leaving() && !m.view().editor_outside
    });
    assert_eq!(main.text_for_test(), texts[2]);
    main.handle(ctrl('r'));
    assert_eq!(main.text_for_test(), texts[3]);
}

#[test]
fn without_an_editor_the_main_window_takes_it_back_with_its_text() {
    let (path, mut main, _engine) = open("swap_alone", Some(SONG));
    main.set_socket_dir(socket_dir(&path));
    main.run_action(Action::EditorWindow);
    assert!(main.view().editor_outside);
    main.run_action(Action::EditorBack);
    assert!(!main.view().editor_outside);
    assert!(!main.view().playing_mode);
    assert_eq!(
        main.view().message,
        "el editor vuelve a estar en esta ventana"
    );
    // En una sola ventana, ya está aquí.
    main.run_action(Action::EditorBack);
    assert_eq!(main.view().message, "el editor ya está en esta ventana");
}

#[test]
fn an_editor_without_a_main_window_stays() {
    let path = super::fixture("swap_orphan", Some(SONG));
    let (mut main, _engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    drop(main);
    editor.tick();
    assert_eq!(editor.run_action(Action::EditorBack), Flow::Continue);
    assert!(!editor.leaving());
    assert!(
        editor.view().message.contains("principal"),
        "{}",
        editor.view().message
    );
}
