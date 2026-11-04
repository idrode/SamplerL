//! Deformaciones en tiempo real de un corte: `bend` (curva de tono dentro
//! del golpe) llega a su tono en el tiempo pedido, `rev` invierte las
//! muestras exactamente (como `'`) y con los valores por defecto todo es
//! lo de siempre, bit a bit.

use std::sync::Arc;

use engine::{
    Curve, HitShape, Pattern, Sample, SliceShape, Slices, Step, detect_mono, new_engine, offline,
};

const RATE: u32 = 48_000;

fn sine(hz: f64, seconds: f64) -> Arc<Sample> {
    let n = (seconds * RATE as f64) as usize;
    let mono: Vec<f32> = (0..n)
        .map(|i| (0.5 * (std::f64::consts::TAU * hz * i as f64 / RATE as f64).sin()) as f32)
        .collect();
    Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap())
}

/// Una rampa de 2000 frames (cada muestra distinta, nunca cero).
fn ramp() -> Arc<Sample> {
    let mono: Vec<f32> = (0..2000).map(|i| 0.1 + 0.3 * i as f32 / 2000.0).collect();
    Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap())
}

fn render_step(sample: Arc<Sample>, shape: SliceShape, step: Step, frames: usize) -> Vec<f32> {
    let slices = Slices::grid(&sample, 1);
    let mut steps = vec![None; 64];
    steps[0] = Some(step);
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_shapes(&[shape])
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

fn render(sample: Arc<Sample>, shape: SliceShape, frames: usize) -> Vec<f32> {
    render_step(sample, shape, Step::new(0), frames)
}

fn with(f: impl FnOnce(&mut SliceShape)) -> SliceShape {
    let mut s = SliceShape::DEFAULT;
    f(&mut s);
    s
}

/// Frecuencia en el instante `at` (frames): por los cruces por cero de
/// subida alrededor, interpolados.
fn hz_at(left: &[f32], at: usize, span: usize) -> f64 {
    hz_around(left, at, span).0
}

/// La frecuencia media entre los cruces de alrededor de `at` y el frame
/// del medio de esos cruces.
fn hz_around(left: &[f32], at: usize, span: usize) -> (f64, f64) {
    let crossings: Vec<f64> = (at - span..at + span)
        .filter(|&i| left[i] <= 0.0 && left[i + 1] > 0.0)
        .map(|i| i as f64 + left[i] as f64 / (left[i] as f64 - left[i + 1] as f64))
        .collect();
    let (first, last) = (crossings[0], crossings[crossings.len() - 1]);
    let periods = (crossings.len() - 1) as f64;
    (RATE as f64 * periods / (last - first), (first + last) / 2.0)
}

#[test]
fn bend_reaches_its_pitch_in_bend_time() {
    // 200 Hz que caen una octava en 200 ms (9600 frames).
    let left = render(
        sine(200.0, 2.0),
        with(|s| {
            s.bend = -12.0;
            s.bend_ms = 200.0;
        }),
        60_000,
    );
    // Lo que toca en el frame `n`: 200 · 2^(−n/9600).
    let theory = |n: f64| 200.0 * 2f64.powf(-(n / 9600.0).min(1.0));
    for n in [600, 2400, 4800, 7200] {
        let (got, center) = hz_around(&left, n, 400);
        assert!(
            (got / theory(center) - 1.0).abs() < 0.003,
            "en {center}, {got} Hz (toca {})",
            theory(center)
        );
    }
    // Al llegar y después, la octava (YIN, sobre un trozo largo).
    let end = detect_mono(&left[10_000..30_000], RATE).unwrap().hz as f64;
    assert!((end / 100.0 - 1.0).abs() < 0.003, "al final, {end} Hz");
    let at = hz_at(&left, 9600 + 1200, 1000);
    assert!((at / 100.0 - 1.0).abs() < 0.01, "en bend_time, {at} Hz");
}

#[test]
fn bend_up_and_with_pitch() {
    // Sube 7 semitonos en 50 ms sobre un paso a ^-12.
    let left = render_step(
        sine(400.0, 2.0),
        with(|s| {
            s.bend = 7.0;
            s.bend_ms = 50.0;
        }),
        Step::new(0).with_pitch(-12.0),
        40_000,
    );
    let end = detect_mono(&left[6000..20_000], RATE).unwrap().hz as f64;
    let want = 200.0 * 2f64.powf(7.0 / 12.0);
    assert!(
        (end / want - 1.0).abs() < 0.003,
        "{end} Hz, se pedían {want}"
    );
}

#[test]
fn bend_changes_how_long_the_slice_lasts() {
    // Al bajar, lee más despacio: el corte dura más. La duración es la
    // integral exacta: ∫ 2^(-t/T) dt hasta leer los 4800 frames.
    let sample = sine(200.0, 0.1);
    let plain = render(sample.clone(), SliceShape::DEFAULT, 30_000);
    let bent = render(
        sample,
        with(|s| {
            s.bend = -12.0;
            s.bend_ms = 50.0;
        }),
        30_000,
    );
    let sounding = |l: &[f32]| l.iter().rposition(|x| *x != 0.0).unwrap() + 1;
    assert_eq!(sounding(&plain), 4799);
    // 2400 frames de curva leen 2400/ln2 · (1 − 1/2) = 1731,2 frames; el
    // resto (3067,8) a media velocidad, 6135,6 frames más.
    let want = 2400.0 + (4799.0 - 2400.0 / std::f64::consts::LN_2 * 0.5) * 2.0;
    let got = sounding(&bent) as f64;
    assert!(
        (got - want).abs() <= 2.0,
        "{got} frames, se esperaban {want}"
    );
}

#[test]
fn rev_reverses_the_samples_exactly() {
    let sample = ramp();
    let rev = render(sample.clone(), with(|s| s.reverse = true), 3000);
    // Igual que `'` en el paso, muestra a muestra.
    let tick = render_step(
        sample.clone(),
        SliceShape::DEFAULT,
        Step::new(0).reversed(),
        3000,
    );
    assert_eq!(rev, tick);
    // Y, fuera de los fundidos de 72 frames, la rampa al revés tal cual.
    for (n, x) in rev.iter().enumerate().take(1900).skip(100) {
        assert_eq!(*x, 0.1 + 0.3 * (1999 - n) as f32 / 2000.0, "frame {n}");
    }
    // `rev` y `'` a la vez: también al revés.
    let both = render_step(
        sample,
        with(|s| s.reverse = true),
        Step::new(0).reversed(),
        3000,
    );
    assert_eq!(both, rev);
}

#[test]
fn the_step_overrides_bend_and_rev() {
    let sample = sine(200.0, 1.0);
    let shape = with(|s| s.bend = -12.0);
    let step = Step::new(0).with_hit(HitShape {
        bend: Some(0.0),
        ..HitShape::default()
    });
    let flat = render_step(sample.clone(), shape, step, 20_000);
    assert_eq!(flat, render(sample.clone(), SliceShape::DEFAULT, 20_000));
    let step = Step::new(0).with_hit(HitShape {
        reverse: Some(true),
        ..HitShape::default()
    });
    assert_eq!(
        render_step(ramp(), SliceShape::DEFAULT, step, 3000),
        render(ramp(), with(|s| s.reverse = true), 3000)
    );
}

#[test]
fn a_curve_moves_bend_hit_by_hit() {
    // De 0 a -12 semitonos a lo largo de 4 pasos: el golpe del paso 0
    // suena sin curva y el del paso 2, con la mitad.
    let sample = sine(200.0, 1.0);
    let mut shape = SliceShape::DEFAULT;
    shape.bend_ms = 1.0;
    shape.curves[engine::SHAPE_BEND] = Some(Curve::Ramp {
        from: 0.0,
        to: -12.0,
        steps: 4.0,
    });
    let slices = Slices::grid(&sample, 1);
    let mut steps = vec![None; 16];
    steps[0] = Some(Step::new(0));
    steps[2] = Some(Step::new(0));
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_shapes(&[shape])
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    let left: Vec<f32> = offline::render(&mut engine, 24_000, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect();
    let first = detect_mono(&left[1000..11_000], RATE).unwrap().hz;
    let second = detect_mono(&left[13_000..23_000], RATE).unwrap().hz;
    assert!((first / 200.0 - 1.0).abs() < 0.003, "{first}");
    assert!(
        (second / (200.0 * 2f32.powf(-0.5)) - 1.0).abs() < 0.003,
        "{second}"
    );
}

#[test]
fn defaults_change_nothing() {
    let sample = sine(220.0, 0.3);
    let plain = render(sample.clone(), SliceShape::DEFAULT, 20_000);
    // Con bend 0, bend_time da igual; rev sin poner, igual.
    let idle = render(
        sample,
        with(|s| {
            s.bend = 0.0;
            s.bend_ms = 900.0;
            s.reverse = false;
        }),
        20_000,
    );
    assert_eq!(plain, idle);
}

#[test]
fn bend_does_not_go_with_grains_at_a_fixed_pitch() {
    let sample = sine(220.0, 0.3);
    let slices = Slices::grid(&sample, 1);
    let mut steps = vec![None; 16];
    steps[0] = Some(Step::new(0).with_kept_pitch(3.0));
    let shape = with(|s| s.bend = -5.0);
    let error = Pattern::from_steps(sample.clone(), &slices, steps)
        .unwrap()
        .with_shapes(&[shape])
        .err()
        .expect("bend con %");
    assert!(error.to_string().contains("bend no va con %"), "{error}");
    let mut steps = vec![None; 16];
    steps[0] = Some(Step::new(0));
    let stretched = with(|s| {
        s.bend = -5.0;
        s.dur = Some(engine::Dur::Ms(500.0));
        s.fit = engine::Fit::Stretch;
    });
    assert!(
        Pattern::from_steps(sample, &slices, steps)
            .unwrap()
            .with_shapes(&[stretched])
            .is_err()
    );
    assert!(with(|s| s.bend = 40.0).validate().is_err());
    assert!(with(|s| s.bend_ms = 0.0).validate().is_err());
}
