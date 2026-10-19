//! EQ paramétrico (`Fx::Eq`): respuesta en frecuencia medida con senos, sin
//! cambios es idéntico bit a bit a no tenerlo, los cambios al compás llegan
//! suavizados y la automatización mueve sus bandas.

use std::sync::Arc;

use engine::{
    Automation, Curve, EqBand, EqKind, Fx, LfoRate, Pattern, Sample, Slices, Step, new_engine,
    offline,
};

const RATE: u32 = 48_000;
const BPM: f64 = 120.0;

/// Un seno de `freq` Hz que dura `seconds` (con un fundido corto al final).
fn sine_sample(freq: f32, seconds: f32) -> (Arc<Sample>, Slices) {
    let len = (seconds * RATE as f32) as usize;
    let mono: Vec<f32> = (0..len)
        .map(|i| {
            let fade = ((len - i) as f32 / 480.0).min(1.0);
            0.25 * fade * (std::f32::consts::TAU * freq * i as f32 / RATE as f32).sin()
        })
        .collect();
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    (sample, slices)
}

/// Un solo golpe en el primer paso; el resto, silencio.
fn one_hit() -> Vec<Option<Step>> {
    let mut steps = vec![None; 16];
    steps[0] = Some(Step::new(0));
    steps
}

/// Renderiza `frames` frames: el primer patrón desde el principio y cada
/// uno de los siguientes un compás después del anterior.
fn render(patterns: Vec<Pattern>, frames: usize) -> Vec<f32> {
    let mut patterns = patterns.into_iter();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(BPM).unwrap();
    controller.set_pattern(patterns.next().unwrap()).unwrap();
    controller.start().unwrap();
    let mut out = Vec::new();
    let bar = engine::bar_frames(BPM, RATE) as usize;
    for next in patterns {
        out.extend(offline::render(&mut engine, bar, 2));
        controller.set_pattern(next).unwrap();
    }
    let rest = frames.saturating_sub(out.len() / 2);
    out.extend(offline::render(&mut engine, rest, 2));
    out
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

/// Ganancia en dB del EQ `fx` sobre un seno de `freq` (medida en régimen,
/// lejos del ataque y del final).
fn gain_db(fx: Fx, freq: f32) -> f32 {
    let (sample, slices) = sine_sample(freq, 1.0);
    let measure = |chain: &[Fx]| {
        let p = Pattern::from_steps(sample.clone(), &slices, one_hit())
            .unwrap()
            .with_fx(chain)
            .unwrap();
        let out = left(&render(vec![p], RATE as usize));
        rms(&out[RATE as usize / 4..3 * RATE as usize / 4])
    };
    20.0 * (measure(&[fx]) / measure(&[])).log10()
}

fn near(got: f32, want: f32, tolerance: f32) -> bool {
    (got - want).abs() <= tolerance
}

#[test]
fn a_bell_boosts_and_cuts_at_its_centre_and_leaves_far_bands_alone() {
    let boost = Fx::eq(&[EqBand::bell(1000.0, 6.0, 1.0)]);
    let at = gain_db(boost, 1000.0);
    assert!(near(at, 6.0, 0.2), "+6 dB en el centro: {at}");
    for far in [50.0, 20_000.0 / 1.2] {
        let g = gain_db(boost, far);
        assert!(near(g, 0.0, 0.3), "lejos ({far} Hz) casi nada: {g}");
    }
    // Media octava por encima con q 1: algo, pero menos que en el centro.
    let half = gain_db(boost, 1414.0);
    assert!(half > 2.0 && half < 5.5, "{half}");

    let cut = Fx::eq(&[EqBand::bell(300.0, -4.0, 4.0)]);
    let at = gain_db(cut, 300.0);
    assert!(near(at, -4.0, 0.2), "−4 dB en el centro: {at}");
    // Estrecha (q 4): a una octava ya casi no hace nada.
    let g = gain_db(cut, 600.0);
    assert!(near(g, 0.0, 0.5), "{g}");
}

#[test]
fn the_shelves_move_everything_beyond_their_frequency() {
    let low = Fx::eq(&[EqBand {
        kind: EqKind::LowShelf,
        freq_hz: 200.0,
        gain_db: 6.0,
        q: 0.707,
    }]);
    assert!(near(gain_db(low, 40.0), 6.0, 0.3));
    assert!(near(gain_db(low, 5000.0), 0.0, 0.2));
    let high = Fx::eq(&[EqBand {
        kind: EqKind::HighShelf,
        freq_hz: 4000.0,
        gain_db: -9.0,
        q: 0.707,
    }]);
    assert!(near(gain_db(high, 16_000.0), -9.0, 0.4));
    assert!(near(gain_db(high, 200.0), 0.0, 0.2));
}

#[test]
fn several_bands_add_up() {
    let eq = Fx::eq(&[
        EqBand::bell(200.0, -3.0, 2.0),
        EqBand::flat(1),
        EqBand::bell(5000.0, 4.0, 2.0),
    ]);
    assert!(near(gain_db(eq, 200.0), -3.0, 0.25));
    assert!(near(gain_db(eq, 5000.0), 4.0, 0.25));
    assert!(near(gain_db(eq, 1000.0), 0.0, 0.4));
}

#[test]
fn a_flat_eq_is_bit_identical_to_none() {
    let (sample, slices) = sine_sample(440.0, 0.6);
    let flat = Fx::eq(&[EqBand::bell(300.0, 0.0, 6.0), EqBand::flat(1)]);
    let pattern = |chain: &[Fx]| {
        let steps: Vec<Option<Step>> = (0..16)
            .map(|k| (k % 3 == 0).then(|| Step::new(0)))
            .collect();
        Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_fx(chain)
            .unwrap()
    };
    let frames = 2 * engine::bar_frames(BPM, RATE) as usize;
    let reference = render(vec![pattern(&[])], frames);
    assert_eq!(render(vec![pattern(&[flat])], frames), reference);
    // Y entrar o salir de un EQ plano al compás tampoco cambia nada.
    assert_eq!(
        render(vec![pattern(&[]), pattern(&[flat])], frames),
        reference
    );
}

#[test]
fn a_new_gain_at_the_bar_glides_instead_of_jumping() {
    // Un seno largo que sigue sonando a través del cambio de compás.
    let (sample, slices) = sine_sample(1000.0, 3.0);
    let first = Pattern::from_steps(sample.clone(), &slices, one_hit())
        .unwrap()
        .with_fx(&[Fx::eq(&[EqBand::bell(1000.0, 0.0, 1.0)])])
        .unwrap();
    let second = Pattern::from_steps(sample, &slices, vec![None; 16])
        .unwrap()
        .with_fx(&[Fx::eq(&[EqBand::bell(1000.0, -12.0, 1.0)])])
        .unwrap();
    let bar = engine::bar_frames(BPM, RATE) as usize;
    let out = left(&render(vec![first, second], bar + RATE as usize / 2));
    let window = |at: usize| rms(&out[at..at + 96]);
    let before = window(bar - 480);
    let just_after = window(bar);
    let later = window(bar + RATE as usize / 10);
    let db = |x: f32| 20.0 * (x / before).log10();
    // Justo tras el compás todavía casi como antes; a los 100 ms, −12 dB.
    assert!(db(just_after) > -3.0, "salto: {} dB", db(just_after));
    assert!(near(db(later), -12.0, 0.3), "{}", db(later));
    // Sin escalones: la diferencia entre muestras seguidas no pasa de la
    // del propio seno.
    let step = out[bar - 200..bar + 2000]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    let sine_step = 0.25 * std::f32::consts::TAU * 1000.0 / RATE as f32;
    assert!(step <= sine_step * 1.05, "{step} > {sine_step}");
}

#[test]
fn automation_moves_a_band() {
    let (sample, slices) = sine_sample(1000.0, 2.0);
    let pattern = |automation: &[Automation]| {
        Pattern::from_steps(sample.clone(), &slices, one_hit())
            .unwrap()
            .with_fx(&[Fx::eq(&[EqBand::bell(1000.0, 0.0, 1.0)])])
            .unwrap()
            .with_automation(automation)
            .unwrap()
    };
    // gain1 (parámetro 1) de 0 a −12 dB y vuelta, una vez por compás.
    let lfo = [Automation {
        fx: 0,
        param: 1,
        curve: Curve::Lfo {
            min: 0.0,
            max: -12.0,
            rate: LfoRate::Steps(16.0),
        },
    }];
    let bar = engine::bar_frames(BPM, RATE) as usize;
    let still = left(&render(vec![pattern(&[])], bar));
    let moved = left(&render(vec![pattern(&lfo)], bar));
    let window = |x: &[f32], at: usize| rms(&x[at..at + 480]);
    // Al principio del ciclo el LFO está en 0 dB; a mitad, en −12.
    let start = 20.0 * (window(&moved, 2400) / window(&still, 2400)).log10();
    let middle = 20.0 * (window(&moved, bar / 2) / window(&still, bar / 2)).log10();
    assert!(near(start, 0.0, 0.5), "{start}");
    assert!(near(middle, -12.0, 0.5), "{middle}");
    // Determinista.
    assert_eq!(left(&render(vec![pattern(&lfo)], bar)), moved);
}

#[test]
fn out_of_range_bands_are_rejected() {
    let bad = |band: EqBand| Fx::eq(&[band]).validate().is_err();
    assert!(bad(EqBand::bell(10.0, 3.0, 1.0)));
    assert!(bad(EqBand::bell(1000.0, 30.0, 1.0)));
    assert!(bad(EqBand::bell(1000.0, 3.0, 0.01)));
    assert!(!bad(EqBand::bell(1000.0, -24.0, 18.0)));
    let eq = Fx::eq(&[]);
    assert_eq!(eq.params().len(), engine::MAX_PARAMS);
    assert_eq!(eq.param_index("gain3"), Some(7));
}
