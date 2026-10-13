//! Pan, parte del corte que suena, bucle e ida y vuelta: muestra a muestra.

use std::sync::Arc;

use engine::{Pattern, PlayMode, STEPS, Sample, SliceShape, Slices, Step, new_engine, offline};

const RATE: u32 = 48_000;
const FADE: usize = 72;

/// Una rampa de 1000 frames (cada muestra distinta, nunca cero).
fn ramp() -> (Arc<Sample>, Slices) {
    let mono: Vec<f32> = (0..1000).map(|i| 0.1 + 0.3 * i as f32 / 1000.0).collect();
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    assert_eq!(slices.get(0), Some(0..1000));
    (sample, slices)
}

fn render_with(slices: &Slices, shape: SliceShape, gate: bool, frames: usize) -> Vec<f32> {
    let (sample, _) = ramp();
    let mut steps = vec![None; STEPS];
    steps[0] = Some(Step::new(0));
    let pattern = Pattern::from_steps(sample, slices, steps)
        .unwrap()
        .with_shapes(&[shape])
        .unwrap()
        .with_gate(gate);
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
}

fn render(shape: SliceShape, frames: usize) -> Vec<f32> {
    render_with(&ramp().1, shape, false, frames)
}

fn shape(f: impl FnOnce(&mut SliceShape)) -> SliceShape {
    let mut s = SliceShape::DEFAULT;
    f(&mut s);
    s
}

#[test]
fn pan_lowers_the_other_side_only() {
    let center = render(SliceShape::DEFAULT, 1200);
    let right = render(shape(|s| s.pan = 1.0), 1200);
    let half_left = render(shape(|s| s.pan = -0.5), 1200);
    for f in 0..1200 {
        let (l, r) = (center[2 * f], center[2 * f + 1]);
        assert_eq!(right[2 * f], 0.0);
        assert_eq!(right[2 * f + 1], r);
        assert_eq!(half_left[2 * f], l);
        assert_eq!(half_left[2 * f + 1], 0.5 * r);
    }
}

#[test]
fn length_plays_only_the_start_of_the_slice() {
    let half = render(shape(|s| s.length = 0.5), 1200);
    // Igual que un corte que acabara en la mitad.
    let (first, _) = ramp().1.split(500, 1).unwrap();
    let cut = render_with(&first, SliceShape::DEFAULT, false, 1200);
    assert_eq!(half, cut);
    // (El último frame es el final del fundido de salida: cero.)
    assert!(half[2 * 450] != 0.0 && half[2 * 499..].iter().all(|s| *s == 0.0));
}

#[test]
fn a_loop_repeats_the_slice_until_cut() {
    let looped = render(shape(|s| s.play = PlayMode::Loop), 4000);
    let left: Vec<f32> = looped.iter().step_by(2).copied().collect();
    // Tras el fundido de entrada, cada 1000 frames lo mismo, sin parar.
    for t in 1000 + FADE..4000 {
        assert_eq!(left[t], left[t - 1000], "t={t}");
    }
    assert!(left[3999] != 0.0);
    // En el modo gate, el bucle acaba al final del paso (6000 frames).
    let gated = render_with(&ramp().1, shape(|s| s.play = PlayMode::Loop), true, 8000);
    assert!(gated[2 * 5900] != 0.0);
    assert!(gated[2 * (6000 + FADE)..].iter().all(|s| *s == 0.0));
}

#[test]
fn ping_pong_goes_back_and_forth() {
    let pp = render(shape(|s| s.play = PlayMode::PingPong), 3000);
    let left: Vec<f32> = pp.iter().step_by(2).copied().collect();
    // Ida hasta el frame 999 y vuelta: el n-ésimo de la vuelta es el
    // n-ésimo antes del extremo.
    for k in 0..(999 - FADE) {
        assert_eq!(left[999 + k], left[999 - k], "k={k}");
    }
    // Y otra ida: periodo de 2·999.
    for t in 1998 + FADE..3000 {
        assert_eq!(left[t], left[t - 1998], "t={t}");
    }
}

#[test]
fn shapes_out_of_range_are_rejected() {
    let (sample, slices) = ramp();
    let p =
        || Pattern::from_steps(sample.clone(), &slices, vec![Some(Step::new(0)); STEPS]).unwrap();
    assert!(p().with_shapes(&[shape(|s| s.pan = 1.5)]).is_err());
    assert!(p().with_shapes(&[shape(|s| s.length = 0.0)]).is_err());
    assert!(p().with_shapes(&[shape(|s| s.length = 1.2)]).is_err());
}
