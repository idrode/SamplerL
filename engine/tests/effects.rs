//! Efectos: retrigger/stutter por paso, filtro paso bajo/alto y bitcrusher
//! por pista. Todo con render offline.

use std::sync::Arc;

use engine::notation::{parse_step, parse_steps};
use engine::{
    Fx, Pattern, STEPS, Sample, SendError, Slices, Step, bar_frames, new_engine, offline,
};

const RATE: u32 = 48_000;

fn sample_from(mono: &[f32], n: usize) -> (Arc<Sample>, Slices) {
    let sample = Arc::new(Sample::from_interleaved(mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, n);
    (sample, slices)
}

/// Bloque constante de 800 frames a 0.5 seguido de silencio: cada disparo
/// se ve como un arranque desde cero.
fn block_sample() -> (Arc<Sample>, Slices) {
    let mut mono = vec![0.0f32; 3000];
    mono[..800].fill(0.5);
    sample_from(&mono, 1)
}

fn sine(freq: f32, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| 0.5 * (std::f32::consts::TAU * freq * i as f32 / RATE as f32).sin())
        .collect()
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

/// Disparos: el fundido de entrada empieza en cero, así que el primer
/// frame no nulo tras silencio es el siguiente al del disparo.
fn triggers(left: &[f32]) -> Vec<usize> {
    (1..left.len())
        .filter(|&i| left[i - 1] == 0.0 && left[i] != 0.0)
        .map(|i| i - 1)
        .collect()
}

fn render_pattern(pattern: Pattern, bpm: f64, frames: usize) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(bpm).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    left(&offline::render(&mut engine, frames, 2))
}

#[test]
fn retrigger_notation() {
    assert_eq!(
        parse_step("3r4").unwrap(),
        Some(Step::new(2).with_retrig(4))
    );
    assert_eq!(
        parse_step("3'r2^7").unwrap(),
        Some(Step::new(2).reversed().with_retrig(2).with_pitch(7.0))
    );
    for bad in ["3r", "3r4r2", "3rx", "r4"] {
        assert!(parse_step(bad).is_err(), "\"{bad}\" debería fallar");
    }
    // El rango lo valida el patrón.
    let (sample, slices) = block_sample();
    for count in [0u8, 17] {
        let mut steps = [None; STEPS];
        steps[0] = Some(Step::new(0).with_retrig(count));
        assert!(matches!(
            Pattern::from_steps(sample.clone(), &slices, steps),
            Err(SendError::InvalidStep { step: 0, .. })
        ));
    }
}

#[test]
fn retrigger_subdivides_the_step_at_exact_frames() {
    let (sample, slices) = block_sample();
    for bpm in [120u64, 170] {
        let steps = parse_steps("1r4 . 1r3 .  . . . .  1r2 . . .  . . . 1").unwrap();
        let pattern = Pattern::from_steps(sample.clone(), &slices, steps).unwrap();
        let bar = bar_frames(bpm as f64, RATE) as usize;
        let found = triggers(&render_pattern(pattern, bpm as f64, bar));

        // Inicio de cada paso: round(k · RATE · 15 / bpm).
        let step_start = |k: u64| ((2 * k * RATE as u64 * 15 + bpm) / (2 * bpm)) as usize;
        let mut expected = Vec::new();
        for (k, count) in [(0u64, 4usize), (2, 3), (8, 2), (15, 1)] {
            let (start, len) = (step_start(k), step_start(k + 1) - step_start(k));
            for j in 0..count {
                expected.push(start + ((j * len) as f64 / count as f64).round() as usize);
            }
        }
        assert_eq!(found, expected, "BPM {bpm}");
    }
}

#[test]
fn retrigger_combines_with_reverse_and_pitch() {
    let (sample, slices) = block_sample();
    // Una octava arriba, el bloque dura 400 frames; al revés, suena al final
    // de los 1500 frames del corte (3000 / 2). Con r4 a 120 BPM, cada
    // subdisparo dura 1500 frames: cabe entero.
    let steps = parse_steps("1'^12r4 . . .  . . . .  . . . .  . . . .").unwrap();
    let pattern = Pattern::from_steps(sample.clone(), &slices, steps).unwrap();
    let l = render_pattern(pattern, 120.0, 6000);
    for j in 0..4 {
        let sub = &l[j * 1500..(j + 1) * 1500];
        assert!(sub[..1050].iter().all(|&s| s == 0.0), "subdisparo {j}");
        assert_eq!(sub[1300], 0.5, "subdisparo {j}");
    }
}

/// Renderiza un seno de un segundo con la cadena de efectos y devuelve el
/// RMS de la parte estable (lejos de fundidos y transitorios del filtro).
fn rms_through(freq: f32, chain: &[Fx]) -> f32 {
    let (sample, slices) = sample_from(&sine(freq, RATE as usize), 1);
    let steps = parse_steps("1 . . .  . . . .  . . . .  . . . .").unwrap();
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_fx(chain)
        .unwrap();
    let l = render_pattern(pattern, 120.0, RATE as usize);
    rms(&l[10_000..40_000])
}

#[test]
fn lowpass_and_highpass_filters() {
    let input = 0.5 / 2f32.sqrt();
    let ratio = |freq, fx| rms_through(freq, &[fx]) / input;

    // Paso bajo a 1 kHz: 200 Hz pasa, 8 kHz queda muy atenuado (12 dB/oct).
    assert!(ratio(200.0, Fx::lowpass(1000.0)) > 0.98);
    assert!(ratio(8000.0, Fx::lowpass(1000.0)) < 0.03);
    // Paso alto a 1 kHz: al revés.
    assert!(ratio(8000.0, Fx::highpass(1000.0)) > 0.98);
    assert!(ratio(200.0, Fx::highpass(1000.0)) < 0.06);
    // En el corte, -3 dB (q = 0.707).
    let at_cutoff = ratio(1000.0, Fx::lowpass(1000.0));
    assert!((at_cutoff - std::f32::consts::FRAC_1_SQRT_2).abs() < 0.02, "{at_cutoff}");
    // Dos filtros en cadena: paso banda.
    let band = [Fx::highpass(500.0), Fx::lowpass(2000.0)];
    assert!(rms_through(1000.0, &band) / input > 0.8);
    assert!(rms_through(8000.0, &band) / input < 0.08);
}

#[test]
fn bitcrusher_quantizes_and_holds() {
    let (sample, slices) = sample_from(&sine(110.0, RATE as usize / 2), 1);
    let steps = parse_steps("1 . . .  . . . .  . . . .  . . . .").unwrap();
    let crushed = |bits, downsample| {
        let pattern = Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_fx(&[Fx::bitcrush(bits, downsample)])
            .unwrap();
        render_pattern(pattern, 120.0, RATE as usize / 2)
    };

    // 2 bits: niveles de 0.5 en 0.5 (el seno tiene amplitud 0.5).
    let two_bits = crushed(2, 1);
    assert!(two_bits.iter().all(|&s| [-0.5, 0.0, 0.5].contains(&s)));
    assert!(two_bits.contains(&0.5) && two_bits.contains(&-0.5));

    // Downsample 4: cada valor se repite en bloques de 4 frames.
    let held = crushed(16, 4);
    for block in held[1000..20_000].chunks(4) {
        assert!(block.iter().all(|&s| s == block[0]), "{block:?}");
    }
    assert!(held[1000..20_000].windows(2).any(|w| w[0] != w[1]));
}

#[test]
fn effect_changes_enter_at_the_bar_and_stay_bounded() {
    let mut x = 0x7f4a_7c15u32;
    let noise: Vec<f32> = (0..RATE as usize)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            0.5 * ((x as f32 / u32::MAX as f32) * 2.0 - 1.0)
        })
        .collect();
    let (sample, slices) = sample_from(&noise, 1);
    let steps = parse_steps("1 . . .  1 . . .  1 . . .  1 . . .").unwrap();
    let with = |chain: &[Fx]| {
        Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_fx(chain)
            .unwrap()
    };
    let bar = bar_frames(120.0, RATE) as usize;
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_pattern(with(&[Fx::lowpass(300.0)])).unwrap();
    controller.start().unwrap();
    let mut l = left(&offline::render(&mut engine, bar / 2, 2));
    controller
        .set_pattern(with(&[Fx::highpass(5000.0), Fx::bitcrush(4, 2)]))
        .unwrap();
    l.extend(left(&offline::render(&mut engine, bar / 2 + bar, 2)));

    // Primer compás entero con el paso bajo (poco contenido rápido), el
    // segundo con paso alto + bitcrusher (mucho). Diferencia entre muestras
    // consecutivas como medida de agudos.
    let roughness = |x: &[f32]| rms(&x.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>());
    let before = roughness(&l[bar / 2..bar - 100]);
    let after = roughness(&l[bar + 1000..2 * bar]);
    assert!(after > 5.0 * before, "antes {before}, después {after}");
    assert!(l.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
}

#[test]
fn rejects_invalid_effects() {
    let (sample, slices) = block_sample();
    let steps = parse_steps("1 . . .  . . . .  . . . .  . . . .").unwrap();
    let try_fx = |chain: &[Fx]| {
        Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_fx(chain)
            .err()
    };
    let lowpass_q = |q| Fx::Filter {
        kind: engine::FilterKind::LowPass,
        cutoff_hz: 1000.0,
        q,
    };
    for chain in [
        vec![Fx::lowpass(5.0)],
        vec![Fx::highpass(30_000.0)],
        vec![Fx::lowpass(f32::NAN)],
        vec![lowpass_q(0.0)],
        vec![Fx::bitcrush(0, 1)],
        vec![Fx::bitcrush(8, 0)],
        vec![Fx::bitcrush(17, 1)],
        vec![Fx::lowpass(1000.0); 5],
    ] {
        assert!(
            matches!(try_fx(&chain), Some(SendError::InvalidFx(_))),
            "{chain:?}"
        );
    }
    assert!(try_fx(&[Fx::lowpass(1000.0); 4]).is_none());
}
