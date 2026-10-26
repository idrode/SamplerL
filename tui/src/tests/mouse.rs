//! El ratón en el editor (fase R): clic lleva el cursor (también con el
//! texto desplazado), arrastrar selecciona, la rueda desplaza, y fuera
//! del editor (o en el modo tocar, o con la paleta abierta) no hace nada.

use super::*;
use crate::app::MouseAction;
use crate::config::Config;

/// Clic en el carácter `nth` de la primera aparición de `needle` en
/// pantalla.
fn click_on(app: &mut App, buffer: &Buffer, needle: &str, nth: u16) {
    let (x, y) = find(buffer, needle).unwrap_or_else(|| panic!("no se ve «{needle}»"));
    app.mouse(MouseAction::Down, x + nth, y);
    app.mouse(MouseAction::Up, x + nth, y);
}

#[test]
fn a_click_moves_the_cursor_where_it_lands() {
    let (_, mut app, _engine) = open("mouse_click", Some(SONG));
    let buffer = draw(&mut app, 120, 40);
    // SONG: la línea 4 (fila 3) es la del kick.
    click_on(&mut app, &buffer, "track(\"kick\"", 7);
    assert_eq!(app.editor.cursor(), (3, 7));
    // Más allá del final de la línea: su final.
    let (x, y) = find(&buffer, "bpm(120)").unwrap();
    app.mouse(MouseAction::Down, x + 40, y);
    assert_eq!(app.editor.cursor(), (0, 8));
    // Sobre los números de línea: el principio.
    app.mouse(MouseAction::Down, x.saturating_sub(1), y + 2);
    assert_eq!(app.editor.cursor(), (2, 0));
    // Fuera del editor (el panel de la derecha): nada.
    app.mouse(MouseAction::Down, 100, 5);
    assert_eq!(app.editor.cursor(), (2, 0));
}

#[test]
fn a_click_follows_the_scroll() {
    let lines: String = (1..=80).map(|i| format!("-- línea {i}\n")).collect();
    let song = format!("{SONG}{lines}");
    let (_, mut app, _engine) = open("mouse_scroll", Some(&song));
    app.editor.move_cursor(ratatui_textarea::CursorMove::Bottom);
    let buffer = draw(&mut app, 120, 40);
    let (top, _) = app.editor.scroll_offset();
    assert!(top > 40, "desplazado: {top}");
    click_on(&mut app, &buffer, "-- línea 70", 3);
    assert_eq!(app.editor.cursor(), (69 + SONG.lines().count(), 3));

    // La rueda desplaza (tres líneas cada paso).
    let (x, y) = find(&buffer, "-- línea 70").unwrap();
    app.mouse(MouseAction::ScrollUp, x, y);
    app.mouse(MouseAction::ScrollUp, x, y);
    draw(&mut app, 120, 40);
    assert_eq!(app.editor.scroll_offset().0, top - 6);
}

#[test]
fn dragging_selects_and_ctrl_c_copies_it() {
    let (_, mut app, _engine) = open("mouse_drag", Some(SONG));
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = find(&buffer, "track(\"kick\"").unwrap();
    app.mouse(MouseAction::Down, x, y);
    // Sin moverse, no hay selección.
    app.mouse(MouseAction::Drag, x, y);
    assert_eq!(app.editor.selection_range(), None);
    app.mouse(MouseAction::Drag, x + 5, y);
    app.mouse(MouseAction::Up, x + 5, y);
    assert_eq!(app.editor.selection_range(), Some(((3, 0), (3, 5))));
    app.editor.copy();
    assert_eq!(app.editor.yank_text(), "track");
    // Un clic nuevo la quita.
    app.mouse(MouseAction::Down, x + 2, y);
    assert_eq!(app.editor.selection_range(), None);
    // Hacia arriba, a otra línea.
    app.mouse(MouseAction::Drag, x, y - 1);
    assert_eq!(app.editor.selection_range(), Some(((2, 0), (3, 2))));
}

#[test]
fn the_mouse_does_nothing_in_play_mode_with_the_palette_or_when_disabled() {
    let (_, mut app, _engine) = open("mouse_off", Some(SONG));
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = find(&buffer, "track(\"kick\"").unwrap();
    app.handle(esc());
    app.mouse(MouseAction::Down, x + 3, y);
    assert_eq!(app.editor.cursor(), (0, 0), "modo tocar");
    app.handle(esc());
    app.open_palette();
    app.mouse(MouseAction::Down, x + 3, y);
    assert_eq!(app.editor.cursor(), (0, 0), "con la paleta");
    app.handle(esc());

    let config = Config::parse("[editor]\nraton = false\n", true, "c.toml");
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    app.set_config(&config);
    assert!(!app.mouse_enabled());
    app.mouse(MouseAction::Down, x + 3, y);
    assert_eq!(app.editor.cursor(), (0, 0), "sin ratón");
    let config = Config::parse("[editor]\nraton = 1\n", true, "c.toml");
    assert_eq!(config.warnings, ["[editor] raton: true o false"]);
}

#[test]
fn the_editor_window_takes_clicks_too_and_the_main_one_does_not() {
    let path = fixture("mouse_split", Some(SONG));
    let (mut main, _engine) = super::split::main_window(&path, None);
    let (mut editor, _idle) = super::split::editor_window(&path);
    super::split::pump(&mut main, &mut editor);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    terminal
        .draw(|frame| ui::draw_editor_window(frame, &mut editor))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    click_on(&mut editor, &buffer, "track(\"kick\"", 4);
    assert_eq!(editor.editor.cursor(), (3, 4));
    // La principal, sin editor, no tiene dónde.
    draw(&mut main, 120, 40);
    assert_eq!(main.editor_area, None);
    main.mouse(MouseAction::Down, 10, 3);
    assert_eq!(main.editor.cursor(), (0, 0));
}

#[test]
fn a_click_on_a_long_line_scrolled_sideways() {
    let long = format!("-- {}fin", "x".repeat(200));
    let song = format!("{SONG}{long}\n");
    let (_, mut app, _engine) = open("mouse_long", Some(&song));
    let row = SONG.lines().count();
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(row as u16, 400));
    let buffer = draw(&mut app, 120, 40);
    let (_, left) = app.editor.scroll_offset();
    assert!(left > 100, "desplazado a la derecha: {left}");
    click_on(&mut app, &buffer, "fin", 1);
    assert_eq!(app.editor.cursor(), (row, long.chars().count() - 2));
}
