//! Cortes en el cruce por cero: `nearest_zero_crossing` y
//! `Slices::snapped`.

use engine::{Sample, Slices, nearest_zero_crossing};

const RATE: u32 = 48_000;

/// Un seno de `hz` (mono) de `frames` frames.
fn sine(hz: f32, frames: usize) -> Sample {
    let mono: Vec<f32> = (0..frames)
        .map(|i| 0.8 * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin())
        .collect();
    Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap()
}

fn mono(sample: &Sample, i: usize) -> f32 {
    let (l, r) = sample.frame(i);
    0.5 * (l + r)
}

#[test]
fn the_nearest_crossing_is_the_closest_frame_to_zero_around_a_sign_change() {
    // 100 Hz: un cruce cada 240 frames (0, 240, 480...).
    let s = sine(100.0, 4800);
    let all = 0..s.frames();
    assert_eq!(nearest_zero_crossing(&s, 0, 10, all.clone()), Some(0));
    // A 30 frames del cruce de 240, por los dos lados.
    assert_eq!(nearest_zero_crossing(&s, 210, 40, all.clone()), Some(240));
    assert_eq!(nearest_zero_crossing(&s, 270, 40, all.clone()), Some(240));
    // Más lejos que el máximo: ninguno.
    assert_eq!(nearest_zero_crossing(&s, 120, 40, all.clone()), None);
    // Fuera de los límites, tampoco.
    assert_eq!(nearest_zero_crossing(&s, 230, 40, 0..235), None);
    // El que queda más cerca de cero de los dos lados del cambio.
    for target in [100, 333, 1000, 2222] {
        let found = nearest_zero_crossing(&s, target, 240, all.clone()).unwrap();
        let x = mono(&s, found).abs();
        assert!(x <= mono(&s, found + 1).abs() || x <= mono(&s, found - 1).abs());
        assert!(
            x < 0.8 * (std::f32::consts::TAU * 100.0 / RATE as f32),
            "{target}: {x}"
        );
    }
}

#[test]
fn a_signal_without_crossings_keeps_its_slices() {
    // Corriente continua: no cruza nunca.
    let s = Sample::from_interleaved(&vec![0.3; 4800], 1, RATE, RATE).unwrap();
    let grid = Slices::grid(&s, 4);
    assert_eq!(grid.snapped(&s, 200, 48), grid);
    assert_eq!(nearest_zero_crossing(&s, 1200, 200, 0..4800), None);
}

#[test]
fn snapped_slices_start_at_a_crossing_and_stay_contiguous() {
    let s = sine(113.0, RATE as usize);
    let grid = Slices::grid(&s, 8);
    // Sin máximo, los mismos.
    assert_eq!(grid.snapped(&s, 0, 48), grid);
    let max = 144; // 3 ms
    let snapped = grid.snapped(&s, max, 48);
    assert_eq!(snapped.len(), grid.len());
    let step = 0.8 * (std::f32::consts::TAU * 113.0 / RATE as f32);
    let mut moved = 0;
    for (i, (a, b)) in grid.iter().zip(snapped.iter()).enumerate() {
        assert!(a.start.abs_diff(b.start) <= max, "corte {i}");
        assert!(
            mono(&s, b.start).abs() <= step,
            "corte {i}: {}",
            mono(&s, b.start)
        );
        if i + 1 < snapped.len() {
            assert_eq!(b.end, snapped.get(i + 1).unwrap().start);
        } else {
            assert_eq!(b.end, s.frames());
        }
        moved += usize::from(a.start != b.start);
    }
    // En rejilla, casi todos caían fuera del cruce.
    assert!(
        moved >= 6,
        "{moved}: {:?}",
        snapped.iter().map(|r| r.start).collect::<Vec<_>>()
    );
}

#[test]
fn a_snap_never_crosses_the_neighbours() {
    // Cortes a 10 frames: con una separación mínima de 8, el cruce más
    // cercano no puede pasar al otro lado de un vecino.
    let s = sine(50.0, 4800);
    let close = Slices::grid(&s, 4800 / 10);
    let snapped = close.snapped(&s, 480, 8);
    let starts: Vec<usize> = snapped.iter().map(|r| r.start).collect();
    assert!(starts.windows(2).all(|w| w[1] >= w[0] + 8), "{starts:?}");
}
