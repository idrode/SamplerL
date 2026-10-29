//! Dispositivos de salida sin tarjeta: el backend falso lista, abre,
//! desenchufa y mueve los streams a mano. El motor pasa de uno a otro sin
//! perder la canción ni el compás, con un fundido corto en cada lado.

use std::sync::Arc;

use engine::device::fake::FakeAudio;
use engine::device::{FADE_SECONDS, HANDOFF_TIMEOUT};
use engine::notation::parse_steps;
use engine::{AudioEvent, AudioOutput, Pattern, Sample, Slices, new_engine, offline};

const RATE: u32 = 48_000;
/// Frames de un fundido a 48 kHz.
const FADE: usize = (FADE_SECONDS * RATE as f32) as usize;

/// Un patrón que suena en todos los pasos (para comparar audio).
fn pattern() -> Pattern {
    let mono: Vec<f32> = (0..6000).map(|i| ((i as f32) * 0.05).sin() * 0.5).collect();
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    let steps = parse_steps("1 2 1 2  1 2 1 2  1 2 1 2  1 2 1 2").unwrap();
    Pattern::from_steps(sample, &slices, steps).unwrap()
}

/// Dos dispositivos: los altavoces (predeterminado) y unos auriculares.
fn fake() -> FakeAudio {
    let fake = FakeAudio::default();
    fake.add("Altavoces", 2, &[44_100, 48_000], true);
    fake.add("Auriculares USB", 2, &[48_000, 96_000], false);
    fake
}

fn playing() -> (engine::Controller, engine::Engine) {
    let (mut controller, engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern()).unwrap();
    controller.start().unwrap();
    (controller, engine)
}

/// Mueve el stream de `device` hasta que el cambio termina.
fn finish_switch(output: &mut AudioOutput, fake: &FakeAudio, old: &str) -> Vec<AudioEvent> {
    for _ in 0..20 {
        fake.pump(old, 256);
        let events = output.poll();
        if !events.is_empty() {
            return events;
        }
    }
    panic!("el cambio no terminó");
}

#[test]
fn lists_and_opens_the_default_or_the_chosen_device() {
    let fake = fake();
    let (_, engine) = new_engine(RATE);
    let (mut output, warning) = AudioOutput::open(fake.backend(), engine, None).unwrap();
    assert_eq!(warning, None);
    let list = output.list().unwrap();
    assert_eq!(list.len(), 2);
    assert!(list[0].default && !list[1].default);
    assert_eq!(list[1].sample_rates, [48_000, 96_000]);
    let info = output.info().unwrap();
    assert_eq!(info.name, "Altavoces");
    // La primera vez el motor toma la frecuencia del dispositivo (la
    // primera de su lista en el falso).
    assert_eq!(info.sample_rate, 44_100);
    assert_eq!(output.sample_rate(), 44_100);

    let (_, engine) = new_engine(RATE);
    let (output, warning) =
        AudioOutput::open(fake.backend(), engine, Some("Auriculares USB")).unwrap();
    assert_eq!(warning, None);
    assert_eq!(output.info().unwrap().name, "Auriculares USB");
    // También por su identificador.
    let (_, engine) = new_engine(RATE);
    let (output, _) =
        AudioOutput::open(fake.backend(), engine, Some("fake:Auriculares USB")).unwrap();
    assert_eq!(output.info().unwrap().name, "Auriculares USB");
}

#[test]
fn a_saved_device_that_is_gone_falls_back_to_the_default_with_a_warning() {
    let fake = fake();
    let (_, engine) = new_engine(RATE);
    let (output, warning) =
        AudioOutput::open(fake.backend(), engine, Some("Bluetooth WH-1000")).unwrap();
    assert_eq!(output.info().unwrap().name, "Altavoces");
    let warning = warning.unwrap();
    assert!(warning.contains("Bluetooth WH-1000"), "{warning}");
    assert!(warning.contains("predeterminado"), "{warning}");

    // Sin ninguno que abrir, error y el motor de vuelta.
    let empty = FakeAudio::default();
    let (_, engine) = new_engine(RATE);
    let Err(error) = AudioOutput::open(empty.backend(), engine, None) else {
        panic!("sin dispositivos no abre");
    };
    assert!(error.message.contains("predeterminado"), "{error}");
    assert_eq!(error.engine.sample_rate(), RATE);
}

#[test]
fn switching_keeps_the_song_and_the_bar_with_short_fades() {
    let fake = fake();
    let (controller, engine) = playing();
    let (mut output, _) =
        AudioOutput::open(fake.backend(), engine, Some("Auriculares USB")).unwrap();
    assert_eq!(output.sample_rate(), RATE);

    // La referencia: el mismo motor sin cambiar de dispositivo.
    let (_, mut reference) = playing();
    let reference = offline::render(&mut reference, 4 * RATE as usize, 2);

    let before = fake.pump("Auriculares USB", RATE as usize).unwrap();
    // Entra subiendo desde cero y después suena como la referencia.
    let energy = |s: &[f32]| s.iter().map(|x| x.abs()).sum::<f32>();
    assert!(energy(&before[..2 * FADE]) < 0.7 * energy(&reference[..2 * FADE]));
    let n = 2 * RATE as usize;
    assert_eq!(before[2 * FADE..n], reference[2 * FADE..n]);
    let bar_before = controller.status().bar;

    output.switch(Some("Altavoces")).unwrap();
    assert!(output.info().unwrap().switching);
    assert_eq!(output.info().unwrap().name, "Altavoces");
    // El nuevo no toca el motor hasta que el viejo lo suelta: silencio.
    let waiting = fake.pump("Altavoces", 256).unwrap();
    assert!(waiting.iter().all(|&s| s == 0.0));
    // El viejo baja a cero en un fundido (el primer bloque) y suelta.
    let fading = fake.pump("Auriculares USB", 512).unwrap();
    assert!(fading[..2 * FADE].iter().any(|&s| s != 0.0));
    assert!(fading[2 * FADE..].iter().all(|&s| s == 0.0));
    let events = output.poll();
    assert_eq!(
        events,
        [AudioEvent::Switched {
            name: "Altavoces".into()
        }]
    );
    assert!(!output.info().unwrap().switching);
    // El viejo se cerró.
    assert_eq!(fake.open_streams(), ["Altavoces"]);

    // El motor siguió donde estaba: lo que suena es la referencia justo
    // donde se quedó (el bloque del fundido también avanzó la canción),
    // después del fundido de entrada.
    let after = fake.pump("Altavoces", RATE as usize).unwrap();
    let done = 2 * (RATE as usize + 256);
    assert!(energy(&after[..2 * FADE]) < 0.7 * energy(&reference[done..done + 2 * FADE]));
    assert_eq!(
        after[2 * FADE..],
        reference[done + 2 * FADE..done + 2 * RATE as usize]
    );
    assert!(controller.status().bar >= bar_before);
    assert_eq!(
        controller.status().bar,
        1,
        "dos segundos y un poco a 120 BPM"
    );
}

#[test]
fn a_device_that_does_not_take_the_engine_rate_is_refused_and_the_old_one_keeps_playing() {
    let fake = fake();
    fake.add("Tarjeta vieja", 2, &[44_100], false);
    let (_, engine) = playing();
    let (mut output, _) =
        AudioOutput::open(fake.backend(), engine, Some("Auriculares USB")).unwrap();
    let error = output.switch(Some("Tarjeta vieja")).unwrap_err();
    assert!(error.contains("48000"), "{error}");
    let error = output.switch(Some("No existe")).unwrap_err();
    assert!(error.contains("No existe"), "{error}");
    assert!(output.poll().is_empty());
    assert_eq!(output.info().unwrap().name, "Auriculares USB");
    let out = fake.pump("Auriculares USB", 4096).unwrap();
    assert!(out.iter().any(|&s| s != 0.0), "sigue sonando");
}

#[test]
fn an_unplugged_device_falls_back_to_the_previous_then_the_default_then_null() {
    let fake = fake();
    fake.add("Bluetooth", 2, &[48_000], false);
    let (controller, engine) = playing();
    let (mut output, _) =
        AudioOutput::open(fake.backend(), engine, Some("Auriculares USB")).unwrap();
    fake.pump("Auriculares USB", 4096);
    output.switch(Some("Bluetooth")).unwrap();
    finish_switch(&mut output, &fake, "Auriculares USB");
    fake.pump("Bluetooth", 4096);
    let global = controller.status().global;

    // Se va el Bluetooth: vuelve a los auriculares (el anterior), sin
    // esperar al fundido (el que se fue ya no suena).
    fake.unplug("Bluetooth");
    let events = output.poll();
    assert_eq!(
        events,
        [
            AudioEvent::Lost {
                name: "Bluetooth".into(),
                to: "Auriculares USB".into(),
                null: false
            },
            AudioEvent::Switched {
                name: "Auriculares USB".into()
            }
        ]
    );
    let out = fake.pump("Auriculares USB", 8192).unwrap();
    assert!(out.iter().any(|&s| s != 0.0));
    assert!(controller.status().global >= global, "la canción sigue");

    // Se van los auriculares: no hay anterior, va al predeterminado.
    fake.unplug("Auriculares USB");
    let events = output.poll();
    assert!(matches!(&events[0], AudioEvent::Lost { to, null: false, .. } if to == "Altavoces"));
    // Y si se va también, a la salida nula: no se cuelga.
    fake.unplug("Altavoces");
    let events = output.poll();
    assert!(
        matches!(&events[0], AudioEvent::Lost { null: true, .. }),
        "{events:?}"
    );
    assert!(output.info().unwrap().null);
    let start = controller.status().global;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while controller.status().global == start {
        assert!(
            std::time::Instant::now() < deadline,
            "la salida nula mueve el motor"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    // Vuelve a enchufarse: se puede elegir otra vez.
    fake.plug("Altavoces");
    output.switch(Some("Altavoces")).unwrap();
    let deadline =
        std::time::Instant::now() + 2 * HANDOFF_TIMEOUT + std::time::Duration::from_secs(1);
    loop {
        if !output.poll().is_empty() {
            break;
        }
        assert!(std::time::Instant::now() < deadline);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(output.info().unwrap().name, "Altavoces");
}

#[test]
fn a_dead_old_device_is_let_go_after_the_timeout() {
    let fake = fake();
    let (_, engine) = playing();
    let (mut output, _) = AudioOutput::open(fake.backend(), engine, None).unwrap();
    fake.pump("Altavoces", 1024);
    // (El motor quedó a 44,1 kHz: se vuelve a abrir el mismo dispositivo.)
    output.switch(Some("Altavoces")).unwrap();
    assert!(output.switch(None).is_err(), "uno cada vez");
    // El viejo no vuelve a llamar a su callback: se suelta por tiempo.
    assert!(output.poll().is_empty());
    std::thread::sleep(HANDOFF_TIMEOUT + std::time::Duration::from_millis(20));
    assert_eq!(output.poll().len(), 1);
    assert_eq!(fake.open_streams().len(), 1);
}

#[test]
fn latency_is_what_the_callback_measured() {
    let fake = fake();
    let (_, engine) = playing();
    let (output, _) = AudioOutput::open(fake.backend(), engine, Some("Auriculares USB")).unwrap();
    fake.pump("Auriculares USB", 256);
    // El falso dice 128 frames de latencia y bloques de 256.
    let info = output.info().unwrap();
    assert_eq!(info.latency_frames, 128 + 256);
    assert!((info.latency_ms() - 8.0).abs() < 1e-9);
}

#[test]
fn the_new_device_latency_moves_what_the_highlight_follows() {
    let fake = fake();
    // Un Bluetooth: 200 ms de latencia.
    fake.add("Bluetooth", 2, &[RATE], false);
    fake.set_latency("Bluetooth", RATE as u64 / 5);
    let (controller, engine) = playing();
    let (mut output, _) =
        AudioOutput::open(fake.backend(), engine, Some("Auriculares USB")).unwrap();
    // Cuánto va lo oído por detrás de lo calculado, en frames (al tempo
    // que diga el motor: el de `bpm` entra al compás).
    let behind = || {
        let status = controller.status();
        let frames_per_step = RATE as f64 * 60.0 / status.bpm / 4.0;
        (controller.position().unwrap() - status.heard.unwrap()) * frames_per_step
    };
    // (Bloques enteros de 256 frames: el retardo es el del último.)
    fake.pump("Auriculares USB", 94 * 256);
    let usb = behind();
    assert!((usb - (128.0 + 256.0)).abs() < 1e-6, "{usb}");
    output.switch(Some("Bluetooth")).unwrap();
    finish_switch(&mut output, &fake, "Auriculares USB");
    fake.pump("Bluetooth", 94 * 256);
    // Lo oído va por detrás lo que dice el Bluetooth de más.
    let delay = behind() - usb;
    let expected = (RATE / 5 - 128) as f64;
    assert!((delay - expected).abs() < 1e-6, "{delay} ≠ {expected}");
    let info = output.info().unwrap();
    assert!(
        (info.latency_ms() - (200.0 + 256.0 / 48.0)).abs() < 1e-6,
        "{info:?}"
    );
}
