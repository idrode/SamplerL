//! Pitch sin cambiar la duración (`%` en los pasos, `keep` en los
//! instrumentos): la frecuencia sube o baja, la duración no, y con pitch 0
//! la salida es la de siempre bit a bit.

mod support;

use std::f64::consts::TAU;
use std::sync::Arc;

use engine::notation::parse_pattern_labeled;
use engine::{Instrument, Pattern, Sample, Slices, Step, new_engine, offline};
use support::synth_break;

const RATE: u32 = 48_000;

fn sine(freq: f64, seconds: f64) -> Arc<Sample> {
    let frames = (seconds * RATE as f64) as usize;
    let mono: Vec<f32> = (0..frames)
        .map(|i| (0.5 * (TAU * freq * i as f64 / RATE as f64).sin()) as f32)
        .collect();
    Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap())
}

fn the_break() -> Arc<Sample> {
    let (audio, _) = synth_break::synth_break();
    Arc::new(Sample::from_interleaved(&audio, 1, synth_break::RATE, RATE).unwrap())
}

/// Canal izquierdo de `frames` frames con el patrón en la pista 0, al
/// tempo `bpm`.
fn render(pattern: Pattern, bpm: f64, frames: usize) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(bpm).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

/// Patrón de 16 pasos en mini-notación.
fn pattern(sample: &Arc<Sample>, slices: &Slices, text: &str) -> Pattern {
    let steps = parse_pattern_labeled(text, 16, &[]).unwrap();
    Pattern::from_steps(sample.clone(), slices, steps).unwrap()
}

/// Frames hasta el último que suena.
fn duration(x: &[f32]) -> usize {
    x.iter().rposition(|v| v.abs() > 1e-3).map_or(0, |i| i + 1)
}

/// Frecuencia por los cruces por cero ascendentes (interpolados).
fn frequency(x: &[f32]) -> f64 {
    let crossings: Vec<f64> = (1..x.len())
        .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
        .map(|i| (i - 1) as f64 + (-x[i - 1] / (x[i] - x[i - 1])) as f64)
        .collect();
    let periods = (crossings.len() - 1) as f64;
    RATE as f64 * periods / (crossings[crossings.len() - 1] - crossings[0])
}

/// Un seno de medio segundo transpuesto con `%`: su duración y su
/// frecuencia.
fn kept_sine(semitones: i32) -> (usize, f64) {
    let sample = sine(440.0, 0.5);
    let slices = Slices::grid(&sample, 1);
    let text = format!("1%{semitones} . . .  . . . .  . . . .  . . . .");
    // A 60 BPM el paso dura 0,25 s y la voz sigue sonando después.
    let out = render(pattern(&sample, &slices, &text), 60.0, RATE as usize);
    let length = duration(&out);
    let middle = &out[length / 5..length * 4 / 5];
    (length, frequency(middle))
}

#[test]
fn a_sine_keeps_its_duration_and_changes_its_frequency() {
    let frames = sine(440.0, 0.5).frames();
    let ms = RATE as f64 / 1000.0;
    for semitones in [7, -12, 12, -5] {
        let (length, hz) = kept_sine(semitones);
        let expected = 440.0 * 2f64.powf(semitones as f64 / 12.0);
        let error_ms = (length as f64 - frames as f64).abs() / ms;
        assert!(
            error_ms < 1.0,
            "%{semitones}: dura {length} frames en vez de {frames} ({error_ms:.2} ms)"
        );
        let error = (hz / expected - 1.0).abs();
        assert!(
            error < 0.005,
            "%{semitones}: {hz:.2} Hz en vez de {expected:.2} ({:.3} %)",
            100.0 * error
        );
    }
    // Con ^ (sampler clásico) la duración sí cambia.
    let sample = sine(440.0, 0.5);
    let slices = Slices::grid(&sample, 1);
    let out = render(
        pattern(&sample, &slices, "1^12 . . .  . . . .  . . . .  . . . ."),
        60.0,
        RATE as usize,
    );
    assert!((duration(&out) as f64 - frames as f64 / 2.0).abs() < 48.0);
}

#[test]
fn kept_pitch_zero_is_bit_identical() {
    for (sample, grid) in [(sine(440.0, 0.5), true), (the_break(), false)] {
        let slices = if grid {
            Slices::grid(&sample, 3)
        } else {
            Slices::detect(&sample, 8)
        };
        let plain = "1 . 2 .  3' . 1 .  2 . 3r4 .  1@0.5 . 2 .";
        let kept = "1%0 . 2%0 .  3'%0 . 1%0 .  2%0 . 3%0r4 .  1%0@0.5 . 2%0 .";
        let bar = 2 * RATE as usize;
        let a = render(pattern(&sample, &slices.clone(), plain), 120.0, bar);
        let b = render(pattern(&sample, &slices, kept), 120.0, bar);
        assert!(a.iter().any(|v| *v != 0.0));
        assert!(a == b, "%0 no es idéntico a sin pitch");
    }
}

#[test]
fn reverse_and_gate_keep_the_duration_too() {
    let sample = sine(440.0, 0.5);
    let slices = Slices::grid(&sample, 1);
    let out = render(
        pattern(&sample, &slices, "1'%7 . . .  . . . .  . . . .  . . . ."),
        60.0,
        RATE as usize,
    );
    assert!((duration(&out) as f64 - sample.frames() as f64).abs() < 48.0);
    let hz = frequency(&out[RATE as usize / 10..RATE as usize * 4 / 10]);
    assert!(
        (hz / (440.0 * 2f64.powf(7.0 / 12.0)) - 1.0).abs() < 0.005,
        "{hz}"
    );
}

#[test]
fn an_instrument_with_keep_keeps_the_duration_of_every_note() {
    let sample = sine(440.0, 0.5);
    let slices = Slices::grid(&sample, 1);
    for keep in [false, true] {
        let instrument = Instrument {
            keep,
            ..Instrument::new(4)
        };
        // Una nota de una quinta y una octava abajo, cada una con su compás
        // entero (a 60 BPM: 4 s; dura lo que el sample).
        for semitones in [7.0f32, -12.0] {
            let mut steps = vec![None; 16];
            steps[0] = Some(Step::new(0).with_pitch(semitones).with_hold(15));
            let pattern = Pattern::from_steps(sample.clone(), &slices, steps)
                .unwrap()
                .with_instrument(Some(instrument))
                .unwrap();
            let out = render(pattern, 60.0, 2 * RATE as usize);
            let rate = 2f64.powf(semitones as f64 / 12.0);
            let expected = if keep {
                sample.frames() as f64
            } else {
                sample.frames() as f64 / rate
            };
            let length = duration(&out);
            assert!(
                (length as f64 - expected).abs() < 48.0,
                "keep {keep}, {semitones}: {length} frames en vez de {expected}"
            );
            let hz = frequency(&out[length / 5..length * 4 / 5]);
            assert!(
                (hz / (440.0 * rate) - 1.0).abs() < 0.005,
                "keep {keep}: {hz}"
            );
        }
    }
}

/// Golpes del break (su frame) tocados uno por paso: a 70 BPM una
/// semicorchea dura lo que una corchea del break a 140.
fn hits_per_step(text: &str) -> (Vec<f32>, Vec<usize>) {
    let sample = the_break();
    let slices = Slices::detect(&sample, 8);
    let starts: Vec<usize> = slices.iter().map(|r| r.start).collect();
    let out = render(pattern(&sample, &slices, text), 70.0, sample.frames());
    (out, starts)
}

#[test]
fn every_hit_of_a_break_starts_on_its_step() {
    let step = (RATE as f64 * 60.0 / 70.0 / 4.0).round() as usize;
    for text in [
        "1%7 2%7 3%7 4%7  5%7 6%7 7%7 8%7  . . . .  . . . .",
        "1%-12 2%-12 3%-12 4%-12  5%-12 6%-12 7%-12 8%-12  . . . .  . . . .",
    ] {
        let (out, _) = hits_per_step(text);
        // El ataque de cada golpe (la mayor subida de nivel de los agudos,
        // la diferencia entre frames, en ventanas de 0,5 ms a ±5 ms del
        // paso) cae a menos de 1 ms de su paso.
        let rms = |at: usize| {
            let w = &out[at - 1..at + 24];
            let sum = w.windows(2).map(|d| (d[1] - d[0]).powi(2)).sum::<f32>();
            (sum / 24.0).sqrt().max(1e-6)
        };
        for k in 1..8 {
            let expected = k * step;
            let attack = (expected - 240..expected + 240)
                .step_by(24)
                .max_by(|&a, &b| (rms(a) / rms(a - 24)).total_cmp(&(rms(b) / rms(b - 24))))
                .unwrap();
            assert!(
                attack.abs_diff(expected) <= 48,
                "{text}: el ataque del golpe {k} está en {attack} y no en {expected}"
            );
        }
        // Determinista.
        assert_eq!(hits_per_step(text).0, out);
    }
}

/// Envolvente en dB cada 2 ms.
fn envelope(x: &[f32]) -> Vec<f32> {
    x.chunks(96)
        .map(|c| {
            let rms = (c.iter().map(|v| v * v).sum::<f32>() / c.len() as f32).sqrt();
            20.0 * rms.max(1e-4).log10()
        })
        .collect()
}

/// Desviación media (dB) de la envolvente respecto al original y ataques
/// de más: subidas de más de 6 dB en 4 ms que el original no tiene.
fn smear(original: &[f32], shifted: &[f32]) -> (f32, usize) {
    let (a, b) = (envelope(original), envelope(shifted));
    let n = a.len().min(b.len());
    let distance = (0..n)
        .map(|i| (a[i].max(-50.0) - b[i].max(-50.0)).abs())
        .sum::<f32>()
        / n as f32;
    let extra = (2..n)
        .filter(|&i| b[i] - b[i - 2] > 6.0 && b[i] > -40.0 && a[i] - a[i - 2] < 3.0)
        .count();
    (distance, extra)
}

/// Pureza (dB) de un seno de `hz`: lo que explica una sinusoide ajustada
/// por bloques de 1024 frente al resto.
fn purity(x: &[f32], hz: f64) -> f64 {
    let w = TAU * hz / RATE as f64;
    let (mut signal, mut residue) = (0.0f64, 0.0f64);
    for block in x.as_chunks::<1024>().0 {
        let (mut cc, mut ss, mut cs, mut xc, mut xs) = (0.0, 0.0, 0.0, 0.0, 0.0);
        for (n, &v) in block.iter().enumerate() {
            let (c, s) = ((w * n as f64).cos(), (w * n as f64).sin());
            cc += c * c;
            ss += s * s;
            cs += c * s;
            xc += v as f64 * c;
            xs += v as f64 * s;
        }
        let det = cc * ss - cs * cs;
        let (a, b) = ((xc * ss - xs * cs) / det, (xs * cc - xc * cs) / det);
        for (n, &v) in block.iter().enumerate() {
            let fit = a * (w * n as f64).cos() + b * (w * n as f64).sin();
            signal += fit * fit;
            residue += (v as f64 - fit).powi(2);
        }
    }
    10.0 * (signal / residue).log10()
}

/// La medida que decidió la técnica: granos alineados en el motor (`%`)
/// frente al WSOLA de `stretched` al cargar más leer a la velocidad del
/// pitch (`^` sobre el sample estirado). `cargo test -p engine --release
/// --test pitch_keep -- --ignored --nocapture`.
#[test]
#[ignore]
fn medida_de_calidad() {
    let tone = sine(440.0, 1.0);
    let one = Slices::grid(&tone, 1);
    let brk = the_break();
    let slices = Slices::detect(&brk, 8);
    let starts: Vec<usize> = slices.iter().map(|r| r.start).collect();
    let plain = render(
        pattern(&brk, &slices, "1 2 3 4  5 6 7 8  . . . .  . . . ."),
        70.0,
        brk.frames(),
    );
    let row = |st: i32| {
        let steps = |mark: char| {
            let hits: Vec<String> = (1..=8).map(|i| format!("{i}{mark}{st}")).collect();
            format!("{}  . . . .  . . . .", hits.join(" "))
        };
        let single = |mark: char| format!("1{mark}{st} . . .  . . . .  . . . .  . . . .");
        let ratio = 2f64.powf(st as f64 / 12.0);
        let hz = 440.0 * ratio;
        // WSOLA al cargar: el sample estirado ×ratio, leído a ×ratio.
        let tone_long = Arc::new(tone.stretched(ratio));
        let tone_long_slices = one.scaled(ratio, tone_long.frames());
        let brk_long = Arc::new(brk.stretched_with(ratio, &starts));
        let brk_long_slices = slices.scaled(ratio, brk_long.frames());
        for (name, tone_out, brk_out) in [
            (
                "granos (%)",
                render(pattern(&tone, &one, &single('%')), 30.0, tone.frames()),
                render(pattern(&brk, &slices, &steps('%')), 70.0, brk.frames()),
            ),
            (
                "WSOLA al cargar",
                render(
                    pattern(&tone_long, &tone_long_slices, &single('^')),
                    30.0,
                    tone.frames(),
                ),
                render(
                    pattern(&brk_long, &brk_long_slices, &steps('^')),
                    70.0,
                    brk.frames(),
                ),
            ),
        ] {
            let middle = &tone_out[tone_out.len() / 10..tone_out.len() * 9 / 10];
            let f = frequency(middle);
            let (distance, extra) = smear(&plain, &brk_out);
            println!(
                "{st:+3} {name:16} seno: {:+.3} % · pureza {:.1} dB · break: envolvente {distance:.2} dB, ataques de más {extra}",
                100.0 * (f / hz - 1.0),
                purity(middle, hz)
            );
        }
    };
    for st in [7, -12, 12, -5, 24, -24] {
        row(st);
    }
    // Coste: una voz sonando todo el rato, con ^ y con %.
    let long = sine(440.0, 3.0);
    let whole = Slices::grid(&long, 1);
    for text in [
        "1^7 . . .  . . . .  . . . .  . . . .",
        "1%7 . . .  . . . .  . . . .  . . . .",
    ] {
        let start = std::time::Instant::now();
        render(pattern(&long, &whole, text), 20.0, long.frames());
        let seconds = start.elapsed().as_secs_f64();
        println!(
            "{text}: {:.2} % de un núcleo por voz",
            100.0 * seconds / 3.0
        );
    }
}
