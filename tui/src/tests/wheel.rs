//! La rueda en el editor: desplaza la vista sin mover el cursor (vertical,
//! horizontal y Shift y la rueda), dentro de los límites del texto, con
//! pasos más finos en las ráfagas del touchpad; la siguiente tecla vuelve
//! al cursor con su margen; las barras se arrastran; y los clics y la
//! selección siguen como siempre.

use std::time::{Duration, Instant};

use super::*;
use crate::app::MouseAction;
use crate::config::Config;

/// Un script largo (120 líneas) con una línea muy ancha en la 10.
fn long_song() -> String {
    let mut text = String::from(SONG);
    for i in 1..=120 {
        if i == 10 {
            text.push_str(&format!("-- ancha {} FINAL_ANCHA\n", "x".repeat(300)));
        } else {
            text.push_str(&format!("-- línea {i}\n"));
        }
    }
    text
}

/// Abre el script con el editor maximizado (Alt-Z): más líneas a la vista.
fn open_big(test: &str) -> (App, Engine) {
    let (_, mut app, engine) = open(test, Some(&long_song()));
    app.handle(Input {
        key: Key::Char('z'),
        alt: true,
        ..Default::default()
    });
    draw(&mut app, 120, 40);
    assert!(app.editor_area.unwrap().height > 30);
    (app, engine)
}

/// Un paso de rueda suelto (un segundo después del anterior).
fn wheel(app: &mut App, action: MouseAction, x: u16, y: u16, t: &mut Instant) {
    *t += Duration::from_secs(1);
    app.mouse_at(action, x, y, false, *t);
}

/// Un punto dentro del texto del editor.
fn inside(app: &App) -> (u16, u16) {
    let area = app.editor_area.expect("el editor se dibujó");
    (area.x + area.width / 2, area.y + area.height / 2)
}

#[test]
fn the_wheel_scrolls_the_view_but_not_the_cursor() {
    let (mut app, _engine) = open_big("wheel_vertical");
    draw(&mut app, 120, 40);
    let (x, y) = inside(&app);
    let mut t = Instant::now();
    assert_eq!(app.editor.cursor(), (0, 0));
    for _ in 0..5 {
        wheel(&mut app, MouseAction::ScrollDown, x, y, &mut t);
    }
    let buffer = draw(&mut app, 120, 40);
    assert_eq!(app.editor.scroll_offset().0, 15);
    assert_eq!(app.editor.cursor(), (0, 0), "el cursor no se mueve");
    assert!(find(&buffer, "-- línea 20").is_some());
    assert!(
        find(&buffer, "bpm(120)").is_none(),
        "la primera línea ya no se ve"
    );
    // Hacia arriba, hasta el principio y no más.
    for _ in 0..10 {
        wheel(&mut app, MouseAction::ScrollUp, x, y, &mut t);
    }
    draw(&mut app, 120, 40);
    assert_eq!(app.editor.scroll_offset().0, 0);
    // Hacia abajo, hasta que la última línea queda abajo del todo.
    for _ in 0..100 {
        wheel(&mut app, MouseAction::ScrollDown, x, y, &mut t);
    }
    let buffer = draw(&mut app, 120, 40);
    let lines = app.editor.lines().len();
    let height = app.editor_area.unwrap().height as usize;
    assert_eq!(app.editor.scroll_offset().0 as usize, lines - height);
    assert!(find(&buffer, "-- línea 120").is_some());
    assert_eq!(app.editor.cursor(), (0, 0));
}

#[test]
fn a_key_brings_the_cursor_back_with_its_margin() {
    let (mut app, _engine) = open_big("wheel_back");
    draw(&mut app, 120, 40);
    let (x, y) = inside(&app);
    let mut t = Instant::now();
    for _ in 0..10 {
        wheel(&mut app, MouseAction::ScrollDown, x, y, &mut t);
    }
    draw(&mut app, 120, 40);
    assert_eq!(app.editor.scroll_offset().0, 30);
    // Una tecla que no mueve el cursor ni cambia el texto (F6 sin nada
    // buscado) no suelta la vista.
    app.handle(Input {
        key: Key::F(6),
        ..Default::default()
    });
    draw(&mut app, 120, 40);
    assert_eq!(app.editor.scroll_offset().0, 30);
    // Escribir vuelve al cursor (arriba del todo, en la línea 1).
    app.handle(key('x'));
    let buffer = draw(&mut app, 120, 40);
    assert_eq!(app.editor.scroll_offset().0, 0);
    assert!(find(&buffer, "xbpm(120)").is_some());
    // Bajar con las flechas deja dos líneas (el margen) por debajo.
    let height = app.editor_area.unwrap().height as usize;
    for _ in 0..height {
        app.handle(Input {
            key: Key::Down,
            ..Default::default()
        });
        draw(&mut app, 120, 40);
    }
    // (El alto puede cambiar en un línea con la ayuda de la función.)
    let (top, _) = app.editor.scroll_offset();
    let height = app.editor_area.unwrap().height as usize;
    let row = app.editor.cursor().0;
    assert_eq!(top as usize, row + 2 + 1 - height, "margen de 2 abajo");
}

#[test]
fn the_margin_is_configurable() {
    let (mut app, _engine) = open_big("wheel_margin");
    app.set_config(&Config::parse("[editor]\nmargen = 0\n", true, "c.toml"));
    draw(&mut app, 120, 40);
    let height = app.editor_area.unwrap().height as usize;
    for _ in 0..height {
        app.handle(Input {
            key: Key::Down,
            ..Default::default()
        });
        draw(&mut app, 120, 40);
    }
    let height = app.editor_area.unwrap().height as usize;
    let row = app.editor.cursor().0;
    assert_eq!(
        app.editor.scroll_offset().0 as usize,
        row + 1 - height,
        "sin margen, lo justo"
    );
}

#[test]
fn the_wheel_scrolls_sideways_through_long_lines() {
    let (mut app, _engine) = open_big("wheel_horizontal");
    let buffer = draw(&mut app, 120, 40);
    assert!(find(&buffer, "FINAL_ANCHA").is_none());
    let (x, y) = inside(&app);
    let mut t = Instant::now();
    // La rueda horizontal (botones 66 y 67)...
    for _ in 0..10 {
        wheel(&mut app, MouseAction::ScrollRight, x, y, &mut t);
    }
    draw(&mut app, 120, 40);
    assert_eq!(app.editor.scroll_offset().1, 60);
    assert_eq!(app.editor.cursor(), (0, 0));
    // ...hasta el final de la línea más ancha a la vista, y no más.
    for _ in 0..100 {
        wheel(&mut app, MouseAction::ScrollRight, x, y, &mut t);
    }
    let buffer = draw(&mut app, 120, 40);
    assert!(find(&buffer, "FINAL_ANCHA").is_some());
    let left = app.editor.scroll_offset().1;
    wheel(&mut app, MouseAction::ScrollRight, x, y, &mut t);
    draw(&mut app, 120, 40);
    assert_eq!(app.editor.scroll_offset().1, left, "tope a la derecha");
    // Sin líneas anchas a la vista no hay a dónde ir de lado.
    for _ in 0..200 {
        wheel(&mut app, MouseAction::ScrollLeft, x, y, &mut t);
    }
    for _ in 0..10 {
        wheel(&mut app, MouseAction::ScrollDown, x, y, &mut t);
    }
    draw(&mut app, 120, 40);
    assert_eq!(app.editor.scroll_offset(), (30, 0));
    wheel(&mut app, MouseAction::ScrollRight, x, y, &mut t);
    draw(&mut app, 120, 40);
    assert_eq!(app.editor.scroll_offset(), (30, 0));
}

#[test]
fn shift_and_the_wheel_scroll_sideways() {
    use ratatui::crossterm::event::{KeyModifiers, MouseEvent, MouseEventKind};
    let event = |kind, modifiers| MouseEvent {
        kind,
        column: 10,
        row: 10,
        modifiers,
    };
    assert_eq!(
        crate::mouse_action(&event(MouseEventKind::ScrollDown, KeyModifiers::SHIFT)),
        Some(MouseAction::ScrollRight)
    );
    assert_eq!(
        crate::mouse_action(&event(MouseEventKind::ScrollUp, KeyModifiers::SHIFT)),
        Some(MouseAction::ScrollLeft)
    );
    assert_eq!(
        crate::mouse_action(&event(MouseEventKind::ScrollRight, KeyModifiers::NONE)),
        Some(MouseAction::ScrollRight)
    );
}

#[test]
fn a_burst_of_wheel_steps_moves_one_line_at_a_time() {
    let (mut app, _engine) = open_big("wheel_burst");
    draw(&mut app, 120, 40);
    let (x, y) = inside(&app);
    let t = Instant::now();
    // Un touchpad: diez pasos en 100 ms.
    for i in 0..10 {
        app.mouse_at(
            MouseAction::ScrollDown,
            x,
            y,
            false,
            t + Duration::from_millis(10 * i),
        );
    }
    draw(&mut app, 120, 40);
    // El primero, suelto (3); los otros nueve, de uno en uno.
    assert_eq!(app.editor.scroll_offset().0, 12);
    // Configurable.
    app.set_config(&Config::parse(
        "[editor]\nrueda_lineas = 6\n",
        true,
        "c.toml",
    ));
    let t = t + Duration::from_secs(5);
    app.mouse_at(MouseAction::ScrollDown, x, y, false, t);
    app.mouse_at(
        MouseAction::ScrollDown,
        x,
        y,
        false,
        t + Duration::from_millis(5),
    );
    draw(&mut app, 120, 40);
    assert_eq!(app.editor.scroll_offset().0, 12 + 6 + 2);
}

#[test]
fn clicks_and_drags_still_work_on_the_scrolled_view() {
    let (mut app, _engine) = open_big("wheel_click");
    draw(&mut app, 120, 40);
    let (x, y) = inside(&app);
    let mut t = Instant::now();
    for _ in 0..10 {
        wheel(&mut app, MouseAction::ScrollDown, x, y, &mut t);
    }
    let buffer = draw(&mut app, 120, 40);
    // Clic en una línea que solo se ve con la vista desplazada: el cursor va
    // ahí y la vista no salta.
    let (cx, cy) = find(&buffer, "-- línea 40").unwrap();
    app.mouse(MouseAction::Down, cx + 3, cy);
    app.mouse(MouseAction::Up, cx + 3, cy);
    let row = 39 + SONG.lines().count();
    assert_eq!(app.editor.cursor(), (row, 3));
    draw(&mut app, 120, 40);
    assert_eq!(app.editor.scroll_offset().0, 30);
    // Arrastrar selecciona.
    app.mouse(MouseAction::Down, cx, cy);
    app.mouse(MouseAction::Drag, cx + 5, cy);
    app.mouse(MouseAction::Up, cx + 5, cy);
    assert_eq!(app.editor.selection_range(), Some(((row, 0), (row, 5))));
    // Con la selección hecha, la rueda la conserva y la vuelve a pintar
    // donde está.
    for _ in 0..3 {
        wheel(&mut app, MouseAction::ScrollUp, x, y, &mut t);
    }
    let buffer = draw(&mut app, 120, 40);
    assert_eq!(app.editor.selection_range(), Some(((row, 0), (row, 5))));
    let (sx, sy) = find(&buffer, "-- línea 40").unwrap();
    assert_eq!(sy, cy + 9);
    let selected = buffer[(sx, sy)].bg;
    assert_ne!(selected, buffer[(sx + 7, sy)].bg, "lo seleccionado se ve");
    // Fuera del editor, la rueda no lo mueve.
    let area = app.editor_area.unwrap();
    assert!(area.y > 1, "{area:?}");
    wheel(&mut app, MouseAction::ScrollDown, x, 1, &mut t);
    draw(&mut app, 120, 40);
    assert_eq!(app.editor.scroll_offset().0, 21);
}

#[test]
fn the_wheel_and_vim_mode_get_along() {
    let (mut app, _engine) = open_big("wheel_vim");
    app.set_config(&Config::parse("[editor]\nmodo = \"vim\"\n", true, "c.toml"));
    draw(&mut app, 120, 40);
    let (x, y) = inside(&app);
    let mut t = Instant::now();
    for _ in 0..10 {
        wheel(&mut app, MouseAction::ScrollDown, x, y, &mut t);
    }
    draw(&mut app, 120, 40);
    assert_eq!(app.editor.scroll_offset().0, 30);
    assert_eq!(app.editor.cursor(), (0, 0));
    // `j` en Normal mueve el cursor: la vista vuelve a él.
    app.handle(key('j'));
    draw(&mut app, 120, 40);
    assert_eq!(app.editor.cursor(), (1, 0));
    assert_eq!(app.editor.scroll_offset().0, 0);
    assert_eq!(app.editor.lines()[0], "bpm(120)", "no escribe nada");
}

#[test]
fn the_scroll_bars_show_where_the_view_is_and_drag() {
    let (mut app, _engine) = open_big("wheel_bars");
    // Apagadas por defecto: nada a la derecha del texto.
    draw(&mut app, 120, 40);
    assert_eq!(app.view.bars, (None, None));
    app.set_config(&Config::parse("[editor]\nbarras = true\n", true, "c.toml"));
    let buffer = draw(&mut app, 120, 40);
    let (Some(vertical), Some(horizontal)) = app.view.bars else {
        panic!("dos barras (hay una línea ancha): {:?}", app.view.bars);
    };
    assert_eq!(buffer[(vertical.x, vertical.y)].symbol(), "┃");
    assert_eq!(buffer[(vertical.x, vertical.bottom() - 1)].symbol(), "│");
    assert_eq!(buffer[(horizontal.x, horizontal.y)].symbol(), "━");
    // Arrastrar la barra vertical hasta abajo lleva la vista al final.
    app.mouse(MouseAction::Down, vertical.x, vertical.y);
    app.mouse(MouseAction::Drag, vertical.x, vertical.bottom() - 1);
    app.mouse(MouseAction::Up, vertical.x, vertical.bottom() - 1);
    let buffer = draw(&mut app, 120, 40);
    assert!(find(&buffer, "-- línea 120").is_some());
    assert_eq!(app.editor.cursor(), (0, 0));
    assert_eq!(
        buffer[(vertical.x, vertical.bottom() - 1)].symbol(),
        "┃",
        "el trozo visible, abajo"
    );
    // La horizontal, a la derecha del todo (con la línea ancha a la vista).
    app.mouse(MouseAction::Down, vertical.x, vertical.y);
    app.mouse(MouseAction::Up, vertical.x, vertical.y);
    draw(&mut app, 120, 40);
    app.mouse(MouseAction::Down, horizontal.right() - 1, horizontal.y);
    app.mouse(MouseAction::Up, horizontal.right() - 1, horizontal.y);
    let buffer = draw(&mut app, 120, 40);
    assert!(find(&buffer, "FINAL_ANCHA").is_some());
    // En ASCII.
    crate::theme::set_ascii(true);
    let buffer = draw(&mut app, 120, 40);
    crate::theme::set_ascii(false);
    let symbol = buffer[(vertical.x, vertical.y)].symbol().to_string();
    assert!(symbol == "#" || symbol == "|", "{symbol}");
}
