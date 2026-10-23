//! Envíos a los buses de delay y reverb: el eco cae en el frame exacto de
//! su división, los cambios entran al compás, la cola nunca pasa de ±1 con
//! el limitador y un bus sin nada que sonar se duerme.

use std::sync::Arc;

use engine::bus::{Buses, DelayParams, Division, ReverbParams, delay_frames};
use engine::{Pattern, STEPS, Sample, SendError, Slices, Step, bar_frames, new_engine, offline};

const RATE: u32 = 48_000;

/// Un golpe de 400 frames a 0,25 y silencio hasta 3000.
fn hit() -> (Arc<Sample>, Slices) {
    let mut mono = vec![0.0f32; 3000];
    mono[..400].fill(0.25);
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    (sample, slices)
}

fn once_per_bar() -> Vec<Option<Step>> {
    let mut steps = vec![None; STEPS];
    steps[0] = Some(Step::new(0));
    steps
}

fn delay(division: Division, feedback: f32) -> Buses {
    Buses {
        delay: DelayParams {
            division,
            feedback,
            level: 1.0,
        },
        ..Buses::default()
    }
}

fn render(bpm: f64, sends: [f32; 2], buses: Buses, frames: usize) -> Vec<f32> {
    let (sample, slices) = hit();
    let pattern = Pattern::from_steps(sample, &slices, once_per_bar())
        .unwrap()
        .with_sends(sends)
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = engine::Batch::new();
    batch
        .set_bpm(bpm)
        .set_buses(buses)
        .set_pattern(0, Some(pattern));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

#[test]
fn echoes_land_on_the_exact_frame_of_their_division() {
    for (bpm, division) in [
        (120.0, Division::Eighth),
        (170.0, Division::Sixteenth),
        (140.0, Division::Quarter),
        (100.0, Division::DottedEighth),
    ] {
        let d = delay_frames(division, bpm, RATE);
        let steps = division.steps() as f64 * RATE as f64 * 60.0 / (bpm * 4.0);
        assert_eq!(d, steps.round() as usize);
        let dry = left(&render(bpm, [0.0, 0.0], delay(division, 0.5), 3 * d + 3000));
        let wet = left(&render(bpm, [0.5, 0.0], delay(division, 0.5), 3 * d + 3000));
        // Antes del primer eco, nada cambia.
        assert_eq!(wet[..d], dry[..d], "{bpm} {division:?}");
        // El primer eco: la mitad del golpe, d frames después; el segundo,
        // otra vez la mitad (feedback 0,5). Exacto (sin limitador: < 0,9).
        for t in 0..3000 {
            assert_eq!(wet[d + t], dry[d + t] + 0.5 * dry[t], "{division:?} t={t}");
            assert_eq!(
                wet[2 * d + t],
                dry[2 * d + t] + 0.25 * dry[t],
                "{division:?} 2º eco t={t}"
            );
        }
    }
}

#[test]
fn bus_changes_enter_at_the_bar() {
    let (sample, slices) = hit();
    let pattern = Pattern::from_steps(sample, &slices, once_per_bar())
        .unwrap()
        .with_sends([1.0, 0.0])
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = engine::Batch::new();
    batch
        .set_buses(delay(Division::Eighth, 0.0))
        .set_pattern(0, Some(pattern));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let bar = bar_frames(120.0, RATE) as usize;
    let mut audio = offline::render(&mut engine, bar / 2, 2);
    // A mitad de compás: entra en el siguiente.
    controller.set_buses(delay(Division::Quarter, 0.0)).unwrap();
    audio.extend(offline::render(&mut engine, bar + bar / 2, 2));
    let l = left(&audio);
    let first_onset = |from: usize| (from..l.len()).find(|&i| l[i] != 0.0).unwrap() - 1;
    let (eighth, quarter) = (12_000, 24_000);
    assert_eq!(first_onset(400), eighth, "compás 0: corchea");
    assert_eq!(first_onset(bar + 400), bar + quarter, "compás 1: negra");
}

#[test]
fn the_tail_never_passes_one_and_a_silent_bus_sleeps() {
    // 8 pistas a tope, con todo al delay (feedback máximo) y a la reverb.
    let mut mono = vec![0.0f32; 3000];
    mono[..2000].fill(0.9);
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = engine::Batch::new();
    batch.set_buses(Buses {
        delay: DelayParams {
            division: Division::Sixteenth,
            feedback: 0.95,
            level: 1.0,
        },
        reverb: ReverbParams {
            size: 1.0,
            damp: 0.0,
            level: 1.0,
        },
        ..Default::default()
    });
    for t in 0..engine::MAX_TRACKS {
        let all = vec![Some(Step::new(0).with_gain(4.0)); STEPS];
        let pattern = Pattern::from_steps(sample.clone(), &slices, all)
            .unwrap()
            .with_gain(4.0)
            .unwrap()
            .with_sends([1.0, 1.0])
            .unwrap();
        batch.set_pattern(t, Some(pattern));
    }
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let audio = offline::render(&mut engine, 4 * bar_frames(120.0, RATE) as usize, 2);
    assert!(audio.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    // Parado: la cola sigue, también dentro de ±1.
    controller.stop().unwrap();
    let tail = offline::render(&mut engine, 4 * RATE as usize, 2);
    assert!(tail.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
    assert!(
        tail[tail.len() - 1000..].iter().any(|s| *s != 0.0),
        "la cola sigue"
    );

    // Sin feedback, en cuanto pasa el eco el delay se duerme: silencio exacto.
    let d = delay_frames(Division::Eighth, 120.0, RATE);
    let audio = left(&render(
        120.0,
        [1.0, 0.0],
        delay(Division::Eighth, 0.0),
        bar_frames(120.0, RATE) as usize,
    ));
    assert!(audio[d + 1..d + 400].iter().all(|s| *s != 0.0));
    assert!(audio[d + 3000..].iter().all(|s| *s == 0.0));
}

#[test]
fn the_reverb_tail_is_deterministic_and_decays() {
    let reverb = Buses {
        reverb: ReverbParams {
            size: 0.8,
            damp: 0.3,
            level: 1.0,
        },
        ..Buses::default()
    };
    let a = left(&render(120.0, [0.0, 1.0], reverb, RATE as usize));
    let b = left(&render(120.0, [0.0, 1.0], reverb, RATE as usize));
    assert_eq!(a, b);
    let energy = |from: usize, to: usize| a[from..to].iter().map(|s| s * s).sum::<f32>();
    let early = energy(3000, 9000);
    let late = energy(40_000, 46_000);
    assert!(early > 0.0 && late < early / 10.0, "{early} {late}");
}

#[test]
fn out_of_range_sends_and_buses_are_rejected() {
    let (sample, slices) = hit();
    let pattern = || Pattern::from_steps(sample.clone(), &slices, once_per_bar()).unwrap();
    assert!(matches!(
        pattern().with_sends([1.5, 0.0]),
        Err(SendError::InvalidBus(_))
    ));
    assert!(pattern().with_sends([0.0, -0.1]).is_err());
    let (mut controller, _engine) = new_engine(RATE);
    let bad = delay(Division::Eighth, 1.0);
    assert!(matches!(
        controller.set_buses(bad),
        Err(SendError::InvalidBus(_))
    ));
    let mut batch = engine::Batch::new();
    batch.set_buses(bad);
    assert!(controller.apply(batch).is_err());
}
