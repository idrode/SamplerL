//! Macros: un parámetro con `Curve::Macro` sigue el valor de la macro
//! (de 0 a 1 entre sus dos extremos); el valor entra con el lote, al
//! compás, y se mueve en vivo con `set_macro`, ya, con el suavizado de
//! siempre. Un lote que espera no deshace lo que se ha movido después.

use std::sync::Arc;

use engine::{
    Batch, Curve, MAX_MACROS, MIX_GAIN, MIX_PARAMS, Pattern, STEPS, Sample, SendError, Slices,
    Step, bar_frames, new_engine, offline,
};

const RATE: u32 = 48_000;
const BPM: f64 = 120.0;

/// Un tono constante a 0,25 que dura el compás entero: el volumen de la
/// pista se lee directamente en la salida.
fn tone() -> Pattern {
    let mono = vec![0.25f32; bar_frames(BPM, RATE) as usize];
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    let mut steps = vec![None; STEPS];
    steps[0] = Some(Step::new(0));
    Pattern::from_steps(sample, &slices, steps).unwrap()
}

/// El volumen de la pista sigue a la macro 2, de 0 a 1.
fn following() -> Pattern {
    let mut curves = [None; MIX_PARAMS];
    curves[MIX_GAIN] = Some(Curve::Macro {
        index: 2,
        from: 0.0,
        to: 1.0,
    });
    tone().with_mix_automation(curves).unwrap()
}

fn macros(value: f32) -> [f32; MAX_MACROS] {
    let mut m = [0.0; MAX_MACROS];
    m[2] = value;
    m
}

#[test]
fn a_parameter_follows_its_macro() {
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_bpm(BPM)
        .set_pattern(0, Some(following()))
        .set_macros(macros(0.5));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let out = offline::render(&mut engine, 4800, 2);
    // Un poco después del fundido de entrada: 0,25 por la mitad.
    for v in &out[2 * 200..] {
        assert!((v - 0.125).abs() < 1e-6, "{v}");
    }
}

#[test]
fn moving_a_macro_takes_effect_at_once_and_smoothly() {
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_bpm(BPM)
        .set_pattern(0, Some(following()))
        .set_macros(macros(0.0));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let before = offline::render(&mut engine, 4800, 2);
    assert!(before.iter().all(|&v| v == 0.0));
    controller.set_macro(2, 1.0).unwrap();
    let after = offline::render(&mut engine, 4800, 2);
    // Sube sin saltos (el suavizado del volumen) y llega en unos ms.
    let left: Vec<f32> = after.iter().step_by(2).copied().collect();
    for pair in left.windows(2) {
        assert!(pair[1] >= pair[0] && pair[1] - pair[0] < 0.01);
    }
    assert!((left[2400] - 0.25).abs() < 1e-4, "{}", left[2400]);
}

#[test]
fn a_waiting_batch_keeps_what_moved_after_it() {
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_bpm(BPM)
        .set_pattern(0, Some(following()))
        .set_macros(macros(1.0));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let bar = bar_frames(BPM, RATE) as usize;
    offline::render(&mut engine, bar / 2, 2);
    // Una recarga con la macro a 1 y, antes del compás, se baja a 0,5.
    let mut batch = Batch::new();
    batch
        .set_pattern(0, Some(following()))
        .set_macros(macros(1.0));
    controller.apply(batch).unwrap();
    controller.set_macro(2, 0.5).unwrap();
    offline::render(&mut engine, bar - bar / 2, 2);
    let next = offline::render(&mut engine, 4800, 2);
    for v in &next[2 * 2400..] {
        assert!((v - 0.125).abs() < 1e-4, "{v}");
    }
}

#[test]
fn macros_are_checked() {
    let (mut controller, _engine) = new_engine(RATE);
    assert!(matches!(
        controller.set_macro(MAX_MACROS, 0.5),
        Err(SendError::InvalidMacro(_))
    ));
    assert!(matches!(
        controller.set_macro(0, 1.5),
        Err(SendError::InvalidMacro(_))
    ));
    let mut batch = Batch::new();
    batch.set_macros(macros(-0.1));
    assert!(matches!(
        controller.apply(batch),
        Err(SendError::InvalidMacro(_))
    ));
    // Una curva de una macro que no existe no entra en el patrón.
    let mut curves = [None; MIX_PARAMS];
    curves[MIX_GAIN] = Some(Curve::Macro {
        index: MAX_MACROS as u8,
        from: 0.0,
        to: 1.0,
    });
    assert!(tone().with_mix_automation(curves).is_err());
}

#[test]
fn frequencies_follow_a_macro_on_a_log_scale() {
    let curve = Curve::Macro {
        index: 0,
        from: 200.0,
        to: 3200.0,
    };
    let mut m = [0.0; MAX_MACROS];
    m[0] = 0.5;
    assert!((curve.value(0.0, 0.0, &m, true) - 800.0).abs() < 0.01);
    assert_eq!(curve.value(0.0, 0.0, &m, false), 1700.0);
    m[0] = 0.0;
    assert_eq!(curve.value(9.0, 1.0, &m, true), 200.0);
    m[0] = 1.0;
    assert_eq!(curve.value(9.0, 1.0, &m, true), 3200.0);
}
