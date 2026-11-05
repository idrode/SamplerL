//! En reposo la interfaz no dibuja de más: la huella de `App::idle_key`
//! no cambia mientras no cambie lo que se ve, y no hay huella (se dibuja
//! siempre) mientras algo se mueve solo.

use super::*;
use crate::midi::{MidiEvent, TimedEvent};

/// Mueve el motor `steps` pasos con un `tick` en cada octavo de paso.
fn play(app: &mut App, engine: &mut Engine, steps: usize) {
    for _ in 0..8 * steps {
        offline::render(engine, STEP / 8, 2);
        app.tick();
    }
}

#[test]
fn stopped_and_silent_the_key_holds_still() {
    let (_, mut app, mut engine) = open("idle_stopped", Some(SONG));
    play(&mut app, &mut engine, 4);
    assert_eq!(app.idle_key(), None, "en marcha se dibuja siempre");
    app.toggle_transport();
    // Que se apaguen las colas y los medidores (los avisos de saturación
    // duran un tiempo de reloj: aquí se quitan a mano).
    play(&mut app, &mut engine, 64);
    assert_eq!(
        app.idle_key(),
        None,
        "con el aviso del limitador, se dibuja"
    );
    app.reset_meters();
    let key = app.idle_key().expect("parado y en silencio");
    play(&mut app, &mut engine, 8);
    assert_eq!(app.idle_key(), Some(key), "nada cambia: el cuadro se salta");
    // Un mensaje nuevo (una carga, un aviso...) sí cambia lo que se ve.
    app.toggle_lanes();
    assert_ne!(app.idle_key(), Some(key));
}

#[test]
fn a_midi_message_with_the_transport_stopped_changes_the_key() {
    let (_, mut app, mut engine) = open("idle_midi", Some(SONG));
    app.toggle_transport();
    play(&mut app, &mut engine, 64);
    app.reset_meters();
    let key = app.idle_key().expect("parado y en silencio");
    // Un mando movido: su valor se ve en los paneles.
    app.midi_event(TimedEvent {
        micros: 0,
        event: MidiEvent::Controller {
            channel: 1,
            number: 21,
            value: 64,
        },
    });
    assert_ne!(app.idle_key(), Some(key));
}

#[test]
fn without_the_engine_status_yet_there_is_no_key() {
    let (_, app, _engine) = open("idle_fresh", Some(SONG));
    assert_eq!(app.idle_key(), None, "hasta el primer tick, se dibuja");
}
