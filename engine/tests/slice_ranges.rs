//! Cortes independientes (`Slices::from_ranges`): cada corte con su inicio
//! y su final, con huecos y solapes; los del detector siguen igual; cada
//! golpe suena justo su rango; y cambiar los cortes mientras suena uno lo
//! deja acabar sin clics.

#![allow(clippy::single_range_in_vec_init, clippy::reversed_empty_ranges)]

use std::sync::Arc;

use engine::notation::parse_steps;
use engine::{Batch, Pattern, RangeError, Sample, Slices, new_engine, offline};

const RATE: u32 = 48_000;
/// A 120 BPM: 6000 frames por paso, 96000 por compás.
const STEP: usize = 6000;
const BAR: usize = 16 * STEP;
const LEVEL: f32 = 0.5;

/// Una rampa: el frame `i` vale `i / frames` (así se ve de dónde se lee).
fn ramp(frames: usize) -> Arc<Sample> {
    let mono: Vec<f32> = (0..frames).map(|i| i as f32 / frames as f32).collect();
    Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap())
}

fn constant(frames: usize) -> Arc<Sample> {
    Arc::new(Sample::from_interleaved(&vec![LEVEL; frames], 1, RATE, RATE).unwrap())
}

fn pattern(sample: &Arc<Sample>, slices: &Slices, text: &str) -> Pattern {
    Pattern::from_steps(sample.clone(), slices, parse_steps(text).unwrap())
        .unwrap()
        .with_raw()
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

#[test]
fn ranges_are_checked_one_by_one() {
    let ok = Slices::from_ranges(vec![0..100, 50..300, 400..500], 1000, 10).unwrap();
    assert_eq!(ok.len(), 3);
    assert_eq!(ok.get(1), Some(50..300));
    assert!(!ok.is_contiguous(1000));
    assert_eq!(
        Slices::from_ranges(vec![], 1000, 10),
        Err((0, RangeError::Empty))
    );
    assert_eq!(
        Slices::from_ranges(vec![0..100, 300..200], 1000, 10),
        Err((1, RangeError::Reversed))
    );
    assert_eq!(
        Slices::from_ranges(vec![0..100, 100..100], 1000, 10),
        Err((1, RangeError::Reversed))
    );
    assert_eq!(
        Slices::from_ranges(vec![900..1001], 1000, 10),
        Err((0, RangeError::OutOfSample))
    );
    assert_eq!(
        Slices::from_ranges(vec![0..5], 1000, 10),
        Err((0, RangeError::TooShort))
    );
}

#[test]
fn detected_and_grid_slices_are_contiguous() {
    let sample = ramp(6000);
    assert!(Slices::grid(&sample, 4).is_contiguous(6000));
    let explicit = Slices::from_ranges(vec![0..3000, 3000..6000], 6000, 1).unwrap();
    assert!(explicit.is_contiguous(6000));
    assert_eq!(explicit, Slices::grid(&sample, 2));
}

#[test]
fn editing_one_slice_leaves_its_neighbours_alone() {
    let slices = Slices::from_ranges(vec![0..1000, 1000..2000, 2000..3000], 3000, 1).unwrap();
    let edited = slices.with_range(0, 0..600).unwrap();
    assert_eq!(
        edited.iter().collect::<Vec<_>>(),
        [0..600, 1000..2000, 2000..3000]
    );
    let edited = slices.with_range(1, 800..2500).unwrap();
    assert_eq!(
        edited.iter().collect::<Vec<_>>(),
        [0..1000, 800..2500, 2000..3000]
    );
    assert_eq!(slices.with_range(3, 0..1), None);
}

#[test]
fn zero_crossings_move_only_the_starts_of_independent_slices() {
    // Un seno de 100 Hz: cruces cada 240 frames.
    let mono: Vec<f32> = (0..9600)
        .map(|i| (i as f32 / 480.0 * std::f32::consts::TAU).sin())
        .collect();
    let sample = Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap();
    let slices = Slices::from_ranges(vec![250..1000, 700..3000], 9600, 48).unwrap();
    let snapped = slices.snapped_starts(&sample, 48, 48);
    assert_eq!(snapped.iter().collect::<Vec<_>>(), [240..1000, 720..3000]);
}

#[test]
fn each_hit_plays_its_own_range_with_gaps_and_overlaps() {
    let sample = ramp(BAR);
    // El 1 y el 2 se solapan; entre el 2 y el 3, un hueco.
    let slices = Slices::from_ranges(vec![0..4000, 3000..5000, 9000..9500], BAR, 1).unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch.set_pattern(
        0,
        Some(pattern(
            &sample,
            &slices,
            "1 . 2 .  3 . . .  . . . .  . . . .",
        )),
    );
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let l = left(&offline::render(&mut engine, BAR, 2));
    let at = |frame: usize| frame as f32 / BAR as f32;
    // Paso 0: el corte 1 entero, y nada después.
    for i in [0, 100, 3999] {
        assert!((l[i] - at(i)).abs() < 1e-6, "{i}");
    }
    assert!(l[4000..2 * STEP].iter().all(|&s| s == 0.0));
    // Paso 2: el 2 desde 3000 (dentro del 1) hasta 5000.
    for k in [0, 1000, 1999] {
        assert!((l[2 * STEP + k] - at(3000 + k)).abs() < 1e-6, "{k}");
    }
    assert!(l[2 * STEP + 2000..4 * STEP].iter().all(|&s| s == 0.0));
    // Paso 4: el 3 desde 9000, pasado el hueco.
    assert!((l[4 * STEP] - at(9000)).abs() < 1e-6);
    assert!((l[4 * STEP + 499] - at(9499)).abs() < 1e-6);
    assert!(l[4 * STEP + 500..].iter().all(|&s| s == 0.0));
}

#[test]
fn new_slices_while_one_sounds_let_it_end_without_a_click() {
    let sample = constant(2 * BAR);
    let long = Slices::from_ranges(vec![0..BAR + STEP], 2 * BAR, 1).unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    // Un corte que dura más que el compás, solo al principio.
    batch.set_pattern(
        0,
        Some(
            Pattern::from_steps(
                sample.clone(),
                &long,
                parse_steps("1 . . .  . . . .  . . . .  . . . .").unwrap(),
            )
            .unwrap(),
        ),
    );
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let mut audio = offline::render(&mut engine, BAR / 2, 2);
    // Cortes nuevos (otros límites) a mitad del compás: entran en el
    // siguiente, cuyo primer paso no dispara nada.
    let other = Slices::from_ranges(vec![100..200, 50..150], 2 * BAR, 1).unwrap();
    let mut batch = Batch::new();
    batch.set_pattern(
        0,
        Some(
            Pattern::from_steps(
                sample.clone(),
                &other,
                parse_steps(". . . .  . . . .  . . . .  . . . .").unwrap(),
            )
            .unwrap(),
        ),
    );
    controller.apply(batch).unwrap();
    audio.extend(offline::render(&mut engine, BAR + BAR / 2, 2));
    let l = left(&audio);
    // El que sonaba sigue hasta su final, a través del cambio de compás.
    assert!(l[BAR - 10..BAR + 10].iter().all(|&s| s == LEVEL));
    assert!(l[BAR + STEP - 200] == LEVEL);
    // Y acaba con su fundido, sin saltos.
    let step = l
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(step < LEVEL / 40.0, "salto de {step}");
    assert!(l[BAR + STEP..].iter().all(|&s| s == 0.0));
}
