//! Interpolación del pitch: aliasing medido con un seno agudo.
//!
//! Se renderiza un seno subido de tono con el motor y se ajusta por mínimos
//! cuadrados el seno que debería salir (a la frecuencia multiplicada por la
//! proporción, o nada si pasa del Nyquist de la salida); lo que queda son
//! las componentes espurias, en dB respecto a la potencia del seno. «Antes»
//! es la interpolación lineal de siempre, calculada aquí sobre las mismas
//! posiciones de lectura.

use std::f64::consts::TAU;
use std::sync::Arc;

use engine::{Pattern, STEPS, Sample, Slices, Step, new_engine, offline};

const RATE: u32 = 48_000;
const AMPLITUDE: f64 = 0.5;

fn sine(freq: f64, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|i| (AMPLITUDE * (TAU * freq * i as f64 / RATE as f64).sin()) as f32)
        .collect()
}

/// Canal izquierdo de un compás con el sample entero en el paso 0.
fn render(mono: &[f32], semitones: f32, frames: usize) -> Vec<f32> {
    let sample = Arc::new(Sample::from_interleaved(mono, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    let mut steps = [None; STEPS];
    steps[0] = Some(Step::new(0).with_pitch(semitones));
    let (mut controller, mut engine) = new_engine(RATE);
    controller
        .set_pattern(Pattern::from_steps(sample, &slices, steps).unwrap())
        .unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

/// La interpolación lineal de antes, en las mismas posiciones (n · r).
fn linear(x: &[f32], rate: f64, frames: usize) -> Vec<f32> {
    (0..frames)
        .map(|n| {
            let pos = n as f64 * rate;
            let i = pos.floor() as usize;
            let t = (pos - i as f64) as f32;
            let (a, b) = (x[i], x[(i + 1).min(x.len() - 1)]);
            a + t * (b - a)
        })
        .collect()
}

/// Potencia de lo que no es el seno esperado, en dB respecto al seno de
/// entrada, y la amplitud del seno ajustado (relativa a la de entrada). Se
/// mide en `y[skip..len - skip]`, lejos de los fundidos.
fn spurious(y: &[f32], freq: f64, skip: usize) -> (f64, f64) {
    let y = &y[skip..y.len() - skip];
    let w = TAU * freq / RATE as f64;
    let basis = |k: usize| ((w * k as f64).cos(), (w * k as f64).sin());
    let (a, b) = if freq < RATE as f64 / 2.0 {
        let (mut cc, mut ss, mut cs, mut yc, mut ys) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (k, &v) in y.iter().enumerate() {
            let (c, s) = basis(k);
            cc += c * c;
            ss += s * s;
            cs += c * s;
            yc += v as f64 * c;
            ys += v as f64 * s;
        }
        let det = cc * ss - cs * cs;
        ((yc * ss - ys * cs) / det, (ys * cc - yc * cs) / det)
    } else {
        // Pasa del Nyquist: lo ideal es que no salga nada.
        (0.0, 0.0)
    };
    let residual: f64 = y
        .iter()
        .enumerate()
        .map(|(k, &v)| {
            let (c, s) = basis(k);
            (v as f64 - a * c - b * s).powi(2)
        })
        .sum::<f64>()
        / y.len() as f64;
    let power = AMPLITUDE * AMPLITUDE / 2.0;
    (
        10.0 * (residual / power).max(1e-30).log10(),
        (a * a + b * b).sqrt() / AMPLITUDE,
    )
}

struct Measure {
    before: f64,
    after: f64,
    amplitude: f64,
}

fn measure(freq: f64, semitones: f32) -> Measure {
    let x = sine(freq, RATE as usize);
    let rate = 2f64.powf(semitones as f64 / 12.0);
    let frames = ((x.len() - 1) as f64 / rate) as usize;
    let target = freq * rate;
    let skip = 600;
    let after = render(&x, semitones, frames);
    let (after_db, amplitude) = spurious(&after[..frames], target, skip);
    let (before_db, _) = spurious(&linear(&x, rate, frames), target, skip);
    println!(
        "{freq} Hz {semitones:+} → {target:.0} Hz: lineal {before_db:.1} dB, ahora {after_db:.1} dB (amplitud {amplitude:.3})"
    );
    Measure {
        before: before_db,
        after: after_db,
        amplitude,
    }
}

/// Un seno agudo subido una y dos octavas cuya nota pasa del Nyquist: con
/// la lineal se pliega entero (a 18, 16 y 8 kHz); ahora se filtra. (Lo
/// que cae entre 0,9 y 1 del Nyquist nuevo, la banda de transición, solo
/// se atenúa en parte: 13 kHz +12 queda a unos -30 dB.)
#[test]
fn pitching_up_past_nyquist_does_not_fold_back() {
    for (freq, semitones) in [(15_000.0, 12.0), (8_000.0, 24.0), (20_000.0, 12.0)] {
        let m = measure(freq, semitones);
        assert!(m.before > -1.0, "lineal: {:.1} dB", m.before);
        assert!(m.after < -75.0, "{freq} Hz +{semitones}: {:.1} dB", m.after);
    }
}

/// Subido +12 y +24 sin pasar del Nyquist: la nota sale limpia y con su
/// nivel (las posiciones son enteras: la lineal tampoco añadía nada).
#[test]
fn pitching_up_below_nyquist_stays_clean() {
    for (freq, semitones) in [(5_000.0, 12.0), (8_000.0, 12.0), (4_000.0, 24.0)] {
        let m = measure(freq, semitones);
        assert!(m.after < -90.0, "{freq} Hz +{semitones}: {:.1} dB", m.after);
        assert!((m.amplitude - 1.0).abs() < 0.03, "amplitud {}", m.amplitude);
    }
}

/// Con proporciones no enteras (una quinta, una octava abajo) la lineal
/// deja imágenes; la sinc, casi nada.
#[test]
fn fractional_rates_have_much_less_imaging() {
    for (freq, semitones) in [(5_000.0, 7.0), (11_000.0, 7.0), (15_000.0, -12.0)] {
        let m = measure(freq, semitones);
        assert!(m.before > -40.0, "lineal: {:.1} dB", m.before);
        assert!(
            m.after < -85.0,
            "{freq} Hz {semitones:+}: {:.1} dB",
            m.after
        );
        assert!((m.amplitude - 1.0).abs() < 0.02, "amplitud {}", m.amplitude);
    }
}

/// Con pitch 0 la voz lee el sample tal cual: idéntico bit a bit fuera de
/// los fundidos.
#[test]
fn pitch_zero_is_bit_exact() {
    let x = sine(15_000.0, 20_000);
    let out = render(&x, 0.0, 20_000);
    for i in 200..19_000 {
        assert_eq!(out[i].to_bits(), x[i].to_bits(), "frame {i}");
    }
}
