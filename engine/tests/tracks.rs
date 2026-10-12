//! Varias pistas sonando a la vez y lotes de cambios atómicos.

use std::sync::Arc;

use engine::notation::parse_steps;
use engine::{Batch, MAX_TRACKS, Pattern, Sample, SendError, Slices, new_engine, offline};

const RATE: u32 = 48_000;
/// A 120 BPM: 6000 frames por paso, 96000 por compás.
const STEP: usize = 6000;
const BAR: usize = 16 * STEP;
/// Pasado el fundido de entrada (≤ 2 ms).
const SETTLE: usize = 200;

/// Dos cortes: bloque constante de 800 frames a 0.5 en 0 y a 0.25 en 3000.
fn two_slices() -> (Arc<Sample>, Slices) {
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    assert_eq!(slices.len(), 2);
    (sample, slices)
}

fn pattern(sample: &Arc<Sample>, slices: &Slices, text: &str) -> Pattern {
    Pattern::from_steps(sample.clone(), slices, parse_steps(text).unwrap()).unwrap()
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

/// Nivel justo después del paso `k` del compás `bar`.
fn level(left: &[f32], bar: usize, k: usize) -> f32 {
    left[bar * BAR + k * STEP + SETTLE]
}

#[test]
fn tracks_play_at_the_same_time() {
    let (sample, slices) = two_slices();
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_pattern(
            0,
            Some(pattern(
                &sample,
                &slices,
                "1 . . .  1 . . .  . . . .  . . . .",
            )),
        )
        .set_pattern(
            3,
            Some(pattern(
                &sample,
                &slices,
                "2 . 2 .  . . . .  . . . .  . . . .",
            )),
        );
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let l = left(&offline::render(&mut engine, BAR, 2));

    // Paso 0: las dos pistas a la vez (se suman). Paso 2: solo la pista 3.
    // Paso 4: solo la pista 0. Paso 1: nada (los bloques duran 800 frames).
    assert_eq!(level(&l, 0, 0), 0.75);
    assert_eq!(level(&l, 0, 1), 0.0);
    assert_eq!(level(&l, 0, 2), 0.25);
    assert_eq!(level(&l, 0, 4), 0.5);
}

#[test]
fn a_batch_enters_whole_at_the_next_bar() {
    let (sample, slices) = two_slices();
    let (mut controller, mut engine) = new_engine(RATE);
    let mut first = Batch::new();
    first
        .set_bpm(120.0)
        .set_pattern(
            0,
            Some(pattern(
                &sample,
                &slices,
                "1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1",
            )),
        )
        .set_pattern(
            1,
            Some(pattern(
                &sample,
                &slices,
                "2 . . .  . . . .  . . . .  . . . .",
            )),
        );
    controller.apply(first).unwrap();
    controller.start().unwrap();
    let mut audio = offline::render(&mut engine, BAR / 2 + 123, 2);

    // A mitad de compás: pista 0 cambia, pista 1 se silencia, y el tempo pasa
    // a 60 BPM (pasos de 12000 frames). Nada de eso puede notarse antes del
    // compás siguiente.
    let mut second = Batch::new();
    second
        .set_bpm(60.0)
        .set_pattern(
            0,
            Some(pattern(
                &sample,
                &slices,
                "2 . . .  . . . .  . . . .  . . . .",
            )),
        )
        .set_pattern(1, None);
    controller.apply(second).unwrap();
    audio.extend(offline::render(&mut engine, BAR / 2 - 123 + 2 * BAR, 2));
    let l = left(&audio);

    // Compás 0 entero con lo viejo: pista 0 en todos los pasos (0.5) y pista
    // 1 en el paso 0 (0.25 sumado).
    assert_eq!(level(&l, 0, 0), 0.75);
    for k in 1..16 {
        assert_eq!(level(&l, 0, k), 0.5, "paso {k}");
    }
    // Compás 1 (empieza en 96000) a 60 BPM: solo la pista 0 con el corte 2,
    // y el paso 1 ya no cae en 96000 + 6000 sino en 96000 + 12000.
    assert_eq!(l[BAR + SETTLE], 0.25);
    assert_eq!(l[BAR + STEP + SETTLE], 0.0);
    assert!(
        l[BAR + 800 + SETTLE..BAR + 2 * BAR]
            .iter()
            .all(|&s| s == 0.0)
    );
}

#[test]
fn a_batch_that_does_not_fit_sends_nothing() {
    let (sample, slices) = two_slices();
    let (mut controller, mut engine) = new_engine(RATE);
    // Llenar la cola sin que el motor la lea.
    while controller.set_bpm(120.0).is_ok() {}

    let mut batch = Batch::new();
    batch.set_pattern(
        0,
        Some(pattern(
            &sample,
            &slices,
            "1 . . .  . . . .  . . . .  . . . .",
        )),
    );
    assert_eq!(controller.apply(batch), Err(SendError::QueueFull));

    // Vaciar la cola, arrancar y comprobar que el patrón no llegó.
    offline::render(&mut engine, 64, 2);
    controller.start().unwrap();
    let audio = offline::render(&mut engine, BAR, 2);
    assert!(audio.iter().all(|&s| s == 0.0));
}

#[test]
fn rejects_invalid_tracks_and_bpm_without_sending() {
    let (sample, slices) = two_slices();
    let (mut controller, _engine) = new_engine(RATE);
    let p = || {
        Some(pattern(
            &sample,
            &slices,
            "1 . . .  . . . .  . . . .  . . . .",
        ))
    };

    assert_eq!(
        controller.set_track_pattern(MAX_TRACKS, p()),
        Err(SendError::NoSuchTrack(MAX_TRACKS))
    );
    let mut batch = Batch::new();
    batch.set_pattern(0, p()).set_pattern(MAX_TRACKS + 1, p());
    assert_eq!(
        controller.apply(batch),
        Err(SendError::NoSuchTrack(MAX_TRACKS + 1))
    );
    let mut batch = Batch::new();
    batch.set_bpm(1000.0).set_pattern(0, p());
    assert_eq!(controller.apply(batch), Err(SendError::InvalidBpm(1000.0)));
}
