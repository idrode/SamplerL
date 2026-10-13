//! Compresor y distorsión como efectos de pista: nivel estable exacto del
//! compresor, tiempo de ataque, y la distorsión igual a su fórmula.

use std::sync::Arc;

use engine::{Fx, Pattern, STEPS, Sample, Slices, Step, new_engine, offline};

const RATE: u32 = 48_000;

/// Un paso a 120 BPM (6000 frames) de nivel constante `level`, en el paso
/// 0 de cada compás, con un efecto.
fn render(level: f32, fx: Fx, frames: usize) -> Vec<f32> {
    let mono = vec![level; 24_000];
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    let mut steps = vec![None; STEPS];
    steps[0] = Some(Step::new(0));
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_fx(&[fx])
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

/// Compresor con ataque, release y makeup a elegir.
fn comp(threshold_db: f32, ratio: f32, attack_ms: f32, makeup_db: f32) -> Fx {
    Fx::Compressor {
        threshold_db,
        ratio,
        attack_ms,
        release_ms: 100.0,
        makeup_db,
    }
}

fn db(x: f32) -> f32 {
    20.0 * x.log10()
}

#[test]
fn the_compressor_settles_at_the_expected_level() {
    // 0,5 (-6 dB) con umbral -12 y ratio 4: sobran 6 dB, se quitan 4,5.
    let out = render(0.5, Fx::compressor(-12.0, 4.0), 20_000);
    let over = db(0.5) + 12.0;
    let expected = 0.5 * 10f32.powf(-over * 0.75 / 20.0);
    assert!(
        (out[15_000] - expected).abs() < 1e-4,
        "{} {expected}",
        out[15_000]
    );
    // Con makeup, lo mismo más los dB pedidos.
    let fx = comp(-12.0, 4.0, 5.0, 6.0);
    let louder = render(0.5, fx, 20_000);
    assert!((db(louder[15_000]) - db(expected) - 6.0).abs() < 0.01);
    // Por debajo del umbral no toca nada (bit a bit).
    let quiet = render(0.1, Fx::compressor(-12.0, 4.0), 20_000);
    let none = {
        let mono = vec![0.1f32; 24_000];
        let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
        let slices = Slices::detect(&sample, 1);
        let mut steps = vec![None; STEPS];
        steps[0] = Some(Step::new(0));
        let (mut controller, mut engine) = new_engine(RATE);
        controller
            .set_pattern(Pattern::from_steps(sample, &slices, steps).unwrap())
            .unwrap();
        controller.start().unwrap();
        offline::render(&mut engine, 20_000, 2)
            .iter()
            .step_by(2)
            .copied()
            .collect::<Vec<f32>>()
    };
    assert_eq!(quiet, none);
}

#[test]
fn the_attack_takes_its_time() {
    // Con 10 ms de ataque, a los 10 ms va por el 63 % de la reducción.
    let fx = comp(-12.0, 4.0, 10.0, 0.0);
    let out = render(0.5, fx, 20_000);
    let target = (db(0.5) + 12.0) * 0.75;
    // Tras el fundido de entrada (1,5 ms = 72 frames), la envolvente sube.
    let at = |ms: f32| db(0.5) - db(out[72 + (ms / 1000.0 * RATE as f32) as usize]);
    let early = at(10.0) / target;
    assert!((0.55..0.72).contains(&early), "{early}");
    assert!(at(60.0) / target > 0.99);
}

#[test]
fn the_distortion_is_tanh_of_the_drive_mixed() {
    let out = render(0.3, Fx::distortion(5.0, 1.0), 10_000);
    assert!(
        (out[5000] - (5.0f32 * 0.3).tanh()).abs() < 1e-5,
        "{}",
        out[5000]
    );
    let half = render(0.3, Fx::distortion(5.0, 0.5), 10_000);
    let expected = 0.3 + 0.5 * ((5.0f32 * 0.3).tanh() - 0.3);
    assert!((half[5000] - expected).abs() < 1e-6);
    // Nunca pasa de 1, por mucho drive que lleve.
    let hot = render(0.9, Fx::distortion(50.0, 1.0), 10_000);
    assert!(hot.iter().all(|s| s.abs() <= 1.0));
}

#[test]
fn ranges_are_checked() {
    for fx in [
        Fx::compressor(5.0, 4.0),
        Fx::compressor(-12.0, 0.5),
        comp(-12.0, 4.0, 0.0, 0.0),
        comp(-12.0, 4.0, 5.0, 30.0),
        Fx::distortion(0.5, 1.0),
        Fx::distortion(4.0, 1.5),
    ] {
        assert!(fx.validate().is_err(), "{fx:?}");
    }
}
