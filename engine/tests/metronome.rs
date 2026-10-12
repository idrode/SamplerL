//! Metrónomo: un clic en cada pulso, al frame exacto del paso, con acento
//! (más agudo y más fuerte) en el primero del compás.

use engine::metronome::{ACCENT_GAIN, ACCENT_HZ, BEAT_GAIN, BEAT_HZ};
use engine::{new_engine, offline};

const RATE: u32 = 48_000;
/// 120 BPM: un pulso cada 24000 frames.
const BEAT: usize = 24_000;

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

/// Frames donde empieza cada clic (tras un silencio).
fn onsets(left: &[f32]) -> Vec<usize> {
    (1..left.len())
        .filter(|&i| left[i] != 0.0 && left[i - 100.min(i)..i].iter().all(|s| *s == 0.0))
        .map(|i| i - 1)
        .collect()
}

/// Frecuencia por los cruces por cero de los 10 ms siguientes a `at`.
fn frequency(left: &[f32], at: usize) -> f32 {
    let window = &left[at + 1..at + 481];
    let crossings = window
        .windows(2)
        .filter(|w| (w[0] <= 0.0) != (w[1] <= 0.0))
        .count();
    crossings as f32 / 2.0 / 0.01
}

fn peak(left: &[f32], at: usize) -> f32 {
    left[at..at + 1440]
        .iter()
        .fold(0.0f32, |m, s| m.max(s.abs()))
}

#[test]
fn clicks_fall_on_every_beat_with_an_accent_on_the_first() {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_metronome(true).unwrap();
    controller.start().unwrap();
    let audio = left(&offline::render(&mut engine, 2 * 4 * BEAT, 2));
    let clicks = onsets(&audio);
    assert_eq!(clicks, (0..8).map(|k| k * BEAT).collect::<Vec<_>>());
    for (k, &at) in clicks.iter().enumerate() {
        let accent = k % 4 == 0;
        let (hz, gain) = if accent {
            (ACCENT_HZ, ACCENT_GAIN)
        } else {
            (BEAT_HZ, BEAT_GAIN)
        };
        assert!(
            (frequency(&audio, at) - hz).abs() < 60.0,
            "clic {k}: {}",
            frequency(&audio, at)
        );
        assert!(
            (peak(&audio, at) - gain).abs() < 0.03,
            "clic {k}: {}",
            peak(&audio, at)
        );
        // Dura 30 ms: luego, silencio hasta el siguiente.
        assert!(audio[at + 1500..at + BEAT - 10].iter().all(|s| *s == 0.0));
    }
}

#[test]
fn the_metronome_is_off_by_default_and_turns_off() {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.start().unwrap();
    assert!(
        offline::render(&mut engine, 4 * BEAT, 2)
            .iter()
            .all(|s| *s == 0.0)
    );
    controller.set_metronome(true).unwrap();
    assert!(
        offline::render(&mut engine, 4 * BEAT, 2)
            .iter()
            .any(|s| *s != 0.0)
    );
    controller.set_metronome(false).unwrap();
    assert!(
        offline::render(&mut engine, 4 * BEAT, 2)
            .iter()
            .all(|s| *s == 0.0)
    );
}
