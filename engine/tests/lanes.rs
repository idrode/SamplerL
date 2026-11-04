//! Lanes de automatización: las cuatro curvas dan lo esperado en los
//! puntos y entre ellos, la validación rechaza lo que no vale y una lane
//! entra por el lote al compás y mueve el parámetro que la sigue.

use std::sync::Arc;

use engine::{
    Batch, Curve, Lane, LaneCurve, LanePoint, Lanes, MIX_GAIN, MIX_PARAMS, Pattern, PlayMode,
    Sample, SliceShape, Slices, Step, new_engine, offline,
};

const RATE: u32 = 48_000;

fn point(at: f64, value: f32, curve: LaneCurve) -> LanePoint {
    LanePoint { at, value, curve }
}

fn two(curve: LaneCurve, log: bool) -> Lane {
    Lane::new(
        &[
            point(0.0, 100.0, curve),
            point(16.0, 1600.0, LaneCurve::Linear),
        ],
        32.0,
        log,
    )
    .unwrap()
}

#[test]
fn the_four_curves_at_the_points_and_between() {
    // En los puntos, su valor exacto con cualquier curva.
    for curve in [
        LaneCurve::Linear,
        LaneCurve::Exp,
        LaneCurve::Smooth,
        LaneCurve::Step,
    ] {
        let lane = two(curve, false);
        assert_eq!(lane.at(0.0), 100.0, "{curve:?}");
        assert_eq!(lane.at(16.0), 1600.0, "{curve:?}");
        // Después del último punto, el último.
        assert_eq!(lane.at(30.0), 1600.0, "{curve:?}");
    }
    let near = |a: f32, b: f32| (a - b).abs() < 1e-3 * b.abs().max(1.0);
    // Recta: a un cuarto, un cuarto del camino.
    assert!(near(two(LaneCurve::Linear, false).at(4.0), 475.0));
    // Exponencial: a mitad, la media geométrica (400); a un cuarto, 200.
    assert!(near(two(LaneCurve::Exp, false).at(8.0), 400.0));
    assert!(near(two(LaneCurve::Exp, false).at(4.0), 200.0));
    // Suave: smoothstep(0.25) = 0.15625; a mitad, la mitad.
    assert!(near(
        two(LaneCurve::Smooth, false).at(4.0),
        100.0 + 0.15625 * 1500.0
    ));
    assert!(near(two(LaneCurve::Smooth, false).at(8.0), 850.0));
    // Salto: se queda en el suyo hasta el siguiente punto.
    assert_eq!(two(LaneCurve::Step, false).at(15.99), 100.0);
    // En escala log, la recta va por octavas (como exp).
    assert!(near(two(LaneCurve::Linear, true).at(8.0), 400.0));
    // Exp con un extremo en 0 o negativo: recta.
    let lane = Lane::new(
        &[
            point(0.0, -1.0, LaneCurve::Exp),
            point(4.0, 1.0, LaneCurve::Linear),
        ],
        4.0,
        false,
    )
    .unwrap();
    assert!(near(lane.at(1.0), -0.5));
    // Antes del primer punto, el del primero.
    let late = Lane::new(&[point(8.0, 3.0, LaneCurve::Linear)], 16.0, false).unwrap();
    assert_eq!(late.at(2.0), 3.0);
    // Se repite cada `length` pasos.
    let lane = two(LaneCurve::Linear, false);
    assert_eq!(lane.value(32.0 + 4.0), lane.at(4.0));
    assert_eq!(lane.value(-28.0), lane.at(4.0));
}

#[test]
fn lanes_are_checked() {
    let p = |at| point(at, 1.0, LaneCurve::Linear);
    assert!(Lane::new(&[], 16.0, false).is_err());
    assert!(Lane::new(&[p(0.0)], 0.0, false).is_err());
    assert!(Lane::new(&[p(0.0)], 2000.0, false).is_err());
    assert!(Lane::new(&[p(4.0), p(2.0)], 16.0, false).is_err());
    assert!(Lane::new(&[p(4.0), p(4.0)], 16.0, false).is_err());
    assert!(Lane::new(&[p(20.0)], 16.0, false).is_err());
    let too_many: Vec<LanePoint> = (0..65).map(|i| p(i as f64)).collect();
    assert!(Lane::new(&too_many, 100.0, false).is_err());
    let zero = point(0.0, 0.0, LaneCurve::Linear);
    assert!(Lane::new(&[zero], 16.0, true).is_err());
    let mut lanes = Lanes::new();
    assert!(
        lanes
            .set(
                engine::MAX_LANES,
                Some(Lane::new(&[p(0.0)], 1.0, false).unwrap())
            )
            .is_err()
    );
    assert_eq!(two(LaneCurve::Linear, false).bounds(), (100.0, 1600.0));
}

/// Una señal constante que suena todo el rato (un corte en bucle) con el
/// volumen de la pista siguiendo `gain`; devuelve el canal izquierdo.
fn render_gain(gain: Option<Curve>, lanes: Option<Lanes>, frames: usize) -> Vec<f32> {
    let dc = Arc::new(Sample::from_interleaved(&[0.5; 4800], 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&dc, 1);
    let mut steps = vec![None; 16];
    steps[0] = Some(Step::new(0));
    let mut curves = [None; MIX_PARAMS];
    curves[MIX_GAIN] = gain;
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
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch.set_bpm(120.0).set_pattern(0, Some(pattern));
    if let Some(lanes) = lanes {
        batch.set_lanes(lanes);
    }
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

#[test]
fn a_lane_moves_its_parameter_in_the_engine() {
    // El volumen de 0.2 a 1 en un compás (16 pasos) y vuelta a 0.2 en el
    // siguiente; a 120 BPM un paso son 6000 frames.
    let lane = Lane::new(
        &[
            point(0.0, 0.2, LaneCurve::Linear),
            point(16.0, 1.0, LaneCurve::Linear),
            point(32.0, 0.2, LaneCurve::Linear),
        ],
        32.0,
        false,
    )
    .unwrap();
    let mut lanes = Lanes::new();
    lanes.set(3, Some(lane)).unwrap();
    let curve = Curve::Lane {
        index: 3,
        min: 0.2,
        max: 1.0,
    };
    let left = render_gain(Some(curve), Some(lanes), 3 * 96_000);
    for step in [4usize, 8, 12, 20, 28, 36] {
        let frame = step * 6000;
        let want = 0.5 * lane.value(step as f64);
        // El volumen se suaviza (~5 ms): en una rampa va un poco detrás.
        assert!(
            (left[frame] - want).abs() < 0.004,
            "paso {step}: {} (se esperaba {want})",
            left[frame]
        );
    }
}

#[test]
fn without_the_lane_in_the_table_the_curve_stays_at_its_minimum() {
    // Una curva que apunta a un hueco vacío no inventa nada: su mínimo.
    let curve = Curve::Lane {
        index: 5,
        min: 0.5,
        max: 1.0,
    };
    let left = render_gain(Some(curve), Some(Lanes::new()), 20_000);
    assert!((left[19_000] - 0.25).abs() < 1e-4, "{}", left[19_000]);
    // Y una tabla vacía no cambia nada: igual que sin lanes, bit a bit.
    assert_eq!(
        render_gain(None, Some(Lanes::new()), 20_000),
        render_gain(None, None, 20_000)
    );
    // Una curva a una lane que no existe en la tabla del motor, error.
    let bad = Curve::Lane {
        index: 40,
        min: 0.0,
        max: 1.0,
    };
    let mut curves = [None; MIX_PARAMS];
    curves[MIX_GAIN] = Some(bad);
    let dc = Arc::new(Sample::from_interleaved(&[0.5; 100], 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&dc, 1);
    assert!(
        Pattern::from_steps(dc, &slices, vec![Some(Step::new(0)); 16])
            .unwrap()
            .with_mix_automation(curves)
            .is_err()
    );
}
