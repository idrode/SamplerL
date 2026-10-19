//! Nubes de granos en vivo: en una pista con `grains`/`freeze`, los pads y
//! las notas MIDI abren una nube mientras se mantienen. Con varios cortes,
//! la tecla elige el corte; con uno, el pitch. Sin dispositivos.

use super::*;
use crate::midi::{MidiEvent, TimedEvent};
use std::time::{Duration, Instant};

/// Un seno de 300 Hz seguido de otro de 900 Hz (medio segundo cada uno), y
/// uno de 300 Hz de un segundo. (Con 50 granos por segundo, cada 20 ms: un
/// número entero de periodos, así los granos de un congelado no se pisan.)
fn write_tones(path: &std::path::Path) {
    let tone = |hz: f32, frames: usize, from: usize| -> Vec<f32> {
        (0..frames)
            .map(|i| 0.4 * (std::f32::consts::TAU * hz * (i + from) as f32 / RATE as f32).sin())
            .collect()
    };
    let mut two = tone(300.0, RATE as usize / 2, 0);
    two.extend(tone(900.0, RATE as usize / 2, RATE as usize / 2));
    offline::write_wav(path.with_file_name("dos.wav"), &two, 1, RATE).unwrap();
    offline::write_wav(
        path.with_file_name("uno.wav"),
        &tone(300.0, RATE as usize, 0),
        1,
        RATE,
    )
    .unwrap();
}

const SONG: &str = r#"bpm(120)
local dos = sample("dos.wav"):slice_grid(2)
dos:name(2, "agudo")
track("nube", dos):steps(". . . .  . . . .  . . . .  . . . ."):grains({ size = 40, density = 50, speed = 0 })
local uno = sample("uno.wav"):slice(1)
track("hielo", uno):steps(". . . .  . . . .  . . . .  . . . ."):freeze(0.5, { size = 40, density = 50 })
local lisa = sample("dos.wav"):slice_grid(2)
track("lisa", lisa):steps(". . . .  . . . .  . . . .  . . . .")
"#;

/// La canción abierta, con la pista `selected` elegida y el modo tocar.
fn open_clouds(test: &str, selected: usize) -> (App, Engine) {
    let path = fixture(test, Some(SONG));
    write_tones(&path);
    let (controller, engine) = new_engine(RATE);
    let mut app = App::open(&path, controller, RATE).unwrap();
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    for _ in 0..selected {
        app.handle(ctrl('o'));
    }
    app.handle(esc());
    assert!(app.view().playing_mode);
    (app, engine)
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

fn frequency(x: &[f32]) -> f64 {
    let crossings: Vec<f64> = (1..x.len())
        .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
        .map(|i| (i - 1) as f64 + (-x[i - 1] / (x[i] - x[i - 1])) as f64)
        .collect();
    RATE as f64 * (crossings.len() - 1) as f64 / (crossings[crossings.len() - 1] - crossings[0])
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, s| m.max(s.abs()))
}

fn note(note: u8, velocity: u8) -> TimedEvent {
    TimedEvent {
        micros: 0,
        event: MidiEvent::NoteOn {
            channel: 1,
            note,
            velocity,
        },
    }
}

fn note_off(note: u8) -> TimedEvent {
    TimedEvent {
        micros: 0,
        event: MidiEvent::NoteOff { channel: 1, note },
    }
}

/// Medio segundo de lo que suena, sin el arranque.
fn tone_of(engine: &mut Engine) -> f64 {
    let out = left(&offline::render(engine, RATE as usize / 2, 2));
    frequency(&out[RATE as usize / 10..])
}

#[test]
fn pads_open_the_cloud_of_a_slice_while_the_key_is_held() {
    let (mut app, mut engine) = open_clouds("grains_live_pads", 0);
    // El pad 2 es el corte 2 (el de 900 Hz); en el motor, parado, ya suena.
    app.handle(key('2'));
    assert_eq!(app.view().message, "⁘ nube: corte 2 (agudo)");
    assert_eq!(app.view().selected_slice, Some(1));
    let hz = tone_of(&mut engine);
    assert!((hz / 900.0 - 1.0).abs() < 0.01, "{hz}");
    // Con aviso de soltar (protocolo de teclado de kitty): la suelta.
    app.set_key_releases(true);
    app.key_release(key('2'));
    let tail = left(&offline::render(&mut engine, RATE as usize / 2, 2));
    assert!(tail[RATE as usize / 10..].iter().all(|s| *s == 0.0));
    // El corte 1.
    app.handle(key('1'));
    let hz = tone_of(&mut engine);
    assert!((hz / 300.0 - 1.0).abs() < 0.01, "{hz}");
    app.key_release(key('1'));
    // Un corte que no existe, lo dice y no suena.
    offline::render(&mut engine, RATE as usize / 2, 2);
    app.handle(key('6'));
    assert_eq!(app.view().message, "nube no tiene corte 6 (tiene 2)");
    let silence = left(&offline::render(&mut engine, 4800, 2));
    assert!(silence.iter().all(|s| *s == 0.0));
}

#[test]
fn without_release_events_the_cloud_lasts_while_the_repeats_arrive() {
    let (mut app, mut engine) = open_clouds("grains_live_repeats", 0);
    let t0 = Instant::now();
    app.press_pad_at(0, t0);
    let hz = tone_of(&mut engine);
    assert!((hz / 300.0 - 1.0).abs() < 0.01, "{hz}");
    // Antes de la primera repetición, y con repeticiones: sigue, y no
    // abre otra nube (la misma, alargada).
    app.release_expired_pads(t0 + Duration::from_millis(600));
    app.press_pad_at(0, t0 + Duration::from_millis(650));
    app.press_pad_at(0, t0 + Duration::from_millis(700));
    app.release_expired_pads(t0 + Duration::from_millis(800));
    let still = left(&offline::render(&mut engine, 4800, 2));
    assert!(peak(&still) > 0.3, "{}", peak(&still));
    // Dejan de llegar: se suelta.
    app.release_expired_pads(t0 + Duration::from_millis(900));
    let tail = left(&offline::render(&mut engine, RATE as usize / 2, 2));
    assert!(tail[RATE as usize / 10..].iter().all(|s| *s == 0.0));
    // Salir del modo tocar suelta lo que suene.
    app.press_pad_at(1, t0 + Duration::from_millis(1000));
    app.handle(esc());
    offline::render(&mut engine, 4800, 2);
    let after = left(&offline::render(&mut engine, RATE as usize / 2, 2));
    assert!(after[RATE as usize / 10..].iter().all(|s| *s == 0.0));
}

#[test]
fn with_a_single_slice_the_keys_are_semitones() {
    let (mut app, mut engine) = open_clouds("grains_live_pitch", 1);
    app.set_key_releases(true);
    for (key_char, semitones) in [('1', 0), ('e', 12)] {
        app.handle(key(key_char));
        assert_eq!(app.view().message, format!("⁘ hielo: {semitones:+} st"));
        let hz = tone_of(&mut engine);
        let expected = 300.0 * 2f64.powf(semitones as f64 / 12.0);
        assert!(
            (hz / expected - 1.0).abs() < 0.01,
            "{semitones}: {hz} frente a {expected}"
        );
        app.key_release(key(key_char));
        offline::render(&mut engine, RATE as usize / 2, 2);
    }
}

#[test]
fn midi_notes_open_clouds_with_their_velocity() {
    let (mut app, mut engine) = open_clouds("grains_live_midi", 0);
    // La nota 37 (base 36) es el corte 2; la velocidad es el volumen.
    app.midi_event(note(37, 127));
    let hz = tone_of(&mut engine);
    assert!((hz / 900.0 - 1.0).abs() < 0.01, "{hz}");
    let full = peak(&left(&offline::render(&mut engine, 4800, 2)));
    app.midi_event(note_off(37));
    offline::render(&mut engine, RATE as usize / 2, 2);
    app.midi_event(note(37, 64));
    offline::render(&mut engine, 4800, 2);
    let half = peak(&left(&offline::render(&mut engine, 4800, 2)));
    assert!((half / full - 64.0 / 127.0).abs() < 0.03, "{half} / {full}");
    app.midi_event(note_off(37));
    let tail = left(&offline::render(&mut engine, RATE as usize / 2, 2));
    assert!(tail[RATE as usize / 10..].iter().all(|s| *s == 0.0));
    // Un corte que no existe y por debajo de la base, nada.
    app.midi_event(note(45, 100));
    assert!(app.view().message.contains("no tiene corte 10"));
    app.midi_event(note(30, 100));
    assert!(app.view().message.contains("no tiene corte -5"));
    assert!(
        left(&offline::render(&mut engine, 4800, 2))
            .iter()
            .all(|s| *s == 0.0)
    );

    // Con un solo corte, la nota es el pitch sobre la nota base.
    app.handle(esc());
    app.handle(ctrl('o'));
    app.handle(esc());
    for (n, semitones) in [(36u8, 0i32), (48, 12), (24, -12)] {
        app.midi_event(note(n, 127));
        let hz = tone_of(&mut engine);
        let expected = 300.0 * 2f64.powf(semitones as f64 / 12.0);
        assert!((hz / expected - 1.0).abs() < 0.01, "{n}: {hz}");
        app.midi_event(note_off(n));
        offline::render(&mut engine, RATE as usize / 2, 2);
    }
}

#[test]
fn a_track_without_grains_keeps_playing_its_slices() {
    // La pista «lisa» no tiene granos: los pads, como siempre.
    let (mut app, mut engine) = open_clouds("grains_live_plain", 2);
    app.handle(key('2'));
    assert_eq!(app.view().message, "▶ lisa: corte 2");
    let out = left(&offline::render(&mut engine, 9600, 2));
    assert!(peak(&out) > 0.3);
}

#[test]
fn a_new_script_sets_the_cloud_at_the_bar_while_the_key_is_held() {
    let (mut app, mut engine) = open_clouds("grains_live_reload", 0);
    // Otra densidad (12 en vez de 50 por segundo: aparecen huecos entre
    // granos): se escribe, y se carga con la tecla apretada.
    app.handle(esc());
    let song = SONG.replace("size = 40, density = 50", "size = 40, density = 12");
    app.editor.select_all();
    app.editor.cut();
    app.editor.insert_str(song);
    app.handle(esc());
    app.handle(key('1'));
    let first = left(&offline::render(&mut engine, RATE as usize / 4, 2));
    app.handle(ctrl('l'));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    // Hasta el final del compás (2 s), la de antes; luego, la nueva.
    let rest = left(&offline::render(&mut engine, 2 * RATE as usize, 2));
    // El tramo de silencio más largo: los huecos entre granos.
    let gap = |x: &[f32]| {
        x.iter()
            .fold((0usize, 0usize), |(run, best), s| {
                let run = if s.abs() < 1e-6 { run + 1 } else { 0 };
                (run, best.max(run))
            })
            .1
    };
    let bar = 2 * RATE as usize - first.len();
    assert!(
        gap(&first[1000..]) < 50,
        "50 granos por segundo: sin huecos"
    );
    assert!(gap(&rest[..bar - 100]) < 50, "hasta el compás, la de antes");
    assert!(
        gap(&rest[bar + 100..]) > 1000,
        "luego, la nueva: {}",
        gap(&rest[bar + 100..])
    );
}

#[test]
fn recording_a_cloud_writes_the_slice_and_how_long_it_was_held() {
    let (mut app, mut engine) = open_clouds("grains_live_record", 0);
    app.handle(esc());
    let mut now = 0.0;
    let go_to = |engine: &mut Engine, now: &mut f64, steps: f64| {
        let frames = ((steps - *now) * STEP as f64).round() as usize;
        offline::render(engine, frames, 2);
        *now = steps;
    };
    go_to(&mut engine, &mut now, 2.3);
    app.tick();
    command(&mut app, "grabar 1");
    app.editor.move_cursor(ratatui_textarea::CursorMove::Bottom);
    app.editor.move_cursor(ratatui_textarea::CursorMove::End);
    type_text(&mut app, "\ntrack(\"nube\"):");
    app.handle(esc());
    app.handle(esc());
    assert!(app.view().playing_mode);
    app.set_key_releases(true);
    go_to(&mut engine, &mut now, 15.8);
    app.handle(key('1'));
    go_to(&mut engine, &mut now, 18.4);
    app.key_release(key('1'));
    go_to(&mut engine, &mut now, 24.0);
    app.handle(key('2'));
    go_to(&mut engine, &mut now, 27.6);
    app.key_release(key('2'));
    go_to(&mut engine, &mut now, 31.6);
    app.tick();
    assert!(app.view().recording.is_none());
    assert_eq!(
        app.editor.lines().last().unwrap(),
        "track(\"nube\"):steps(\"1 _ . .  . . . .  agudo _ _ _  . . . .\")"
    );
}
