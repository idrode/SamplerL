//! Detección de transitorios sobre un audio sintético con golpes en
//! posiciones conocidas, y reproducción de cortes por número.

mod support;

use std::sync::Arc;

use engine::{DetectOptions, Detector, Sample, SendError, Slices, new_engine};
use support::synth_break::{Hit, render_hits};

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

    // El detector por energía sigue disponible y también los encuentra.
    let energy = DetectOptions {
        detector: Detector::Energy,
        ..Default::default()
    };
    let slices_energy = Slices::detect_with(&sample, 8, &energy);
    assert_eq!(slices_energy.len(), HITS.len());
    for (i, (range, &(ms, _))) in slices_energy.iter().zip(&HITS).enumerate() {
        assert_near(
            range.start,
            ms_to_frames(ms),
            &format!("corte {i} (energía)"),
        );
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

    let (mut controller, mut engine) = new_engine(RATE);
    controller.play_slice(&sample, &slices, 2).unwrap();

    let block = 512;
    let mut rendered = vec![0.0f32; (slice.len() + 4 * block) * 2];
    for chunk in rendered.chunks_mut(block * 2) {
        engine.process(chunk, 2);
    }
    assert!(!controller.is_playing());

    // Lo renderizado es el corte: mismos frames salvo los fundidos de los
    // extremos (como mucho 2 ms), que empiezan y acaban en cero; luego silencio.
    let (body, tail) = rendered.split_at(slice.len() * 2);
    let frames = body.as_chunks::<2>().0;
    let fade = (RATE / 500) as usize;
    for (i, frame) in frames
        .iter()
        .enumerate()
        .skip(fade)
        .take(slice.len() - 2 * fade)
    {
        let (l, r) = sample.frame(slice.start + i);
        assert_eq!(*frame, [l, r], "frame {i} del corte");
    }
    assert_eq!(frames[0], [0.0, 0.0]);
    assert_eq!(frames[slice.len() - 1], [0.0, 0.0]);
    assert!(tail.iter().all(|&s| s == 0.0));

    // El arranque del corte es el golpe: hay energía en los primeros 5 ms.
    let attack = &body[..(RATE as usize / 200) * 2];
    assert!(attack.iter().any(|s| s.abs() > 0.3));

    assert_eq!(
        controller.play_slice(&sample, &slices, 8),
        Err(SendError::NoSuchSlice { index: 8, count: 8 })
    );
}

/// Dos golpes a `gap_frames` de distancia, el segundo más débil.
fn two_close_hits(gap_frames: usize) -> Arc<Sample> {
    let mut noise = Noise(0x0bad_cafe);
    let frames = RATE as usize / 2;
    let mut out = vec![0.0f32; frames];
    for (start, amp) in [(4800, 0.9), (4800 + gap_frames, 0.6)] {
        for (i, s) in out[start..].iter_mut().enumerate() {
            let t = i as f32 / RATE as f32;
            *s += amp * (-t / 0.01).exp() * noise.next();
        }
    }
    Arc::new(Sample::from_interleaved(&out, 1, RATE, RATE).unwrap())
}

#[test]
fn min_gap_is_measured_in_frames_not_analysis_hops() {
    // 41,7 ms: por encima de los 40 ms por defecto, así que son dos cortes.
    let sample = two_close_hits(2000);
    let slices = Slices::detect(&sample, 8);
    assert_eq!(slices.len(), 2);
    assert_near(slices.get(0).unwrap().start, 4800, "golpe 1");
    assert_near(slices.get(1).unwrap().start, 6800, "golpe 2");
}

#[test]
fn min_gap_is_configurable() {
    let sample = two_close_hits(2000);
    // Con 50 ms de separación mínima solo queda el golpe más fuerte.
    let wide = DetectOptions {
        min_gap_seconds: 0.050,
        ..Default::default()
    };
    let slices = Slices::detect_with(&sample, 8, &wide);
    assert_eq!(slices.len(), 1);
    assert_near(slices.get(0).unwrap().start, 4800, "golpe fuerte");

    // Con 10 ms, dos golpes a 20 ms se separan; con el valor por defecto, no.
    let closer = two_close_hits(960);
    assert_eq!(Slices::detect(&closer, 8).len(), 1);
    let narrow = DetectOptions {
        min_gap_seconds: 0.010,
        ..Default::default()
    };
    assert_eq!(Slices::detect_with(&closer, 8, &narrow).len(), 2);
}

#[test]
fn flux_finds_a_weak_hat_over_a_kick_tail() {
    // Bombo grave con cola larga y, 214 ms después, un charles más débil
    // que la cola del bombo en ese momento.
    let hat_at = ms_to_frames(214);
    let length = RATE as usize / 2;
    let kick_only = render_hits(RATE, &[(Hit::Kick, 0, 1.0)], length);
    let hat_only = render_hits(RATE, &[(Hit::Hat, hat_at, 0.4)], length);
    let audio: Vec<f32> = kick_only
        .iter()
        .zip(&hat_only)
        .map(|(k, h)| k + h)
        .collect();

    // Precondición: en los 10 ms del charles, la cola del bombo es más fuerte
    // en pico y en RMS. Por amplitud total, el charles no destaca.
    let window = hat_at..hat_at + ms_to_frames(10);
    let peak = |x: &[f32]| x.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let rms = |x: &[f32]| (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt();
    let (tail, hat) = (&kick_only[window.clone()], &hat_only[window]);
    assert!(
        peak(hat) < peak(tail),
        "pico: charles {} >= cola {}",
        peak(hat),
        peak(tail)
    );
    assert!(
        rms(hat) < rms(tail),
        "RMS: charles {} >= cola {}",
        rms(hat),
        rms(tail)
    );

    let sample = Sample::from_interleaved(&audio, 1, RATE, RATE).unwrap();
    let slices = Slices::detect(&sample, 8);
    assert_eq!(slices.len(), 2, "{slices:?}");
    assert_near(slices.get(0).unwrap().start, 0, "bombo");
    assert_near(slices.get(1).unwrap().start, hat_at, "charles");
}

#[test]
fn slice_starts_can_be_moved_by_hand_between_their_neighbours() {
    let mut mono = vec![0.0f32; 9000];
    for k in 0..3 {
        mono[k * 3000..k * 3000 + 300].fill(0.5);
    }
    let sample = engine::Sample::from_interleaved(&mono, 1, 48_000, 48_000).unwrap();
    let slices = engine::Slices::detect(&sample, 3);
    assert_eq!(
        slices.iter().collect::<Vec<_>>(),
        vec![0..3000, 3000..6000, 6000..9000]
    );
    let moved = slices.with_start(1, 3500, 48).unwrap();
    assert_eq!(
        moved.iter().collect::<Vec<_>>(),
        vec![0..3500, 3500..6000, 6000..9000]
    );
    let first = slices.with_start(0, 100, 48).unwrap();
    assert_eq!(first.get(0), Some(100..3000));
    // No puede pasar a sus vecinos (con margen).
    // (El corte 2 puede adelantarse hasta 1 ms tras el inicio del 1.)
    assert_eq!(slices.with_start(1, 5990, 48), Err(48..5952));
    assert_eq!(slices.with_start(1, 10, 48), Err(48..5952));
    assert_eq!(slices.with_start(2, 8999, 48), Err(3048..8952));
    assert!(slices.with_start(5, 1, 48).is_err());
}

#[test]
fn grid_split_and_without() {
    let sample = Sample::from_interleaved(&[0.0; 1000], 1, 1000, 1000).unwrap();
    let grid = Slices::grid(&sample, 3);
    assert_eq!(
        grid.iter().collect::<Vec<_>>(),
        [0..333, 333..667, 667..1000]
    );
    assert_eq!(Slices::grid(&sample, 16).len(), 16);

    let (split, k) = grid.split(500, 10).unwrap();
    assert_eq!(k, 2);
    assert_eq!(
        split.iter().collect::<Vec<_>>(),
        [0..333, 333..500, 500..667, 667..1000]
    );
    // Demasiado cerca de un límite: los límites válidos.
    assert_eq!(grid.split(338, 10).unwrap_err(), 343..657);
    // Quitar el único otro corte deja uno; partir uno solo da dos.
    let late = Slices::grid(&sample, 2).without(1).unwrap();
    assert_eq!(late.len(), 1);
    assert_eq!(late.get(0), Some(0..1000));
    let (shifted, _) = Slices::grid(&sample, 1).split(400, 10).unwrap();
    assert_eq!(shifted.iter().collect::<Vec<_>>(), [0..400, 400..1000]);

    assert_eq!(
        split.without(2).unwrap().iter().collect::<Vec<_>>(),
        grid.iter().collect::<Vec<_>>()
    );
    assert!(grid.without(0).is_none());
    assert!(grid.without(3).is_none());
}
