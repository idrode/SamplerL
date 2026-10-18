//! Formantes al transponer con `%`: una vocal sintética (tren de pulsos
//! por tres resonadores) subida una quinta conserva su envolvente
//! espectral con la corrección y la desplaza sin ella.

use std::f64::consts::{PI, TAU};
use std::sync::Arc;

use engine::notation::parse_pattern_labeled;
use engine::{Pattern, Sample, Slices, new_engine, offline};

const RATE: u32 = 48_000;
/// Formantes de la vocal (Hz) y sus anchos de banda.
const FORMANTS: [(f64, f64); 3] = [(700.0, 80.0), (1200.0, 90.0), (2600.0, 120.0)];
const F0: f64 = 150.0;

/// Coeficientes (b0, a1, a2) de un resonador de dos polos.
fn resonator(freq: f64, bandwidth: f64) -> (f64, f64, f64) {
    let r = (-PI * bandwidth / RATE as f64).exp();
    let theta = TAU * freq / RATE as f64;
    (1.0 - r, -2.0 * r * theta.cos(), r * r)
}

/// Módulo de la envolvente de la vocal en `hz`, con los formantes
/// multiplicados por `scale`.
fn envelope(hz: f64, scale: f64) -> f64 {
    FORMANTS
        .iter()
        .map(|&(f, bw)| {
            let (b0, a1, a2) = resonator(f * scale, bw * scale);
            let w = TAU * hz / RATE as f64;
            // |1 + a1 e^-jw + a2 e^-2jw|
            let re = 1.0 + a1 * w.cos() + a2 * (2.0 * w).cos();
            let im = -(a1 * w.sin() + a2 * (2.0 * w).sin());
            b0 / (re * re + im * im).sqrt()
        })
        .product()
}

/// Un segundo de vocal: pulsos cada 320 frames (150 Hz) por los tres
/// resonadores en serie.
fn vowel() -> Arc<Sample> {
    let frames = RATE as usize;
    let period = (RATE as f64 / F0) as usize;
    let mut x: Vec<f64> = (0..frames)
        .map(|i| if i % period == 0 { 1.0 } else { 0.0 })
        .collect();
    for &(f, bw) in &FORMANTS {
        let (b0, a1, a2) = resonator(f, bw);
        let (mut y1, mut y2) = (0.0, 0.0);
        for v in x.iter_mut() {
            let y = b0 * *v - a1 * y1 - a2 * y2;
            (y2, y1) = (y1, y);
            *v = y;
        }
    }
    let peak = x.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let mono: Vec<f32> = x.iter().map(|v| (0.5 * v / peak) as f32).collect();
    Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap())
}

/// Canal izquierdo de la vocal tocada con `text` (un compás a 60 BPM).
fn play(text: &str, formant: Option<f32>) -> Vec<f32> {
    play_sample(vowel(), text, formant)
}

fn play_sample(sample: Arc<Sample>, text: &str, formant: Option<f32>) -> Vec<f32> {
    let slices = Slices::grid(&sample, 1);
    let steps = parse_pattern_labeled(text, 16, &[]).unwrap();
    let pattern = Pattern::from_steps(sample.clone(), &slices, steps)
        .unwrap()
        .with_formant(formant)
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(60.0).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, RATE as usize, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

/// Amplitud (dB) de cada armónico de `f0` hasta 4 kHz en el tramo central.
fn harmonics(x: &[f32], f0: f64) -> Vec<(f64, f64)> {
    let x = &x[x.len() / 5..x.len() * 4 / 5];
    let n = x.len() as f64;
    (1..)
        .map(|k| k as f64 * f0)
        .take_while(|&hz| hz < 4000.0)
        .map(|hz| {
            let w = TAU * hz / RATE as f64;
            let (mut c, mut s, mut sum) = (0.0, 0.0, 0.0);
            for (i, &v) in x.iter().enumerate() {
                let hann = 0.5 - 0.5 * (TAU * i as f64 / n).cos();
                c += hann * v as f64 * (w * i as f64).cos();
                s += hann * v as f64 * (w * i as f64).sin();
                sum += hann;
            }
            (
                hz,
                20.0 * (2.0 * (c * c + s * s).sqrt() / sum).max(1e-9).log10(),
            )
        })
        .collect()
}

/// Error (dB, cuadrático medio, con la mejor ganancia) entre los
/// armónicos medidos y la envolvente con los formantes por `scale`.
fn distance(measured: &[(f64, f64)], scale: f64) -> f64 {
    let diffs: Vec<f64> = measured
        .iter()
        .map(|&(hz, db)| db - 20.0 * envelope(hz, scale).log10())
        .collect();
    let mean = diffs.iter().sum::<f64>() / diffs.len() as f64;
    (diffs.iter().map(|d| (d - mean).powi(2)).sum::<f64>() / diffs.len() as f64).sqrt()
}

const UP: &str = "1%7 . . .  . . . .  . . . .  . . . .";

/// (error frente a los formantes donde estaban, frente a los movidos ×r).
fn errors(out: &[f32], semitones: f64) -> (f64, f64) {
    let ratio = 2f64.powf(semitones / 12.0);
    let measured = harmonics(out, F0 * ratio);
    (distance(&measured, 1.0), distance(&measured, ratio))
}

#[test]
fn the_vowel_is_what_it_says() {
    // Sin transponer, los armónicos siguen la envolvente.
    let out = play("1 . . .  . . . .  . . . .  . . . .", None);
    let (kept, _) = errors(&out, 0.0);
    assert!(kept < 1.5, "{kept:.2} dB");
}

/// Medio segundo de caja (ruido y tono de 190 Hz) con el cuerpo resonando
/// a 900 Hz (un resonador de 300 Hz de ancho).
fn snare() -> Arc<Sample> {
    snare_with(0.25)
}

/// La caja con el tono a ese nivel (la del break sintético: ruido a 0,55
/// y tono a 0,25, cayendo con τ = 90 ms).
fn snare_with(tone: f64) -> Arc<Sample> {
    let mut seed = 0x5eed_1234u32;
    let (b0, a1, a2) = resonator(900.0, 300.0);
    let (mut y1, mut y2) = (0.0f64, 0.0f64);
    let body: Vec<f64> = (0..RATE as usize / 2)
        .map(|i| {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let white = seed as f64 / u32::MAX as f64 * 2.0 - 1.0;
            let t = i as f64 / RATE as f64;
            let x = (t / 0.0005).min(1.0)
                * (-t / 0.09).exp()
                * (0.55 * white + tone * (TAU * 190.0 * t).sin());
            let y = b0 * x - a1 * y1 - a2 * y2;
            (y2, y1) = (y1, y);
            y
        })
        .collect();
    let peak = body.iter().fold(0.0f64, |m, v| m.max(v.abs()));
    let mono: Vec<f32> = body.iter().map(|v| (0.5 * v / peak) as f32).collect();
    Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap())
}

/// Frecuencia del pico de la envolvente (potencia en bandas de 200 Hz,
/// cada 25 Hz, de 450 Hz a 4 kHz: por encima del tono) de los primeros
/// 200 ms.
fn body_peak(x: &[f32]) -> f64 {
    let x = &x[..(RATE as usize / 5).min(x.len())];
    let n = x.len() as f64;
    let power: Vec<(f64, f64)> = (100..6000)
        .step_by(25)
        .map(f64::from)
        .map(|hz| {
            let w = TAU * hz / RATE as f64;
            let (mut c, mut s) = (0.0, 0.0);
            for (i, &v) in x.iter().enumerate() {
                let hann = 0.5 - 0.5 * (TAU * i as f64 / n).cos();
                c += hann * v as f64 * (w * i as f64).cos();
                s += hann * v as f64 * (w * i as f64).sin();
            }
            (hz, c * c + s * s)
        })
        .collect();
    let band = |hz: f64| -> f64 {
        power
            .iter()
            .filter(|(f, _)| (f - hz).abs() <= 100.0)
            .map(|(_, p)| p)
            .sum()
    };
    (450..4000)
        .step_by(25)
        .map(f64::from)
        .max_by(|a, b| band(*a).total_cmp(&band(*b)))
        .unwrap()
}

/// `cargo test -p engine --release --test formant -- --ignored --nocapture`
#[test]
#[ignore]
fn medida() {
    // Desplazar los formantes sin cambiar el pitch.
    for f in [5.0f32, -5.0] {
        let out = play("1%0 . . .  . . . .  . . . .  . . . .", Some(f));
        let measured = harmonics(&out, F0);
        let scale = 2f64.powf(f as f64 / 12.0);
        println!(
            "%0 con formant({f:+}): frente a la original {:.2} dB, frente a la desplazada {:.2} dB",
            distance(&measured, 1.0),
            distance(&measured, scale)
        );
    }
    // Una caja con cuerpo: su resonancia sube con el pitch y la corrección
    // la deja donde estaba.
    let plain = body_peak(&play_sample(
        snare(),
        "1 . . .  . . . .  . . . .  . . . .",
        None,
    ));
    for st in [7, -7] {
        let text = format!("1%{st} . . .  . . . .  . . . .  . . . .");
        let moved = body_peak(&play_sample(snare(), &text, None));
        let kept = body_peak(&play_sample(snare(), &text, Some(0.0)));
        println!(
            "caja {st:+}: cuerpo en {plain:.0} Hz → {moved:.0} sin corrección, {kept:.0} con formant(0)"
        );
    }
    for (name, formant) in [("sin corrección", None), ("formant(0)", Some(0.0))] {
        for st in [7.0, 12.0, -5.0, -12.0] {
            let text = format!("1%{st} . . .  . . . .  . . . .  . . . .");
            let out = play(&text, formant);
            let (kept, moved) = errors(&out, st);
            println!(
                "{name:15} {st:+}: frente a la envolvente original {kept:.2} dB, frente a la movida {moved:.2} dB"
            );
        }
    }
}

#[test]
fn up_a_fifth_the_envelope_stays_with_the_correction_and_moves_without() {
    let (kept, moved) = errors(&play(UP, Some(0.0)), 7.0);
    assert!(
        kept < 2.0,
        "con formant(0): {kept:.2} dB de la envolvente original"
    );
    assert!(
        moved > kept + 8.0,
        "con formant(0): {moved:.2} dB de la movida"
    );
    let (kept, moved) = errors(&play(UP, None), 7.0);
    assert!(
        moved < 2.0,
        "sin corrección: {moved:.2} dB de la envolvente movida"
    );
    assert!(
        kept > moved + 8.0,
        "sin corrección: {kept:.2} dB de la original"
    );
}

#[test]
fn down_an_octave_the_envelope_stays_too() {
    let down = "1%-12 . . .  . . . .  . . . .  . . . .";
    let (kept, moved) = errors(&play(down, Some(0.0)), -12.0);
    assert!(
        kept < 5.0 && moved > kept + 10.0,
        "{kept:.2} / {moved:.2} dB"
    );
}

#[test]
fn formants_move_on_their_own() {
    // Sin cambiar el pitch, los formantes cinco semitonos arriba.
    let out = play("1%0 . . .  . . . .  . . . .  . . . .", Some(5.0));
    let measured = harmonics(&out, F0);
    let shifted = distance(&measured, 2f64.powf(5.0 / 12.0));
    let original = distance(&measured, 1.0);
    assert!(
        shifted < 2.0 && original > shifted + 8.0,
        "{shifted:.2} / {original:.2} dB"
    );
}

#[test]
fn a_snare_body_stays_where_it_was() {
    let plain = body_peak(&play_sample(
        snare(),
        "1 . . .  . . . .  . . . .  . . . .",
        None,
    ));
    assert!((plain / 900.0 - 1.0).abs() < 0.1, "{plain}");
    for st in [7, -7] {
        let text = format!("1%{st} . . .  . . . .  . . . .  . . . .");
        let moved = body_peak(&play_sample(snare(), &text, None));
        let kept = body_peak(&play_sample(snare(), &text, Some(0.0)));
        let ratio = 2f64.powf(st as f64 / 12.0);
        assert!(
            (moved / (plain * ratio) - 1.0).abs() < 0.15,
            "{st}: sin corrección {moved}"
        );
        assert!(
            (kept / plain - 1.0).abs() < 0.15,
            "{st}: con corrección {kept}"
        );
    }
}

#[test]
fn without_formant_nothing_changes() {
    // `formant` solo actúa en los pasos con %: con ^ o sin pitch, y con
    // %0 y formant(0), lo de siempre bit a bit.
    for (text, formant) in [
        ("1 . 1^7 .  1' . . .  . . . .  . . . .", Some(0.0)),
        ("1%0 . . .  1%0' . . .  . . . .  . . . .", Some(0.0)),
        ("1%7 . . .  . . . .  . . . .  . . . .", None),
    ] {
        let plain = text.replace("%0", "");
        let reference = if formant.is_some() {
            play(&plain, None)
        } else {
            play(text, None)
        };
        assert!(play(text, formant) == reference, "{text}");
    }
    let sample = vowel();
    let slices = Slices::grid(&sample, 1);
    let steps = parse_pattern_labeled(UP, 16, &[]).unwrap();
    let pattern = Pattern::from_steps(sample, &slices, steps).unwrap();
    assert!(pattern.with_formant(Some(30.0)).is_err());
}

#[test]
fn an_instrument_keeps_its_formants() {
    let sample = vowel();
    let slices = Slices::grid(&sample, 1);
    let mut steps = vec![None; 16];
    steps[0] = Some(engine::Step::new(0).with_pitch(7.0).with_hold(15));
    let play_instrument = |formant: Option<f32>| {
        let instrument = engine::Instrument {
            keep: true,
            formant,
            ..engine::Instrument::new(2)
        };
        let pattern = Pattern::from_steps(sample.clone(), &slices, steps.clone())
            .unwrap()
            .with_instrument(Some(instrument))
            .unwrap();
        let (mut controller, mut engine) = new_engine(RATE);
        controller.set_bpm(60.0).unwrap();
        controller.set_pattern(pattern).unwrap();
        controller.start().unwrap();
        offline::render(&mut engine, RATE as usize, 2)
            .iter()
            .step_by(2)
            .copied()
            .collect::<Vec<f32>>()
    };
    let (kept, moved) = errors(&play_instrument(Some(0.0)), 7.0);
    assert!(
        kept < 2.0 && moved > kept + 8.0,
        "{kept:.2} / {moved:.2} dB"
    );
    let (kept, moved) = errors(&play_instrument(None), 7.0);
    assert!(
        moved < 2.0 && kept > moved + 8.0,
        "{kept:.2} / {moved:.2} dB"
    );
    // Sin keep no hay granos que corregir.
    let instrument = engine::Instrument {
        formant: Some(0.0),
        ..engine::Instrument::new(2)
    };
    assert!(instrument.validate(sample.frames()).is_err());
}
