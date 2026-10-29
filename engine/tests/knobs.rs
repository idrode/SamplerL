//! Mandos MIDI (`engine::knob`): un mando mueve el volumen, el pan o un
//! envío de una pista, un parámetro de un efecto, de los buses o de un
//! sintetizador, entre su mínimo y su máximo; la tabla entra con el lote al
//! compás y el valor ya, suavizado y sin clics; sobre una curva manda
//! mientras se mueve y la devuelve al soltarlo, sobre un valor fijo se
//! queda. Sin mandos, todo igual.

use std::sync::Arc;

use engine::{
    Batch, Curve, Fx, KnobCurve, KnobMap, KnobTarget, Knobs, MAX_KNOBS, MIX_GAIN, MIX_PARAMS,
    Pattern, STEPS, Sample, Slices, Step, bar_frames, new_engine, offline,
};

const RATE: u32 = 48_000;
const BPM: f64 = 120.0;
/// Un compás a 120 BPM.
fn bar() -> usize {
    bar_frames(BPM, RATE) as usize
}

/// Un tono constante a 0,25 de un compás, que vuelve a sonar cada compás
/// sin fundidos (`with_raw`): el volumen de la pista se lee en la salida,
/// sin escalones de por medio.
fn dc() -> Pattern {
    let mono = vec![0.25f32; bar()];
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    let mut steps = vec![None; STEPS];
    steps[0] = Some(Step::new(0));
    Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_raw()
}

/// Un seno de 4 kHz de 8 compases.
fn sine() -> Pattern {
    let mono: Vec<f32> = (0..8 * bar())
        .map(|i| 0.5 * (std::f32::consts::TAU * 4000.0 * i as f32 / RATE as f32).sin())
        .collect();
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    let mut steps = vec![None; STEPS];
    steps[0] = Some(Step::new(0));
    Pattern::from_steps(sample, &slices, steps).unwrap()
}

fn one(map: KnobMap) -> Knobs {
    let mut knobs = [None; MAX_KNOBS];
    knobs[3] = Some(map);
    knobs
}

fn gain_knob(latch: bool) -> Knobs {
    one(KnobMap {
        target: KnobTarget::Mix {
            track: 0,
            index: MIX_GAIN as u8,
        },
        min: 0.0,
        max: 1.0,
        curve: KnobCurve::Linear,
        latch,
    })
}

fn left(x: &[f32]) -> Vec<f32> {
    x.iter().step_by(2).copied().collect()
}

/// El salto más grande entre dos muestras seguidas.
fn max_step(x: &[f32]) -> f32 {
    x.windows(2).fold(0.0f32, |m, w| m.max((w[1] - w[0]).abs()))
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()
}

#[test]
fn a_knob_moves_the_track_gain_smoothly_and_stays_on_a_fixed_value() {
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_bpm(BPM)
        .set_pattern(0, Some(dc()))
        .set_knobs(gain_knob(true));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let before = left(&offline::render(&mut engine, 4800, 2));
    // Sin moverlo, no manda: el volumen fijo (1).
    assert!((before[4000] - 0.25).abs() < 1e-6);
    controller.move_knob(3, 0.5).unwrap();
    let moving = left(&offline::render(&mut engine, 48_000, 2));
    // A la mitad, sin escalones (el CC de 7 bits llega suavizado).
    assert!((moving[47_000] - 0.125).abs() < 1e-4, "{}", moving[47_000]);
    assert!(max_step(&moving) < 0.002, "{}", max_step(&moving));
    // Sobre un valor fijo, se queda aunque no se mueva (más de 1,5 s).
    let later = left(&offline::render(&mut engine, 3 * 48_000, 2));
    assert!((later.last().unwrap() - 0.125).abs() < 1e-4);
}

#[test]
fn over_a_curve_it_takes_over_while_moving_and_gives_it_back() {
    // El volumen sigue una rampa fija en 1 (un valor con curva).
    let mut curves = [None; MIX_PARAMS];
    curves[MIX_GAIN] = Some(Curve::Ramp {
        from: 1.0,
        to: 1.0,
        steps: 16.0,
    });
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_bpm(BPM)
        .set_pattern(0, Some(dc().with_mix_automation(curves).unwrap()))
        .set_knobs(gain_knob(false));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, 4800, 2);
    controller.move_knob(3, 0.0).unwrap();
    let held = left(&offline::render(&mut engine, 24_000, 2));
    assert!(
        held[20_000].abs() < 1e-4,
        "manda el mando: {}",
        held[20_000]
    );
    // Quieto 1,5 s y medio segundo de vuelta: otra vez la curva (1).
    let back = left(&offline::render(&mut engine, 2 * 48_000 + 4800, 2));
    assert!(
        (back.last().unwrap() - 0.25).abs() < 1e-4,
        "{}",
        back.last().unwrap()
    );
    let worst = back
        .windows(2)
        .enumerate()
        .max_by(|a, b| (a.1[1] - a.1[0]).abs().total_cmp(&(b.1[1] - b.1[0]).abs()))
        .unwrap();
    assert!(max_step(&back) < 0.002, "{worst:?}");
}

#[test]
fn the_table_enters_with_the_bar_and_the_value_at_once() {
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch.set_bpm(BPM).set_pattern(0, Some(dc()));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, bar() / 2, 2);
    // La tabla, a mitad de compás: hasta el siguiente, nada.
    let mut batch = Batch::new();
    batch.set_knobs(gain_knob(true));
    controller.apply(batch).unwrap();
    controller.move_knob(3, 0.0).unwrap();
    let rest = left(&offline::render(&mut engine, bar() / 2, 2));
    assert!(rest.iter().skip(100).all(|v| (v - 0.25).abs() < 1e-6));
    let next = left(&offline::render(&mut engine, 9600, 2));
    assert!(next[9000].abs() < 1e-3, "{}", next[9000]);
}

#[test]
fn a_knob_moves_an_effect_a_bus_and_a_synth() {
    // El corte del paso bajo, en escala logarítmica de 200 a 8000 Hz: abajo
    // del todo, el seno de 4 kHz casi desaparece.
    let lowpass = sine().with_fx(&[Fx::lowpass(8000.0)]).unwrap();
    let knob = one(KnobMap {
        target: KnobTarget::Fx {
            track: 0,
            slot: 0,
            param: 0,
        },
        min: 200.0,
        max: 8000.0,
        curve: KnobCurve::Log,
        latch: true,
    });
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_bpm(BPM)
        .set_pattern(0, Some(lowpass))
        .set_knobs(knob);
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let open = left(&offline::render(&mut engine, 4800, 2));
    controller.move_knob(3, 0.0).unwrap();
    let closed = left(&offline::render(&mut engine, 24_000, 2));
    assert!(rms(&closed[19_000..]) < 0.05 * rms(&open[1000..]));

    // El nivel del bus de reverb: con el mando a 0, no devuelve nada (la
    // mezcla es la de sin envío).
    let bus = |send: f32, knob: Option<f32>| {
        let pattern = sine().with_sends([0.0, send]).unwrap();
        let (mut controller, mut engine) = new_engine(RATE);
        let mut batch = Batch::new();
        batch
            .set_bpm(BPM)
            .set_pattern(0, Some(pattern))
            .set_knobs(one(KnobMap {
                target: KnobTarget::Bus {
                    param: engine::bus::REVERB_LEVEL as u8,
                },
                min: 0.0,
                max: 1.0,
                curve: KnobCurve::Linear,
                latch: true,
            }));
        controller.apply(batch).unwrap();
        controller.start().unwrap();
        if let Some(v) = knob {
            controller.move_knob(3, v).unwrap();
        }
        left(&offline::render(&mut engine, 24_000, 2))
    };
    let dry = bus(0.0, None);
    let wet = bus(1.0, None);
    let cut = bus(1.0, Some(0.0));
    let diff = |a: &[f32], b: &[f32]| -> f32 {
        let d: Vec<f32> = a[12_000..]
            .iter()
            .zip(&b[12_000..])
            .map(|(x, y)| x - y)
            .collect();
        rms(&d)
    };
    assert!(diff(&wet, &dry) > 0.01, "{}", diff(&wet, &dry));
    assert!(diff(&cut, &dry) < 1e-4, "{}", diff(&cut, &dry));

    // El corte de un sintetizador: más cerrado, menos agudos.
    let acid = |knob: Option<f32>| {
        let mut steps = vec![None; STEPS];
        steps[0] = Some(Step::new(0).with_pitch(-12.0).with_hold(15));
        let silence = Arc::new(Sample::silence(RATE));
        let pattern = Pattern::from_steps(silence.clone(), &Slices::grid(&silence, 1), steps)
            .unwrap()
            .with_synth(
                Some(engine::synth::acid::patch(48.0)),
                [None; engine::synth::PARAM_COUNT],
            )
            .unwrap();
        let cutoff = engine::synth::CUTOFF as u8;
        let (mut controller, mut engine) = new_engine(RATE);
        let mut batch = Batch::new();
        batch
            .set_bpm(BPM)
            .set_pattern(0, Some(pattern))
            .set_knobs(one(KnobMap {
                target: KnobTarget::Synth {
                    track: 0,
                    param: cutoff,
                },
                min: 60.0,
                max: 12_000.0,
                curve: KnobCurve::Log,
                latch: true,
            }));
        controller.apply(batch).unwrap();
        controller.start().unwrap();
        if let Some(v) = knob {
            controller.move_knob(3, v).unwrap();
        }
        let out = left(&offline::render(&mut engine, 24_000, 2));
        // Lo que cambia de una muestra a otra: cuanto más abierto, más.
        out[12_000..]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .sum::<f32>()
    };
    let (closed, open) = (acid(Some(0.0)), acid(Some(1.0)));
    assert!(closed < 0.5 * open, "{closed} {open}");
}

#[test]
fn without_knobs_nothing_changes() {
    let render = |knobs: bool| {
        let (mut controller, mut engine) = new_engine(RATE);
        let mut batch = Batch::new();
        batch
            .set_bpm(BPM)
            .set_pattern(0, Some(sine().with_fx(&[Fx::lowpass(3000.0)]).unwrap()));
        if knobs {
            // Una tabla asignada que nadie mueve.
            batch.set_knobs(gain_knob(true));
        }
        controller.apply(batch).unwrap();
        controller.start().unwrap();
        offline::render(&mut engine, 2 * bar(), 2)
    };
    assert_eq!(render(false), render(true));
}

#[test]
fn a_target_that_does_not_exist_is_rejected() {
    let (mut controller, _engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch.set_knobs(one(KnobMap {
        target: KnobTarget::Fx {
            track: 9,
            slot: 0,
            param: 0,
        },
        min: 0.0,
        max: 1.0,
        curve: KnobCurve::Linear,
        latch: true,
    }));
    assert!(controller.apply(batch).is_err());
    assert!(controller.move_knob(MAX_KNOBS, 0.5).is_err());
    assert!(controller.move_knob(0, 1.5).is_err());
}
