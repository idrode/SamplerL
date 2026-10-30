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

/// Abre SONG como proyecto (para que los ajustes de cortes se guarden).
fn open_project(test: &str) -> (PathBuf, App, Engine) {
    let path = fixture(test, Some(SONG));
    let project = path.parent().unwrap().join("p.sampler");
    std::fs::write(&project, "script = song.lua\n").unwrap();
    let (controller, engine) = new_engine(RATE);
    let app = App::open_project(&project, controller, RATE).unwrap();
    (project, app, engine)
}

/// Columna de la terminal donde se dibuja la marca del corte que empieza
/// en `start` (break.wav: 6000 frames).
fn mark_column(app: &App, start: usize) -> u16 {
    let area = app.sample_area.expect("se dibujó el sample");
    area.x + (start as f64 / 6000.0 * area.width as f64).floor() as u16
}

#[test]
fn dragging_a_slice_mark_moves_it_and_saves_the_project() {
    let (project, mut app, _engine) = open_project("mouse_slice_drag");
    draw(&mut app, 160, 40);
    let area = app.sample_area.unwrap();
    let (x, y) = (mark_column(&app, 3000), area.y + 1);
    app.mouse(MouseAction::Down, x, y);
    assert!(
        app.view().message.contains("corte 2 agarrado"),
        "{}",
        app.view().message
    );
    // Mientras se arrastra, solo cambia la vista: el anterior acaba ahí.
    app.mouse(MouseAction::Drag, x + 5, y);
    let moved = 3000 + (5.0 * 6000.0 / area.width as f64).round() as usize;
    let view = app.view();
    assert_eq!(view.tracks[0].slices[1].start, moved);
    assert_eq!(view.tracks[0].slices[0].end, moved);
    assert_eq!(view.selected_slice, Some(1));
    assert!(
        !std::fs::read_to_string(&project)
            .unwrap()
            .contains("corte =")
    );
    // Al soltar, se pone como con «.» (redondeado a 0,1 ms) y se guarda.
    app.mouse(MouseAction::Up, x + 5, y);
    let view = app.view();
    let start = view.tracks[0].slices[1].start;
    assert!(
        start.abs_diff(moved) <= RATE as usize / 10_000 + 1,
        "{start} vs {moved}"
    );
    assert_eq!(view.tracks[0].adjusted, vec![false, true]);
    assert!(
        view.message.starts_with("corte 2 de drums: empieza en"),
        "{}",
        view.message
    );
    let saved = std::fs::read_to_string(&project).unwrap();
    assert!(saved.contains("corte = break.wav 2 2 "), "{saved}");
    // Se sigue moviendo desde donde quedó.
    draw(&mut app, 160, 40);
    let x = mark_column(&app, start);
    app.mouse(MouseAction::Down, x, y);
    app.mouse(MouseAction::Drag, x - 2, y);
    app.mouse(MouseAction::Up, x - 2, y);
    assert!(app.view().tracks[0].slices[1].start < start);
}

#[test]
fn a_dragged_mark_stops_at_its_neighbours() {
    let (_, mut app, _engine) = open_project("mouse_slice_clamp");
    draw(&mut app, 160, 40);
    let area = app.sample_area.unwrap();
    let (x, y) = (mark_column(&app, 3000), area.y + 1);
    app.mouse(MouseAction::Down, x, y);
    // Mucho más allá del principio: se queda a 1,2 ms del corte 1.
    app.mouse(MouseAction::Drag, area.x, y);
    let gap = (1.2 / 1000.0 * RATE as f64).ceil() as usize;
    assert_eq!(app.view().tracks[0].slices[1].start, gap);
    app.mouse(MouseAction::Up, area.x, y);
    let view = app.view();
    assert!(!view.message.starts_with("no se mueve"), "{}", view.message);
    assert!(view.tracks[0].slices[1].start.abs_diff(gap) <= 5);
    // Y por la derecha, a 1,2 ms del final del sample.
    draw(&mut app, 160, 40);
    let x = mark_column(&app, view.tracks[0].slices[1].start);
    app.mouse(MouseAction::Down, x, y);
    app.mouse(MouseAction::Drag, area.right() + 20, y);
    assert_eq!(app.view().tracks[0].slices[1].start, 6000 - gap);
}

#[test]
fn a_click_on_the_wave_selects_and_plays_the_slice_there() {
    let (project, mut app, mut engine) = open_project("mouse_slice_select");
    // En el modo tocar también; parado, para oír solo el clic.
    app.handle(esc());
    app.handle(ctrl('t'));
    offline::render(&mut engine, 2000, 2);
    let heard = |engine: &mut Engine| offline::render(engine, 2000, 2).iter().any(|&s| s != 0.0);
    draw(&mut app, 160, 40);
    let area = app.sample_area.unwrap();
    let y = area.y + 1;
    // Lejos de las marcas, en el corte 2: lo selecciona sin moverlo y
    // suena.
    let x = mark_column(&app, 4500);
    app.mouse(MouseAction::Down, x, y);
    app.mouse(MouseAction::Drag, x + 3, y);
    app.mouse(MouseAction::Up, x + 3, y);
    let view = app.view();
    assert_eq!(view.selected_slice, Some(1));
    assert_eq!(view.tracks[0].slices[1].start, 3000);
    assert_eq!(view.message, "preescucha drums: corte 2");
    assert!(heard(&mut engine), "el clic suena");
    assert!(!heard(&mut engine));
    // En el corte 1.
    app.mouse(MouseAction::Down, mark_column(&app, 1500), y);
    app.mouse(MouseAction::Up, mark_column(&app, 1500), y);
    assert_eq!(app.view().selected_slice, Some(0));
    assert!(heard(&mut engine));
    // Agarrar y soltar sin moverse es un clic sobre ese corte: suena y no
    // cambia nada.
    let x = mark_column(&app, 3000);
    app.mouse(MouseAction::Down, x, y);
    assert!(!heard(&mut engine), "agarrar todavía no suena");
    app.mouse(MouseAction::Up, x, y);
    assert_eq!(app.view().selected_slice, Some(1));
    assert!(heard(&mut engine));
    assert!(
        !std::fs::read_to_string(&project)
            .unwrap()
            .contains("corte =")
    );
    // Arrastrar la marca la mueve sin sonar.
    app.mouse(MouseAction::Down, x, y);
    app.mouse(MouseAction::Drag, x + 4, y);
    app.mouse(MouseAction::Up, x + 4, y);
    assert!(app.view().tracks[0].slices[1].start > 3000);
    assert!(!heard(&mut engine), "mover un corte no suena");
    // Con la paleta abierta, nada.
    app.open_palette();
    app.mouse(MouseAction::Down, mark_column(&app, 1500), y);
    assert_eq!(app.view().selected_slice, Some(1));
    assert!(!heard(&mut engine));
}

#[test]
fn the_big_sample_view_of_the_main_window_drags_marks_too() {
    let path = fixture("mouse_slice_main", Some(SONG));
    let (mut main, _engine) = super::split::main_window(&path, None);
    draw(&mut main, 120, 40);
    let area = main.sample_area.expect("la onda grande");
    let x = mark_column(&main, 3000);
    // En la fila de la onda (bajo la de los números).
    main.mouse(MouseAction::Down, x, area.y + 2);
    main.mouse(MouseAction::Drag, x + 4, area.y + 2);
    main.mouse(MouseAction::Up, x + 4, area.y + 2);
    let view = main.view();
    assert!(view.tracks[0].slices[1].start > 3000);
    assert!(
        view.message.starts_with("corte 2 de drums: empieza en"),
        "{}",
        view.message
    );
}
