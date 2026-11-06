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

/// `[script]`: un script que reserva sin parar se corta en el límite de la
/// configuración, con su error en el editor; sigue sonando lo anterior y
/// la carga siguiente entra.
#[test]
fn the_script_limits_come_from_the_config() {
    let (_, mut app, mut engine) = open("config_script_limits", Some(SONG));
    app.set_config(&Config::parse(
        "[script]\nmemoria = 16\ntiempo = 1\n",
        true,
        "config.toml",
    ));
    offline::render(&mut engine, 4 * STEP, 2);
    app.tick();
    assert!(app.view().transport.running, "suena al abrir");
    let before = app.view().tracks.len();
    let bomb = format!(
        "{SONG}\nlocal t = {{}}\nwhile true do t[#t + 1] = string.rep(\"x\", 100000) .. #t end\n"
    );
    replace_all(&mut app, &bomb);
    app.load();
    let error = app.view().error.expect("el error en el editor");
    assert!(
        error.message.contains("más de 16 MB") && error.message.contains("[script] memoria"),
        "{}",
        error.message
    );
    assert!(error.line.is_some(), "con su línea: {}", error.message);
    // Sigue sonando lo de antes.
    offline::render(&mut engine, 4 * STEP, 2);
    app.tick();
    assert!(app.view().transport.running);
    assert_eq!(app.view().tracks.len(), before);
    // Y lo siguiente, bueno, entra.
    replace_all(&mut app, SONG);
    app.load();
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
}
