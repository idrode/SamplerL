//! El editor abajo: su alto con Ctrl-Alt-↑↓ y arrastrando su borde de
//! arriba (con sus límites y guardado en `[interfaz]`), maximizado con
//! Alt-Z, y el ancho del explorador con Ctrl-Alt-←→.

use super::*;
use crate::app::MouseAction;
use crate::config::Config;

pub fn ctrl_alt(key: Key) -> Input {
    Input {
        key,
        ctrl: true,
        alt: true,
        ..Input::default()
    }
}

pub fn alt(c: char) -> Input {
    Input {
        key: Key::Char(c),
        alt: true,
        ..Input::default()
    }
}

fn editor(app: &App) -> ratatui::layout::Rect {
    app.last_grid().unwrap().editor.unwrap()
}

#[test]
pub fn ctrl_alt_up_and_down_change_the_height_within_its_limits() {
    let (_, mut app, _engine) = open("dock_keys", Some(SONG));
    draw(&mut app, 120, 40);
    let start = editor(&app).height;
    app.handle(ctrl_alt(Key::Up));
    draw(&mut app, 120, 40);
    assert_eq!(editor(&app).height, start + 1);
    // El borde de abajo no se mueve: crece hacia arriba.
    assert_eq!(
        editor(&app).bottom(),
        app.last_grid().unwrap().message.unwrap().y
    );
    for _ in 0..5 {
        app.handle(ctrl_alt(Key::Down));
    }
    draw(&mut app, 120, 40);
    assert_eq!(editor(&app).height, start - 4);
    // Ni menos del mínimo ni más de lo que deja al sonido.
    for _ in 0..50 {
        app.handle(ctrl_alt(Key::Down));
    }
    draw(&mut app, 120, 40);
    assert_eq!(editor(&app).height, crate::layout::EDITOR_MIN);
    for _ in 0..80 {
        app.handle(ctrl_alt(Key::Up));
    }
    draw(&mut app, 120, 40);
    let body = app.last_grid().unwrap().body.unwrap();
    assert_eq!(editor(&app).height, body.height - crate::layout::SOUND_MIN);
    assert!(
        app.last_grid().unwrap().wave.is_some(),
        "el sonido sigue a la vista"
    );
    // El texto no se ha tocado.
    assert_eq!(app.text_for_test(), SONG);
}

#[test]
fn dragging_the_top_border_resizes_and_a_click_there_gives_the_focus() {
    let (_, mut app, _engine) = open("dock_drag", Some(SONG));
    draw(&mut app, 120, 40);
    let before = editor(&app);
    let dock_before = app.last_grid().unwrap().dock.as_ref().unwrap().area;
    app.mouse(MouseAction::Down, before.x + 30, before.y);
    app.mouse(MouseAction::Drag, before.x + 30, before.y - 5);
    app.mouse(MouseAction::Up, before.x + 30, before.y - 5);
    draw(&mut app, 120, 40);
    let after = editor(&app);
    assert_eq!(after.height, before.height + 5);
    assert_eq!(after.y, before.y - 5);
    // Lo de arriba se aparta.
    let dock = app.last_grid().unwrap().dock.as_ref().unwrap().area;
    assert!(dock.bottom() <= after.y && dock.bottom() < dock_before.bottom());
    // Desde el modo tocar, un clic sin arrastrar en ese borde da el foco.
    app.handle(esc());
    assert!(app.view().playing_mode);
    app.mouse(MouseAction::Down, after.x + 30, after.y);
    app.mouse(MouseAction::Up, after.x + 30, after.y);
    assert!(!app.view().playing_mode);
    draw(&mut app, 120, 40);
    assert_eq!(
        editor(&app).height,
        after.height,
        "sin arrastrar, mismo alto"
    );
}

#[test]
fn alt_z_maximizes_and_restores_and_the_audio_view_comes_back() {
    let (_, mut app, _engine) = open("dock_max", Some(SONG));
    draw(&mut app, 120, 40);
    let docked = editor(&app);
    app.handle(alt('z'));
    let screen = text(&draw(&mut app, 120, 40));
    let grid = app.last_grid().unwrap();
    let files = grid.explorer.unwrap();
    assert_eq!(editor(&app).height, files.height);
    assert!(grid.wave.is_none() && grid.tracker.is_none());
    assert!(!screen.contains("┌ sample"), "{screen}");
    assert!(screen.contains("bpm(120)"), "{screen}");
    // La cabecera (el transporte) sigue.
    assert!(screen.lines().nth(1).unwrap().contains("BPM"), "{screen}");
    // Ctrl-Alt-↑ no hace nada maximizado.
    app.handle(ctrl_alt(Key::Up));
    assert!(app.view().message.contains("maximizado"));
    app.handle(alt('z'));
    draw(&mut app, 120, 40);
    assert_eq!(editor(&app), docked);
    assert!(app.last_grid().unwrap().wave.is_some());
}

#[test]
fn the_height_comes_from_the_config_and_is_saved_there() {
    let (path, mut app, _engine) = open("dock_config", Some(SONG));
    let config_path = path.parent().unwrap().join("config.toml");
    std::fs::write(&config_path, "# mío\n[interfaz]\nalto_editor = 12\n").unwrap();
    let config = Config::load(&config_path, true);
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    app.set_config(&config);
    app.set_devices_config(Some(config_path.clone()));
    draw(&mut app, 120, 40);
    assert_eq!(editor(&app).height, 12);
    app.handle(ctrl_alt(Key::Up));
    let saved = std::fs::read_to_string(&config_path).unwrap();
    assert!(saved.starts_with("# mío\n[interfaz]\n"), "{saved}");
    assert!(saved.contains("alto_editor = 13"), "{saved}");
    // Y el explorador con Ctrl-Alt-→ (sin flotante): más ancho, guardado.
    let width = app.last_grid().unwrap().explorer.unwrap().width;
    app.handle(ctrl_alt(Key::Right));
    draw(&mut app, 120, 40);
    assert_eq!(app.last_grid().unwrap().explorer.unwrap().width, width + 2);
    let saved = std::fs::read_to_string(&config_path).unwrap();
    assert!(
        saved.contains(&format!("ancho_explorador = {}", width + 2)),
        "{saved}"
    );
    // Valores fuera de rango: aviso y el de siempre.
    let bad = Config::parse("[interfaz]\nalto_editor = 2\n", true, "c");
    assert_eq!(
        bad.warnings,
        ["[interfaz] alto_editor: 0 o filas, de 4 a 200"]
    );
}

#[test]
fn arrows_with_ctrl_alt_can_be_rebound() {
    let config = Config::parse(
        "[teclas]\neditor_mas_alto = \"ctrl-alt-k\"\nmenos_ancho = \"left\"\n",
        true,
        "c",
    );
    assert_eq!(
        config.warnings,
        [
            "[teclas] menos_ancho: «left»: una flecha sin ctrl- ni alt- es del panel que tiene el foco"
        ]
    );
    let (_, mut app, _engine) = open("dock_rebind", Some(SONG));
    app.set_config(&config);
    draw(&mut app, 120, 40);
    let start = editor(&app).height;
    app.handle(ctrl_alt(Key::Char('k')));
    draw(&mut app, 120, 40);
    assert_eq!(editor(&app).height, start + 1);
}
