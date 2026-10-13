//! Rampas de tempo y cambios de compás: cada compás a su tempo y con sus
//! pasos, al frame exacto; recargar la misma rampa no la reinicia; el
//! delay no hace clic al cambiar de tempo.

use std::sync::Arc;

use engine::{
    Batch, Meter, Pattern, STEPS, Sample, Slices, Step, TempoRamp, bar_frames, bar_frames_steps,
    new_engine, offline,
};

const RATE: u32 = 48_000;

fn hit() -> (Arc<Sample>, Slices) {
    let mut mono = vec![0.0f32; 2000];
    mono[..300].fill(0.5);
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    (sample, slices)
}

fn onsets(audio: &[f32]) -> Vec<usize> {
    let left: Vec<f32> = audio.iter().step_by(2).copied().collect();
    (1..left.len())
        .filter(|&i| left[i - 1] == 0.0 && left[i] != 0.0)
        .map(|i| i - 1)
        .collect()
}

/// Un golpe al empezar cada vuelta de `length` pasos.
fn first_step(length: usize) -> Pattern {
    let (sample, slices) = hit();
    let mut steps = vec![None; length];
    steps[0] = Some(Step::new(0));
    Pattern::from_steps(sample, &slices, steps).unwrap()
}

#[test]
fn a_ramp_gives_each_bar_its_tempo() {
    let ramp = TempoRamp {
        from: 120.0,
        to: 150.0,
        bars: 3,
    };
    assert_eq!(
        (0..5).map(|k| ramp.bpm_at(k)).collect::<Vec<_>>(),
        [120.0, 130.0, 140.0, 150.0, 150.0]
    );
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_tempo_ramp(ramp)
        .set_pattern(0, Some(first_step(STEPS)));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let bars: Vec<u64> = (0..5).map(|k| bar_frames(ramp.bpm_at(k), RATE)).collect();
    let total: u64 = bars.iter().sum();
    let audio = offline::render(&mut engine, total as usize, 2);
    let mut expected = vec![0usize];
    for b in &bars[..4] {
        expected.push(expected.last().unwrap() + *b as usize);
    }
    assert_eq!(onsets(&audio), expected);
    assert_eq!(controller.status().bpm, 150.0);
}

#[test]
fn the_same_ramp_again_keeps_going_and_a_bpm_stops_it() {
    let ramp = TempoRamp {
        from: 100.0,
        to: 200.0,
        bars: 10,
    };
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_tempo_ramp(ramp).unwrap();
    controller.set_pattern(first_step(STEPS)).unwrap();
    controller.start().unwrap();
    let first: u64 = (0..3).map(|k| bar_frames(ramp.bpm_at(k), RATE)).sum();
    offline::render(&mut engine, first as usize + 10, 2);
    assert_eq!(controller.status().bpm, 130.0);
    // Recargar envía la misma rampa: el compás 4 sigue a 140 (no vuelve a
    // 100).
    controller.set_tempo_ramp(ramp).unwrap();
    offline::render(&mut engine, bar_frames(130.0, RATE) as usize, 2);
    assert_eq!(controller.status().bpm, 140.0);
    // Un bpm fijo la para.
    controller.set_bpm(90.0).unwrap();
    offline::render(&mut engine, 2 * bar_frames(140.0, RATE) as usize, 2);
    assert_eq!(controller.status().bpm, 90.0);
}

#[test]
fn meters_change_the_bar_length_and_the_beat() {
    assert_eq!(Meter::new(3, 4), Some(Meter { steps: 12, beat: 4 }));
    assert_eq!(Meter::new(7, 8), Some(Meter { steps: 14, beat: 2 }));
    assert_eq!(Meter::new(9, 16), Some(Meter { steps: 9, beat: 1 }));
    assert_eq!(Meter::new(3, 5), None);
    assert_eq!(Meter::new(65, 16), None);

    // 3/4: un compás son 12 pasos; una pista de 12 cae en cada compás.
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_bpm(120.0)
        .set_meter(Meter::new(3, 4).unwrap())
        .set_pattern(0, Some(first_step(12)));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let bar = bar_frames_steps(12, 120.0, RATE) as usize;
    assert_eq!(bar, 72_000);
    let audio = offline::render(&mut engine, 3 * bar, 2);
    assert_eq!(onsets(&audio), [0, bar, 2 * bar]);
    let status = controller.status();
    assert_eq!(status.bar_steps, 12);
    assert_eq!(status.global, 3 * 12 - 1);

    // 7/8 con metrónomo: un clic cada corchea (2 pasos), acento al empezar.
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch.set_bpm(120.0).set_meter(Meter::new(7, 8).unwrap());
    controller.apply(batch).unwrap();
    controller.set_metronome(true).unwrap();
    controller.start().unwrap();
    let bar = bar_frames_steps(14, 120.0, RATE) as usize;
    let audio = offline::render(&mut engine, bar + 10, 2);
    let clicks: Vec<usize> = (0..7).map(|k| k * 12_000).collect();
    for &c in &clicks {
        assert!(audio[2 * (c + 20)] != 0.0, "clic en {c}");
    }
    assert_eq!(audio[2 * (6000 + 2000)], 0.0, "sin clic en el paso 1");
}

#[test]
fn the_delay_glides_when_the_tempo_changes() {
    // Un seno continuo al delay mientras el tempo sube de 100 a 160 en 2
    // compases: el eco cambia de retardo deslizándose, sin saltos.
    let sine: Vec<f32> = (0..48_000)
        .map(|i| 0.3 * (std::f32::consts::TAU * 220.0 * i as f32 / RATE as f32).sin())
        .collect();
    let sample = Arc::new(Sample::from_interleaved(&sine, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    let mut steps = vec![None; STEPS];
    steps[0] = Some(Step::new(0));
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_sends([1.0, 0.0])
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_tempo_ramp(TempoRamp {
            from: 100.0,
            to: 160.0,
            bars: 2,
        })
        .set_buses(engine::Buses {
            delay: engine::DelayParams {
                division: engine::Division::Eighth,
                feedback: 0.0,
                level: 1.0,
            },
            ..engine::Buses::default()
        })
        .set_pattern(0, Some(pattern));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let frames = (bar_frames(100.0, RATE) + bar_frames(130.0, RATE) + 20_000) as usize;
    let audio = offline::render(&mut engine, frames, 2);
    let left: Vec<f32> = audio.iter().step_by(2).copied().collect();
    // Dos senos de 0,3 (seco y eco): el salto máximo entre muestras de su
    // suma, con algo de margen por el cambio de retardo.
    let slope = 2.0 * 0.3 * std::f32::consts::TAU * 220.0 / RATE as f32;
    let around = bar_frames(100.0, RATE) as usize;
    let jump = left[around - 2000..around + 4000]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(jump < 1.2 * slope, "{jump} > {slope}");
}
