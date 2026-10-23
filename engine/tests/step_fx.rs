//! Efectos por paso: la cadena de un paso suena solo en su golpe (en su
//! voz), junto con la de su corte (manda el paso), con rampas que empiezan
//! en su paso; los demás golpes, bit a bit como sin ella.

use std::sync::Arc;

use engine::{Automation, Curve, Fx, Pattern, Sample, SliceFx, Slices, Step, new_engine, offline};

const RATE: u32 = 48_000;
/// Un paso a 120 BPM.
const STEP: usize = 6000;

/// Cuatro cortes de 3500 frames (más cortos que un paso) de un seno con
/// armónicos y 500 frames de silencio antes y después.
fn sample() -> (Arc<Sample>, Slices) {
    let mut mono = vec![0.0f32; 4 * 3500];
    for (k, s) in mono.iter_mut().enumerate() {
        let i = k % 3500;
        if (500..3000).contains(&i) {
            let t = i as f32 / RATE as f32;
            *s = 0.5 * (std::f32::consts::TAU * 220.0 * t).sin()
                + 0.25 * (std::f32::consts::TAU * 3300.0 * t).sin();
        }
    }
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 4);
    (sample, slices)
}

fn render(pattern: Pattern, steps: usize) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, steps * STEP, 2)
}

fn segment(audio: &[f32], from: usize, to: usize) -> &[f32] {
    &audio[2 * from..2 * to]
}

fn chain(fx: &[Fx], automation: &[Automation]) -> Option<SliceFx> {
    Some(SliceFx::new(fx, automation).unwrap())
}

/// 16 golpes de los cortes 0-3; el del paso `k` con `fx`.
fn steps_with(k: usize, fx: Option<SliceFx>) -> Vec<Option<Step>> {
    (0..16)
        .map(|i| {
            let step = Step::new(i % 4);
            Some(if i == k { step.with_fx(fx) } else { step })
        })
        .collect()
}

#[test]
fn only_the_hit_with_effects_changes() {
    let (sample, slices) = sample();
    let plain = render(
        Pattern::from_steps(sample.clone(), &slices, steps_with(99, None)).unwrap(),
        16,
    );
    // Sin memoria al acabar (callan con silencio): todo lo demás, bit a bit.
    let dry = chain(
        &[Fx::bitcrush(4, 3), Fx::distortion(8.0, 0.7), Fx::gain(0.5)],
        &[],
    );
    let crushed = render(
        Pattern::from_steps(sample.clone(), &slices, steps_with(5, dry)).unwrap(),
        16,
    );
    assert_eq!(segment(&crushed, 0, 5 * STEP), segment(&plain, 0, 5 * STEP));
    assert_ne!(
        segment(&crushed, 5 * STEP, 6 * STEP),
        segment(&plain, 5 * STEP, 6 * STEP)
    );
    assert_eq!(
        segment(&crushed, 6 * STEP, 16 * STEP),
        segment(&plain, 6 * STEP, 16 * STEP)
    );
    // Un filtro resonante suena aún su cola (100 ms tras acabar la voz):
    // pasada esa cola, otra vez bit a bit.
    let ringing = chain(
        &[Fx::Filter {
            kind: engine::FilterKind::LowPass,
            cutoff_hz: 900.0,
            q: 6.0,
        }],
        &[],
    );
    let filtered = render(
        Pattern::from_steps(sample.clone(), &slices, steps_with(5, ringing)).unwrap(),
        16,
    );
    let tail_end = 5 * STEP + 3500 + 4800;
    assert_eq!(
        segment(&filtered, 0, 5 * STEP),
        segment(&plain, 0, 5 * STEP)
    );
    assert_ne!(
        segment(&filtered, 5 * STEP, 6 * STEP),
        segment(&plain, 5 * STEP, 6 * STEP)
    );
    assert_eq!(
        segment(&filtered, tail_end, 16 * STEP),
        segment(&plain, tail_end, 16 * STEP)
    );
    // Sin efectos (o con una cadena vacía), el mismo patrón bit a bit.
    let empty = render(
        Pattern::from_steps(sample.clone(), &slices, steps_with(5, chain(&[], &[]))).unwrap(),
        16,
    );
    assert_eq!(empty, plain);
}

#[test]
fn the_same_chain_in_two_steps_is_stored_once_and_sounds_the_same() {
    let (sample, slices) = sample();
    let lp = chain(&[Fx::lowpass(600.0)], &[]);
    let steps: Vec<Option<Step>> = (0..16)
        .map(|i| Some(Step::new(0).with_fx(if i % 4 == 0 { lp } else { None })))
        .collect();
    let audio = render(
        Pattern::from_steps(sample.clone(), &slices, steps).unwrap(),
        16,
    );
    // Cada golpe con filtro empieza sin memoria: el 0 y el 8 suenan igual.
    assert_eq!(
        segment(&audio, 0, STEP - 1000),
        segment(&audio, 8 * STEP, 9 * STEP - 1000)
    );
}

/// Pico de un trozo.
fn peak(audio: &[f32]) -> f32 {
    audio.iter().fold(0.0, |m, x| m.max(x.abs()))
}

#[test]
fn a_ramp_of_the_hit_starts_at_its_step_and_holds_at_the_end() {
    let (sample, slices) = sample();
    // El filtro se abre de 200 a 12000 Hz durante el paso del golpe y,
    // con retrigger, sigue abriéndose en sus subdisparos.
    let sweep = |steps: f64| {
        chain(
            &[Fx::lowpass(200.0)],
            &[Automation {
                fx: 0,
                param: 0,
                curve: Curve::Hit {
                    from: 200.0,
                    to: 12_000.0,
                    steps,
                },
            }],
        )
    };
    let mut steps = vec![None; 16];
    steps[2] = Some(Step::new(0).with_retrig(4).with_fx(sweep(1.0)));
    steps[8] = Some(Step::new(0).with_fx(sweep(1.0)));
    let audio = render(
        Pattern::from_steps(sample.clone(), &slices, steps).unwrap(),
        16,
    );
    // Los agudos (3300 Hz) pasan cada vez más en los cuatro subdisparos.
    let part = STEP / 4;
    let sub = |j: usize| {
        let from = 2 * STEP + j * part;
        peak(segment(&audio, from + 600, from + 1400))
    };
    let levels: Vec<f32> = (0..4).map(sub).collect();
    assert!(levels.windows(2).all(|w| w[0] < w[1]), "{levels:?}");
    // El golpe del paso 8 vuelve a empezar desde 200 Hz (desde su paso).
    let start = peak(segment(&audio, 8 * STEP + 600, 8 * STEP + 1400));
    assert!(
        (start - levels[0]).abs() < 0.2 * levels[0],
        "{start} {levels:?}"
    );
    // Sin clics: el mayor salto entre muestras, como el del seno sin filtro.
    let plain = render(
        Pattern::from_steps(sample.clone(), &slices, steps_with(99, None)).unwrap(),
        16,
    );
    let jump = |a: &[f32]| {
        a.windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0, f32::max)
    };
    assert!(
        jump(&audio) <= 1.1 * jump(&plain),
        "{} {}",
        jump(&audio),
        jump(&plain)
    );
    // Al acabar la rampa se queda arriba: pasado el suavizado (~5 ms), como
    // el filtro fijo en 12000.
    let long = sweep(0.05);
    let mut steps = vec![None; 16];
    steps[0] = Some(Step::new(0).with_fx(long));
    let held = render(
        Pattern::from_steps(sample.clone(), &slices, steps).unwrap(),
        2,
    );
    let mut steps = vec![None; 16];
    steps[0] = Some(Step::new(0).with_fx(chain(&[Fx::lowpass(12_000.0)], &[])));
    let open = render(
        Pattern::from_steps(sample.clone(), &slices, steps).unwrap(),
        2,
    );
    let after = 300 + 2000;
    let diff = segment(&held, after, 3000)
        .iter()
        .zip(segment(&open, after, 3000))
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    assert!(diff < 1e-3, "{diff}");
}

#[test]
fn the_step_wins_over_its_slice() {
    let (sample, slices) = sample();
    let slice_chain = chain(&[Fx::lowpass(300.0)], &[]);
    let step_chain = chain(&[Fx::lowpass(5000.0)], &[]);
    let with_slice = |step_fx| {
        Pattern::from_steps(sample.clone(), &slices, steps_with(5, step_fx))
            .unwrap()
            .with_slice_fx(&[None, slice_chain])
            .unwrap()
    };
    // Paso 5: corte 1, que tiene un paso bajo de 300; el del paso (5000) lo
    // sustituye: suena como si el corte tuviera el de 5000.
    let replaced = render(with_slice(step_chain), 16);
    let expected = render(
        Pattern::from_steps(sample.clone(), &slices, steps_with(99, None))
            .unwrap()
            .with_slice_fx(&[None, step_chain])
            .unwrap(),
        16,
    );
    assert_eq!(
        segment(&replaced, 5 * STEP, 6 * STEP),
        segment(&expected, 5 * STEP, 6 * STEP)
    );
    // Los demás golpes del corte 1 siguen con su filtro de 300.
    let untouched = render(with_slice(None), 16);
    assert_eq!(
        segment(&replaced, 9 * STEP, 10 * STEP),
        segment(&untouched, 9 * STEP, 10 * STEP)
    );
    // Otro tipo de efecto: detrás de los del corte.
    let crush = chain(&[Fx::bitcrush(5, 1)], &[]);
    let appended = render(with_slice(crush), 16);
    let both = render(
        Pattern::from_steps(sample.clone(), &slices, steps_with(99, None))
            .unwrap()
            .with_slice_fx(&[None, chain(&[Fx::lowpass(300.0), Fx::bitcrush(5, 1)], &[])])
            .unwrap(),
        16,
    );
    assert_eq!(
        segment(&appended, 5 * STEP, 6 * STEP),
        segment(&both, 5 * STEP, 6 * STEP)
    );
    // Si no caben (3 del corte y 2 del paso), error al crear el patrón.
    let three = chain(
        &[Fx::lowpass(300.0), Fx::gain(1.0), Fx::distortion(2.0, 0.5)],
        &[],
    );
    let two = chain(&[Fx::bitcrush(5, 1), Fx::compressor(-20.0, 3.0)], &[]);
    assert!(
        Pattern::from_steps(sample.clone(), &slices, steps_with(5, two))
            .unwrap()
            .with_slice_fx(&[None, three])
            .is_err()
    );
}

#[test]
fn instruments_and_grains_have_no_step_effects() {
    let (sample, slices) = sample();
    let lp = chain(&[Fx::lowpass(600.0)], &[]);
    let pattern = || Pattern::from_steps(sample.clone(), &slices, steps_with(3, lp)).unwrap();
    assert!(
        pattern()
            .with_instrument(Some(engine::Instrument::new(4)))
            .is_err()
    );
    assert!(
        pattern()
            .with_granular(Some(engine::Granular::new(1)))
            .is_err()
    );
    // Una rampa de golpe no vale fuera de un paso.
    let hit = Curve::Hit {
        from: 200.0,
        to: 800.0,
        steps: 1.0,
    };
    assert!(
        pattern()
            .with_fx(&[Fx::lowpass(200.0)])
            .unwrap()
            .with_automation(&[Automation {
                fx: 0,
                param: 0,
                curve: hit
            }])
            .is_err()
    );
}

#[test]
fn eight_tracks_with_everything_stay_within_one() {
    let (sample, slices) = sample();
    let loud = chain(
        &[
            Fx::distortion(30.0, 1.0),
            Fx::gain(4.0),
            Fx::bitcrush(3, 2),
            Fx::lowpass(8000.0),
        ],
        &[Automation {
            fx: 3,
            param: 1,
            curve: Curve::Hit {
                from: 0.5,
                to: 18.0,
                steps: 2.0,
            },
        }],
    );
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = engine::Batch::new();
    batch.set_bpm(170.0);
    for t in 0..engine::MAX_TRACKS {
        let steps: Vec<Option<Step>> = (0..16)
            .map(|i| {
                Some(
                    Step::new(i % 4)
                        .with_gain(4.0)
                        .with_retrig(1 + (i % 3) as u8)
                        .with_fx(loud),
                )
            })
            .collect();
        let pattern = Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_gain(4.0)
            .unwrap()
            .with_fx(&[Fx::distortion(10.0, 1.0)])
            .unwrap()
            .with_slice_fx(&[chain(&[Fx::gain(4.0)], &[]), None, None, None])
            .unwrap()
            .with_sends([1.0, 1.0])
            .unwrap();
        batch.set_pattern(t, Some(pattern));
    }
    batch.set_buses(engine::Buses {
        delay: engine::DelayParams {
            feedback: 0.95,
            ..Default::default()
        },
        ..Default::default()
    });
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let audio = offline::render(&mut engine, 4 * 16 * STEP, 2);
    let max = peak(&audio);
    assert!(max <= 1.0, "{max}");
    assert!(max > 0.9, "{max}");
}
