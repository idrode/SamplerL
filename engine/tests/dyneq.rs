//! EQ dinámico (`Fx::DynEq`): baja su banda solo mientras el detector pasa
//! del umbral, escuchando la propia pista o, con sidechain, otra; por
//! debajo del umbral (o con la clave callada) es bit a bit lo de siempre, y
//! el resultado no depende de en qué ranura esté cada pista.

use std::sync::Arc;

use engine::{DynEqParams, Fx, Pattern, Sample, Slices, Step, new_engine, offline};

const RATE: u32 = 48_000;
const BPM: f64 = 120.0;
/// A 120 BPM, un paso son 6000 frames y un compás 96000.
const STEP: usize = 6000;
const BAR: usize = 16 * STEP;

/// Un seno de `freq` con amplitud `amp` que dura `frames`, con fundidos de
/// 5 ms.
fn sine_sample(freq: f32, amp: f32, frames: usize) -> (Arc<Sample>, Slices) {
    let fade = 240.0;
    let mono: Vec<f32> = (0..frames)
        .map(|i| {
            let env = (i as f32 / fade).min(1.0).min((frames - i) as f32 / fade);
            env * amp * (std::f32::consts::TAU * freq * i as f32 / RATE as f32).sin()
        })
        .collect();
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    (sample, slices)
}

/// Golpes en los pasos `hits` (el resto, silencio).
fn steps(hits: &[usize]) -> Vec<Option<Step>> {
    (0..16)
        .map(|k| hits.contains(&k).then(|| Step::new(0)))
        .collect()
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

fn db(a: f32, b: f32) -> f32 {
    20.0 * (a / b).log10()
}

/// Renderiza un compás con los patrones en sus ranuras.
fn render(patterns: Vec<(usize, Pattern)>) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(BPM).unwrap();
    for (slot, pattern) in patterns {
        controller.set_track_pattern(slot, Some(pattern)).unwrap();
    }
    controller.start().unwrap();
    left(&offline::render(&mut engine, BAR, 2))
}

/// Un seno de 1 kHz que suena el compás entero, con `chain` de efectos.
fn tone(amp: f32, chain: &[Fx]) -> Pattern {
    let (sample, slices) = sine_sample(1000.0, amp, BAR);
    Pattern::from_steps(sample, &slices, steps(&[0]))
        .unwrap()
        .with_fx(chain)
        .unwrap()
}

#[test]
fn above_the_threshold_its_band_goes_down_by_ratio_up_to_range() {
    let dynamic = |range_db: f32| {
        Fx::DynEq(DynEqParams {
            threshold_db: -20.0,
            ratio: 4.0,
            range_db,
            ..DynEqParams::new(1000.0)
        })
    };
    let plain = render(vec![(0, tone(0.5, &[]))]);
    let middle = BAR / 4..3 * BAR / 4;
    // 0,5 es −6 dB: 14 dB de más, ×(1 − 1/4) = 10,5 dB menos.
    let out = render(vec![(0, tone(0.5, &[dynamic(24.0)]))]);
    let got = db(rms(&out[middle.clone()]), rms(&plain[middle.clone()]));
    assert!((got + 10.5).abs() < 0.6, "{got}");
    // Con range 6, como mucho 6.
    let out = render(vec![(0, tone(0.5, &[dynamic(6.0)]))]);
    let got = db(rms(&out[middle.clone()]), rms(&plain[middle]));
    assert!((got + 6.0).abs() < 0.3, "{got}");
}

#[test]
fn below_the_threshold_or_out_of_its_band_nothing_changes() {
    let fx = Fx::DynEq(DynEqParams {
        threshold_db: -20.0,
        ..DynEqParams::new(1000.0)
    });
    // −40 dB: por debajo del umbral, bit a bit.
    assert_eq!(
        render(vec![(0, tone(0.01, &[fx]))]),
        render(vec![(0, tone(0.01, &[]))])
    );
    // Un seno fuerte a 100 Hz: el detector de 1 kHz casi no lo oye.
    let low = |chain: &[Fx]| {
        let (sample, slices) = sine_sample(100.0, 0.5, BAR);
        Pattern::from_steps(sample, &slices, steps(&[0]))
            .unwrap()
            .with_fx(chain)
            .unwrap()
    };
    assert_eq!(render(vec![(0, low(&[fx]))]), render(vec![(0, low(&[]))]));
}

/// Un «bajo»: 100 Hz durante el compás entero, en solo (para medirlo), con
/// `chain`.
fn bass(chain: &[Fx]) -> Pattern {
    let (sample, slices) = sine_sample(100.0, 0.3, BAR);
    Pattern::from_steps(sample, &slices, steps(&[0]))
        .unwrap()
        .with_fx(chain)
        .unwrap()
        .with_solo(true)
}

/// Un «bombo»: 100 Hz durante 100 ms en cada negra.
fn kick() -> Pattern {
    let (sample, slices) = sine_sample(100.0, 0.8, 4800);
    Pattern::from_steps(sample, &slices, steps(&[0, 4, 8, 12])).unwrap()
}

fn ducking(key: usize) -> Fx {
    Fx::DynEq(
        DynEqParams {
            threshold_db: -30.0,
            ratio: 20.0,
            range_db: 6.0,
            attack_ms: 2.0,
            release_ms: 60.0,
            ..DynEqParams::new(100.0)
        }
        .with_sidechain(key),
    )
}

#[test]
fn with_a_sidechain_the_band_dips_only_while_the_other_track_plays() {
    // El bombo en la pista 0 (fuera del solo: no se oye, pero se escucha).
    let plain = render(vec![(0, kick()), (1, bass(&[]))]);
    let ducked = render(vec![(0, kick()), (1, bass(&[ducking(0)]))]);
    let window = |x: &[f32], at: usize| rms(&x[at..at + 2400]);
    for beat in 0..4 {
        let hit = beat * 4 * STEP;
        // Durante el golpe (de 20 a 70 ms), unos 6 dB menos.
        let during = db(window(&ducked, hit + 960), window(&plain, hit + 960));
        assert!((during + 6.0).abs() < 1.0, "golpe {beat}: {during}");
        // Entre golpes (300 ms después), ya de vuelta.
        let after = db(window(&ducked, hit + 14_400), window(&plain, hit + 14_400));
        assert!(after.abs() < 0.3, "entre golpes {beat}: {after}");
    }
    // Sin escalones al bajar y subir: la diferencia entre muestras seguidas
    // no pasa de la del propio seno.
    let step = |x: &[f32]| {
        x[2400..]
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    };
    assert!(
        step(&ducked) <= step(&plain) * 1.01,
        "{} > {}",
        step(&ducked),
        step(&plain)
    );
    // Sin golpes en la clave: idéntico a no tenerlo.
    let silent = Pattern::from_steps(kick_sample().0, &kick_sample().1, steps(&[])).unwrap();
    assert_eq!(
        render(vec![(0, silent), (1, bass(&[ducking(0)]))]),
        render(vec![(1, bass(&[]))])
    );
}

fn kick_sample() -> (Arc<Sample>, Slices) {
    sine_sample(100.0, 0.8, 4800)
}

#[test]
fn the_result_does_not_depend_on_the_slots() {
    // La clave antes o después de la que escucha: lo mismo.
    let forward = render(vec![(0, kick()), (3, bass(&[ducking(0)]))]);
    let backward = render(vec![(5, kick()), (2, bass(&[ducking(5)]))]);
    assert_eq!(forward, backward);
    // Una cadena: la 2 escucha a la 1, que escucha a la 0.
    let middle = {
        let (sample, slices) = sine_sample(100.0, 0.3, BAR);
        Pattern::from_steps(sample, &slices, steps(&[0]))
            .unwrap()
            .with_fx(&[ducking(4)])
            .unwrap()
    };
    let chained = render(vec![(4, kick()), (1, middle), (0, bass(&[ducking(1)]))]);
    assert_ne!(chained, render(vec![(4, kick()), (0, bass(&[]))]));
}

#[test]
fn out_of_range_values_are_rejected() {
    let bad = |p: DynEqParams| Fx::DynEq(p).validate().is_err();
    let base = DynEqParams::new(1000.0);
    assert!(!bad(base));
    assert!(bad(DynEqParams {
        threshold_db: 3.0,
        ..base
    }));
    assert!(bad(DynEqParams { ratio: 0.5, ..base }));
    assert!(bad(DynEqParams {
        range_db: 30.0,
        ..base
    }));
    assert!(bad(base.with_sidechain(8)));
    assert_eq!(
        Fx::DynEq(base).params(),
        [
            "freq",
            "q",
            "threshold",
            "range",
            "ratio",
            "attack",
            "release"
        ]
    );
}
