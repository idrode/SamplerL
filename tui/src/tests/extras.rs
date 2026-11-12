//! Las comodidades del editor: la pareja del paréntesis resaltada, el
//! cierre de pares, las guías de sangría y los números relativos.

use super::*;
use crate::config::Config;

#[test]
fn the_pair_of_the_bracket_at_the_cursor_is_highlighted() {
    let (_, mut app, _engine) = open("extras_pair", Some(SONG));
    // SONG: la línea 1 es `bpm(120)`; el cursor sobre su `(`.
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(0, 3));
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = find(&buffer, "bpm(120)").unwrap();
    assert_eq!(buffer[(x + 3, y)].bg, crate::ui::accent());
    assert_eq!(buffer[(x + 7, y)].bg, crate::ui::accent());
    assert_ne!(buffer[(x + 4, y)].bg, crate::ui::accent());
    // Apagado: nada.
    app.set_config(&Config::parse("[editor]\npareja = false\n", true, "c.toml"));
    let buffer = draw(&mut app, 120, 40);
    assert_ne!(buffer[(x + 7, y)].bg, crate::ui::accent());
}

/// Teclea `text` en el cursor.
fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        app.handle(key(c));
    }
}

#[test]
fn pairs_close_themselves_when_asked() {
    let (_, mut app, _engine) = open("extras_close", Some("\n"));
    // Apagado por defecto: lo de siempre.
    typed(&mut app, "f(");
    assert_eq!(app.editor.lines()[0], "f(");
    app.set_config(&Config::parse(
        "[editor]\ncerrar_pares = true\n",
        true,
        "c.toml",
    ));
    app.handle(special(Key::End));
    app.handle(special(Key::Enter));
    // Escribiendo todo, también lo que cierra: sale igual (se salta).
    typed(&mut app, "track(\"kick\"):fx(\"gain\", { gain = 1 })");
    assert_eq!(
        app.editor.lines()[1],
        "track(\"kick\"):fx(\"gain\", { gain = 1 })"
    );
    // Sin escribir lo que cierra: queda cerrado, con el cursor dentro.
    app.handle(special(Key::Enter));
    typed(&mut app, "bpm(");
    assert_eq!(app.editor.lines()[2], "bpm()");
    assert_eq!(app.editor.cursor(), (2, 4));
    // Retroceso entre los dos: los dos, y un solo deshacer los devuelve.
    app.handle(special(Key::Backspace));
    assert_eq!(app.editor.lines()[2], "bpm");
    app.handle(ctrl('z'));
    assert_eq!(app.editor.lines()[2], "bpm()");
    app.handle(ctrl('r'));
    assert_eq!(app.editor.lines()[2], "bpm");
    // Una comilla tras una letra no se dobla; delante de una palabra, nada
    // se dobla.
    typed(&mut app, " -- don't");
    assert_eq!(app.editor.lines()[2], "bpm -- don't");
    app.handle(special(Key::Home));
    typed(&mut app, "(");
    assert_eq!(app.editor.lines()[2], "(bpm -- don't");
}

#[test]
fn indent_guides_mark_each_level_when_asked() {
    let song = "track(\"a\"):every(4, function(t)\n  if true then\n    t:reverse()\n  end\nend)\n";
    let (_, mut app, _engine) = open("extras_guides", Some(song));
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = find(&buffer, "t:reverse()").unwrap();
    assert_eq!(buffer[(x - 4, y)].symbol(), " ", "apagadas por defecto");
    app.set_config(&Config::parse("[editor]\nguias = true\n", true, "c.toml"));
    let buffer = draw(&mut app, 120, 40);
    // Dos niveles de dos espacios: en las columnas 0 y 2.
    assert_eq!(buffer[(x - 4, y)].symbol(), "│");
    assert_eq!(buffer[(x - 3, y)].symbol(), " ");
    assert_eq!(buffer[(x - 2, y)].symbol(), "│");
    assert_eq!(
        buffer[(x - 4, y - 1)].symbol(),
        "│",
        "un nivel en la línea del if"
    );
    assert_eq!(buffer[(x - 2, y - 1)].symbol(), "i");
    crate::theme::set_ascii(true);
    let buffer = draw(&mut app, 120, 40);
    crate::theme::set_ascii(false);
    assert_eq!(buffer[(x - 2, y)].symbol(), "|");
}

#[test]
fn relative_line_numbers_count_from_the_cursor_when_asked() {
    let lines: String = (1..=20).map(|i| format!("-- línea {i}\n")).collect();
    let (_, mut app, _engine) = open("extras_relative", Some(&lines));
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(3, 0));
    let number = |app: &mut App, needle: &str| {
        let buffer = draw(app, 120, 40);
        let area = app.editor_area.unwrap();
        let (_, y) = find(&buffer, needle).unwrap();
        (area.x..area.x + 4)
            .map(|x| buffer[(x, y)].symbol().to_string())
            .collect::<String>()
            .trim()
            .to_string()
    };
    assert_eq!(
        number(&mut app, "-- línea 2 "),
        "2",
        "apagado: los de siempre"
    );
    app.set_config(&Config::parse(
        "[editor]\nnumeros_relativos = true\n",
        true,
        "c.toml",
    ));
    assert_eq!(number(&mut app, "-- línea 2 "), "2");
    assert_eq!(
        number(&mut app, "-- línea 4 "),
        "4",
        "la del cursor, la de siempre"
    );
    assert_eq!(number(&mut app, "-- línea 1 "), "3");
    assert_eq!(number(&mut app, "-- línea 7 "), "3");
}
