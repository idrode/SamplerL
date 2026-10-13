//! Longitud variable de patrón por pista: una pista de n pasos toca la
//! semicorchea global g como su paso g % n, así que pistas de distinta
//! longitud hacen polirritmia. Con 16 pasos, todo es como antes.

use std::sync::Arc;

use engine::{
    Engine, MAX_STEPS, Pattern, STEPS, Sample, SendError, Slices, Step, new_engine, offline,
};

const RATE: u32 = 48_000;
/// A 120 BPM, un paso son 6000 frames exactos.
const STEP: usize = 6000;

/// Un corte de 800 frames a 0.5 seguido de silencio: cada disparo es un
/// arranque desde cero.
fn one_slice() -> (Arc<Sample>, Slices) {
    let mut mono = vec![0.0f32; 3000];
    mono[..800].fill(0.5);
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    (sample, slices)
}

/// Pasos globales (desde 0) en los que empieza un golpe en el canal
/// izquierdo.
fn onsets(audio: &[f32]) -> Vec<usize> {
    let left: Vec<f32> = audio.iter().step_by(2).copied().collect();
    (1..left.len())
        .filter(|&i| left[i - 1] == 0.0 && left[i] != 0.0)
        .map(|i| {
            assert_eq!((i - 1) % STEP, 0, "golpe fuera de paso en el frame {i}");
            (i - 1) / STEP
        })
        .collect()
}

fn steps(text: &str) -> Vec<Option<Step>> {
    text.split_whitespace()
        .map(|t| (t == "x").then(|| Step::new(0)))
        .collect()
}

fn run(patterns: Vec<(usize, Pattern)>, bars: usize) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    for (track, pattern) in patterns {
        controller.set_track_pattern(track, Some(pattern)).unwrap();
    }
    controller.start().unwrap();
    render(&mut engine, bars * STEPS * STEP)
}

fn render(engine: &mut Engine, frames: usize) -> Vec<f32> {
    offline::render(engine, frames, 2)
}

#[test]
fn a_twelve_step_pattern_loops_against_the_bar() {
    let (sample, slices) = one_slice();
    let pattern = Pattern::from_steps(sample, &slices, steps("x . . .  . . x .  . . . .")).unwrap();
    assert_eq!(pattern.length(), 12);
    let audio = run(vec![(0, pattern)], 3);
    // Vueltas de 12 pasos: golpes en 0, 6, 12, 18, 24, 30, 36, 42.
    assert_eq!(onsets(&audio), [0, 6, 12, 18, 24, 30, 36, 42]);
}

#[test]
fn tracks_of_different_length_make_polyrhythms() {
    let (sample, slices) = one_slice();
    let three = Pattern::from_steps(sample.clone(), &slices, steps("x . .")).unwrap();
    let four = Pattern::from_steps(sample, &slices, steps("x . . .")).unwrap();
    let a = run(vec![(0, three)], 3);
    let (sample, slices) = one_slice();
    let b = run(
        vec![(
            1,
            Pattern::from_steps(sample, &slices, steps("x . . .")).unwrap(),
        )],
        3,
    );
    assert_eq!(onsets(&a), (0..48).step_by(3).collect::<Vec<_>>());
    assert_eq!(onsets(&b), (0..48).step_by(4).collect::<Vec<_>>());
    // Juntas, se suman; coinciden cada 12 pasos.
    let (sample, slices) = one_slice();
    let both = run(
        vec![
            (
                0,
                Pattern::from_steps(sample.clone(), &slices, steps("x . .")).unwrap(),
            ),
            (1, four),
        ],
        3,
    );
    let mut expected: Vec<usize> = (0..48).filter(|g| g % 3 == 0 || g % 4 == 0).collect();
    expected.dedup();
    assert_eq!(onsets(&both), expected);
    for (l, (x, y)) in both.iter().zip(a.iter().zip(&b)).step_by(2) {
        assert_eq!(*l, engine::soft_limit(x + y));
    }
}

#[test]
fn a_32_step_pattern_spans_two_bars_and_its_cycle_counts_laps() {
    let (sample, slices) = one_slice();
    let mut long = vec![None; 32];
    long[0] = Some(Step::new(0));
    long[20] = Some(Step::new(0));
    let pattern = Pattern::from_steps(sample.clone(), &slices, &long).unwrap();
    assert_eq!(onsets(&run(vec![(0, pattern)], 4)), [0, 20, 32, 52]);

    // Un ciclo de dos vueltas de 6 pasos: la variante cambia cada vuelta.
    let laps = [steps("x . . . . ."), steps(". . . x . .")];
    let pattern = Pattern::from_bars(sample, &slices, &laps).unwrap();
    assert_eq!(pattern.cycle(), 2);
    assert_eq!(onsets(&run(vec![(0, pattern)], 2)), [0, 9, 12, 21, 24]);
}

#[test]
fn sixteen_steps_as_a_vec_sound_like_the_array() {
    let (sample, slices) = one_slice();
    let text = "x . x x  . . x .  x . . .  . x . x";
    let array: [Option<Step>; STEPS] = steps(text).try_into().unwrap();
    let a = run(
        vec![(
            0,
            Pattern::from_steps(sample.clone(), &slices, array).unwrap(),
        )],
        2,
    );
    let b = run(
        vec![(
            0,
            Pattern::from_steps(sample, &slices, steps(text)).unwrap(),
        )],
        2,
    );
    assert_eq!(a, b);
}

#[test]
fn lengths_out_of_range_are_rejected() {
    let (sample, slices) = one_slice();
    let empty: Vec<Option<Step>> = Vec::new();
    assert!(matches!(
        Pattern::from_steps(sample.clone(), &slices, &empty),
        Err(SendError::InvalidLength(0))
    ));
    let too_long = vec![None; MAX_STEPS + 1];
    assert!(matches!(
        Pattern::from_steps(sample.clone(), &slices, &too_long),
        Err(SendError::InvalidLength(65))
    ));
    assert!(Pattern::from_steps(sample.clone(), &slices, vec![None; MAX_STEPS]).is_ok());
    // Todas las vueltas, con la misma longitud.
    let mixed = [steps("x . ."), steps("x . . .")];
    assert!(matches!(
        Pattern::from_bars(sample, &slices, &mixed),
        Err(SendError::InvalidLength(4))
    ));
}
