//! Curvas por golpe: el pitch de la pista, el formante y la forma del corte
//! (`attack`, `release`, `pan`, `length`, `crossfade`) toman el valor de su
//! curva en el momento en que suena cada golpe. Cada golpe suena bit a bit
//! como el mismo golpe con ese valor fijo.

use std::sync::Arc;

use engine::{
    Curve, Pattern, PlayMode, SHAPE_ATTACK, SHAPE_CROSSFADE, SHAPE_LENGTH, SHAPE_PAN, SHAPE_PARAMS,
    SHAPE_RELEASE, Sample, SliceShape, Slices, Step, new_engine, offline,
};

const RATE: u32 = 48_000;
/// Un paso a 120 BPM: 6000 frames exactos (la posición de cada paso es
/// un número entero de pasos).
const STEP: usize = 6000;

/// Dos cortes de 2500 frames (más cortos que un paso: los golpes no se
/// solapan) con 500 de silencio antes y después de cada uno, de un seno
/// con armónicos.
fn sample() -> (Arc<Sample>, Slices) {
    let mut mono = vec![0.0f32; 7000];
    for (k, s) in mono.iter_mut().enumerate() {
        let i = k % 3500;
        if (500..3000).contains(&i) {
            let t = i as f32 / RATE as f32;
            *s = 0.5 * (std::f32::consts::TAU * 330.0 * t).sin()
                + 0.2 * (std::f32::consts::TAU * 1700.0 * t).sin();
        }
    }
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 2);
    (sample, slices)
}

fn render(pattern: Pattern, steps: usize) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, steps * STEP, 2)
}

/// Los frames (estéreo) del paso `k`.
fn segment(audio: &[f32], k: usize) -> &[f32] {
    &audio[2 * k * STEP..2 * (k + 1) * STEP]
}

fn ramp(from: f32, to: f32) -> Curve {
    Curve::Ramp {
        from,
        to,
        steps: 16.0,
    }
}

/// Un golpe del corte 0 en cada paso.
fn every_step() -> Vec<Option<Step>> {
    vec![Some(Step::new(0)); 16]
}

/// Solo el golpe del paso `k`, con `step`.
fn only(k: usize, step: Step) -> Vec<Option<Step>> {
    let mut steps = vec![None; 16];
    steps[k] = Some(step);
    steps
}

#[test]
fn the_track_pitch_curve_transposes_each_hit_by_its_value() {
    let (sample, slices) = sample();
    // Hacia arriba: cada golpe dura menos que un paso (no se solapan).
    let curve = ramp(0.0, 12.0);
    let moving = Pattern::from_steps(sample.clone(), &slices, every_step())
        .unwrap()
        .with_pitch_curve(Some(curve))
        .unwrap();
    let audio = render(moving, 16);
    for k in 0..16 {
        let st = curve.value(k as f64, 0.0, &[0.0; engine::MAX_MACROS], false);
        let fixed = Pattern::from_steps(
            sample.clone(),
            &slices,
            only(k, Step::new(0).with_pitch(st)),
        )
        .unwrap();
        let alone = render(fixed, 16);
        // Los golpes graves duran más que un paso: el siguiente los corta
        // con su fundido. Hasta el siguiente golpe, igual.
        assert_eq!(
            segment(&audio, k),
            segment(&alone, k),
            "golpe {k} ({st} semitonos)"
        );
    }
}

#[test]
fn a_pitch_curve_adds_to_the_step_pitch_and_stays_within_36() {
    let (sample, slices) = sample();
    let pattern = |pitch: f32| {
        Pattern::from_steps(
            sample.clone(),
            &slices,
            only(0, Step::new(0).with_pitch(pitch)),
        )
        .unwrap()
    };
    let flat = |st: f32| Curve::Ramp {
        from: st,
        to: st,
        steps: 1.0,
    };
    // 5 del paso y 7 de la pista: como 12.
    let summed = render(pattern(5.0).with_pitch_curve(Some(flat(7.0))).unwrap(), 1);
    assert_eq!(summed, render(pattern(12.0), 1));
    // 30 y 30: se queda en 36.
    let capped = render(pattern(30.0).with_pitch_curve(Some(flat(30.0))).unwrap(), 1);
    assert_eq!(capped, render(pattern(36.0), 1));
    // Con 0, bit a bit lo de siempre.
    let zero = render(pattern(3.0).with_pitch_curve(Some(flat(0.0))).unwrap(), 1);
    assert_eq!(zero, render(pattern(3.0), 1));
    assert!(
        pattern(0.0)
            .with_pitch_curve(Some(ramp(0.0, 40.0)))
            .is_err()
    );
}

/// La forma con una curva en `param` y el resto por defecto.
fn shape_with(param: usize, curve: Curve) -> SliceShape {
    let mut curves = [None; SHAPE_PARAMS];
    curves[param] = Some(curve);
    SliceShape {
        curves,
        ..SliceShape::DEFAULT
    }
}

/// La forma fija con el valor `value` en `param`.
fn shape_fixed(param: usize, value: f32, base: SliceShape) -> SliceShape {
    let mut shape = SliceShape {
        curves: [None; SHAPE_PARAMS],
        ..base
    };
    match param {
        SHAPE_ATTACK => shape.attack_ms = value,
        SHAPE_RELEASE => shape.release_ms = value,
        SHAPE_PAN => shape.pan = value,
        SHAPE_LENGTH => shape.length = value,
        _ => shape.crossfade_ms = value,
    }
    shape
}

#[test]
fn shape_curves_give_each_hit_its_value() {
    let (sample, slices) = sample();
    let cases = [
        (SHAPE_ATTACK, "attack", ramp(0.0, 40.0), SliceShape::DEFAULT),
        (
            SHAPE_RELEASE,
            "release",
            ramp(0.0, 40.0),
            SliceShape::DEFAULT,
        ),
        (
            SHAPE_PAN,
            "pan",
            Curve::Lfo {
                min: -1.0,
                max: 1.0,
                rate: engine::LfoRate::Steps(8.0),
            },
            SliceShape::DEFAULT,
        ),
        (SHAPE_LENGTH, "length", ramp(1.0, 0.1), SliceShape::DEFAULT),
        (
            SHAPE_CROSSFADE,
            "crossfade",
            ramp(0.0, 20.0),
            // En bucle, con la puerta al acabar el paso (si no, el bucle
            // seguiría hasta el siguiente golpe igual).
            SliceShape {
                play: PlayMode::Loop,
                length: 0.4,
                ..SliceShape::DEFAULT
            },
        ),
    ];
    // Los golpes alternan los dos cortes (iguales), cada uno con la curva:
    // así, el golpe `k` y la cola del `k - 1` se pueden comparar con los dos
    // golpes solos, cada corte con su valor fijo.
    let alternate: Vec<Option<Step>> = (0..16).map(|j| Some(Step::new(j % 2))).collect();
    for (param, what, curve, base) in cases {
        let shape = SliceShape {
            curves: shape_with(param, curve).curves,
            ..base
        };
        let moving = Pattern::from_steps(sample.clone(), &slices, alternate.clone())
            .unwrap()
            .with_shapes(&[shape, shape])
            .unwrap()
            .with_gate(true);
        let audio = render(moving, 16);
        let value = |k: usize| {
            curve.value(
                k as f64,
                k as f64 * STEP as f64 / RATE as f64,
                &[0.0; engine::MAX_MACROS],
                false,
            )
        };
        let mut differ = 0;
        for k in 0..16 {
            let mut steps = vec![None; 16];
            let mut shapes = [base; 2];
            steps[k] = Some(Step::new(k % 2));
            shapes[k % 2] = shape_fixed(param, value(k), base);
            if k > 0 {
                steps[k - 1] = Some(Step::new((k - 1) % 2));
                shapes[(k - 1) % 2] = shape_fixed(param, value(k - 1), base);
            }
            let fixed = Pattern::from_steps(sample.clone(), &slices, steps)
                .unwrap()
                .with_shapes(&shapes)
                .unwrap()
                .with_gate(true);
            let alone = render(fixed, 16);
            let (a, b) = (segment(&audio, k), segment(&alone, k));
            if let Some(i) = a.iter().zip(b).position(|(x, y)| x != y) {
                panic!(
                    "{what}: golpe {k} ({}): distinto desde el frame {} ({} y {})",
                    value(k),
                    i / 2,
                    a[i],
                    b[i]
                );
            }
            differ += usize::from(segment(&audio, k) != segment(&audio, 0));
        }
        assert!(differ > 8, "{what}: la curva casi no cambia nada");
    }
}

#[test]
fn shape_curves_are_checked() {
    let bad = shape_with(SHAPE_PAN, ramp(-2.0, 0.0));
    assert!(bad.validate().is_err());
    assert!(shape_with(SHAPE_LENGTH, ramp(0.0, 1.0)).validate().is_err());
    assert!(shape_with(SHAPE_LENGTH, ramp(0.01, 1.0)).validate().is_ok());
    assert!(
        shape_with(SHAPE_RELEASE, ramp(0.0, 5000.0))
            .validate()
            .is_ok()
    );
}

#[test]
fn the_formant_curve_gives_each_kept_hit_its_value() {
    let (sample, slices) = sample();
    let curve = ramp(-6.0, 6.0);
    let kept = Step::new(0).with_kept_pitch(7.0);
    let moving = Pattern::from_steps(sample.clone(), &slices, vec![Some(kept); 16])
        .unwrap()
        .with_formant(Some(0.0))
        .unwrap()
        .with_formant_curve(Some(curve))
        .unwrap();
    let audio = render(moving, 16);
    for k in [0, 3, 8, 15] {
        let value = curve.value(k as f64, 0.0, &[0.0; engine::MAX_MACROS], false);
        let fixed = Pattern::from_steps(sample.clone(), &slices, only(k, kept))
            .unwrap()
            .with_formant(Some(value))
            .unwrap();
        assert_eq!(
            segment(&audio, k),
            segment(&render(fixed, 16), k),
            "golpe {k} ({value})"
        );
    }
    assert!(segment(&audio, 2) != segment(&audio, 12));
}
