//! Todo lo automatizable se mueve sin clics: cada parámetro de cada efecto,
//! el volumen, el pan y los envíos de la pista y los parámetros de los
//! buses, con una rampa que recorre su rango entero mientras suena un tono.
//!
//! «Sin clics»: el mayor salto entre dos muestras seguidas con la rampa no
//! pasa (con un margen) del mayor salto del mismo efecto fijo en alguno de
//! los valores por los que pasa. Un clic es un escalón que el efecto no
//! hace en ninguno de sus ajustes.

use std::sync::Arc;

use engine::bus::{BUS_PARAMS, DELAY_FEEDBACK, REVERB_DAMP, REVERB_LEVEL, REVERB_SIZE};
use engine::{
    Automation, Buses, Curve, DynEqParams, EqBand, Fx, MIX_GAIN, MIX_PAN, MIX_PARAMS, MIX_SEND,
    MixCurves, Pattern, Sample, Slices, Step, new_engine, offline,
};

const RATE: u32 = 48_000;
/// Dos compases a 120 BPM.
const FRAMES: usize = 2 * 16 * 6000;
const STEPS: f64 = 32.0;
/// Valores fijos con los que se compara cada rampa (más uno).
const POINTS: usize = 32;

/// Un tono largo (más que el render) con graves y agudos, para que todos
/// los efectos tengan algo que tocar, y un poco de silencio al final.
fn tone() -> (Arc<Sample>, Slices) {
    let len = FRAMES + RATE as usize;
    let mono: Vec<f32> = (0..len)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            // Con trémolo (de 0,2 a 1, 3 veces por segundo): así el ataque y
            // el release de los compresores también se oyen.
            let tremolo = 0.6 - 0.4 * (std::f32::consts::TAU * 3.0 * t).cos();
            tremolo
                * (0.6 * (std::f32::consts::TAU * 110.0 * t).sin()
                    + 0.2 * (std::f32::consts::TAU * 2500.0 * t).sin())
        })
        .collect();
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    (sample, slices)
}

/// Un solo golpe al empezar, que suena todo el render.
fn one_hit() -> Vec<Option<Step>> {
    let mut steps = vec![None; 16];
    steps[0] = Some(Step::new(0));
    steps
}

/// El render (canal izquierdo y derecho) de `pattern` con `buses`.
fn play(pattern: Pattern, buses: Buses) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = engine::Batch::new();
    batch.set_buses(buses).set_pattern(0, Some(pattern));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, FRAMES, 2)
}

/// Mayor salto entre muestras seguidas de cada canal (sin el primer
/// fundido de entrada).
fn max_jump(audio: &[f32]) -> f32 {
    (0..2)
        .map(|c| {
            let ch: Vec<f32> = audio.iter().skip(c).step_by(2).copied().collect();
            ch[200..]
                .windows(2)
                .map(|w| (w[1] - w[0]).abs())
                .fold(0.0, f32::max)
        })
        .fold(0.0, f32::max)
}

/// De `lo` a `hi` en `k` de `n` partes (en escala logarítmica con `log`).
fn between(lo: f32, hi: f32, k: usize, n: usize, log: bool) -> f32 {
    let t = k as f32 / n as f32;
    let v = if log {
        (lo.ln() + t * (hi.ln() - lo.ln())).exp()
    } else {
        lo + t * (hi - lo)
    };
    v.clamp(lo, hi)
}

/// Comprueba que la rampa de `lo` a `hi` (en el render entero) cambia el
/// audio y no hace clics. `make(curve)` da el patrón y los buses con esa
/// curva en el parámetro.
fn ramps_without_clicks(
    what: &str,
    (lo, hi, log): (f32, f32, bool),
    make: impl Fn(Curve) -> (Pattern, Buses),
) {
    let constant = |v: f32| Curve::Ramp {
        from: v,
        to: v,
        steps: STEPS,
    };
    let fixed: Vec<f32> = (0..=POINTS)
        .map(|k| {
            let (pattern, buses) = make(constant(between(lo, hi, k, POINTS, log)));
            max_jump(&play(pattern, buses))
        })
        .collect();
    let ceiling = fixed.iter().copied().fold(0.0, f32::max);
    let (pattern, buses) = make(Curve::Ramp {
        from: lo,
        to: hi,
        steps: STEPS,
    });
    let ramp = play(pattern, buses);
    let jump = max_jump(&ramp);
    assert!(
        jump <= 1.2 * ceiling + 0.01,
        "{what}: salto de {jump} con la rampa, {ceiling} como mucho fijo ({fixed:?})"
    );
    let (pattern, buses) = make(constant(lo));
    assert!(
        ramp != play(pattern, buses),
        "{what}: la rampa no cambia nada"
    );
}

/// Un efecto de cada tipo, con valores que dejan oír sus parámetros.
fn effects() -> Vec<(&'static str, Fx)> {
    vec![
        ("lowpass", Fx::lowpass(1000.0)),
        ("bitcrush", Fx::bitcrush(8, 1)),
        ("compressor", Fx::compressor(-30.0, 4.0)),
        ("distortion", Fx::distortion(4.0, 1.0)),
        ("gain", Fx::gain(1.0)),
        (
            "eq",
            Fx::eq(&[
                EqBand::bell(110.0, 6.0, 1.0),
                EqBand::bell(500.0, -6.0, 1.0),
                EqBand::bell(2500.0, 6.0, 2.0),
                EqBand::bell(8000.0, -6.0, 1.0),
            ]),
        ),
        (
            "dyneq",
            Fx::DynEq(DynEqParams {
                threshold_db: -40.0,
                ..DynEqParams::new(110.0)
            }),
        ),
    ]
}

#[test]
fn every_effect_parameter_ramps_without_clicks() {
    let (sample, slices) = tone();
    for (name, fx) in effects() {
        for (param, key) in fx.params().iter().enumerate() {
            let range = fx.param_range(param).unwrap();
            ramps_without_clicks(&format!("{name}.{key}"), range, |curve| {
                let pattern = Pattern::from_steps(sample.clone(), &slices, one_hit())
                    .unwrap()
                    .with_fx(&[fx])
                    .unwrap()
                    .with_automation(&[Automation {
                        fx: 0,
                        param,
                        curve,
                    }])
                    .unwrap();
                (pattern, Buses::default())
            });
        }
    }
}

#[test]
fn the_new_parameters_are_automatable() {
    let names = |fx: Fx| fx.params().to_vec();
    assert_eq!(
        names(Fx::compressor(-18.0, 4.0)),
        ["threshold", "makeup", "ratio", "attack", "release"]
    );
    assert_eq!(names(Fx::bitcrush(8, 1)), ["bits", "downsample"]);
    assert_eq!(
        names(Fx::DynEq(DynEqParams::new(100.0)))[4..],
        ["ratio", "attack", "release"]
    );
}

#[test]
fn track_gain_pan_and_sends_ramp_without_clicks() {
    let (sample, slices) = tone();
    let ranges: [(usize, &str, (f32, f32, bool)); MIX_PARAMS] = [
        (MIX_GAIN, "gain", (0.0, 4.0, false)),
        (MIX_PAN, "pan", (-1.0, 1.0, false)),
        (MIX_SEND, "send delay", (0.0, 1.0, false)),
        (MIX_SEND + 1, "send reverb", (0.0, 1.0, false)),
    ];
    for (param, what, range) in ranges {
        ramps_without_clicks(what, range, |curve| {
            let mut curves: MixCurves = [None; MIX_PARAMS];
            curves[param] = Some(curve);
            let pattern = Pattern::from_steps(sample.clone(), &slices, one_hit())
                .unwrap()
                .with_gain(0.5)
                .unwrap()
                .with_mix_automation(curves)
                .unwrap();
            (pattern, Buses::default())
        });
    }
}

#[test]
fn bus_parameters_ramp_without_clicks() {
    let (sample, slices) = tone();
    let ranges = [
        (DELAY_FEEDBACK, "feedback", (0.0, 0.95, false)),
        (engine::bus::DELAY_LEVEL, "delay level", (0.0, 1.0, false)),
        (REVERB_SIZE, "size", (0.0, 1.0, false)),
        (REVERB_DAMP, "damp", (0.0, 1.0, false)),
        (REVERB_LEVEL, "reverb level", (0.0, 1.0, false)),
    ];
    for (param, what, range) in ranges {
        ramps_without_clicks(what, range, |curve| {
            let pattern = Pattern::from_steps(sample.clone(), &slices, one_hit())
                .unwrap()
                .with_gain(0.5)
                .unwrap()
                .with_sends([0.5, 0.5])
                .unwrap();
            let mut buses = Buses::default();
            buses.curves[param] = Some(curve);
            (pattern, buses)
        });
    }
}

#[test]
fn a_track_pan_moves_the_sound_and_centre_changes_nothing() {
    let (sample, slices) = tone();
    let pattern = |pan: f32| {
        Pattern::from_steps(sample.clone(), &slices, one_hit())
            .unwrap()
            .with_pan(pan)
            .unwrap()
    };
    let centre = play(pattern(0.0), Buses::default());
    let plain = play(
        Pattern::from_steps(sample.clone(), &slices, one_hit()).unwrap(),
        Buses::default(),
    );
    assert_eq!(centre, plain);
    // A la izquierda: el derecho baja a la mitad y el izquierdo no cambia.
    let left = play(pattern(-0.5), Buses::default());
    for (frame, (l, c)) in left.chunks(2).zip(centre.chunks(2)).enumerate().skip(2000) {
        assert_eq!(l[0], c[0], "frame {frame}");
        assert!((l[1] - 0.5 * c[1]).abs() < 1e-6, "frame {frame}");
    }
}

#[test]
fn curves_out_of_range_are_rejected() {
    let (sample, slices) = tone();
    let pattern = || Pattern::from_steps(sample.clone(), &slices, one_hit()).unwrap();
    let ramp = |from, to| {
        Some(Curve::Ramp {
            from,
            to,
            steps: 16.0,
        })
    };
    let mut curves: MixCurves = [None; MIX_PARAMS];
    curves[MIX_PAN] = ramp(-2.0, 0.0);
    assert!(pattern().with_mix_automation(curves).is_err());
    curves[MIX_PAN] = ramp(-1.0, 1.0);
    assert!(pattern().with_mix_automation(curves).is_ok());
    assert!(pattern().with_pan(1.5).is_err());
    let mut buses = Buses::default();
    buses.curves[DELAY_FEEDBACK] = ramp(0.0, 1.0);
    assert!(buses.validate().is_err());
    buses.curves[DELAY_FEEDBACK] = ramp(0.0, 0.9);
    assert!(buses.validate().is_ok());
    assert_eq!(BUS_PARAMS, 5);
}
