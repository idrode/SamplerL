//! Fundido cruzado en el salto de los bucles de corte (`SliceShape::
//! crossfade_ms`): sin discontinuidad en la unión con un seno, el bucle
//! conserva su periodo, funde con lo de antes del inicio o, si no hay, con
//! lo de después del final; con 0, como siempre. El ida y vuelta no salta.

use std::sync::Arc;

use engine::{Pattern, PlayMode, STEPS, Sample, SliceShape, Slices, Step, new_engine, offline};

const RATE: u32 = 48_000;
/// Cuatro cortes de 1000 frames.
const SPAN: usize = 1000;

/// Un seno de 444 Hz a 0,5: 1000 frames son 9,25 periodos, así que el
/// salto del bucle va de un paso por cero a una cresta (o al revés).
fn sine() -> Vec<f32> {
    (0..4 * SPAN)
        .map(|i| 0.5 * (std::f32::consts::TAU * 444.0 * i as f32 / RATE as f32).sin())
        .collect()
}

/// Lo más que cambia el seno de una muestra a la siguiente.
const SINE_STEP: f32 = 0.5 * std::f32::consts::TAU * 444.0 / RATE as f32;

/// Canal izquierdo de un compás a 120 BPM con el corte `slice` en bucle
/// desde el paso 0 (`reverse`: al revés) y ese fundido.
fn render_loop(
    mono: &[f32],
    slices: usize,
    slice: usize,
    crossfade_ms: f32,
    play: PlayMode,
    reverse: bool,
) -> Vec<f32> {
    let sample = Arc::new(Sample::from_interleaved(mono, 1, RATE, RATE).unwrap());
    let cuts = Slices::grid(&sample, slices);
    let mut steps = vec![None; STEPS];
    let step = Step::new(slice);
    steps[0] = Some(if reverse { step.reversed() } else { step });
    let shape = SliceShape {
        play,
        crossfade_ms,
        ..SliceShape::DEFAULT
    };
    let pattern = Pattern::from_steps(sample, &cuts, steps)
        .unwrap()
        .with_shapes(&vec![shape; slices])
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, 6 * SPAN, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

/// El salto más grande de una muestra a la siguiente, pasado el ataque.
fn max_jump(x: &[f32]) -> f32 {
    x[100..]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

#[test]
fn without_crossfade_the_loop_is_the_slice_again_and_again() {
    // Con 0, el de siempre: el corte repetido tal cual (y su salto).
    let mono = sine();
    let out = render_loop(&mono, 4, 1, 0.0, PlayMode::Loop, false);
    for (k, &v) in out.iter().enumerate().skip(100) {
        assert_eq!(v, mono[SPAN + k % SPAN], "frame {k}");
    }
    assert!(max_jump(&out) > 10.0 * SINE_STEP, "{}", max_jump(&out));
}

#[test]
fn the_crossfade_removes_the_jump_and_keeps_the_period() {
    let mono = sine();
    // Corte 2: con sitio antes del inicio. Corte 1: sin él, funde con lo
    // que sigue al final. Al revés, igual en su sentido.
    for (slice, reverse) in [(1, false), (0, false), (1, true), (3, true)] {
        let out = render_loop(&mono, 4, slice, 10.0, PlayMode::Loop, reverse);
        let jump = max_jump(&out);
        assert!(
            jump < 1.2 * SINE_STEP,
            "corte {} reverse {reverse}: {jump}",
            slice + 1
        );
        // Cada vuelta dura lo que el corte (no se acorta). Fundiendo con lo
        // que sigue al final, la primera vuelta empieza sin fundido: desde
        // la segunda.
        for k in SPAN..out.len() - SPAN {
            assert_eq!(
                out[k],
                out[k + SPAN],
                "corte {} reverse {reverse}, frame {k}",
                slice + 1
            );
        }
        // Fuera del fundido, el corte tal cual.
        let k = 2 * SPAN + SPAN / 2;
        let at = if reverse {
            slice * SPAN + SPAN - 1 - SPAN / 2
        } else {
            slice * SPAN + SPAN / 2
        };
        assert_eq!(out[k], mono[at], "corte {} reverse {reverse}", slice + 1);
    }
}

#[test]
fn without_room_outside_the_slice_there_is_no_crossfade() {
    // Un solo corte con todo el sample: nada antes ni después.
    let mono = sine();
    let with = render_loop(&mono, 1, 0, 10.0, PlayMode::Loop, false);
    let without = render_loop(&mono, 1, 0, 0.0, PlayMode::Loop, false);
    assert_eq!(with, without);
    assert_eq!(engine::loop_seam_room(0, 4 * SPAN, 4 * SPAN), 0);
    assert_eq!(engine::loop_seam_room(SPAN, 2 * SPAN, 4 * SPAN), 2 * SPAN);
}

#[test]
fn ping_pong_turns_without_a_jump_and_ignores_the_crossfade() {
    let mono = sine();
    let out = render_loop(&mono, 4, 1, 0.0, PlayMode::PingPong, false);
    assert!(max_jump(&out) <= 1.01 * SINE_STEP, "{}", max_jump(&out));
    assert_eq!(
        render_loop(&mono, 4, 1, 10.0, PlayMode::PingPong, false),
        out
    );
    // Y una vez (sin bucle), tampoco cambia nada.
    assert_eq!(
        render_loop(&mono, 4, 1, 10.0, PlayMode::Once, false),
        render_loop(&mono, 4, 1, 0.0, PlayMode::Once, false)
    );
}

#[test]
fn the_range_is_checked() {
    let shape = |ms| SliceShape {
        crossfade_ms: ms,
        ..SliceShape::DEFAULT
    };
    assert!(shape(0.0).validate().is_ok());
    assert!(shape(1000.0).validate().is_ok());
    assert!(shape(-1.0).validate().is_err());
    assert!(shape(1000.5).validate().is_err());
}
