//! El ratón en el tracker, la vista de ocupación y el mezclador: cada clic
//! cae en la celda que se dibuja ahí (la geometría es la misma) y hace lo
//! que ya hacen las teclas.

use super::split::{editor_window, main_window, pump};
use super::*;
use crate::app::MouseAction;

fn click(app: &mut App, x: u16, y: u16) {
    app.mouse(MouseAction::Down, x, y);
    app.mouse(MouseAction::Up, x, y);
}

/// Dónde se dibujó la cabecera de la pista `i` y la celda `(i, step)` del
/// tracker (comprobando que ahí se ve lo que dice).
fn tracker_spots(app: &App, buffer: &Buffer, i: usize, step: usize) -> ((u16, u16), (u16, u16)) {
    let layout = app.panels.tracker.expect("el tracker se ve");
    let x = layout.inner.x + 3 + (i * layout.column) as u16;
    let header = (x, layout.inner.y);
    let cell = (x, layout.inner.y + 1 + (step - layout.first) as u16);
    let name = &app.view().tracks[i].name;
    let shown: String = (0..name.chars().count() as u16)
        .map(|dx| buffer[(x + dx, header.1)].symbol().to_string())
        .collect();
    assert_eq!(&shown, name);
    let number: String = (0..2)
        .map(|dx| buffer[(layout.inner.x + dx, cell.1)].symbol().to_string())
        .collect();
    assert_eq!(number, format!("{step:02}"));
    (header, cell)
}

#[test]
fn in_the_tracker_a_name_selects_or_mutes_and_a_cell_opens_the_cursor_there() {
    let (_, mut app, _engine) = open("panels_mouse_tracker", Some(SONG));
    let buffer = draw(&mut app, 120, 40);
    let (header, _) = tracker_spots(&app, &buffer, 1, 0);
    click(&mut app, header.0, header.1);
    assert_eq!(app.view().selected, Some(1));
    // El clic derecho en el nombre: mute en vivo, y otra vez lo quita.
    app.mouse(MouseAction::RightDown, header.0, header.1);
    assert!(app.view().tracks[1].mute);
    assert!(!app.view().tracks[0].mute);
    app.mouse(MouseAction::RightDown, header.0, header.1);
    assert!(!app.view().tracks[1].mute);

    // Un clic en una celda: el cursor de edición ahí, y la pista elegida.
    let (_, cell) = tracker_spots(&app, &buffer, 0, 2);
    click(&mut app, cell.0 + 1, cell.1);
    assert_eq!(app.view().tracker_cursor, Some((0, 2)));
    assert_eq!(app.view().selected, Some(0));
    assert!(app.view().prompt.is_none());
    // La rueda lo mueve de paso en paso.
    let buffer = draw(&mut app, 120, 40);
    app.mouse(MouseAction::ScrollDown, cell.0, cell.1);
    assert_eq!(app.view().tracker_cursor, Some((0, 3)));
    app.mouse(MouseAction::ScrollUp, cell.0, cell.1);
    // Otro clic en la celda del cursor: sus efectos, como Enter.
    let (_, cell) = tracker_spots(&app, &buffer, 0, 2);
    click(&mut app, cell.0, cell.1);
    let prompt = app.view().prompt.expect("la línea de órdenes");
    assert!(prompt.starts_with("efectos"), "{prompt}");
    // Nada se ha escrito en el código.
    assert_eq!(app.text_for_test(), SONG);
}

#[test]
fn in_the_occupancy_view_a_cell_places_the_cursor_and_a_second_click_fills() {
    let song = SONG.replace(
        "1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .",
        "1 . 2 .  1 . 2 .  1 . 2 .  . . . .",
    );
    let (_, mut app, _engine) = open("panels_mouse_occupancy", Some(&song));
    app.set_occupancy(true);
    let buffer = draw(&mut app, 120, 40);
    let layout = app.panels.occupancy.expect("la vista se ve");
    let column =
        |c: usize| layout.inner.x + (layout.label + (c - layout.first) * layout.width) as u16;
    let row = |r: usize| layout.inner.y + 1 + (r - layout.first_track) as u16;
    // Lo dibujado en esa fila es la pista «drums».
    assert_eq!(buffer[(layout.inner.x, row(0))].symbol(), "d");
    // Un clic en el silencio del paso 13 de drums: el cursor ahí.
    click(&mut app, column(12), row(0));
    assert_eq!(app.view().occupancy_cursor, Some((0, 12)));
    assert!(app.view().tracker_cursor.is_none());
    // El nombre elige la pista (y lleva el cursor a su fila).
    click(&mut app, layout.inner.x, row(1));
    assert_eq!(app.view().selected, Some(1));
    assert_eq!(app.view().occupancy_cursor, Some((1, 12)));
    // La rueda, de pista en pista; Shift y la rueda, de paso en paso.
    app.mouse(MouseAction::ScrollUp, column(12), row(1));
    assert_eq!(app.view().occupancy_cursor, Some((0, 12)));
    app.mouse(MouseAction::ScrollRight, column(12), row(1));
    app.mouse(MouseAction::ScrollLeft, column(12), row(1));
    assert_eq!(app.view().occupancy_cursor, Some((0, 12)));
    // Otro clic en la celda del cursor: «rellenar», como Enter.
    draw(&mut app, 120, 40);
    let layout = app.panels.occupancy.unwrap();
    click(
        &mut app,
        layout.inner.x + (layout.label + (12 - layout.first) * layout.width) as u16,
        layout.inner.y + 1,
    );
    let prompt = app.view().prompt.expect("la línea de órdenes");
    assert!(prompt.starts_with("rellenar "), "{prompt}");
}

#[test]
fn in_the_mixer_a_strip_selects_and_its_buttons_mute_and_solo() {
    let path = fixture("panels_mouse_mixer", Some(SONG));
    let (mut main, _engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    let buffer = draw(&mut main, 120, 40);
    let mixer = main.panels.mixer.expect("el mezclador se ve");
    // La tira de kick: su nombre arriba, y la fila de M y S.
    let (name_x, name_y) = find(&buffer, "kick").unwrap();
    assert_eq!(
        crate::ui::MixerLayout::hit(&mixer, name_x, name_y),
        Some(crate::ui::MixerHit::Strip(1))
    );
    click(&mut main, name_x, name_y);
    assert_eq!(main.view().selected, Some(1));
    let flags = (name_y..mixer.inner.bottom())
        .find(|&y| buffer[(name_x, y)].symbol() == "M" && buffer[(name_x + 2, y)].symbol() == "S")
        .expect("los botones de kick");
    click(&mut main, name_x, flags);
    assert!(main.view().tracks[1].mute);
    click(&mut main, name_x + 2, flags);
    assert!(main.view().tracks[1].solo);
    click(&mut main, name_x, flags);
    click(&mut main, name_x + 2, flags);
    assert!(!main.view().tracks[1].mute && !main.view().tracks[1].solo);
    // Y el tracker de esta ventana también responde.
    let layout = main.panels.tracker.expect("el tracker se ve");
    click(&mut main, layout.inner.x + 3, layout.inner.y + 3);
    assert_eq!(main.view().tracker_cursor, Some((0, 2)));
}

#[test]
fn with_the_palette_open_or_the_mouse_off_the_panels_do_nothing() {
    let (_, mut app, _engine) = open("panels_mouse_off", Some(SONG));
    let buffer = draw(&mut app, 120, 40);
    let (header, cell) = tracker_spots(&app, &buffer, 1, 2);
    app.open_palette();
    click(&mut app, cell.0, cell.1);
    app.mouse(MouseAction::RightDown, header.0, header.1);
    assert!(app.view().tracker_cursor.is_none());
    assert!(!app.view().tracks[1].mute);
}
