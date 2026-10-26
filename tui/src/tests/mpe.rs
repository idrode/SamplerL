//! Expresión por nota desde el MIDI de entrada (fase Q), con mensajes
//! simulados: en MPE, el bend que trae el canal de la nota antes de
//! pulsarla y el que llega después mueven su altura en un sintetizador con
//! `:mpe{ bend = "pitch" }`; lo de otro canal, no.

use super::*;
use crate::config::Config;
use crate::midi::{MidiEvent, TimedEvent};

const SUB: &str = "bpm(120)\n\
                   track(\"bajo\", synth(\"sub\", { tone = 2000 }))\n  \
                   :mpe{ bend = \"pitch\", pressure = { \"gain\", range = 0.5 } }\n  \
                   :melody(\". . . .  . . . .  . . . .  . . . .\")\n";

fn at(event: MidiEvent) -> TimedEvent {
    TimedEvent { micros: 0, event }
}

fn note_on(channel: u8, note: u8) -> TimedEvent {
    at(MidiEvent::NoteOn {
        channel,
        note,
        velocity: 100,
    })
}

fn bend(channel: u8, value: i16) -> TimedEvent {
    at(MidiEvent::PitchBend { channel, value })
}

/// Frecuencia (izquierda) por los cruces por cero que suben.
fn frequency(stereo: &[f32]) -> f64 {
    let y: Vec<f32> = stereo.iter().step_by(2).copied().collect();
    let ups: Vec<f64> = y
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] <= 0.0 && w[1] > 0.0)
        .map(|(i, w)| i as f64 + (-w[0] / (w[1] - w[0])) as f64)
        .collect();
    RATE as f64 * (ups.len() - 1) as f64 / (ups[ups.len() - 1] - ups[0])
}

fn mpe_app(test: &str) -> (App, Engine) {
    let (_, mut app, engine) = open(test, Some(SUB));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let config = Config::parse("[midi]\nmpe = true\nbend = 24\ncanal = 1\n", true, "c.toml");
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    app.set_config(&config);
    (app, engine)
}

#[test]
fn each_mpe_channel_bends_its_own_note() {
    let (mut app, mut engine) = mpe_app("mpe_bend");
    offline::render(&mut engine, 512, 2);
    // Sin bend: la nota tal cual (en MPE, el canal 2 aunque `canal` sea 1).
    app.midi_event(note_on(2, 48));
    app.tick();
    offline::render(&mut engine, 4800, 2);
    let plain = frequency(&offline::render(&mut engine, 9600, 2));
    app.midi_event(at(MidiEvent::NoteOff {
        channel: 2,
        note: 48,
    }));
    offline::render(&mut engine, 24_000, 2);

    // El canal 3 trae medio bend arriba antes de la nota: 12 de 24
    // semitonos, una octava desde el principio.
    app.midi_event(bend(3, 4096));
    app.midi_event(note_on(3, 48));
    app.tick();
    offline::render(&mut engine, 4800, 2);
    let octave = frequency(&offline::render(&mut engine, 9600, 2));
    assert!(
        (octave / plain / 2.0 - 1.0).abs() < 0.01,
        "{plain} → {octave}"
    );

    // Lo de otro canal no la mueve; lo del suyo, sí (vuelve al centro).
    app.midi_event(bend(4, -8192));
    app.tick();
    offline::render(&mut engine, 4800, 2);
    let same = frequency(&offline::render(&mut engine, 9600, 2));
    assert!((same / octave - 1.0).abs() < 0.01);
    app.midi_event(bend(3, 0));
    app.tick();
    offline::render(&mut engine, 4800, 2);
    let back = frequency(&offline::render(&mut engine, 9600, 2));
    assert!((back / plain - 1.0).abs() < 0.01, "{plain} → {back}");
}

#[test]
fn the_config_reads_mpe_and_the_bend_range() {
    let config = Config::parse("[midi]\nmpe = true\nbend = 48\n", true, "c.toml");
    assert!(config.warnings.is_empty());
    assert!(config.midi.mpe);
    assert_eq!(config.midi.bend, Some(48));
    let config = Config::parse("[midi]\nmpe = 1\nbend = 200\n", true, "c.toml");
    assert_eq!(config.warnings.len(), 2, "{:?}", config.warnings);
    assert!(config.warnings[0].contains("bend 1-96"));
}
