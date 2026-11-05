//! El foco entre paneles: Alt-1…9, Esc, sus teclas, el borde resaltado y
//! el ratón.

use super::*;
use crate::app::MouseAction;
use crate::focus::Panel;
use crate::layout::Tab;

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
fn alt_numbers_move_the_focus_and_esc_comes_back() {
    let (_, mut app, _engine) = open("focus_keys", Some(SONG));
    assert_eq!(app.view().focus, Panel::Code);
    // 2: la onda, el modo tocar.
    app.handle(alt('2'));
    assert_eq!(app.view().focus, Panel::Wave);
    assert!(app.view().playing_mode);
    app.handle(alt('1'));
    assert_eq!(app.view().focus, Panel::Code);
    assert!(!app.view().playing_mode);
    // 3: el tracker con su cursor; Esc lo suelta y se vuelve al editor.
    app.handle(alt('3'));
    assert_eq!(app.view().focus, Panel::Tracker);
    assert!(app.view().tracker_cursor.is_some());
    app.handle(esc());
    assert_eq!(app.view().focus, Panel::Code);
    // 9: la ocupación, abierta y con su cursor.
    app.handle(alt('9'));
    assert_eq!(app.view().focus, Panel::Occupancy);
    assert!(app.view().occupancy.is_some() && app.view().occupancy_cursor.is_some());
    app.handle(esc());
    assert_eq!(app.view().focus, Panel::Code);
    // Los paneles sin cursor: su pestaña y Esc.
    for (c, panel) in [
        ('4', Panel::Mixer),
        ('5', Panel::Fx),
        ('7', Panel::Scope),
        ('8', Panel::Spectrum),
    ] {
        app.handle(alt(c));
        assert_eq!(app.view().focus, panel, "Alt-{c}");
        assert_eq!(app.view().dock_tab, Tab::Panel(panel));
        app.handle(esc());
        assert_eq!(app.view().focus, Panel::Code, "Esc desde {panel:?}");
    }
    // Desde el modo tocar, Esc vuelve a tocar.
    app.handle(alt('2'));
    app.handle(alt('4'));
    assert_eq!(app.view().focus, Panel::Mixer);
    app.handle(esc());
    assert_eq!(app.view().focus, Panel::Wave);
}

#[test]
fn the_mixer_keys_are_its_own_and_the_shortcuts_still_work() {
    let (path, mut app, _engine) = open("focus_mixer", Some(SONG));
    let text_before = app.text_for_test();
    app.handle(alt('4'));
    assert!(
        app.view().message.starts_with("MEZCLA"),
        "{}",
        app.view().message
    );
    assert_eq!(app.view().selected, Some(0));
    app.handle(special(Key::Right));
    assert_eq!(app.view().selected, Some(1));
    // m: mute en vivo de la elegida.
    app.handle(key('m'));
    assert!(app.view().tracks[1].mute);
    app.handle(key('m'));
    assert!(!app.view().tracks[1].mute);
    // Lo que no usa no llega al editor.
    app.handle(key('x'));
    app.handle(key('q'));
    assert_eq!(app.text_for_test(), text_before);
    // Ctrl-S sigue guardando.
    std::fs::write(&path, "-- otro\n").unwrap();
    app.handle(ctrl('s'));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), text_before);
    // La línea de las teclas dice las del mezclador.
    let screen = text(&draw(&mut app, 120, 40));
    assert!(
        screen.lines().last().unwrap().starts_with("MEZCLA:"),
        "{screen}"
    );
}

#[test]
fn the_focused_panel_has_its_border_in_the_focus_color() {
    let (_, mut app, _engine) = open("focus_border", Some(SONG));
    let theme = crate::theme::get();
    let buffer = draw(&mut app, 120, 40);
    let (ex, ey) = find(&buffer, "┌ song.lua").unwrap();
    assert_eq!(buffer[(ex, ey)].fg, theme.border_focus);
    let (mx, my) = find(&buffer, "┌ mezclador").unwrap();
    assert_eq!(buffer[(mx, my)].fg, theme.border);
    app.handle(alt('4'));
    let buffer = draw(&mut app, 120, 40);
    assert_eq!(buffer[(mx, my)].fg, theme.border_focus);
    assert_eq!(buffer[(ex, ey)].fg, theme.border);
}

#[test]
fn lanes_focus_picks_the_first_point_or_says_why_not() {
    let (_, mut app, _engine) = open("focus_lanes_none", Some(SONG));
    app.handle(alt('6'));
    assert!(
        app.view().message.contains("pista:auto"),
        "{}",
        app.view().message
    );
    assert_eq!(app.view().dock_tab, Tab::Panel(Panel::Lanes));
    let song = format!(
        "{SONG}track(\"drums\"):auto(\"gain\", {{ {{0, 0.5}}, {{1, 1}} }}, {{ len = 1 }})\n"
    );
    let (_, mut app, _engine) = open("focus_lanes", Some(&song));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    app.handle(alt('6'));
    assert_eq!(app.view().focus, Panel::Lanes, "{}", app.view().message);
    app.handle(esc());
    assert_eq!(app.view().focus, Panel::Code);
}

#[test]
fn a_click_gives_the_focus_and_a_tab_click_shows_its_tab() {
    let (_, mut app, _engine) = open("focus_mouse", Some(SONG));
    let buffer = draw(&mut app, 120, 40);
    // En el mezclador, fuera de sus tiras: el foco es suyo.
    let mixer = app.panels.mixer.unwrap().inner;
    click(&mut app, mixer.right() - 1, mixer.bottom() - 1);
    assert_eq!(app.view().focus, Panel::Mixer);
    // Un clic en el editor lo devuelve.
    let (ex, ey) = find(&buffer, "bpm(120)").unwrap();
    click(&mut app, ex, ey);
    assert_eq!(app.view().focus, Panel::Code);
    // Una pestaña.
    let (x, y) = find(&buffer, " 6 lanes ").unwrap_or_else(|| panic!("{}", text(&buffer)));
    click(&mut app, x + 1, y);
    assert_eq!(app.view().dock_tab, Tab::Panel(Panel::Lanes));
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("┌ lanes"), "{screen}");
    // Desde el modo tocar, un clic en el editor vuelve a editar.
    app.handle(alt('2'));
    click(&mut app, ex, ey);
    assert_eq!(app.view().focus, Panel::Code);
}

#[test]
fn alt_0_focuses_the_explorer_and_esc_comes_back_from_each_new_place() {
    let (_, mut app, _engine) = open("focus_files", Some(SONG));
    draw(&mut app, 120, 40);
    // Desde el editor.
    app.handle(alt('0'));
    assert_eq!(app.focus(), Panel::Files);
    assert!(
        app.view().message.starts_with("ARCHIVOS"),
        "{}",
        app.view().message
    );
    // Las letras son del árbol: no llegan al editor.
    let before = app.text_for_test();
    app.handle(key('j'));
    app.handle(key('k'));
    assert_eq!(app.text_for_test(), before);
    app.handle(esc());
    assert_eq!(app.focus(), Panel::Code);
    // Desde el modo tocar, Esc vuelve al modo tocar.
    app.handle(esc());
    assert_eq!(app.focus(), Panel::Wave);
    app.handle(alt('0'));
    app.handle(esc());
    assert_eq!(app.focus(), Panel::Wave);
    // Con el editor flotante, Alt-1 le da el foco como siempre.
    app.handle(alt('w'));
    app.handle(alt('1'));
    assert_eq!(app.focus(), Panel::Code);
    // Los atajos de siempre siguen con el foco en el árbol.
    app.handle(alt('0'));
    app.handle(alt('4'));
    assert_eq!(app.focus(), Panel::Mixer);
}
