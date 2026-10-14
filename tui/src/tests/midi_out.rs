//! Las notas de una pista con `:midi()` llegan al destino (simulado).

use super::*;
use crate::midi::Recorder;

#[test]
fn track_notes_reach_the_midi_output() {
    let song = "bpm(120)\nlocal amen = sample(\"break.wav\"):slice(2)\n\
                track(\"bajo\", amen):steps(\"1 . 2 .  . . . .  . . . .  . . . .\"):midi(2, 48)\n";
    let (_, mut app, mut engine) = open("midi_out", Some(song));
    let sent = Recorder::default();
    app.set_midi_output(Box::new(sent.clone()));
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("bajo · MIDI 2/48"), "{screen}");
    offline::render(&mut engine, 4 * STEP + 10, 2);
    app.tick();
    assert_eq!(
        *sent.0.borrow(),
        [
            vec![0x91, 48, 100],
            vec![0x81, 48, 0],
            vec![0x91, 49, 100],
            vec![0x81, 49, 0],
        ]
    );
}

#[test]
fn instrument_notes_reach_the_midi_output_at_their_pitch() {
    let song = "bpm(120)\n\
                local piano = sample(\"break.wav\"):instrument({ root = \"C3\" })\n\
                track(\"lead\", piano):melody(\"c+g _ e .  . . . .  . . . .  . . . .\"):midi(3)\n";
    let (_, mut app, mut engine) = open("midi_out_instrument", Some(song));
    let sent = Recorder::default();
    app.set_midi_output(Box::new(sent.clone()));
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("lead · MIDI 3 · ♪ C3"), "{screen}");
    offline::render(&mut engine, 4 * STEP + 10, 2);
    app.tick();
    assert_eq!(
        *sent.0.borrow(),
        [
            vec![0x92, 48, 100],
            vec![0x92, 55, 100],
            vec![0x82, 48, 0],
            vec![0x82, 55, 0],
            vec![0x92, 52, 100],
            vec![0x82, 52, 0],
        ]
    );
}
