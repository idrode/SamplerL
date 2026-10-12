//! Fundidos de entrada y salida al reproducir cortes. Con una señal
//! constante, cualquier corte brusco aparece como un salto grande entre
//! dos frames consecutivos.

use std::sync::Arc;

use engine::{Sample, new_engine, offline};

const RATE: u32 = 48_000;
const LEVEL: f32 = 0.5;
/// Fundido mínimo y máximo pedidos: 1 y 2 ms.
const MIN_FADE: usize = (RATE / 1000) as usize;
const MAX_FADE: usize = (RATE / 500) as usize;

fn constant_sample(frames: usize) -> Arc<Sample> {
    Arc::new(Sample::from_interleaved(&vec![LEVEL; frames], 1, RATE, RATE).unwrap())
}

/// Mayor salto entre frames consecutivos del canal izquierdo, contando el
/// silencio antes del primer frame.
fn max_step(audio: &[f32]) -> f32 {
    let left: Vec<f32> = audio.iter().step_by(2).copied().collect();
    std::iter::once(0.0)
        .chain(left.iter().copied())
        .zip(left.iter().copied())
        .map(|(a, b)| (b - a).abs())
        .fold(0.0, f32::max)
}

/// Un fundido de al menos 1 ms sube como mucho LEVEL/MIN_FADE por frame
/// (con un pequeño margen por redondeo).
fn assert_no_click(audio: &[f32]) {
    let limit = LEVEL / MIN_FADE as f32 * 1.01;
    let step = max_step(audio);
    assert!(
        step <= limit,
        "salto de {step} entre frames (límite {limit})"
    );
}

#[test]
fn slice_fades_in_and_out() {
    let sample = constant_sample(RATE as usize / 10);
    let (mut controller, mut engine) = new_engine();
    controller.play_range(sample, 1000..3000).unwrap();
    let audio = offline::render(&mut engine, 4000, 2);

    assert_no_click(&audio);
    let left: Vec<f32> = audio.iter().step_by(2).copied().collect();
    assert_eq!(left[0], 0.0);
    assert_eq!(left[1999], 0.0);
    // Pasados como mucho 2 ms está a nivel completo, y lo sigue estando
    // hasta 2 ms antes del final.
    assert!(left[MAX_FADE..2000 - MAX_FADE].iter().all(|&s| s == LEVEL));
    assert!(left[2000..].iter().all(|&s| s == 0.0));
}

#[test]
fn interrupting_a_slice_crossfades_without_clicks() {
    let sample = constant_sample(RATE as usize);
    let (mut controller, mut engine) = new_engine();

    controller.play_range(sample.clone(), 0..20_000).unwrap();
    let mut audio = offline::render(&mut engine, 3000, 2);
    // Se interrumpe a mitad del corte (no coincide con el borde de bloque).
    controller.play_range(sample.clone(), 5000..20_000).unwrap();
    audio.extend(offline::render(&mut engine, 3000, 2));
    controller.stop().unwrap();
    audio.extend(offline::render(&mut engine, 1000, 2));

    assert_no_click(&audio);
    // Tras el stop y su fundido, silencio, y el motor ya no reproduce.
    let tail = &audio[(6000 + MAX_FADE) * 2..];
    assert!(tail.iter().all(|&s| s == 0.0));
    assert!(!controller.is_playing());
}

#[test]
fn rapid_retriggers_stay_bounded() {
    // Retriggers más rápidos que el fundido: nunca más de dos voces.
    let sample = constant_sample(RATE as usize);
    let (mut controller, mut engine) = new_engine();
    let mut audio = Vec::new();
    for _ in 0..50 {
        controller.play_range(sample.clone(), 0..10_000).unwrap();
        audio.extend(offline::render(&mut engine, 20, 2));
    }
    assert!(audio.iter().all(|&s| s.abs() <= 2.0 * LEVEL));
    controller.collect_garbage();
}
