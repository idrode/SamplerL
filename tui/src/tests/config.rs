//! El archivo de configuración: atajos que cambian, la paleta siempre a
//! mano (F1), avisos en el mensaje y el tema en lo que se dibuja.

use ratatui::style::Color;

use super::*;
use crate::config::Config;
use crate::theme;

fn alt(c: char) -> Input {
    Input {
        key: Key::Char(c),
        alt: true,
        ..Input::default()
    }
}

#[test]
fn rebound_keys_work_and_the_palette_is_always_reachable() {
    let (path, mut app, _engine) = open("config_keys", Some(SONG));
    let config = Config::parse(
        "[teclas]\nguardar = \"alt-s\"\npaleta = \"ctrl-p\"\nsalir = \"f1\"\n",
        true,
        "config.toml",
    );
    app.set_config(&config);
    assert!(
        app.view()
            .message
            .contains("⚠ [teclas] salir: F1 es siempre la paleta")
    );

    // Alt-S guarda; Ctrl-S ya no.
    type_text(&mut app, "-- uno\n");
    app.handle(ctrl('s'));
    assert!(app.view().dirty);
    app.handle(alt('s'));
    assert!(!app.view().dirty);
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .starts_with("-- uno")
    );

    // F1 y Ctrl-P abren la paleta, que enseña los atajos nuevos.
    for open in [f(1), ctrl('p')] {
        app.handle(open);
        let view = app.view().palette.expect("paleta abierta");
        assert!(
            view.items
                .contains(&("Guardar y cargar".into(), "Alt-S".into()))
        );
        app.handle(esc());
    }
    // La línea de ayuda, también.
    let screen = text(&draw(&mut app, 200, 40));
    assert!(screen.contains("Alt-S guardar y cargar"), "{screen}");
    // Salir sigue en Ctrl-Q (F1 no se le pudo dar).
    app.handle(ctrl('q'));
    assert_eq!(app.handle(ctrl('q')), Flow::Quit);
}

#[test]
fn the_theme_colors_what_is_drawn() {
    let (_, mut app, _engine) = open("config_theme", Some(SONG));
    let config = Config::parse(
        "[colores]\nacento = \"#ff8700\"\nerror = \"magenta\"\n",
        false,
        "config.toml",
    );
    theme::set(config.theme);
    app.handle(f(1));
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = find(&buffer, "paleta de comandos").unwrap();
    // Sin truecolor, el naranja pasa al 208 de la paleta de 256.
    assert_eq!(buffer[(x, y)].fg, Color::Indexed(208));
    theme::set(theme::Theme::default());
}
