//! El editor flotante dentro de la ventana: Alt-W lo saca encima de los
//! paneles (que se siguen dibujando detrás), se mueve y cambia de tamaño
//! con el ratón y las teclas, y el ratón no llega a lo que tapa.

use super::dock::{alt, ctrl_alt};
use super::*;
use crate::app::MouseAction;

fn float(app: &App) -> ratatui::layout::Rect {
    let grid = app.last_grid().unwrap();
    assert!(grid.floating, "el editor no flota");
    grid.editor.unwrap()
}

fn alt_shift(key: Key) -> Input {
    Input {
        key,
        alt: true,
        shift: true,
        ..Input::default()
    }
}

#[test]
fn alt_w_floats_it_over_the_panels_that_keep_drawing_behind() {
    let (_, mut app, _engine) = open("float_toggle", Some(SONG));
    draw(&mut app, 120, 40);
    app.handle(alt('w'));
    let buffer = draw(&mut app, 120, 40);
    let screen = text(&buffer);
    let r = float(&app);
    let grid = app.last_grid().unwrap();
    // El sonido llena el centro (como sin editor) y el flotante, encima.
    let body = grid.body.unwrap();
    assert!(r.x > body.x && r.right() < body.right(), "{r:?}");
    assert_eq!(buffer[(r.x, r.y)].symbol(), "┌", "{screen}");
    assert!(screen.contains("bpm(120)"), "{screen}");
    // Lo de detrás se sigue viendo fuera de él.
    assert!(screen.contains("┌ sample"), "{screen}");
    assert!(screen.contains("┌ 0 archivos"), "{screen}");
    // Lo conserva todo: el resaltado y la ayuda de la función.
    assert!(screen.contains("ⓘ bpm(n)"), "{screen}");
    // Otra vez, abajo.
    app.handle(alt('w'));
    draw(&mut app, 120, 40);
    assert!(!app.last_grid().unwrap().floating);
    // Y vuelve a donde se dejó.
    app.handle(alt('w'));
    draw(&mut app, 120, 40);
    assert_eq!(float(&app), r);
}

#[test]
fn the_title_moves_it_and_the_borders_resize_it_inside_the_window() {
    let (_, mut app, _engine) = open("float_mouse", Some(SONG));
    draw(&mut app, 120, 40);
    app.handle(alt('w'));
    draw(&mut app, 120, 40);
    let r = float(&app);
    // Agarrar el título y moverlo.
    app.mouse(MouseAction::Down, r.x + 10, r.y);
    app.mouse(MouseAction::Drag, r.x + 4, r.y - 3);
    app.mouse(MouseAction::Up, r.x + 4, r.y - 3);
    draw(&mut app, 120, 40);
    let moved = float(&app);
    assert_eq!((moved.x, moved.y), (r.x - 6, r.y - 3));
    assert_eq!((moved.width, moved.height), (r.width, r.height));
    // El borde de abajo y la esquina: el tamaño.
    app.mouse(MouseAction::Down, moved.right() - 1, moved.bottom() - 1);
    app.mouse(MouseAction::Drag, moved.right() + 9, moved.bottom() + 1);
    app.mouse(MouseAction::Up, moved.right() + 9, moved.bottom() + 1);
    draw(&mut app, 120, 40);
    let big = float(&app);
    assert_eq!(
        (big.width, big.height),
        (moved.width + 10, moved.height + 2)
    );
    // Nunca sale de la ventana ni baja de su mínimo.
    app.mouse(MouseAction::Down, big.x + 10, big.y);
    app.mouse(MouseAction::Drag, 500, 500);
    app.mouse(MouseAction::Up, 500, 500);
    draw(&mut app, 120, 40);
    let body = app.last_grid().unwrap().body.unwrap();
    let edge = float(&app);
    assert_eq!((edge.right(), edge.bottom()), (body.right(), body.bottom()));
    app.mouse(MouseAction::Down, edge.right() - 1, edge.bottom() - 1);
    app.mouse(MouseAction::Drag, 0, 0);
    app.mouse(MouseAction::Up, 0, 0);
    draw(&mut app, 120, 40);
    let small = float(&app);
    assert_eq!((small.width, small.height), crate::layout::FLOAT_MIN);
}

#[test]
fn the_keys_move_and_resize_it() {
    let (_, mut app, _engine) = open("float_keys", Some(SONG));
    draw(&mut app, 120, 40);
    // Sin flotar, Alt-Shift-flechas solo avisan.
    app.handle(alt_shift(Key::Left));
    assert!(app.view().message.contains("no flota"));
    app.handle(alt('w'));
    draw(&mut app, 120, 40);
    let r = float(&app);
    app.handle(alt_shift(Key::Left));
    app.handle(alt_shift(Key::Down));
    app.handle(ctrl_alt(Key::Up));
    app.handle(ctrl_alt(Key::Right));
    draw(&mut app, 120, 40);
    let after = float(&app);
    assert_eq!((after.x, after.y), (r.x - 2, r.y + 1));
    assert_eq!((after.width, after.height), (r.width + 2, r.height + 1));
    // El explorador no ha cambiado (Ctrl-Alt-→ era del flotante).
    let files = app.last_grid().unwrap().explorer.unwrap();
    assert_eq!(files.width, crate::layout::EXPLORER_WIDTH);
}

#[test]
fn the_mouse_does_not_reach_what_it_covers() {
    let (_, mut app, _engine) = open("float_cover", Some(SONG));
    draw(&mut app, 120, 40);
    app.handle(alt('w'));
    draw(&mut app, 120, 40);
    let r = float(&app);
    let grid = app.last_grid().unwrap().clone();
    // Algo de debajo: la onda, el tracker, una pestaña...
    let under: Vec<_> = [grid.wave, grid.tracker, grid.mixer]
        .into_iter()
        .flatten()
        .filter(|a| a.intersects(r))
        .collect();
    assert!(!under.is_empty(), "el flotante tapa algún panel");
    let inner = r.inner(ratatui::layout::Margin::new(2, 2));
    let (x, y) = (inner.x, inner.y);
    assert!(grid.covered(crate::focus::Panel::Wave, x, y));
    // Desde el modo tocar, el primer clic da el foco al editor (no toca
    // la onda ni elige otra pista ni otro corte).
    app.handle(esc());
    let before = app.view();
    app.mouse(MouseAction::Down, x, y);
    app.mouse(MouseAction::Up, x, y);
    let after = app.view();
    assert!(!after.playing_mode);
    assert_eq!(after.selected, before.selected);
    assert_eq!(after.selected_slice, before.selected_slice);
    // Con el foco, el clic mueve el cursor del texto; la rueda desplaza
    // el editor, no el tracker de debajo.
    draw(&mut app, 120, 40);
    let buffer = draw(&mut app, 120, 40);
    let (kx, ky) = find(&buffer, "track(\"kick\"").unwrap();
    assert!(r.contains((kx, ky).into()));
    app.mouse(MouseAction::Down, kx + 2, ky);
    assert_eq!(app.editor.cursor(), (3, 2));
    app.mouse(MouseAction::ScrollDown, x, y);
    assert_eq!(app.view().tracker_cursor, None);
    // Fuera del flotante, lo de debajo responde como siempre.
    let wave = grid.wave.unwrap();
    let outside = (wave.x + 1..wave.right())
        .find(|&cx| !r.contains((cx, wave.y + 2).into()))
        .unwrap();
    assert!(!grid.covered(crate::focus::Panel::Wave, outside, wave.y + 2));
}
