//! Time-stretch (WSOLA): la duración sale a la proporción pedida, el tono
//! de un seno no cambia y los golpes de un break caen donde deben.

mod support;

use engine::{Sample, Slices, detect_bpm};
use support::synth_break;

const RATE: u32 = 48_000;

fn sine(hz: f32, seconds: f32) -> Sample {
    let frames = (seconds * RATE as f32) as usize;
    let mono: Vec<f32> = (0..frames)
        .map(|i| 0.5 * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin())
        .collect();
    Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap()
}

/// Frecuencia por los cruces por cero ascendentes (interpolados) en el
/// tramo central, lejos de los bordes.
fn frequency(sample: &Sample) -> f64 {
    let left: Vec<f32> = (0..sample.frames()).map(|i| sample.frame(i).0).collect();
    let (from, to) = (left.len() / 5, left.len() * 4 / 5);
    let crossings: Vec<f64> = (from..to)
        .filter(|&i| left[i - 1] < 0.0 && left[i] >= 0.0)
        .map(|i| (i - 1) as f64 + (-left[i - 1] / (left[i] - left[i - 1])) as f64)
        .collect();
    let periods = (crossings.len() - 1) as f64;
    RATE as f64 * periods / (crossings.last().unwrap() - crossings[0])
}

#[test]
fn a_sine_keeps_its_pitch_at_any_ratio() {
    let original = sine(440.0, 1.0);
    assert!((frequency(&original) - 440.0).abs() < 0.05);
    for ratio in [120.0 / 140.0, 140.0 / 120.0, 0.5, 2.0, 0.8, 1.25] {
        let stretched = original.stretched(ratio);
        let expected = (original.frames() as f64 * ratio).round() as usize;
        assert_eq!(stretched.frames(), expected, "×{ratio}");
        let hz = frequency(&stretched);
        assert!(
            (hz - 440.0).abs() < 440.0 * 0.002,
            "×{ratio}: {hz:.2} Hz en vez de 440"
        );
        // Sin huecos ni picos: el nivel sigue en 0,5.
        let peak = (stretched.frames() / 10..stretched.frames() * 9 / 10)
            .map(|i| stretched.frame(i).0.abs())
            .fold(0.0f32, f32::max);
        assert!((peak - 0.5).abs() < 0.03, "×{ratio}: pico {peak}");
    }
}

#[test]
fn ratio_one_is_an_exact_copy_and_it_is_deterministic() {
    let original = sine(330.0, 0.3);
    let same = original.stretched(1.0);
    assert_eq!(same.frames(), original.frames());
    assert!((0..original.frames()).all(|i| same.frame(i) == original.frame(i)));
    let a = original.stretched(0.7);
    let b = original.stretched(0.7);
    assert!((0..a.frames()).all(|i| a.frame(i) == b.frame(i)));
}

#[test]
fn a_break_at_140_stretched_to_170_bpm_lands_on_the_new_grid() {
    let (audio, hits) = synth_break::synth_break();
    let original = Sample::from_interleaved(&audio, 1, synth_break::RATE, RATE).unwrap();
    let ratio = 140.0 / 170.0;
    let stretched = original.stretched(ratio);
    assert_eq!(
        stretched.frames(),
        (original.frames() as f64 * ratio).round() as usize
    );
    // El tempo detectado del sample estirado es el pedido.
    let guess = detect_bpm(&stretched).expect("tempo");
    assert!((guess.rounded() - 170.0).abs() < 0.5, "{}", guess.bpm);
    // Cada golpe, en su sitio escalado (a menos de 2 ms).
    let slices = Slices::detect(&stretched, hits.len());
    let tolerance = (0.002 * RATE as f64) as i64;
    for (slice, &hit) in slices.iter().zip(&hits) {
        let expected = (hit as f64 * RATE as f64 / synth_break::RATE as f64 * ratio) as i64;
        assert!(
            (slice.start as i64 - expected).abs() <= tolerance,
            "golpe en {} en vez de {expected}",
            slice.start
        );
    }
}

#[test]
fn slices_scale_with_the_sample() {
    let (audio, _) = synth_break::synth_break();
    let original = Sample::from_interleaved(&audio, 1, synth_break::RATE, RATE).unwrap();
    let slices = Slices::detect(&original, 8);
    let stretched = original.stretched(1.25);
    let scaled = slices.scaled(1.25, stretched.frames());
    assert_eq!(scaled.len(), slices.len());
    for (a, b) in slices.iter().zip(scaled.iter()) {
        assert_eq!(b.start, (a.start as f64 * 1.25).round() as usize);
    }
    assert_eq!(scaled.iter().last().unwrap().end, stretched.frames());
}
