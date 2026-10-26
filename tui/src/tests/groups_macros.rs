//! Grupos y macros en la interfaz: el panel de efectos muestra cada grupo
//! en su columna (y a qué grupo va cada pista) y las macros en su título;
//! el panel «Macros» las mueve con el teclado, un controlador MIDI de
//! `[midi] macros` también, y recargar las deja donde estaban.

use super::*;
use crate::config::Config;
use crate::midi::{MidiEvent, TimedEvent};
use crate::palette::Action;

const GROUPED: &str = "group(\"bateria\", { \"drums\", \"kick\" })\n  \
                       :fx(\"compressor\", { threshold = -20, ratio = 4 })\n  \
                       :fx(\"lowpass\", { cutoff = macro(\"brillo\", 300, 8000) })\n  \
                       :gain(0.8):send(\"reverb\", 0.2)\n\
                       macro_init(\"brillo\", 0.5)\n\
                       track(\"drums\"):send(\"delay\", macro(\"eco\", 0, 0.6))\n";

#[test]
fn the_fx_panel_shows_groups_and_macros() {
    let (_, mut app, _engine) = open("groups_panel", Some(&format!("{SONG}{GROUPED}")));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let view = app.view();
    assert_eq!(view.groups.len(), 1);
    assert_eq!(view.groups[0].tracks, ["drums", "kick"]);
    assert_eq!(view.tracks[0].group.as_deref(), Some("bateria"));
    assert_eq!(
        view.macros,
        [("brillo".to_string(), 0.5), ("eco".to_string(), 0.0)]
    );
    let screen = text(&draw(&mut app, 160, 45));
    assert!(screen.contains("▣ bateria ← drums, kick"), "{screen}");
    assert!(screen.contains("· ▣ bateria"), "{screen}");
    assert!(screen.contains("vol 0.80"), "{screen}");
    assert!(screen.contains("∿cutoff"), "{screen}");
    assert!(
        screen.contains("macros: brillo 0.50 · eco 0.00"),
        "{screen}"
    );
}

#[test]
fn the_macros_panel_moves_them_and_reloading_keeps_them() {
    let (_, mut app, _engine) = open("macros_panel", Some(&format!("{SONG}{GROUPED}")));
    app.run_action(Action::Macros);
    assert_eq!(app.view().macro_panel, Some(0));
    app.handle(special(Key::Right));
    app.handle(special(Key::Right));
    assert_eq!(app.view().macros[0].1, 0.6);
    assert!(
        app.view().message.contains("«brillo»: 0.60"),
        "{}",
        app.view().message
    );
    app.handle(key('-'));
    assert_eq!(app.view().macros[0].1, 0.59);
    // La segunda: 0-9 salta, Fin lleva a 1 y no pasa de ahí.
    app.handle(special(Key::Down));
    app.handle(key('3'));
    assert_eq!(app.view().macros[1].1, 0.3);
    app.handle(special(Key::End));
    app.handle(special(Key::Right));
    assert_eq!(app.view().macros[1].1, 1.0);
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("macros · ←→ -+ 0-9 · Esc"), "{screen}");
    app.handle(esc());
    assert_eq!(app.view().macro_panel, None);
    // Recargar (Ctrl-L): siguen donde estaban.
    app.handle(ctrl('l'));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    assert_eq!(
        app.view().macros,
        [("brillo".to_string(), 0.59), ("eco".to_string(), 1.0)]
    );
}

#[test]
fn without_macros_the_panel_says_how_to_add_them() {
    let (_, mut app, _engine) = open("macros_none", Some(SONG));
    app.run_action(Action::Macros);
    assert_eq!(app.view().macro_panel, None);
    assert!(
        app.view().message.contains("no hay macros"),
        "{}",
        app.view().message
    );
}

#[test]
fn a_midi_controller_moves_its_macro() {
    let (_, mut app, _engine) = open("macros_midi", Some(&format!("{SONG}{GROUPED}")));
    let config = Config::parse("[midi]\nmacros = [21, 22]\n", true, "c.toml");
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    app.set_config(&config);
    let cc = |number: u8, value: u8| TimedEvent {
        micros: 0,
        event: MidiEvent::Controller {
            channel: 1,
            number,
            value,
        },
    };
    app.midi_event(cc(22, 127));
    app.midi_event(cc(21, 0));
    // Otro controlador no mueve ninguna.
    app.midi_event(cc(23, 64));
    assert_eq!(
        app.view().macros,
        [("brillo".to_string(), 0.0), ("eco".to_string(), 1.0)]
    );
    // Lo que no es una lista de controladores se avisa.
    let config = Config::parse("[midi]\nmacros = [300]\n", true, "c.toml");
    assert_eq!(config.warnings.len(), 1, "{:?}", config.warnings);
}

#[test]
fn group_methods_complete_and_help_like_a_track() {
    let (_, mut app, _engine) = open("groups_complete", Some(SONG));
    app.editor.move_cursor(ratatui_textarea::CursorMove::Bottom);
    app.editor.move_cursor(ratatui_textarea::CursorMove::End);
    type_text(
        &mut app,
        "\nlocal g = group(\"b\", { \"kick\" })\ng:fx(\"comp",
    );
    assert_eq!(completion_items(&app), ["compressor"]);
    app.handle(esc());
    let help = app.view().help.unwrap();
    assert_eq!(help.signature, "grupo:fx(nombre, parámetros) → grupo");
}
