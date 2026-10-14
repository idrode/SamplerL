//! Efectos por paso del secuenciador: volumen, pitch y reverse.

use std::sync::Arc;

use engine::{Pattern, STEPS, Sample, SendError, Slices, Step, new_engine, offline};

const RATE: u32 = 48_000;
/// 120 BPM: 6000 frames por paso, 96000 por compás.
const STEP_FRAMES: usize = 6000;

fn sample_from(mono: &[f32]) -> Arc<Sample> {
    Arc::new(Sample::from_interleaved(mono, 1, RATE, RATE).unwrap())
}

/// Ruido determinista que arranca desde silencio: un único corte.
fn noise_sample(frames: usize) -> Arc<Sample> {
    let mut x = 0x2545_f491u32;
    let mono: Vec<f32> = (0..frames)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            0.5 * ((x as f32 / u32::MAX as f32) * 2.0 - 1.0)
        })
        .collect();
    sample_from(&mono)
}

fn sine_sample(freq: f32, frames: usize) -> Arc<Sample> {
    let mono: Vec<f32> = (0..frames)
        .map(|i| 0.5 * (std::f32::consts::TAU * freq * i as f32 / RATE as f32).sin())
        .collect();
    sample_from(&mono)
}

/// Renderiza un compás con un único paso (el 0) y devuelve el canal izquierdo.
fn render_one_step(sample: &Arc<Sample>, slices: &Slices, step: Step, frames: usize) -> Vec<f32> {
    let mut steps = [None; STEPS];
    steps[0] = Some(step);
    let (mut controller, mut engine) = new_engine(RATE);
    controller
        .set_pattern(Pattern::from_steps(sample.clone(), slices, steps).unwrap())
        .unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

/// Frames con sonido (último no nulo + 1).
fn sounding_length(x: &[f32]) -> usize {
    x.iter().rposition(|&s| s != 0.0).map_or(0, |i| i + 1)
}

/// Frecuencia estimada en el tramo central: semiperiodos entre el primer y
/// el último cruce por cero, con la posición de cada cruce interpolada.
fn frequency(x: &[f32]) -> f32 {
    let mid = &x[x.len() / 4..3 * x.len() / 4];
    let crossings: Vec<f32> = mid
        .windows(2)
        .enumerate()
        .filter(|(_, w)| (w[0] < 0.0) != (w[1] < 0.0))
        .map(|(i, w)| i as f32 + w[0] / (w[0] - w[1]))
        .collect();
    let (first, last) = (crossings[0], crossings[crossings.len() - 1]);
    let half_periods = (crossings.len() - 1) as f32;
    half_periods / 2.0 / ((last - first) / RATE as f32)
}

#[test]
fn gain_scales_the_step() {
    let sample = noise_sample(4000);
    let slices = Slices::detect(&sample, 1);
    let full = render_one_step(&sample, &slices, Step::new(0), 5000);
    let half = render_one_step(&sample, &slices, Step::new(0).with_gain(0.5), 5000);
    assert!(full.iter().any(|&s| s.abs() > 0.3));
    for (i, (f, h)) in full.iter().zip(&half).enumerate() {
        assert!((h - 0.5 * f).abs() <= 1e-6, "frame {i}: {h} != 0.5 · {f}");
    }
}

#[test]
fn reverse_is_the_time_reversal_of_forward() {
    let sample = noise_sample(4000);
    let slices = Slices::detect(&sample, 1);
    let len = slices.get(0).unwrap().len();
    let forward = render_one_step(&sample, &slices, Step::new(0), 5000);
    let backward = render_one_step(&sample, &slices, Step::new(0).reversed(), 5000);

    // Los fundidos son simétricos, así que la igualdad es exacta.
    assert_eq!(sounding_length(&backward), sounding_length(&forward));
    for n in 0..len {
        assert_eq!(backward[n], forward[len - 1 - n], "frame {n}");
    }
}

#[test]
fn pitch_changes_speed_and_frequency() {
    let frames = 9600;
    let sample = sine_sample(300.0, frames);
    let slices = Slices::detect(&sample, 1);
    // El seno empieza en sin(0) = 0: el corte arranca en el primer frame con sonido.
    let len = slices.get(0).unwrap().len();
    assert!(len >= frames - 1);

    let render = |semitones: f32| {
        let step = Step::new(0).with_pitch(semitones);
        render_one_step(&sample, &slices, step, 3 * frames)
    };
    let (normal, up, down) = (render(0.0), render(12.0), render(-12.0));

    // Duración: una octava arriba dura la mitad; abajo, el doble.
    // (El fundido de salida acaba en 0: el último frame de la voz es silencio.)
    let voice_length = |x: &[f32]| sounding_length(x) + 1;
    assert_eq!(voice_length(&normal), len);
    assert_eq!(voice_length(&up), (len - 1) / 2 + 1);
    assert_eq!(voice_length(&down), 2 * (len - 1) + 1);

    // Frecuencia: el doble y la mitad (±0,5 %).
    let f0 = frequency(&normal[..len]);
    let f_up = frequency(&up[..sounding_length(&up)]);
    let f_down = frequency(&down[..sounding_length(&down)]);
    assert!((f0 - 300.0).abs() < 0.5, "f0 = {f0}");
    assert!((f_up / f0 - 2.0).abs() < 0.01, "arriba: {f_up} / {f0}");
    assert!((f_down / f0 - 0.5).abs() < 0.005, "abajo: {f_down} / {f0}");

    // Una quinta (+7) da una razón de 2^(7/12).
    let fifth = render(7.0);
    let f_fifth = frequency(&fifth[..sounding_length(&fifth)]);
    let expected = 2f32.powf(7.0 / 12.0);
    assert!(
        (f_fifth / f0 - expected).abs() < 0.02,
        "quinta: {}",
        f_fifth / f0
    );
}

#[test]
fn each_step_uses_its_own_effects_in_the_sequence() {
    // Dos cortes: bloque constante de 800 frames a 0.5 en 0 y a 0.25 en 3000.
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    let sample = sample_from(&mono);
    let slices = Slices::detect(&sample, 2);
    assert_eq!(slices.iter().collect::<Vec<_>>(), vec![0..3000, 3000..6000]);

    let mut steps = [None; STEPS];
    steps[0] = Some(Step::new(0).with_gain(0.5));
    steps[1] = Some(Step::new(1).reversed());
    steps[2] = Some(Step::new(0).with_pitch(12.0));
    let (mut controller, mut engine) = new_engine(RATE);
    controller
        .set_pattern(Pattern::from_steps(sample, &slices, steps).unwrap())
        .unwrap();
    controller.start().unwrap();
    let left: Vec<f32> = offline::render(&mut engine, 3 * STEP_FRAMES, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect();
    let step = |k: usize| &left[k * STEP_FRAMES..(k + 1) * STEP_FRAMES];

    // Paso 0: corte 0 a media ganancia.
    assert_eq!(step(0)[400], 0.25);

    // Paso 1: corte 1 al revés. El bloque está al final del corte, así que
    // suena en los últimos 800 frames de los 3000.
    let s1 = step(1);
    assert!(s1[..2200].iter().all(|&s| s == 0.0));
    assert_eq!(s1[2600], 0.25);
    assert_eq!(sounding_length(s1), 3000 - 1);

    // Paso 2: corte 0 una octava arriba: el bloque de 800 dura 400. El
    // filtro de la interpolación (que quita lo que pasaría del Nyquist)
    // suaviza el borde del bloque unos frames: se mide a media altura.
    let s2 = step(2);
    let half = s2.iter().rposition(|&s| s > 0.25).unwrap() + 1;
    assert_eq!(half, 400);
    assert!((s2[200] - 0.5).abs() < 1e-6, "{}", s2[200]);
    assert!(s2[420..].iter().all(|&s| s.abs() < 1e-3));
}

#[test]
fn rejects_out_of_range_step_parameters() {
    let sample = noise_sample(4000);
    let slices = Slices::detect(&sample, 1);
    let one = |step: Step| {
        let mut steps = [None; STEPS];
        steps[3] = Some(step);
        Pattern::from_steps(sample.clone(), &slices, steps).err()
    };
    let invalid = |e: Option<SendError>| matches!(e, Some(SendError::InvalidStep { step: 3, .. }));
    assert!(invalid(one(Step::new(0).with_gain(-0.1))));
    assert!(invalid(one(Step::new(0).with_gain(f32::NAN))));
    assert!(invalid(one(Step::new(0).with_gain(5.0))));
    assert!(invalid(one(Step::new(0).with_pitch(40.0))));
    assert!(invalid(one(Step::new(0).with_pitch(f32::NAN))));
    assert!(one(Step::new(0).with_gain(4.0).with_pitch(-36.0).reversed()).is_none());
}
