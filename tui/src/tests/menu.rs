//! La barra de menús (F10): teclas, dibujo y ratón.

use super::*;
use crate::app::MouseAction;
use crate::menu::Menu;

fn alt(c: char) -> Input {
    Input {
        key: Key::Char(c),
        alt: true,
        ..Input::default()
    }
}

fn special(key: Key) -> Input {
    Input {
        key,
        ..Input::default()
    }
}

fn click(app: &mut App, x: u16, y: u16) {
    app.mouse(MouseAction::Down, x, y);
    app.mouse(MouseAction::Up, x, y);
}

#[test]
fn f10_opens_the_menus_and_the_keys_move_through_them() {
    let (_, mut app, _engine) = open("menu_keys", Some(SONG));
    app.handle(f(10));
    let menu = app.view().menu.expect("abierto");
    assert_eq!(menu.open, 0);
    assert_eq!(Menu::ALL[menu.open], Menu::File);
    // ←→ de menú en menú (y da la vuelta).
    app.handle(special(Key::Left));
    assert_eq!(app.view().menu.unwrap().open, Menu::ALL.len() - 1);
    app.handle(special(Key::Right));
    app.handle(special(Key::Right));
    assert_eq!(app.view().menu.unwrap().open, 1);
    // Alt y la letra de un menú lo abre.
    app.handle(alt('t'));
    assert_eq!(Menu::ALL[app.view().menu.unwrap().open], Menu::Transport);
    // Esc cierra, y F10 también.
    app.handle(esc());
    assert!(app.view().menu.is_none());
    app.handle(f(10));
    app.handle(f(10));
    assert!(app.view().menu.is_none());
    // Lo que se escribe con el menú abierto no llega al editor.
    let before = app.text_for_test();
    app.handle(f(10));
    app.handle(key('z'));
    app.handle(esc());
    assert_eq!(app.text_for_test(), before);
}

#[test]
fn enter_runs_the_chosen_entry_like_the_palette() {
    let (_, mut app, _engine) = open("menu_enter", Some(SONG));
    app.handle(f(10));
    app.handle(alt('y'));
    let items = app.view().menu.unwrap().items;
    assert_eq!(items[0].name, "Paleta de comandos");
    let info = items
        .iter()
        .position(|i| i.name == "Qué sabe hacer esta terminal")
        .unwrap();
    for _ in 0..info {
        app.handle(special(Key::Down));
    }
    app.handle(special(Key::Enter));
    assert!(app.view().menu.is_none());
    assert!(
        app.view().message.starts_with("terminal: truecolor"),
        "{}",
        app.view().message
    );
    // La paleta, desde Ayuda.
    app.handle(f(10));
    app.handle(alt('y'));
    app.handle(special(Key::Enter));
    assert!(app.view().palette.is_some());
}

#[test]
fn the_bar_is_drawn_on_top_with_the_dropdown_and_its_keys() {
    let (_, mut app, _engine) = open("menu_draw", Some(SONG));
    let screen = text(&draw(&mut app, 120, 40));
    let top = screen.lines().next().unwrap().to_string();
    assert!(top.starts_with(" Archivo   Editar   Pista"), "{screen}");
    assert!(top.contains("F10 menú"), "{screen}");
    assert!(
        screen.lines().nth(1).unwrap().starts_with("■ parado"),
        "{screen}"
    );
    app.handle(f(10));
    let buffer = draw(&mut app, 120, 40);
    let screen = text(&buffer);
    let (x, y) = find(&buffer, "Guardar y cargar").expect(&screen);
    assert_eq!(y, 2, "{screen}");
    let row: String = screen
        .lines()
        .nth(y as usize)
        .unwrap()
        .chars()
        .skip(x as usize)
        .collect();
    assert!(row.contains("Ctrl-S"), "{screen}");
    // El abierto, resaltado en la barra.
    assert_eq!(buffer[(2, 0)].bg, crate::theme::get().accent);
    assert!(
        screen.lines().last().unwrap().starts_with("MENÚ:"),
        "{screen}"
    );
}

#[test]
fn a_small_terminal_has_no_bar_row_but_f10_shows_it_over_the_header() {
    let (_, mut app, _engine) = open("menu_small", Some(SONG));
    let screen = text(&draw(&mut app, 80, 24));
    assert!(screen.starts_with("■ parado"), "{screen}");
    app.handle(f(10));
    let screen = text(&draw(&mut app, 80, 24));
    assert!(screen.starts_with(" Archivo "), "{screen}");
    assert!(screen.contains("Guardar y cargar"), "{screen}");
}

#[test]
fn the_mouse_opens_a_menu_runs_an_entry_and_closes_it_outside() {
    let (_, mut app, _engine) = open("menu_mouse", Some(SONG));
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = find(&buffer, "Ayuda").unwrap();
    click(&mut app, x, y);
    assert_eq!(Menu::ALL[app.view().menu.unwrap().open], Menu::Help);
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = find(&buffer, "Qué sabe hacer").unwrap_or_else(|| panic!("{}", text(&buffer)));
    click(&mut app, x, y);
    assert!(app.view().menu.is_none());
    assert!(app.view().message.starts_with("terminal:"));
    // Abierto, un clic fuera lo cierra (y no hace nada más).
    let (x, y) = find(&buffer, "Pista").unwrap();
    click(&mut app, x, y);
    assert_eq!(Menu::ALL[app.view().menu.unwrap().open], Menu::Track);
    draw(&mut app, 120, 40);
    let before = app.view().selected;
    click(&mut app, 100, 30);
    assert!(app.view().menu.is_none());
    assert_eq!(app.view().selected, before);
}
