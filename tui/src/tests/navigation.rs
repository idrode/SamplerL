//! Navegación por el editor: las teclas del widget de texto que ya había
//! y las propias (principio y final del texto, ir a una línea, buscar).

use super::*;

/// Una tecla con modificadores.
fn with(key: Key, ctrl: bool, alt: bool, shift: bool) -> Input {
    Input {
        key,
        ctrl,
        alt,
        shift,
    }
}

const TEXT: &str = "uno dos tres\n  cuatro cinco\nseis";

fn open_text(test: &str, text: &str) -> (App, Engine) {
    let (_, mut app, engine) = open(test, Some(text));
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(0, 0));
    (app, engine)
}

fn selected(app: &App) -> Option<String> {
    crate::editing::selected_text(&app.editor)
}

#[test]
fn words_lines_and_pages_move_and_shift_selects() {
    let (mut app, _engine) = open_text("nav_existing", TEXT);
    // Ctrl-→ y Ctrl-← por palabras.
    app.handle(with(Key::Right, true, false, false));
    assert_eq!(app.editor.cursor(), (0, 4));
    app.handle(with(Key::Right, true, false, false));
    assert_eq!(app.editor.cursor(), (0, 8));
    app.handle(with(Key::Left, true, false, false));
    assert_eq!(app.editor.cursor(), (0, 4));
    // Con Shift seleccionan palabra a palabra.
    app.handle(with(Key::Right, true, false, true));
    assert_eq!(selected(&app).as_deref(), Some("dos "));
    app.handle(with(Key::Left, true, false, true));
    assert_eq!(selected(&app), None);
    // Inicio y Fin de línea (Shift selecciona).
    app.handle(with(Key::End, false, false, false));
    assert_eq!(app.editor.cursor(), (0, 12));
    assert_eq!(selected(&app), None, "moverse sin Shift quita la selección");
    app.handle(with(Key::Home, false, false, true));
    assert_eq!(selected(&app).as_deref(), Some("uno dos tres"));
    // Shift-flechas, también entre líneas.
    app.handle(with(Key::Home, false, false, false));
    app.handle(with(Key::Down, false, false, true));
    app.handle(with(Key::Right, false, false, true));
    assert_eq!(selected(&app).as_deref(), Some("uno dos tres\n "));
    // AvPág y RePág mueven el cursor una pantalla del editor.
    let long: String = (1..=100).map(|i| format!("-- {i}\n")).collect();
    replace_all(&mut app, &long);
    app.editor.move_cursor(ratatui_textarea::CursorMove::Top);
    draw(&mut app, 130, 40);
    app.handle(special(Key::PageDown));
    let page = app.editor.cursor().0;
    assert!(page > 10, "AvPág baja una pantalla ({page})");
    app.handle(with(Key::PageDown, false, false, true));
    assert_eq!(app.editor.cursor().0, 2 * page);
    assert_eq!(selected(&app).map(|s| s.lines().count()), Some(page));
    for _ in 0..2 {
        draw(&mut app, 130, 40);
        app.handle(special(Key::PageUp));
    }
    // (Ya arriba del todo, RePág deja el cursor en la pantalla de arriba,
    // no en la primera línea: es del widget.)
    assert!(app.editor.cursor().0 < page);
}

#[test]
fn ctrl_home_and_ctrl_end_go_to_the_ends_of_the_text() {
    let (mut app, _engine) = open_text("nav_edges", TEXT);
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(1, 4));
    app.handle(with(Key::End, true, false, false));
    assert_eq!(app.editor.cursor(), (2, 4));
    app.handle(with(Key::Home, true, false, false));
    assert_eq!(app.editor.cursor(), (0, 0));
    // Inicio y Fin solos siguen siendo de la línea.
    app.handle(special(Key::End));
    assert_eq!(app.editor.cursor(), (0, 12));
    // Con Shift seleccionan hasta el principio o el final.
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(1, 2));
    app.handle(with(Key::End, true, false, true));
    assert_eq!(selected(&app).as_deref(), Some("cuatro cinco\nseis"));
    app.handle(with(Key::Home, true, false, true));
    assert_eq!(selected(&app).as_deref(), Some("uno dos tres\n  "));
    // Desde la paleta: «Ir al final del texto».
    app.open_palette();
    type_text(&mut app, "final del texto");
    app.handle(special(Key::Enter));
    assert_eq!(app.editor.cursor(), (2, 4));
    assert_eq!(selected(&app), None);
    // En el modo tocar no mueven el texto.
    app.handle(esc());
    app.handle(with(Key::Home, true, false, false));
    assert_eq!(app.editor.cursor(), (2, 4));
    // Y se ven en la paleta con su atajo.
    app.handle(esc());
    app.open_palette();
    type_text(&mut app, "principio del texto");
    let screen = text_of(&mut app);
    assert!(screen.contains("Ir al principio del texto"), "{screen}");
    assert!(screen.contains("Ctrl-Inicio"), "{screen}");
}
