//! Automatización: las curvas dependen solo de la posición de la canción,
//! los valores por paso (`~`) dan el mismo audio que el filtro fijo, los
//! cambios no hacen clic y todo es determinista.

use std::sync::Arc;

use engine::{
    Automation, Curve, Fx, LfoRate, Pattern, STEPS, Sample, SendError, Slices, Step, new_engine,
    offline,
};

const RATE: u32 = 48_000;

#[test]
fn curves_follow_the_song_position() {
    let ramp = Curve::Ramp {
        from: 200.0,
        to: 4000.0,
        steps: 64.0,
    };
    assert_eq!(
        ramp.value(0.0, 0.0, &[0.0; engine::MAX_MACROS], true),
        200.0
    );
    // A mitad: la media geométrica (escala logarítmica).
    let mid = ramp.value(32.0, 0.0, &[0.0; engine::MAX_MACROS], true);
    assert!((mid - (200.0f32 * 4000.0).sqrt()).abs() < 0.5, "{mid}");
    // Lineal: la media.
    assert_eq!(
        ramp.value(32.0, 0.0, &[0.0; engine::MAX_MACROS], false),
        2100.0
    );
    // Diente de sierra: vuelve a empezar cada 64 pasos.
    assert_eq!(
        ramp.value(64.0, 0.0, &[0.0; engine::MAX_MACROS], true),
        200.0
    );
    assert_eq!(ramp.value(96.0, 9.0, &[0.0; engine::MAX_MACROS], true), mid);
    // LFO en pasos y en Hz: empieza en el mínimo, a medio ciclo el máximo.
    let lfo = Curve::Lfo {
        min: 0.0,
        max: 1.0,
        rate: LfoRate::Steps(16.0),
    };
    assert_eq!(lfo.value(0.0, 0.0, &[0.0; engine::MAX_MACROS], false), 0.0);
    assert!((lfo.value(8.0, 0.0, &[0.0; engine::MAX_MACROS], false) - 1.0).abs() < 1e-6);
    let hz = Curve::Lfo {
        min: 0.0,
        max: 1.0,
        rate: LfoRate::Hz(2.0),
    };
    assert!((hz.value(123.0, 0.25, &[0.0; engine::MAX_MACROS], false) - 1.0).abs() < 1e-6);
}

/// Un golpe de un seno de 2 kHz que dura `len` frames, en cada paso que
/// marca `steps`.
fn render(steps: Vec<Option<Step>>, fx: Fx, automation: &[Automation], frames: usize) -> Vec<f32> {
    // 208 periodos exactos: acaba en un cruce por cero (sin escalón).
    render_len(4992, steps, fx, automation, frames)
}

/// Como [`render`], con un golpe de `len` frames (múltiplo de 24).
fn render_len(
    len: usize,
    steps: Vec<Option<Step>>,
    fx: Fx,
    automation: &[Automation],
    frames: usize,
) -> Vec<f32> {
    let mono: Vec<f32> = (0..len)
        .map(|i| 0.5 * (std::f32::consts::TAU * 2000.0 * i as f32 / RATE as f32).sin())
        .chain(std::iter::repeat_n(0.0, 1000))
        .collect();
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_fx(&[fx])
        .unwrap()
        .with_automation(automation)
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

fn every_step(step: Step) -> Vec<Option<Step>> {
    vec![Some(step); STEPS]
}

fn peak(audio: &[f32]) -> f32 {
    audio.iter().fold(0.0, |m, s| m.max(s.abs()))
}

#[test]
fn a_locked_cutoff_sounds_like_that_fixed_filter() {
    // Cada golpe empieza en silencio: el valor del paso entra ya.
    let bar = 16 * 6000;
    let locked = render(
        every_step(Step::new(0).with_lock(500.0)),
        Fx::lowpass(8000.0),
        &[],
        bar,
    );
    let fixed = render(every_step(Step::new(0)), Fx::lowpass(500.0), &[], bar);
    assert_eq!(locked, fixed);
    // Un golpe sin `~` vuelve al corte fijo (suavizado): al final de su
    // paso suena como el filtro de 8000.
    let mut steps = vec![None; STEPS];
    steps[0] = Some(Step::new(0).with_lock(500.0));
    steps[1] = Some(Step::new(0));
    let back = render(steps, Fx::lowpass(8000.0), &[], 2 * 6000);
    let open = render(every_step(Step::new(0)), Fx::lowpass(8000.0), &[], 2 * 6000);
    let tail = 6000 + 3000..6000 + 4000;
    assert!((peak(&back[tail.clone()]) - peak(&open[tail])).abs() < 1e-3);
    // Y el de 500 apaga los 2 kHz.
    assert!(peak(&back[2000..4000]) < 0.1 * peak(&open[2000..4000]));
}

#[test]
fn a_ramp_opens_the_filter_and_never_clicks() {
    // De 200 a 20000 Hz en un compás: los 2 kHz pasan cada vez más.
    let ramp = Automation {
        fx: 0,
        param: 0,
        curve: Curve::Ramp {
            from: 200.0,
            to: 20_000.0,
            steps: 16.0,
        },
    };
    let audio = render(
        every_step(Step::new(0)),
        Fx::lowpass(200.0),
        &[ramp],
        16 * 6000,
    );
    let at = |step: usize| peak(&audio[step * 6000 + 2000..step * 6000 + 4000]);
    let levels: Vec<f32> = [1, 4, 8, 12, 15].iter().map(|&s| at(s)).collect();
    assert!(levels.windows(2).all(|w| w[0] < w[1]), "{levels:?}");
    assert!(levels[4] > 0.45, "{levels:?}");
    // Sin clics: el mayor salto entre muestras no pasa del de un seno de
    // 2 kHz a 0,5 (más un margen por el filtro).
    let slope = 0.5 * std::f32::consts::TAU * 2000.0 / RATE as f32;
    let (at_frame, jump) = audio
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .enumerate()
        .fold((0, 0.0), |m, (i, j)| if j > m.1 { (i, j) } else { m });
    assert!(jump < 1.5 * slope, "{jump} > {slope} en {at_frame}");

    // Con valores por paso que saltan de 200 a 12000 en cada golpe, igual:
    // los golpes duran más que el paso (se solapan con su fundido), así que
    // el filtro no vuelve a empezar y el corte cambia suavizado.
    let steps: Vec<Option<Step>> = (0..STEPS)
        .map(|k| Some(Step::new(0).with_lock(if k % 2 == 0 { 200.0 } else { 12_000.0 })))
        .collect();
    let audio = render_len(9600, steps, Fx::lowpass(1000.0), &[], 16 * 6000);
    let (at_frame, jump) = audio
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .enumerate()
        .fold((0, 0.0), |m, (i, j)| if j > m.1 { (i, j) } else { m });
    assert!(jump < 1.5 * slope, "{jump} en {at_frame}");
    // Y de verdad cambia: los pasos impares dejan pasar los 2 kHz.
    let at = |step: usize| peak(&audio[step * 6000 + 3000..step * 6000 + 5000]);
    assert!(at(5) > 4.0 * at(4), "{} {}", at(5), at(4));
}

#[test]
fn automation_is_deterministic_and_leaves_fixed_params_alone() {
    let lfo = Automation {
        fx: 0,
        param: 1,
        curve: Curve::Lfo {
            min: 0.0,
            max: 1.0,
            rate: LfoRate::Hz(3.0),
        },
    };
    let fx = Fx::distortion(8.0, 0.5);
    let a = render(every_step(Step::new(0)), fx, &[lfo], 2 * 16 * 6000);
    let b = render(every_step(Step::new(0)), fx, &[lfo], 2 * 16 * 6000);
    assert_eq!(a, b);
    assert_ne!(a, render(every_step(Step::new(0)), fx, &[], 2 * 16 * 6000));
}

#[test]
fn automations_are_checked_against_the_chain() {
    let sample = Arc::new(Sample::from_interleaved(&[0.5; 100], 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    let pattern = || {
        Pattern::from_steps(sample.clone(), &slices, every_step(Step::new(0)))
            .unwrap()
            .with_fx(&[Fx::lowpass(1000.0)])
            .unwrap()
    };
    let ramp = |fx, param, to| Automation {
        fx,
        param,
        curve: Curve::Ramp {
            from: 100.0,
            to,
            steps: 16.0,
        },
    };
    assert!(pattern().with_automation(&[ramp(0, 0, 5000.0)]).is_ok());
    for bad in [ramp(1, 0, 5000.0), ramp(0, 2, 5000.0), ramp(0, 0, 30_000.0)] {
        assert!(matches!(
            pattern().with_automation(&[bad]),
            Err(SendError::InvalidFx(_))
        ));
    }
    // Un corte por paso fuera de rango.
    let steps = every_step(Step::new(0).with_lock(5.0));
    assert!(Pattern::from_steps(sample.clone(), &slices, steps).is_err());
}
