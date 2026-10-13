//! Entrada MIDI simulada: las notas disparan cortes por la misma ruta que
//! los pads y el reloj pone el tempo. Sin dispositivos.

use super::*;
use crate::config::Config;
use crate::midi::{MidiEvent, TimedEvent};

fn note(note: u8, velocity: u8, channel: u8) -> TimedEvent {
    TimedEvent {
        micros: 0,
        event: MidiEvent::NoteOn {
            channel,
            note,
            velocity,
        },
    }
}

const QUIET: &str = "bpm(120)\nlocal amen = sample(\"break.wav\"):slice(2)\n\
                     track(\"drums\", amen):steps(\". . . .  . . . .  . . . .  . . . .\")\n";

#[test]
fn notes_play_slices_like_the_pads() {
    let (_, mut app, mut engine) = open("midi_notes", Some(QUIET));
    let (mut tx, rx) = rtrb::RingBuffer::new(16);
    app.set_midi_input(rx);
    offline::render(&mut engine, 100, 2);
    // La nota 37 (base 36) es el corte 2; velocidad 127/2 → la mitad.
    tx.push(note(37, 127, 1)).unwrap();
    app.poll_midi();
    assert_eq!(app.view().message, "▶ drums: corte 2");
    assert_eq!(app.view().selected_slice, Some(1));
    let audio = offline::render(&mut engine, 1000, 2);
    assert_eq!(audio[2 * 500], 0.25, "el corte 2 a velocidad máxima");
    tx.push(note(37, 64, 1)).unwrap();
    app.poll_midi();
    let audio = offline::render(&mut engine, 1000, 2);
    assert!((audio[2 * 500] - 0.25 * 64.0 / 127.0).abs() < 1e-6);
    // Por debajo de la base, nada; un corte que no existe, lo dice.
    tx.push(note(30, 100, 1)).unwrap();
    tx.push(note(40, 100, 1)).unwrap();
    app.poll_midi();
    assert!(
        app.view().message.contains("no tiene corte 5"),
        "{}",
        app.view().message
    );

    // Con canal y otra base en la configuración.
    let config = Config::parse("[midi]\nnota_base = 60\ncanal = 10\n", true, "c.toml");
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    app.set_config(&config);
    app.midi_event(note(60, 100, 1));
    assert!(!app.view().message.contains("corte 1"), "otro canal");
    app.midi_event(note(60, 100, 10));
    assert_eq!(app.view().message, "▶ drums: corte 1");
}

#[test]
fn the_midi_clock_sets_the_tempo_when_enabled() {
    let (_, mut app, mut engine) = open("midi_clock", Some(QUIET));
    let pulse = |k: u64| TimedEvent {
        micros: 10_000 + k * 60_000_000 / (150 * 24),
        event: MidiEvent::Clock,
    };
    // Sin `reloj = true`, se ignora.
    for k in 0..200 {
        app.midi_event(pulse(k));
    }
    offline::render(&mut engine, 2 * 16 * 6000, 2);
    app.tick();
    assert_eq!(app.view().transport.bpm, 120.0);
    // Con él, pone el tempo (al compás) y Stop/Start mandan.
    app.set_config(&Config::parse("[midi]\nreloj = true\n", true, "c.toml"));
    for k in 0..200 {
        app.midi_event(pulse(k));
    }
    assert!(
        app.view().message.starts_with("reloj MIDI: 150 BPM"),
        "{}",
        app.view().message
    );
    offline::render(&mut engine, 2 * 16 * 6000, 2);
    app.tick();
    assert_eq!(app.view().transport.bpm, 150.0);
    app.midi_event(TimedEvent {
        micros: 0,
        event: MidiEvent::Stop,
    });
    offline::render(&mut engine, 100, 2);
    app.tick();
    assert!(!app.view().transport.running);
    let bad = Config::parse("[midi]\nnota_base = 300\nvelocidad = 1\n", true, "c.toml");
    assert_eq!(bad.warnings.len(), 2, "{:?}", bad.warnings);
}
