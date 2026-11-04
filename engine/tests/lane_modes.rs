//! Lanes con el reloj de la canción: `loop`, `once` y `hold` con `offset`,
//! sin deriva tras miles de ciclos contra un patrón de otra longitud, sin
//! clics en los saltos `step` ni al cambiar la tabla al compás, y con la
//! prioridad de siempre frente al corte por paso (`~`) y a las llaves.

use std::sync::Arc;

use engine::{
    Batch, Curve, Fx, HitShape, Lane, LaneCurve, LaneMode, LanePoint, Lanes, MIX_GAIN, MIX_PARAMS,
    Pattern, PlayMode, STEPS, Sample, SliceShape, Slices, Step, new_engine, offline,
};

const RATE: u32 = 48_000;

fn point(at: f64, value: f32, curve: LaneCurve) -> LanePoint {
    LanePoint { at, value, curve }
}

/// De 0 a 8 en 8 pasos, recta, sobre 16 pasos.
fn ramp(mode: LaneMode, offset: f64) -> Lane {
    Lane::new(
        &[
            point(0.0, 0.0, LaneCurve::Linear),
            point(8.0, 8.0, LaneCurve::Linear),
        ],
        16.0,
        false,
    )
    .unwrap()
    .with_timing(mode, offset, -1.0)
    .unwrap()
}

#[test]
fn loop_once_and_hold_with_offset() {
    let lane = ramp(LaneMode::Loop, 0.0);
    assert_eq!(lane.value(4.0), 4.0);
    assert_eq!(lane.value(20.0), 4.0);
    assert_eq!(lane.value(12.0), 8.0);
    // Con offset, la fase se corre.
    let shifted = ramp(LaneMode::Loop, 2.0);
    assert_eq!(shifted.value(6.0), 4.0);
    assert_eq!(shifted.value(1.0), 8.0);
    // Once: solo dentro de su tramo; fuera, el valor fijo.
    let once = ramp(LaneMode::Once, 4.0);
    assert_eq!(once.value(2.0), -1.0);
    assert_eq!(once.value(8.0), 4.0);
    assert_eq!(once.value(19.9), 8.0);
    assert_eq!(once.value(20.0), -1.0);
    assert_eq!(once.value(500.0), -1.0);
    assert_eq!(once.bounds(), (-1.0, 8.0));
    // Hold: antes, el primero; después, el último.
    let hold = ramp(LaneMode::Hold, 4.0);
    assert_eq!(hold.value(0.0), 0.0);
    assert_eq!(hold.value(8.0), 4.0);
    assert_eq!(hold.value(500.0), 8.0);
    assert_eq!(hold.bounds(), (0.0, 8.0));
    assert!(
        ramp(LaneMode::Loop, 0.0)
            .with_timing(LaneMode::Loop, 5000.0, 0.0)
            .is_err()
    );
}

/// Una señal constante (0.5, un corte en bucle de un patrón de `pattern`
/// pasos que solo golpea en el 0) con el volumen siguiendo la lane 0.
fn engine_with_gain_lane(
    lane: Lane,
    bpm: f64,
    pattern_steps: usize,
) -> (engine::Controller, engine::Engine) {
    let dc = Arc::new(Sample::from_interleaved(&[0.5; 4800], 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&dc, 1);
    let mut steps = vec![None; pattern_steps];
    steps[0] = Some(Step::new(0));
    let (min, max) = lane.bounds();
    let mut curves = [None; MIX_PARAMS];
    curves[MIX_GAIN] = Some(Curve::Lane { index: 0, min, max });
    let pattern = Pattern::from_steps(dc, &slices, steps)
        .unwrap()
        .with_shapes(&[SliceShape {
            play: PlayMode::Loop,
            crossfade_ms: 0.0,
            ..SliceShape::DEFAULT
        }])
        .unwrap()
        .with_mix_automation(curves)
        .unwrap();
    let mut lanes = Lanes::new();
    lanes.set(0, Some(lane)).unwrap();
    let (mut controller, engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_bpm(bpm)
        .set_lanes(lanes)
        .set_pattern(0, Some(pattern));
    controller.apply(batch).unwrap();
    (controller, engine)
}

/// Lo exacto: la posición en pasos en el frame `frame` con compases de
/// `bar` frames enteros (los del transporte) y `fps` frames por paso.
fn exact_steps(frame: u64, bar: u64, fps: f64) -> f64 {
    let k = frame / bar;
    16.0 * k as f64 + (frame - k * bar) as f64 / fps
}

#[test]
fn no_drift_after_thousands_of_cycles() {
    // Una lane de 3 compases contra un patrón de 4, a 170 BPM (un paso no
    // es un número entero de frames: 4235,29...).
    let lane = Lane::new(
        &[
            point(0.0, 0.2, LaneCurve::Linear),
            point(16.0, 1.0, LaneCurve::Smooth),
            point(32.0, 0.5, LaneCurve::Step),
            point(40.0, 0.8, LaneCurve::Linear),
            point(48.0, 0.2, LaneCurve::Linear),
        ],
        48.0,
        false,
    )
    .unwrap();
    let bpm = 170.0;
    let fps = RATE as f64 * 60.0 / (bpm * 4.0);
    let bar = engine::bar_frames(bpm, RATE);
    let (mut controller, mut engine) = engine_with_gain_lane(lane, bpm, 64);
    controller.start().unwrap();
    let cycle = 3 * bar as usize;
    // Puntos de una vuelta de la lane, lejos de los saltos y de los
    // golpes del patrón (cada 4 compases).
    let probes: Vec<usize> = [0.1, 0.27, 0.4, 0.58, 0.79, 0.93]
        .iter()
        .map(|f| (f * cycle as f64) as usize)
        .collect();
    // Miles de vueltas en release (`cargo test -p engine --release --test
    // lane_modes`, ~20 s); en debug, 200 (lo mismo, más corto).
    let cycles = if cfg!(debug_assertions) { 200 } else { 2000 };
    let mut first: Vec<f32> = Vec::new();
    let mut worst_exact = 0.0f32;
    let mut worst_cycle = 0.0f32;
    let mut buf = vec![0.0f32; 2 * cycle];
    for c in 0..cycles {
        engine.process(&mut buf, 2);
        let got: Vec<f32> = probes.iter().map(|&i| buf[2 * i] / 0.5).collect();
        for (&i, &g) in probes.iter().zip(&got) {
            let frame = (c * cycle + i) as u64;
            let want = lane.value(exact_steps(frame, bar, fps));
            worst_exact = worst_exact.max((g - want).abs());
        }
        if c == 4 {
            first = got;
        } else if c > 4 && c % 12 == 4 {
            // La misma vuelta de la lane con el patrón en la misma fase
            // (cada 4 vueltas de la lane): igual que la quinta.
            for (a, b) in first.iter().zip(&got) {
                worst_cycle = worst_cycle.max((a - b).abs());
            }
        }
    }
    // Frente a lo exacto, solo el suavizado (~5 ms) en las rampas; entre
    // la vuelta 5 y la 2000, nada.
    assert!(worst_exact < 0.008, "frente a lo exacto: {worst_exact}");
    assert!(worst_cycle < 1e-5, "entre vueltas: {worst_cycle}");
}

#[test]
fn far_into_the_song_the_phase_is_exact() {
    // Arrancar en el compás 30 000 (10 000 vueltas de 3 compases) es
    // como haber llegado: la lane suena en su sitio.
    let lane = Lane::new(
        &[
            point(0.0, 0.2, LaneCurve::Step),
            point(16.0, 0.6, LaneCurve::Step),
            point(32.0, 1.0, LaneCurve::Step),
        ],
        48.0,
        false,
    )
    .unwrap();
    let bpm = 170.0;
    let bar = engine::bar_frames(bpm, RATE) as usize;
    for start in [30_000u64, 30_001, 30_002, engine::MAX_START_BAR - 1] {
        // Un golpe por compás: suena desde el compás en que arranca.
        let (mut controller, mut engine) = engine_with_gain_lane(lane, bpm, 16);
        controller.start_at(start).unwrap();
        let left: Vec<f32> = offline::render(&mut engine, bar, 2)
            .iter()
            .step_by(2)
            .copied()
            .collect();
        let want = [0.2, 0.6, 1.0][(start % 3) as usize];
        let got = left[bar / 2] / 0.5;
        assert!((got - want).abs() < 1e-4, "compás {start}: {got}");
    }
}

#[test]
fn step_jumps_do_not_click() {
    // Saltos grandes de volumen cada compás: el suavizado (~5 ms) los deja
    // en una rampa.
    let lane = Lane::new(
        &[
            point(0.0, 0.1, LaneCurve::Step),
            point(4.0, 1.0, LaneCurve::Step),
            point(8.0, 0.1, LaneCurve::Step),
            point(12.0, 1.0, LaneCurve::Step),
        ],
        16.0,
        false,
    )
    .unwrap();
    // Un solo golpe (un patrón de 4 compases): el corte no se redispara.
    let (mut controller, mut engine) = engine_with_gain_lane(lane, 120.0, 64);
    controller.start().unwrap();
    let left: Vec<f32> = offline::render(&mut engine, 4 * 96_000 - 500, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect();
    let jump = |l: &[f32]| {
        l.windows(2)
            .skip(200)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0f32, f32::max)
    };
    // El salto de 0.45 en un frame quedaría en 0.45; suavizado, en lo que
    // da un polo de 5 ms (0.45 · (1 − e^(−1/240)) ≈ 0.0019).
    let worst = jump(&left);
    assert!(worst < 0.0025, "salto {worst}");
    // Y la lane sí salta: tras los 5 ms, cada valor en su sitio.
    assert!((left[4 * 6000 + 3000] / 0.5 - 1.0).abs() < 1e-3);
    assert!((left[8 * 6000 + 3000] / 0.5 - 0.1).abs() < 1e-3);

    // Cambiar la tabla al compás (otra lane, otro valor) tampoco salta.
    let other = Lane::new(&[point(0.0, 0.9, LaneCurve::Step)], 16.0, false).unwrap();
    let (mut controller, mut engine) = engine_with_gain_lane(lane, 120.0, 64);
    controller.start().unwrap();
    let mut left: Vec<f32> = offline::render(&mut engine, 48_000, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect();
    let mut lanes = Lanes::new();
    lanes.set(0, Some(other)).unwrap();
    let mut batch = Batch::new();
    batch.set_lanes(lanes);
    controller.apply(batch).unwrap();
    left.extend(
        offline::render(&mut engine, 2 * 96_000, 2)
            .iter()
            .step_by(2)
            .copied(),
    );
    // Hasta el compás, la de antes (en el paso 5, 1.0); después, la nueva.
    assert!((left[30_000] / 0.5 - 1.0).abs() < 1e-3);
    assert!((left[96_000 + 3000] / 0.5 - 0.9).abs() < 1e-3);
    let worst = jump(&left);
    assert!(worst < 0.0025, "salto al cambiar la tabla {worst}");
}

/// Un seno de 440 Hz con un filtro paso bajo cuyo corte sigue `curve` y
/// pasos `steps`.
fn filtered(
    curve: Option<Curve>,
    lanes: Lanes,
    steps: Vec<Option<Step>>,
    frames: usize,
) -> Vec<f32> {
    let mono: Vec<f32> = (0..96_000)
        .map(|i| 0.5 * (std::f32::consts::TAU * 440.0 * i as f32 / RATE as f32).sin())
        .collect();
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    let automation: Vec<engine::Automation> = curve
        .map(|curve| engine::Automation {
            fx: 0,
            param: 0,
            curve,
        })
        .into_iter()
        .collect();
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_fx(&[Fx::lowpass(1000.0)])
        .unwrap()
        .with_automation(&automation)
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_bpm(120.0)
        .set_lanes(lanes)
        .set_pattern(0, Some(pattern));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

#[test]
fn the_step_lock_wins_over_the_lane() {
    // La lane lleva el corte a 300 Hz; el `~` del golpe manda hasta el
    // siguiente golpe (como con `ramp`): suena como un filtro fijo en
    // 5000 Hz.
    let lane = Lane::new(&[point(0.0, 300.0, LaneCurve::Linear)], 16.0, true).unwrap();
    let mut lanes = Lanes::new();
    lanes.set(0, Some(lane)).unwrap();
    let curve = Curve::Lane {
        index: 0,
        min: 300.0,
        max: 300.0,
    };
    let mut locked = vec![None; 64];
    locked[0] = Some(Step::new(0).with_lock(5000.0));
    let with_lane = filtered(Some(curve), lanes, locked.clone(), 40_000);
    let fixed = filtered(None, Lanes::new(), locked, 40_000);
    let diff = with_lane[20_000..]
        .iter()
        .zip(&fixed[20_000..])
        .map(|(a, b)| (a - b).abs())
        .fold(0.0f32, f32::max);
    assert!(diff < 1e-4, "{diff}");
    // Sin `~`, la lane sí baja el corte (el seno sale mucho más flojo).
    let mut plain = vec![None; 64];
    plain[0] = Some(Step::new(0));
    let mut lanes = Lanes::new();
    lanes.set(0, Some(lane)).unwrap();
    let low = filtered(Some(curve), lanes, plain, 40_000);
    let peak = |l: &[f32]| l.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak(&low[20_000..]) < 0.5 * peak(&fixed[20_000..]));
}

#[test]
fn braces_of_a_step_win_over_a_per_hit_lane() {
    // El bend del corte sigue una lane (por golpe); un paso con
    // `{bend=0}` suena sin bend, como si no hubiera lane.
    let tone: Vec<f32> = (0..24_000)
        .map(|i| 0.4 * (std::f32::consts::TAU * 200.0 * i as f32 / RATE as f32).sin())
        .collect();
    let sample = Arc::new(Sample::from_interleaved(&tone, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    let lane = Lane::new(&[point(0.0, -12.0, LaneCurve::Linear)], 16.0, false).unwrap();
    let render = |shape: SliceShape, step: Step| {
        let mut steps = vec![None; STEPS];
        steps[0] = Some(step);
        let pattern = Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_shapes(&[shape])
            .unwrap();
        let mut lanes = Lanes::new();
        lanes.set(0, Some(lane)).unwrap();
        let (mut controller, mut engine) = new_engine(RATE);
        let mut batch = Batch::new();
        batch
            .set_bpm(120.0)
            .set_lanes(lanes)
            .set_pattern(0, Some(pattern));
        controller.apply(batch).unwrap();
        controller.start().unwrap();
        offline::render(&mut engine, 30_000, 2)
    };
    let mut laned = SliceShape::DEFAULT;
    laned.curves[engine::SHAPE_BEND] = Some(Curve::Lane {
        index: 0,
        min: -12.0,
        max: -12.0,
    });
    let overridden = Step::new(0).with_hit(HitShape {
        bend: Some(0.0),
        ..HitShape::default()
    });
    assert_eq!(
        render(laned, overridden),
        render(SliceShape::DEFAULT, Step::new(0))
    );
    // Sin las llaves, el bend de la lane suena.
    assert_ne!(
        render(laned, Step::new(0)),
        render(SliceShape::DEFAULT, Step::new(0))
    );
}
