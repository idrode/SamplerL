//! Volumen por pista y limitador suave al final de la mezcla.

use std::sync::Arc;

use engine::notation::parse_steps;
use engine::{
    Batch, LIMIT_THRESHOLD, MAX_TRACKS, Pattern, Sample, Slices, new_engine, offline, soft_limit,
};

const RATE: u32 = 48_000;
/// A 120 BPM: 6000 frames por paso, 96000 por compás.
const STEP: usize = 6000;
const BAR: usize = 16 * STEP;

fn sample_from(mono: &[f32]) -> (Arc<Sample>, Slices) {
    let sample = Arc::new(Sample::from_interleaved(mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    (sample, slices)
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

#[test]
fn soft_limit_is_transparent_below_the_threshold_and_never_exceeds_one() {
    for i in 0..=900 {
        let x = i as f32 / 1000.0;
        assert_eq!(soft_limit(x), x);
        assert_eq!(soft_limit(-x), -x);
    }
    let mut previous = soft_limit(LIMIT_THRESHOLD);
    for i in 1..100_000 {
        let x = LIMIT_THRESHOLD + i as f32 * 0.001;
        let y = soft_limit(x);
        assert!(y <= 1.0 && y >= previous, "x = {x}: {y}");
        assert_eq!(soft_limit(-x), -y);
        previous = y;
    }
    assert!(soft_limit(f32::INFINITY) <= 1.0);
    assert_eq!(soft_limit(f32::NAN), 0.0);
    // Continuo en el umbral.
    assert!((soft_limit(LIMIT_THRESHOLD + 1e-4) - (LIMIT_THRESHOLD + 1e-4)).abs() < 1e-5);
}

#[test]
fn saturated_tracks_never_exceed_one() {
    // Ruido fuerte (con picos de ±0.9) en las 8 pistas a la vez, con
    // volumen de paso y de pista al máximo: la suma llegaría a ~±115.
    let mut x = 0x9e37_79b9u32;
    let noise: Vec<f32> = (0..20_000)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let n = (x as f32 / u32::MAX as f32) * 2.0 - 1.0;
            if i < 10 { 0.9 } else { 0.9 * n }
        })
        .collect();
    let (sample, slices) = sample_from(&noise);
    let steps = parse_steps("1@4 1@4 1@4 1@4  1@4 1@4 1@4 1@4  1@4 1@4 1@4 1@4  1@4 1@4 1@4 1@4");
    let mut batch = Batch::new();
    for t in 0..MAX_TRACKS {
        let pattern = Pattern::from_steps(sample.clone(), &slices, steps.clone().unwrap())
            .unwrap()
            .with_gain(4.0)
            .unwrap();
        batch.set_pattern(t, Some(pattern));
    }
    let (mut controller, mut engine) = new_engine(RATE);
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    // También en mono (la mezcla se promedia antes de salir).
    for channels in [2, 1] {
        let audio = offline::render(&mut engine, 2 * BAR, channels);
        let peak = audio.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak <= 1.0, "{channels} canales: pico {peak}");
        assert!(
            peak > 0.99,
            "{channels} canales: el limitador no debería apagar la señal ({peak})"
        );
    }
}

#[test]
fn track_gain_scales_the_track_and_enters_at_the_bar() {
    let mut mono = vec![0.0f32; 4000];
    mono[..3000].fill(0.5);
    let (sample, slices) = sample_from(&mono);
    let steps = parse_steps("1 . . .  1 . . .  1 . . .  1 . . .").unwrap();
    let pattern = |gain: f32| {
        Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_gain(gain)
            .unwrap()
    };

    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_pattern(pattern(0.5)).unwrap();
    controller.start().unwrap();
    let mut l = left(&offline::render(&mut engine, BAR / 2 + 333, 2));
    // A mitad de compás se pide volumen 0.25: entra en el compás siguiente.
    controller.set_pattern(pattern(0.25)).unwrap();
    l.extend(left(&offline::render(&mut engine, BAR / 2 - 333 + BAR, 2)));

    // Desde el primer golpe ya con el volumen de la pista (estaba en silencio).
    assert_eq!(l[200], 0.25);
    assert_eq!(l[4 * STEP + 1000], 0.25);
    assert_eq!(l[12 * STEP + 1000], 0.25);
    // Compás 1: 0.5 · 0.25 (el cambio llegó con la pista en silencio).
    assert_eq!(l[BAR + 1000], 0.125);

    assert!(
        Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_gain(4.5)
            .is_err()
    );
    assert!(
        Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_gain(-1.0)
            .is_err()
    );
}

#[test]
fn track_gain_change_while_sounding_is_smoothed() {
    // Una nota larga que sigue sonando al cruzar el compás.
    let (sample, slices) = sample_from(&vec![0.5f32; 4 * BAR]);
    let first = parse_steps("1 . . .  . . . .  . . . .  . . . .").unwrap();
    let rest = parse_steps(". . . .  . . . .  . . . .  . . . .").unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller
        .set_pattern(Pattern::from_steps(sample.clone(), &slices, first).unwrap())
        .unwrap();
    controller.start().unwrap();
    let mut l = left(&offline::render(&mut engine, BAR / 2, 2));
    let quieter = Pattern::from_steps(sample.clone(), &slices, rest)
        .unwrap()
        .with_gain(0.25)
        .unwrap();
    controller.set_pattern(quieter).unwrap();
    l.extend(left(&offline::render(&mut engine, BAR, 2)));

    // Antes del compás, a 0.5; justo después baja sin saltos: cada frame
    // cambia poco (suavizado de ~5 ms) y llega a 0.5 · 0.25.
    assert_eq!(l[BAR - 1], 0.5);
    let max_jump = l[BAR - 10..BAR + 2000]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    assert!(max_jump < 0.01, "salto de {max_jump}");
    assert!((l[BAR + 2000] - 0.125).abs() < 1e-4, "{}", l[BAR + 2000]);
}
