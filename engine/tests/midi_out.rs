//! Salida MIDI de una pista: cada golpe es una nota con su frame exacto, y
//! se apaga al acabar su paso.

use std::sync::Arc;

use engine::{MidiNote, Pattern, STEPS, Sample, Slices, Step, new_engine, offline};

const RATE: u32 = 48_000;
const STEP: u64 = 6000;

fn pattern(steps: Vec<Option<Step>>, midi: Option<(u8, u8)>) -> Pattern {
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_midi(midi)
        .unwrap()
}

fn notes(p: Pattern, frames: u64) -> Vec<MidiNote> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_pattern(p).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames as usize, 2);
    controller.midi_events().collect()
}

fn on(frame: u64, note: u8, velocity: u8) -> MidiNote {
    MidiNote {
        frame,
        channel: 10,
        note,
        velocity,
    }
}

#[test]
fn each_hit_is_a_note_for_its_step() {
    let mut steps = vec![None; STEPS];
    steps[0] = Some(Step::new(0));
    steps[4] = Some(Step::new(1).with_gain(0.5));
    steps[8] = Some(Step::new(0).with_retrig(2));
    let got = notes(pattern(steps, Some((10, 36))), 16 * STEP);
    assert_eq!(
        got,
        [
            on(0, 36, 100),
            on(STEP, 36, 0),
            on(4 * STEP, 37, 50),
            on(5 * STEP, 37, 0),
            on(8 * STEP, 36, 100),
            on(8 * STEP + STEP / 2, 36, 0),
            on(8 * STEP + STEP / 2, 36, 100),
            on(9 * STEP, 36, 0),
        ]
    );
    assert_eq!(got[0].bytes(), [0x99, 36, 100]);
    assert_eq!(got[1].bytes(), [0x89, 36, 0]);
}

#[test]
fn muted_tracks_send_nothing_and_stop_turns_notes_off() {
    let all = vec![Some(Step::new(0)); STEPS];
    let muted = pattern(all.clone(), Some((1, 60))).with_mute(true);
    assert!(notes(muted, 4 * STEP).is_empty());
    assert!(notes(pattern(all.clone(), None), 4 * STEP).is_empty());

    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_pattern(pattern(all, Some((1, 60)))).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, 1000, 2);
    controller.stop().unwrap();
    offline::render(&mut engine, 1000, 2);
    let got: Vec<MidiNote> = controller.midi_events().collect();
    assert_eq!(got.len(), 2);
    assert_eq!((got[1].frame, got[1].velocity), (1000, 0));
    assert!(
        pattern(vec![None; STEPS], None)
            .with_midi(Some((17, 36)))
            .is_err()
    );
}

fn instrument_pattern(steps: Vec<Option<Step>>, voices: u8, midi: Option<(u8, u8)>) -> Pattern {
    let sample = Arc::new(Sample::from_interleaved(&vec![0.3; 48_000], 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    let mut instrument = engine::Instrument::new(voices);
    instrument.adsr.release_ms = 500.0;
    Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_instrument(Some(instrument))
        .unwrap()
        .with_midi(midi)
        .unwrap()
}

/// Las notas de un instrumento salen a su altura (raíz + semitonos), una
/// por nota del acorde, con su note off al acabar su paso y sus `_` (no
/// el release, que es del sample), al frame exacto.
#[test]
fn instrument_notes_go_out_with_their_pitch_and_length() {
    let mut steps = vec![None; STEPS];
    steps[0] = Some(Step::new(0).with_hold(3));
    steps[4] = Some(Step::new(0).with_chord(&[4.0, 7.0]).with_gain(0.5));
    steps[6] = Some(Step::new(0).with_pitch(-12.0).with_retrig(2));
    let got = notes(instrument_pattern(steps, 8, Some((10, 48))), 16 * STEP);
    assert_eq!(
        got,
        [
            on(0, 48, 100),
            on(4 * STEP, 48, 0),
            on(4 * STEP, 48, 50),
            on(4 * STEP, 52, 50),
            on(4 * STEP, 55, 50),
            on(5 * STEP, 48, 0),
            on(5 * STEP, 52, 0),
            on(5 * STEP, 55, 0),
            on(6 * STEP, 36, 100),
            on(6 * STEP + STEP / 2, 36, 0),
            on(6 * STEP + STEP / 2, 36, 100),
            on(7 * STEP, 36, 0),
        ]
    );
}

/// Una nota robada (sin voces libres) apaga su nota MIDI en ese frame; en
/// mute no sale nada, y parar apaga lo que quede.
#[test]
fn stolen_muted_and_stopped_instrument_notes() {
    let mut steps = vec![None; STEPS];
    steps[0] = Some(Step::new(0).with_hold(7));
    steps[2] = Some(Step::new(0).with_pitch(2.0).with_hold(7));
    let got = notes(
        instrument_pattern(steps.clone(), 1, Some((10, 60))),
        12 * STEP,
    );
    assert_eq!(
        got,
        [
            on(0, 60, 100),
            on(2 * STEP, 60, 0),
            on(2 * STEP, 62, 100),
            on(10 * STEP, 62, 0),
        ]
    );
    let muted = instrument_pattern(steps.clone(), 4, Some((10, 60))).with_mute(true);
    assert!(notes(muted, 12 * STEP).is_empty());
    // Sin MIDI, nada (como siempre).
    assert!(notes(instrument_pattern(steps.clone(), 4, None), 12 * STEP).is_empty());

    let (mut controller, mut engine) = new_engine(RATE);
    controller
        .set_pattern(instrument_pattern(steps, 4, Some((10, 60))))
        .unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, 3 * STEP as usize, 2);
    controller.stop().unwrap();
    offline::render(&mut engine, 100, 2);
    let got: Vec<MidiNote> = controller.midi_events().collect();
    assert_eq!(
        got,
        [
            on(0, 60, 100),
            on(2 * STEP, 62, 100),
            on(3 * STEP, 60, 0),
            on(3 * STEP, 62, 0),
        ]
    );
}
