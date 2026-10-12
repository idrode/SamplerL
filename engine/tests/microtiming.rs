//! Retraso de golpes (base de swing y humanize): cada golpe retrasado
//! dispara en `frame del paso + round(retraso · frames del paso)`, también
//! sus subdisparos de retrigger, y nunca después del paso siguiente.

use std::sync::Arc;

use engine::{Engine, MAX_DELAY, Pattern, STEPS, Sample, SendError, Slices, Step, new_engine};

const RATE: u32 = 48_000;
const SETTLE: usize = (RATE / 250) as usize;

fn sample() -> (Arc<Sample>, Slices) {
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    (sample, slices)
}

/// round(k · RATE · 60 / (bpm · 4)), con enteros.
fn step_offset(k: u64, bpm: u64) -> u64 {
    let num = k * RATE as u64 * 15;
    (2 * num + bpm) / (2 * bpm)
}

/// Frames donde arranca un golpe (el anterior es cero y este no).
fn onsets(audio: &[f32]) -> Vec<u64> {
    let left: Vec<f32> = audio.iter().step_by(2).copied().collect();
    (1..left.len() - SETTLE)
        .filter(|&i| left[i - 1] == 0.0 && left[i] != 0.0)
        .map(|i| (i - 1) as u64)
        .collect()
}

fn render(engine: &mut Engine, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    for block in out.chunks_mut(333 * 2) {
        engine.process(block, 2);
    }
    out
}

fn play(bpm: f64, steps: [Option<Step>; STEPS], frames: usize) -> Vec<u64> {
    let (sample, slices) = sample();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(bpm).unwrap();
    controller
        .set_pattern(Pattern::from_steps(sample, &slices, steps).unwrap())
        .unwrap();
    controller.start().unwrap();
    onsets(&render(&mut engine, frames))
}

#[test]
fn delayed_steps_fire_on_the_exact_frame() {
    for bpm in [120u64, 170] {
        let mut steps = [None; STEPS];
        steps[0] = Some(Step::new(0));
        steps[1] = Some(Step::new(1).with_delay(0.32));
        steps[5] = Some(Step::new(0).with_delay(0.5));
        steps[15] = Some(Step::new(1).with_delay(0.9));
        // (El compás siguiente empieza con el golpe del paso 15 aún sonando:
        // no se ve como arranque y no cuenta.)
        let got = play(bpm as f64, steps, step_offset(16, bpm) as usize + 2000);
        let at = |k: u64, delay: f64| {
            let len = step_offset(k + 1, bpm) - step_offset(k, bpm);
            step_offset(k, bpm) + (delay * len as f64).round() as u64
        };
        let expected = vec![at(0, 0.0), at(1, 0.32), at(5, 0.5), at(15, 0.9)];
        assert_eq!(got, expected, "{bpm} BPM");
    }
    // 120 BPM: 6000 frames por paso; 0,32 de paso son 1920 frames.
    let mut steps = [None; STEPS];
    steps[1] = Some(Step::new(0).with_delay(0.32));
    assert_eq!(play(120.0, steps, 12_000)[0], 6000 + 1920);
}

#[test]
fn retrigs_are_delayed_too_and_survive_an_empty_next_step() {
    // Paso 2 retrasado medio paso con retrigger 4: subdisparos cada 1500
    // frames desde 12000 + 3000; los dos últimos caen ya en el paso 3
    // (vacío) y suenan igual.
    let mut steps = [None; STEPS];
    steps[2] = Some(Step::new(0).with_retrig(4).with_delay(0.5));
    let got = play(120.0, steps, 30_000);
    assert_eq!(got, vec![15_000, 16_500, 18_000, 19_500]);
    // Si el paso siguiente tiene golpe, corta los pendientes, como un tracker.
    steps[3] = Some(Step::new(1));
    let got = play(120.0, steps, 30_000);
    assert_eq!(got, vec![15_000, 16_500, 18_000]);
}

#[test]
fn delays_are_validated() {
    let (sample, slices) = sample();
    let mut steps = [None; STEPS];
    steps[3] = Some(Step::new(0).with_delay(MAX_DELAY + 0.01));
    assert!(matches!(
        Pattern::from_steps(sample.clone(), &slices, steps),
        Err(SendError::InvalidStep { step: 3, .. })
    ));
    steps[3] = Some(Step::new(0).with_delay(-0.1));
    assert!(Pattern::from_steps(sample, &slices, steps).is_err());
}
