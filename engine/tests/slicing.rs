//! Detección de transitorios sobre un audio sintético con golpes en
//! posiciones conocidas, y reproducción de cortes por número.

use std::sync::Arc;

use engine::{Sample, SendError, Slices, new_engine};

const RATE: u32 = 48_000;
const LENGTH_SECONDS: f32 = 2.4;
/// (posición en ms, amplitud) de cada golpe. Los fuertes van alternados con
/// golpes débiles para comprobar la selección de los N más fuertes.
const HITS: [(u32, f32); 8] = [
    (0, 0.9),
    (300, 0.3),
    (520, 0.8),
    (750, 0.3),
    (1000, 0.9),
    (1310, 0.3),
    (1600, 0.8),
    (1900, 0.3),
];
/// Tolerancia de la posición de cada corte: 2 ms.
const TOLERANCE: usize = (RATE / 500) as usize;

fn ms_to_frames(ms: u32) -> usize {
    (ms * RATE / 1000) as usize
}

/// Generador pseudoaleatorio determinista (xorshift32) en [-1, 1).
struct Noise(u32);

impl Noise {
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        (self.0 as f32 / u32::MAX as f32) * 2.0 - 1.0
    }
}

/// Golpes tipo batería (ruido + cuerpo grave, ataque de 1 ms y caída
/// exponencial) sobre un fondo de ruido débil. Mono.
fn synth_break() -> Vec<f32> {
    let frames = (LENGTH_SECONDS * RATE as f32) as usize;
    let mut noise = Noise(0x1234_5678);
    let mut out: Vec<f32> = (0..frames).map(|_| 0.005 * noise.next()).collect();
    for &(ms, amp) in &HITS {
        let start = ms_to_frames(ms);
        for (i, s) in out[start..].iter_mut().enumerate() {
            let t = i as f32 / RATE as f32;
            let attack = (t / 0.001).min(1.0);
            let env = amp * attack * (-t / 0.05).exp();
            let body = (2.0 * std::f32::consts::PI * 120.0 * t).sin();
            *s += env * (0.6 * noise.next() + 0.4 * body);
        }
    }
    out
}

fn load() -> Arc<Sample> {
    Arc::new(Sample::from_interleaved(&synth_break(), 1, RATE, RATE).unwrap())
}

fn assert_near(actual: usize, expected: usize, what: &str) {
    assert!(
        actual.abs_diff(expected) <= TOLERANCE,
        "{what}: en {actual}, esperado {expected} (±{TOLERANCE})"
    );
}

#[test]
fn detects_all_hits_at_known_positions() {
    let sample = load();
    let slices = Slices::detect(&sample, 8);
    assert_eq!(slices.len(), HITS.len());

    for (i, (range, &(ms, _))) in slices.iter().zip(&HITS).enumerate() {
        assert_near(range.start, ms_to_frames(ms), &format!("corte {i}"));
    }
    // Contiguos: cada corte acaba donde empieza el siguiente; el último en el final.
    let ranges: Vec<_> = slices.iter().collect();
    for pair in ranges.windows(2) {
        assert_eq!(pair[0].end, pair[1].start);
    }
    assert_eq!(ranges.last().unwrap().end, sample.frames());
}

#[test]
fn asking_for_more_slices_than_hits_gives_no_false_positives() {
    let slices = Slices::detect(&load(), 32);
    assert_eq!(slices.len(), HITS.len());
}

#[test]
fn fewer_slices_keep_the_strongest_hits() {
    let slices = Slices::detect(&load(), 4);
    let strong: Vec<usize> = HITS
        .iter()
        .filter(|&&(_, amp)| amp > 0.5)
        .map(|&(ms, _)| ms_to_frames(ms))
        .collect();
    assert_eq!(slices.len(), strong.len());
    for (i, (range, expected)) in slices.iter().zip(strong).enumerate() {
        assert_near(range.start, expected, &format!("corte {i}"));
    }
}

#[test]
fn plays_a_slice_by_number() {
    let sample = load();
    let slices = Slices::detect(&sample, 8);
    let slice = slices.get(2).unwrap();

    let (mut controller, mut engine) = new_engine();
    controller.play_slice(&sample, &slices, 2).unwrap();

    let block = 512;
    let mut rendered = vec![0.0f32; (slice.len() + 4 * block) * 2];
    for chunk in rendered.chunks_mut(block * 2) {
        engine.process(chunk, 2);
    }
    assert!(!controller.is_playing());

    // Lo renderizado es exactamente el corte: mismos frames, luego silencio.
    let (body, tail) = rendered.split_at(slice.len() * 2);
    for (i, frame) in body.as_chunks::<2>().0.iter().enumerate() {
        let (l, r) = sample.frame(slice.start + i);
        assert_eq!(*frame, [l, r], "frame {i} del corte");
    }
    assert!(tail.iter().all(|&s| s == 0.0));

    // El arranque del corte es el golpe: hay energía en los primeros 5 ms.
    let attack = &body[..(RATE as usize / 200) * 2];
    assert!(attack.iter().any(|s| s.abs() > 0.3));

    assert_eq!(
        controller.play_slice(&sample, &slices, 8),
        Err(SendError::NoSuchSlice { index: 8, count: 8 })
    );
}
