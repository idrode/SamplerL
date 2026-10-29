//! Dispositivos sin tarjeta ni ALSA: la salida de audio con el backend
//! falso del motor (sus streams se mueven a mano) y el MIDI con el falso de
//! `devices`. Cambiar de dispositivo no pierde el compás, uno que se va no
//! cuelga nada y el MIDI se conecta y desconecta sin tocar el audio.

use engine::AudioOutput;
use engine::device::fake::FakeAudio;

use super::*;
use crate::devices::fake::FakeMidi;
use crate::devices::{Devices, Kind, VIRTUAL_IN, VIRTUAL_OUT, port_key};

/// Los altavoces (predeterminados) y unos auriculares.
pub(super) fn fake_audio() -> FakeAudio {
    let fake = FakeAudio::default();
    fake.add("Altavoces", 2, &[RATE], true);
    fake.add("Auriculares USB", 2, &[RATE, 96_000], false);
    fake
}

pub(super) const SEABOARD: &str = "Seaboard:Seaboard MIDI 1 24:0";
pub(super) const SURGE: &str = "Surge XT:Surge XT MIDI In 129:0";

pub(super) fn fake_midi() -> FakeMidi {
    FakeMidi::with_ports(
        &[
            SEABOARD,
            "Midi Through:Midi Through Port-0 14:0",
            "sampler:sampler salida 128:1",
        ],
        &[SURGE, "Midi Through:Midi Through Port-0 14:0"],
    )
}

/// Una principal con la canción sonando por el dispositivo falso
/// predeterminado y el MIDI falso abierto en sus puertos virtuales.
pub(super) fn with_devices(test: &str) -> (App, FakeAudio, FakeMidi) {
    let path = fixture(test, Some(SONG));
    let (controller, engine) = new_engine(RATE);
    let audio = fake_audio();
    let (output, warning) = AudioOutput::open(audio.backend(), engine, None).unwrap();
    assert_eq!(warning, None);
    let mut app = App::open(&path, controller, RATE).unwrap();
    let midi = fake_midi();
    let mut devices = Devices::new(Some(output), true, Some(midi.backend()));
    let changes = devices.start_midi(None, None);
    app.set_devices(devices, changes);
    (app, audio, midi)
}

/// Mueve el stream de `device` y atiende la aplicación hasta que el cambio
/// de audio termina.
fn finish_switch(app: &mut App, audio: &FakeAudio, old: &str) {
    for _ in 0..20 {
        audio.pump(old, 256);
        app.tick();
        if app.view().message.starts_with("audio: suena por") {
            return;
        }
    }
    panic!("el cambio no terminó: {}", app.view().message);
}

#[test]
fn port_names_lose_the_alsa_numbers() {
    assert_eq!(port_key(SEABOARD), "Seaboard:Seaboard MIDI 1");
    assert_eq!(
        port_key("Seaboard:Seaboard MIDI 1"),
        "Seaboard:Seaboard MIDI 1"
    );
    assert_eq!(port_key("Algo 2:3x"), "Algo 2:3x");
    assert_eq!(port_key("Raro :1"), "Raro :1");
    assert_eq!(port_key("solo"), "solo");
}

#[test]
fn the_lists_show_what_there_is_and_what_is_in_use() {
    let (app, _audio, _midi) = with_devices("devices_lists");
    let view = app.devices_view().unwrap();
    assert_eq!(view.audio.len(), 2);
    assert!(view.audio[0].active && view.audio[0].default);
    assert_eq!(view.audio[1].detail, "2 canales · 48000, 96000 Hz");
    // El puerto virtual va primero; los propios no salen (serían un
    // bucle) y los nombres van sin los números de ALSA.
    let names: Vec<&str> = view.midi_in.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        [
            VIRTUAL_IN,
            "Seaboard:Seaboard MIDI 1",
            "Midi Through:Midi Through Port-0"
        ]
    );
    assert!(view.midi_in[0].active && !view.midi_in[1].active);
    assert_eq!(view.midi_out[0].name, VIRTUAL_OUT);
    assert!(
        view.status.contains("«Altavoces» · 48000 Hz · 2 canales"),
        "{}",
        view.status
    );
    assert!(view.notes.iter().all(String::is_empty));
}

#[test]
fn switching_audio_keeps_the_bar_and_says_where_it_plays() {
    let (mut app, audio, _midi) = with_devices("devices_switch");
    // Un compás y medio (a 120 BPM, 16 pasos de 6000 frames).
    for _ in 0..(24 * STEP / 256) {
        audio.pump("Altavoces", 256);
    }
    app.tick();
    let before = app.view().transport;
    assert_eq!(before.bar, 1);

    app.choose_device(Kind::Audio, "Auriculares USB");
    assert_eq!(app.view().message, "audio: cambiando a «Auriculares USB»…");
    finish_switch(&mut app, &audio, "Altavoces");
    let message = app.view().message;
    assert!(
        message.contains("«Auriculares USB» (latencia ≈"),
        "{message}"
    );
    assert_eq!(audio.open_streams(), ["Auriculares USB"]);
    // El mismo compás y el mismo paso (como mucho un bloque más).
    app.tick();
    let after = app.view().transport;
    assert_eq!(after.bar, before.bar);
    assert!(after.step >= before.step);
    // Y sigue: el compás siguiente llega por los auriculares.
    for _ in 0..(8 * STEP / 256 + 2) {
        audio.pump("Auriculares USB", 256);
    }
    app.tick();
    assert_eq!(app.view().transport.bar, 2);
    let view = app.devices_view().unwrap();
    assert!(view.audio[1].active && !view.audio[0].active);
    assert_eq!(
        app.device_choice().unwrap().audio.as_deref(),
        Some("Auriculares USB")
    );

    // El mismo otra vez: nada que hacer.
    app.choose_device(Kind::Audio, "Auriculares USB");
    assert_eq!(app.view().message, "audio: ya suena por «Auriculares USB»");
}

#[test]
fn unplugging_the_headphones_goes_back_to_the_speakers_and_keeps_going() {
    let (mut app, audio, _midi) = with_devices("devices_unplug");
    app.choose_device(Kind::Audio, "Auriculares USB");
    finish_switch(&mut app, &audio, "Altavoces");
    for _ in 0..40 {
        audio.pump("Auriculares USB", 256);
    }
    app.tick();
    let step = app.view().transport.step;
    audio.unplug("Auriculares USB");
    app.tick();
    assert_eq!(
        app.view().message,
        "audio: suena por «Altavoces» (latencia ≈ 0.0 ms)",
        "el aviso de la pérdida y luego el del cambio"
    );
    assert_eq!(
        app.device_choice().unwrap().audio.as_deref(),
        Some("Altavoces")
    );
    for _ in 0..40 {
        audio.pump("Altavoces", 256);
    }
    app.tick();
    assert!(app.view().transport.step > step, "la canción sigue");
    // Al refrescar, ya no está en la lista.
    app.refresh_devices();
    assert_eq!(app.devices_view().unwrap().audio.len(), 1);
    // Elegir uno que no está: error y sigue el que suena.
    app.choose_device(Kind::Audio, "Auriculares USB");
    let message = app.view().message;
    assert!(
        message.contains("no se pudo abrir «Auriculares USB»"),
        "{message}"
    );
    assert!(message.contains("sigue el que estaba"), "{message}");
}

#[test]
fn midi_in_connects_to_a_port_by_name_and_the_notes_arrive() {
    let (mut app, _audio, midi) = with_devices("devices_midi_in");
    assert_eq!(midi.0.borrow().input, Some(None), "el virtual al arrancar");
    app.choose_device(Kind::MidiIn, "Seaboard:Seaboard MIDI 1");
    assert_eq!(
        app.view().message,
        "MIDI de entrada: «Seaboard:Seaboard MIDI 1»"
    );
    assert_eq!(midi.0.borrow().input, Some(Some(SEABOARD.into())));
    let view = app.devices_view().unwrap();
    assert!(view.midi_in[1].active && !view.midi_in[0].active);
    // Una nota llega a la aplicación por la cola nueva (la 37, el corte 2)
    // y mueve el indicador de actividad.
    assert_eq!(view.activity, 0);
    midi.receive(&[0x90, 37, 100]);
    app.poll_midi();
    assert_eq!(app.view().message, "▶ drums: corte 2");
    assert_eq!(app.devices_view().unwrap().activity, 1);
    // El reloj no cuenta como actividad.
    midi.receive(&[0xf8]);
    app.poll_midi();
    assert_eq!(app.devices_view().unwrap().activity, 1);
    assert_eq!(
        app.device_choice().unwrap().midi_in.as_deref(),
        Some("Seaboard:Seaboard MIDI 1")
    );

    // Uno que falla al abrir: sigue el que había.
    midi.0
        .borrow_mut()
        .broken
        .push("Midi Through:Midi Through Port-0 14:0".into());
    app.choose_device(Kind::MidiIn, "Midi Through:Midi Through Port-0");
    let message = app.view().message;
    assert!(message.contains("no se pudo abrir"), "{message}");
    assert!(message.contains("sigue la que había"), "{message}");
    assert_eq!(midi.0.borrow().input, Some(Some(SEABOARD.into())));
    midi.receive(&[0x90, 36, 100]);
    app.poll_midi();
    assert_eq!(app.view().message, "▶ drums: corte 1");

    // Se desenchufa: midir no avisa; al refrescar vuelve al virtual.
    midi.0.borrow_mut().inputs.retain(|p| p != SEABOARD);
    app.refresh_devices();
    assert_eq!(
        app.view().message,
        "MIDI de entrada: «sampler entrada (puerto virtual)»"
    );
    assert_eq!(midi.0.borrow().input, Some(None));
    assert_eq!(app.device_choice().unwrap().midi_in, None);
}

#[test]
fn midi_out_moves_to_a_synth_and_back_to_virtual_if_it_fails() {
    let (mut app, _audio, midi) = with_devices("devices_midi_out");
    app.choose_device(Kind::MidiOut, "Surge XT:Surge XT MIDI In");
    assert_eq!(
        app.view().message,
        "MIDI de salida: «Surge XT:Surge XT MIDI In»"
    );
    assert_eq!(midi.0.borrow().output, Some(Some(SURGE.into())));
    midi.0.borrow_mut().failing = true;
    app.tick();
    let message = app.view().message;
    assert_eq!(message, "MIDI de salida: «sampler salida (puerto virtual)»");
    assert_eq!(midi.0.borrow().output, Some(None));
    // Uno que no está en la lista: lo dice y no cambia.
    app.choose_device(Kind::MidiOut, "Nada:Nada");
    assert!(app.view().message.contains("no está «Nada:Nada»"));
    assert_eq!(midi.0.borrow().output, Some(None));
}

#[test]
fn at_start_a_missing_port_opens_the_virtual_one_with_a_warning() {
    let midi = fake_midi();
    let mut devices = Devices::new(None, false, Some(midi.backend()));
    let changes = devices.start_midi(Some("Launchpad:Launchpad MIDI 1"), Some(SURGE));
    let messages: Vec<String> = changes
        .into_iter()
        .filter_map(|c| match c {
            crate::devices::Change::Message(m) => Some(m),
            _ => None,
        })
        .collect();
    assert_eq!(
        messages,
        [
            "MIDI de entrada: no está «Launchpad:Launchpad MIDI 1»; abre el puerto virtual",
            "MIDI de salida: «Surge XT:Surge XT MIDI In»"
        ]
    );
    assert_eq!(midi.0.borrow().input, Some(None));
    // (Por el nombre con números también vale.)
    assert_eq!(midi.0.borrow().output, Some(Some(SURGE.into())));
}

#[test]
fn without_permission_nothing_is_listed_or_opened() {
    let (controller, engine) = new_engine(RATE);
    drop(controller);
    let output = AudioOutput::open_null(engine, RATE);
    let mut devices = Devices::new(Some(output), false, None);
    let view = devices.view();
    assert_eq!(view.audio.len(), 1);
    assert_eq!(view.audio[0].name, engine::NULL_NAME);
    assert!(view.notes[0].contains("--allow-audio"));
    assert!(view.midi_in.is_empty() && view.notes[1].contains("--allow-midi"));
    let message = |changes: Vec<crate::devices::Change>| match changes.into_iter().next() {
        Some(crate::devices::Change::Message(m)) => m,
        _ => panic!("un aviso"),
    };
    assert!(message(devices.choose(Kind::Audio, "Altavoces")).contains("--allow-audio"));
    assert!(message(devices.choose(Kind::MidiIn, VIRTUAL_IN)).contains("--allow-midi"));
}
