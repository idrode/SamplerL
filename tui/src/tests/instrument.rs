//! Instrumentos en la interfaz: la tabla de `instrument` en el
//! autocompletado y la ayuda, notas por su nombre en el tracker y las voces
//! que suenan en el panel de efectos.

use super::*;
use std::time::{Duration, Instant};

const SONG_SYNTH: &str = r#"bpm(120)
local piano = sample("break.wav"):instrument({ root = "C3", loop = { 10, 100 }, voices = 4 })
track("lead", piano):melody("c+e+g _ _ _  e . c4 .  . . . .  . . . .")
"#;

#[test]
fn the_tracker_writes_notes_and_the_panel_counts_voices() {
    let (_, mut app, mut engine) = open("instrument_view", Some(SONG_SYNTH));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let view = app.view();
    let track = &view.tracks[0];
    assert_eq!(track.instrument.map(|i| (i.root, i.voices)), Some((48, 4)));
    assert_eq!(track.cell_text(&track.steps[0]), "C3+E3+G3");
    assert_eq!(track.cell_text(&track.steps[6]), "C4");
    assert!(track.held(1) && track.held(3) && !track.held(4) && !track.held(5));

    // El acorde suena: tres voces de cuatro.
    offline::render(&mut engine, STEP, 2);
    app.tick();
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("lead · ♪ C3 3/4"), "{screen}");
    assert!(screen.contains("C3+E3+G3"), "{screen}");
    assert!(screen.contains(" │"), "{screen}");
}

#[test]
fn instrument_options_complete_and_explain_themselves() {
    let (mut app, _engine) = open_at_end("instrument_complete");
    type_text(&mut app, "local i = sample(\"break.wav\"):instrument({ ");
    app.handle(f(4));
    assert_eq!(
        completion_items(&app),
        ["root", "loop", "voices", "adsr", "crossfade"]
    );
    app.handle(esc());
    assert_eq!(
        app.view().help.unwrap().signature,
        "sample:instrument({ ... })"
    );
    type_text(&mut app, "voices = 2 })\ntrack(\"lead\", i):mel");
    assert_eq!(completion_items(&app), ["melody"]);
    app.handle(special(Key::Tab));
    app.handle(ctrl('s'));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    assert_eq!(
        app.view().tracks.last().unwrap().instrument.unwrap().voices,
        2
    );
}

/// Un seno de 220 Hz (A3) y una canción con él como instrumento, raíz A3,
/// bucle sostenido y 50 ms de release; el patrón no toca nada.
fn open_tone(test: &str) -> (App, Engine) {
    let song = "bpm(120)\n\
                local tono = sample(\"tone.wav\"):instrument({ root = \"A3\", loop = { 100, 900 }, adsr = { 2, 0, 1, 50 } })\n\
                track(\"lead\", tono):steps(\". . . .  . . . .  . . . .  . . . .\")\n";
    let path = fixture(test, Some(song));
    let mono: Vec<f32> = (0..RATE as usize)
        .map(|i| 0.4 * (std::f32::consts::TAU * 220.0 * i as f32 / RATE as f32).sin())
        .collect();
    offline::write_wav(path.with_file_name("tone.wav"), &mono, 1, RATE).unwrap();
    let (controller, engine) = new_engine(RATE);
    let app = App::open(&path, controller, RATE).unwrap();
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    (app, engine)
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

/// Frecuencia por los cruces por cero.
fn frequency(x: &[f32]) -> f64 {
    let crossings: Vec<f64> = x
        .windows(2)
        .enumerate()
        .filter(|(_, w)| (w[0] < 0.0) != (w[1] < 0.0))
        .map(|(i, w)| i as f64 + w[0] as f64 / (w[0] - w[1]) as f64)
        .collect();
    let (first, last) = (crossings[0], crossings[crossings.len() - 1]);
    (crossings.len() - 1) as f64 / 2.0 / ((last - first) / RATE as f64)
}

fn peak(x: &[f32]) -> f32 {
    x.iter().fold(0.0, |m, s| m.max(s.abs()))
}

/// En un instrumento, los pads tocan notas desde la raíz (el pad 1 es la
/// raíz, cada uno un semitono más) y suenan mientras se mantienen. Sin
/// avisos de soltar tecla, mientras lleguen las repeticiones del teclado.
#[test]
fn pads_play_notes_while_the_key_repeats() {
    let (mut app, mut engine) = open_tone("instrument_pads");
    app.handle(esc());
    let t0 = Instant::now();
    app.press_pad_at(4, t0);
    assert_eq!(app.view().message, "♪ lead: C#4");
    let note = left(&offline::render(&mut engine, 9600, 2));
    let f = frequency(&note[1000..]);
    let expected = 220.0 * 2f64.powf(4.0 / 12.0);
    assert!((f / expected - 1.0).abs() < 0.002, "{f}");
    // Antes de la primera repetición (retardo del teclado), sigue.
    app.release_expired_pads(t0 + Duration::from_millis(600));
    // Repeticiones: no vuelven a empezar la nota, la alargan.
    app.press_pad_at(4, t0 + Duration::from_millis(650));
    app.press_pad_at(4, t0 + Duration::from_millis(700));
    app.release_expired_pads(t0 + Duration::from_millis(800));
    let still = left(&offline::render(&mut engine, 4800, 2));
    assert!(peak(&still) > 0.35, "{}", peak(&still));
    // Dejan de llegar: la nota se suelta y hace su release (50 ms).
    app.release_expired_pads(t0 + Duration::from_millis(900));
    let tail = left(&offline::render(&mut engine, 4800, 2));
    assert!(peak(&tail[..1000]) > 0.1);
    assert!(tail[2500..].iter().all(|&s| s == 0.0));
    // Volver al editor suelta lo que suene.
    app.press_pad_at(0, t0 + Duration::from_millis(1000));
    app.handle(esc());
    offline::render(&mut engine, 4800, 2);
    assert!(
        left(&offline::render(&mut engine, 1000, 2))
            .iter()
            .all(|&s| s == 0.0)
    );
}

/// Si la terminal avisa al soltar las teclas, la nota dura hasta ese aviso
/// (no por tiempo).
#[test]
fn with_key_releases_the_note_lasts_until_the_key_is_released() {
    let (mut app, mut engine) = open_tone("instrument_release");
    app.set_key_releases(true);
    app.handle(esc());
    let t0 = Instant::now();
    app.handle(key('1'));
    app.release_expired_pads(t0 + Duration::from_secs(10));
    let held = left(&offline::render(&mut engine, 24_000, 2));
    assert!(peak(&held[20_000..]) > 0.35);
    let f = frequency(&held[1000..]);
    assert!((f / 220.0 - 1.0).abs() < 0.002, "la raíz: {f}");
    app.key_release(key('1'));
    offline::render(&mut engine, 2400, 2);
    assert!(
        left(&offline::render(&mut engine, 1000, 2))
            .iter()
            .all(|&s| s == 0.0)
    );
}

/// Las notas MIDI tocan el instrumento seleccionado a su altura real
/// (respecto a la raíz) y su note off dispara el release.
#[test]
fn midi_notes_play_the_instrument_until_their_note_off() {
    use crate::midi::{MidiEvent, TimedEvent};
    let (mut app, mut engine) = open_tone("instrument_midi");
    let (mut tx, rx) = rtrb::RingBuffer::new(16);
    app.set_midi_input(rx);
    let event = |event| TimedEvent { micros: 0, event };
    // E4 (64) sobre A3 (57): 7 semitonos; velocidad 64 → la mitad.
    tx.push(event(MidiEvent::NoteOn {
        channel: 1,
        note: 64,
        velocity: 64,
    }))
    .unwrap();
    app.poll_midi();
    assert_eq!(app.view().message, "♪ lead: E4");
    let note = left(&offline::render(&mut engine, 24_000, 2));
    let f = frequency(&note[1000..]);
    assert!(
        (f / (220.0 * 2f64.powf(7.0 / 12.0)) - 1.0).abs() < 0.002,
        "{f}"
    );
    let level = peak(&note[20_000..]);
    assert!((level - 0.4 * 64.0 / 127.0).abs() < 0.01, "{level}");
    tx.push(event(MidiEvent::NoteOff {
        channel: 1,
        note: 64,
    }))
    .unwrap();
    app.poll_midi();
    offline::render(&mut engine, 2400, 2);
    assert!(
        left(&offline::render(&mut engine, 1000, 2))
            .iter()
            .all(|&s| s == 0.0)
    );
}

/// Grabar un instrumento: cada nota con su tono y lo que dura hasta que se
/// suelta.
#[test]
fn recording_an_instrument_keeps_pitch_and_length() {
    let (mut app, mut engine) = open_tone("instrument_record");
    app.set_key_releases(true);
    let mut now = 0.0;
    let mut go_to = |engine: &mut Engine, steps: f64| {
        let frames = ((steps - now) * STEP as f64).round() as usize;
        offline::render(engine, frames, 2);
        now = steps;
    };
    go_to(&mut engine, 2.0);
    app.tick();
    app.run_command("grabar 1");
    app.editor.move_cursor(ratatui_textarea::CursorMove::Bottom);
    app.editor.move_cursor(ratatui_textarea::CursorMove::End);
    type_text(&mut app, "\ntrack(\"lead\"):");
    app.handle(esc());
    app.handle(esc());
    go_to(&mut engine, 16.0);
    app.handle(key('1'));
    go_to(&mut engine, 19.9);
    app.key_release(key('1'));
    go_to(&mut engine, 24.1);
    app.handle(key('8'));
    go_to(&mut engine, 25.0);
    app.key_release(key('8'));
    go_to(&mut engine, 31.6);
    app.tick();
    assert_eq!(
        app.editor.lines().last().unwrap(),
        "track(\"lead\"):steps(\"1^0 _ _ _  . . . .  1^7 . . .  . . . .\")"
    );
}
