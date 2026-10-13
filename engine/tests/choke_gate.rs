//! Modo gate y one-shot, envolvente por corte (ataque y cola) y grupos de
//! choke: cada cosa al frame exacto.

use std::sync::Arc;

use engine::{Pattern, STEPS, Sample, SendError, SliceShape, Slices, Step, new_engine, offline};

const RATE: u32 = 48_000;
/// A 120 BPM, un paso son 6000 frames.
const STEP: usize = 6000;
/// El fundido mínimo: 1,5 ms.
const FADE: usize = 72;

/// Un corte constante a 0,5 que dura dos pasos y medio.
fn long() -> (Arc<Sample>, Slices) {
    let mono = vec![0.5f32; 15_000];
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    (sample, slices)
}

fn at(steps: &[usize]) -> Vec<Option<Step>> {
    let mut out = vec![None; STEPS];
    for &s in steps {
        out[s] = Some(Step::new(0));
    }
    out
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

fn render(patterns: Vec<(usize, Pattern)>, frames: usize) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    for (t, p) in patterns {
        controller.set_track_pattern(t, Some(p)).unwrap();
    }
    controller.start().unwrap();
    left(&offline::render(&mut engine, frames, 2))
}

fn pattern(gate: bool, shape: SliceShape, steps: &[usize]) -> Pattern {
    let (sample, slices) = long();
    Pattern::from_steps(sample, &slices, at(steps))
        .unwrap()
        .with_gate(gate)
        .with_shapes(&[shape])
        .unwrap()
}

#[test]
fn gate_stops_at_the_end_of_the_step_and_one_shot_does_not() {
    let shot = render(
        vec![(0, pattern(false, SliceShape::DEFAULT, &[0]))],
        3 * STEP,
    );
    let gated = render(
        vec![(0, pattern(true, SliceShape::DEFAULT, &[0]))],
        3 * STEP,
    );
    // Hasta el final del paso, igual.
    assert_eq!(gated[..STEP], shot[..STEP]);
    // Luego, la cola mínima y silencio; el one-shot sigue.
    assert!(gated[STEP + FADE..].iter().all(|s| *s == 0.0));
    assert_eq!(shot[2 * STEP], 0.5);
    // Con un retrigger de 2, cada mitad del paso.
    let (sample, slices) = long();
    let mut steps = vec![None; STEPS];
    steps[0] = Some(Step::new(0).with_retrig(2));
    let retrig = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_gate(true);
    let out = render(vec![(0, retrig)], 6 * STEP);
    // La segunda parte (desde 3000) se cierra al acabar el paso.
    assert_eq!(out[STEP - 1], 0.5);
    assert!(out[STEP + FADE..].iter().all(|s| *s == 0.0));
}

#[test]
fn attack_and_release_shape_the_slice() {
    // Ataque de 20 ms (960 frames): a la mitad, la mitad del nivel.
    let attack = SliceShape {
        attack_ms: 20.0,
        release_ms: 0.0,
        ..SliceShape::DEFAULT
    };
    let out = render(vec![(0, pattern(false, attack, &[0]))], STEP);
    assert!((out[480] - 0.25).abs() < 1e-3, "{}", out[480]);
    assert_eq!(out[960], 0.5);
    // Cola de 10 ms (480 frames) al cerrarse la puerta.
    let release = SliceShape {
        attack_ms: 0.0,
        release_ms: 10.0,
        ..SliceShape::DEFAULT
    };
    let out = render(vec![(0, pattern(true, release, &[0]))], 6 * STEP);
    assert!((out[STEP + 240] - 0.25).abs() < 2e-3, "{}", out[STEP + 240]);
    assert!(out[STEP + 480..].iter().all(|s| *s == 0.0));
    // En one-shot, la cola va al final del corte (15000 frames).
    let out = render(vec![(0, pattern(false, release, &[0]))], 3 * STEP);
    assert!((out[15_000 - 240] - 0.25).abs() < 2e-3);
    assert_eq!(out[15_000 - 600], 0.5);
}

#[test]
fn a_hit_chokes_the_other_tracks_of_its_group() {
    let open = |group| {
        pattern(false, SliceShape::DEFAULT, &[0])
            .with_choke(group)
            .unwrap()
    };
    let closed = |group| {
        pattern(false, SliceShape::DEFAULT, &[2])
            .with_choke(group)
            .unwrap()
    };
    let alone = render(vec![(0, open(Some(1)))], 6 * STEP);
    // Mismo grupo: en el paso 2 la pista 0 se calla en el fundido mínimo y
    // solo queda la 1.
    let both = render(vec![(0, open(Some(1))), (1, closed(Some(1)))], 6 * STEP);
    let only_closed = render(vec![(1, closed(Some(1)))], 6 * STEP);
    assert_eq!(both[..2 * STEP], alone[..2 * STEP]);
    let after = 2 * STEP + FADE + 1;
    assert_eq!(both[after..], only_closed[after..]);
    // Otro grupo (o ninguno): suenan las dos.
    let apart = render(vec![(0, open(Some(1))), (1, closed(Some(2)))], 6 * STEP);
    assert!(apart[after] > only_closed[after]);
    let none = render(vec![(0, open(None)), (1, closed(None))], 6 * STEP);
    assert_eq!(apart, none);
}

#[test]
fn live_pads_choke_and_keep_their_shape() {
    let (sample, slices) = long();
    let (mut controller, mut engine) = new_engine(RATE);
    controller
        .set_track_pattern(
            0,
            Some(
                pattern(false, SliceShape::DEFAULT, &[0])
                    .with_choke(Some(3))
                    .unwrap(),
            ),
        )
        .unwrap();
    controller
        .set_track_pattern(
            1,
            Some(
                pattern(false, SliceShape::DEFAULT, &[])
                    .with_choke(Some(3))
                    .unwrap(),
            ),
        )
        .unwrap();
    controller.start().unwrap();
    let _ = offline::render(&mut engine, 1000, 2);
    let attack = SliceShape {
        attack_ms: 20.0,
        release_ms: 0.0,
        ..SliceShape::DEFAULT
    };
    controller
        .play_slice_shaped(1, &sample, &slices, 0, attack)
        .unwrap();
    let out = left(&offline::render(&mut engine, 2000, 2));
    // La pista 0 se calla; la 1 entra con su ataque de 20 ms.
    assert!((out[480] - 0.25).abs() < 2e-3, "{}", out[480]);
}

#[test]
fn shapes_and_groups_are_checked() {
    let (sample, slices) = long();
    let p = || Pattern::from_steps(sample.clone(), &slices, at(&[0])).unwrap();
    for bad in [
        SliceShape {
            attack_ms: -1.0,
            release_ms: 0.0,
            ..SliceShape::DEFAULT
        },
        SliceShape {
            attack_ms: 0.0,
            release_ms: 6000.0,
            ..SliceShape::DEFAULT
        },
    ] {
        assert!(matches!(
            p().with_shapes(&[bad]),
            Err(SendError::InvalidStep { .. })
        ));
    }
    assert!(p().with_choke(Some(0)).is_err());
    assert!(p().with_choke(Some(17)).is_err());
    assert!(p().with_choke(Some(16)).is_ok());
}
