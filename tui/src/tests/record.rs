//! Grabar lo tocado: pads y notas MIDI simuladas, cuantizados al paso, se
//! escriben como `steps("...")` en el cursor.

use super::*;
use crate::midi::{MidiEvent, TimedEvent};

const QUIET: &str = "bpm(120)\nlocal amen = sample(\"break.wav\"):slice(2)\n\
                     track(\"drums\", amen):steps(\". . . .  . . . .  . . . .  . . . .\")\n";

/// Lleva el motor hasta la posición `steps` (pasos de la canción).
fn go_to(engine: &mut Engine, now: &mut f64, steps: f64) {
    let frames = ((steps - *now) * STEP as f64).round() as usize;
    offline::render(engine, frames, 2);
    *now = steps;
}

#[test]
fn pads_and_midi_are_recorded_on_the_nearest_step() {
    let (_, mut app, mut engine) = open("record", Some(QUIET));
    let mut now = 0.0;
    go_to(&mut engine, &mut now, 2.3);
    app.tick();
    // Desde la paleta: deja escrita la orden.
    palette(&mut app, "grabar lo tocado");
    assert_eq!(app.view().prompt.as_deref(), Some("grabar 1"));
    app.handle(special(Key::Enter));
    let screen = text(&draw(&mut app, 170, 40));
    assert!(screen.contains("● REC drums: al compás"), "{screen}");

    // El cursor, en una línea nueva al final; a tocar.
    app.editor.move_cursor(ratatui_textarea::CursorMove::Bottom);
    app.editor.move_cursor(ratatui_textarea::CursorMove::End);
    type_text(&mut app, "\ntrack(\"drums\"):");
    // (El primer Esc cierra el autocompletado que abrió «:».)
    app.handle(esc());
    app.handle(esc());
    assert!(app.view().playing_mode);
    // Un poco antes del compás: cuenta como el paso 0.
    go_to(&mut engine, &mut now, 15.8);
    app.handle(key('1'));
    go_to(&mut engine, &mut now, 20.4);
    app.midi_event(TimedEvent {
        micros: 0,
        event: MidiEvent::NoteOn {
            channel: 1,
            note: 37,
            velocity: 64,
        },
    });
    go_to(&mut engine, &mut now, 23.6);
    app.handle(key('2'));
    app.tick();
    assert!(text(&draw(&mut app, 170, 40)).contains("● REC drums 1/1"));
    go_to(&mut engine, &mut now, 31.6);
    app.tick();
    assert!(app.view().recording.is_none());
    assert_eq!(
        app.editor.lines().last().unwrap(),
        "track(\"drums\"):steps(\"1 . . .  2@0.5 . . .  2 . . .  . . . .\")"
    );
    assert!(app.view().message.starts_with("grabado drums"));
    // Se deshace de una vez.
    app.handle(esc());
    app.handle(ctrl('z'));
    assert_eq!(app.editor.lines().last().unwrap(), "track(\"drums\"):");
}

#[test]
fn longer_takes_stopping_early_and_cancelling() {
    let (_, mut app, mut engine) = open("record_long", Some(QUIET));
    let mut now = 0.0;
    go_to(&mut engine, &mut now, 1.0);
    app.tick();
    command(&mut app, "grabar 2");
    app.handle(esc());
    go_to(&mut engine, &mut now, 16.0 + 18.0);
    app.handle(key('2'));
    // «Grabar lo tocado» otra vez: para antes y escribe lo que hay.
    palette(&mut app, "grabar lo tocado");
    let last = app.editor.lines().join("\n");
    assert!(
        last.contains("length(32):steps(\". . . .  . . . .  . . . .  . . . .  . . 2 .  "),
        "{last}"
    );

    // Sin marcha no se graba; si se para, se cancela.
    command(&mut app, "grabar 9");
    assert!(app.view().message.starts_with("uso: grabar N (de 1 a 4"));
    command(&mut app, "grabar");
    app.handle(ctrl('t'));
    offline::render(&mut engine, 100, 2);
    app.tick();
    assert!(app.view().recording.is_none());
    assert_eq!(
        app.view().message,
        "grabación cancelada: se paró el secuenciador"
    );
    command(&mut app, "grabar");
    assert_eq!(
        app.view().message,
        "arranca el secuenciador (marcha) para grabar"
    );
}
