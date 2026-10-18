//! Granular y congelar: cuántos granos nacen, de dónde leen, el azar con
//! semilla, las curvas y que la suma de muchas nubes no pase de ±1.

use std::f64::consts::TAU;
use std::sync::Arc;

use engine::notation::parse_pattern_labeled;
use engine::{
    Curve, Granular, Knob, LfoRate, Pattern, Sample, SendError, Slices, new_engine, offline,
};

const RATE: u32 = 48_000;

fn sample_from(mono: &[f32]) -> Arc<Sample> {
    Arc::new(Sample::from_interleaved(mono, 1, RATE, RATE).unwrap())
}

/// Un segundo de continua a 0,5.
fn dc() -> Arc<Sample> {
    sample_from(&vec![0.5; RATE as usize])
}

/// Un segundo: la primera mitad, un seno de 300 Hz; la segunda, de 900.
fn two_tones() -> Arc<Sample> {
    let mono: Vec<f32> = (0..RATE as usize)
        .map(|i| {
            let hz = if i < RATE as usize / 2 { 300.0 } else { 900.0 };
            (0.5 * (TAU * hz * i as f64 / RATE as f64).sin()) as f32
        })
        .collect();
    sample_from(&mono)
}

fn granular(sample: &Arc<Sample>, text: &str, params: Granular) -> Pattern {
    let slices = Slices::grid(sample, 1);
    let steps = parse_pattern_labeled(text, 16, &[]).unwrap();
    Pattern::from_steps(sample.clone(), &slices, steps)
        .unwrap()
        .with_granular(Some(params))
        .unwrap()
}

/// Todo el compás: una nube que dura los 16 pasos.
const WHOLE_BAR: &str = "1 _ _ _  _ _ _ _  _ _ _ _  _ _ _ _";

/// Canal izquierdo y granos que nacieron, con los patrones en sus pistas.
fn render(patterns: Vec<Pattern>, frames: usize) -> (Vec<f32>, u32) {
    let (mut controller, mut engine) = new_engine(RATE);
    for (t, p) in patterns.into_iter().enumerate() {
        controller.set_track_pattern(t, Some(p)).unwrap();
    }
    controller.start().unwrap();
    let audio = offline::render(&mut engine, frames, 2);
    let grains = controller.status().tracks.iter().map(|t| t.grains).sum();
    (audio.iter().step_by(2).copied().collect(), grains)
}

fn fixed(v: f32) -> Knob {
    Knob::Fixed(v)
}

/// Granos de 5 ms, sin azar, sobre continua: cada uno es una campana.
fn bumps(x: &[f32]) -> usize {
    (1..x.len())
        .filter(|&i| x[i - 1] <= 0.25 && x[i] > 0.25)
        .count()
}

#[test]
fn density_gives_the_grains_per_second() {
    let sample = dc();
    for density in [4.0, 10.0, 25.0, 60.0] {
        let params = Granular {
            size_ms: fixed(5.0),
            density: fixed(density),
            ..Granular::new(1)
        };
        // A 120 BPM, un compás dura 2 s.
        let (out, grains) = render(
            vec![granular(&sample, WHOLE_BAR, params)],
            2 * RATE as usize,
        );
        let expected = (2.0 * density) as u32;
        assert_eq!(grains, expected, "density {density}: granos según el motor");
        assert_eq!(
            bumps(&out) as u32,
            expected,
            "density {density}: granos en el audio"
        );
    }
}

#[test]
fn grains_only_start_while_the_step_lasts() {
    // Un paso a 120 BPM: 125 ms; con 40 granos por segundo nacen 5.
    let params = Granular {
        size_ms: fixed(5.0),
        density: fixed(40.0),
        ..Granular::new(1)
    };
    let pattern = granular(&dc(), "1 . . .  . . . .  . . . .  . . . .", params);
    let (_, grains) = render(vec![pattern], RATE as usize);
    assert_eq!(grains, 5);
    // Con `_`, el doble.
    let pattern = granular(&dc(), "1 _ . .  . . . .  . . . .  . . . .", params);
    let (_, grains) = render(vec![pattern], RATE as usize);
    assert_eq!(grains, 10);
}

fn frequency(x: &[f32]) -> f64 {
    let crossings: Vec<f64> = (1..x.len())
        .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
        .map(|i| (i - 1) as f64 + (-x[i - 1] / (x[i] - x[i - 1])) as f64)
        .collect();
    RATE as f64 * (crossings.len() - 1) as f64 / (crossings[crossings.len() - 1] - crossings[0])
}

#[test]
fn freeze_reads_one_place_and_pitch_moves_it() {
    let sample = two_tones();
    // Granos que se solapan sin jitter: un tono continuo.
    for (pos, pitch, expected) in [
        (0.25, 0.0, 300.0),
        (0.75, 0.0, 900.0),
        (0.75, 12.0, 1800.0),
        (0.25, -12.0, 150.0),
    ] {
        let params = Granular {
            size_ms: fixed(40.0),
            density: fixed(50.0),
            position: fixed(pos),
            speed: fixed(0.0),
            pitch: fixed(pitch),
            ..Granular::new(1)
        };
        let (out, _) = render(vec![granular(&sample, WHOLE_BAR, params)], RATE as usize);
        let hz = frequency(&out[RATE as usize / 10..]);
        assert!(
            (hz / expected - 1.0).abs() < 0.01,
            "pos {pos}, pitch {pitch}: {hz:.1} Hz en vez de {expected}"
        );
    }
    // Con speed 1 la lectura avanza: al final del primer medio segundo
    // suena ya la segunda mitad.
    let params = Granular {
        size_ms: fixed(40.0),
        density: fixed(50.0),
        speed: fixed(1.0),
        ..Granular::new(1)
    };
    let (out, _) = render(vec![granular(&sample, WHOLE_BAR, params)], RATE as usize);
    let early = frequency(&out[RATE as usize / 10..RATE as usize * 4 / 10]);
    let late = frequency(&out[RATE as usize * 6 / 10..RATE as usize * 9 / 10]);
    assert!((early / 300.0 - 1.0).abs() < 0.02, "{early}");
    assert!((late / 900.0 - 1.0).abs() < 0.02, "{late}");
}

#[test]
fn the_seed_makes_it_repeatable() {
    let sample = two_tones();
    let params = |seed| Granular {
        size_ms: fixed(60.0),
        density: fixed(30.0),
        position: fixed(0.5),
        speed: fixed(0.0),
        jitter_ms: fixed(200.0),
        spread: fixed(7.0),
        ..Granular::new(seed)
    };
    let a = render(vec![granular(&sample, WHOLE_BAR, params(7))], RATE as usize).0;
    let b = render(vec![granular(&sample, WHOLE_BAR, params(7))], RATE as usize).0;
    let c = render(vec![granular(&sample, WHOLE_BAR, params(8))], RATE as usize).0;
    assert!(a.iter().any(|v| *v != 0.0));
    assert!(a == b, "misma semilla, otra salida");
    assert!(a != c, "otra semilla, misma salida");
}

#[test]
fn eight_dense_clouds_never_pass_one() {
    // Lo más fuerte que se puede pedir: continua a 1, granos largos y
    // densos (32 a la vez), volumen de paso y de pista al máximo.
    let loud = sample_from(&vec![1.0; RATE as usize]);
    let slices = Slices::grid(&loud, 1);
    let patterns: Vec<Pattern> = (0..8)
        .map(|t| {
            let steps =
                parse_pattern_labeled("1@4 _ _ _  _ _ _ _  _ _ _ _  _ _ _ _", 16, &[]).unwrap();
            Pattern::from_steps(loud.clone(), &slices, steps)
                .unwrap()
                .with_gain(4.0)
                .unwrap()
                .with_granular(Some(Granular {
                    size_ms: fixed(320.0),
                    density: fixed(100.0),
                    jitter_ms: fixed(50.0),
                    spread: fixed(3.0),
                    ..Granular::new(t)
                }))
                .unwrap()
        })
        .collect();
    let (controller_out, _) = {
        let (mut controller, mut engine) = new_engine(RATE);
        for (t, p) in patterns.into_iter().enumerate() {
            controller.set_track_pattern(t, Some(p)).unwrap();
        }
        controller.start().unwrap();
        let out = offline::render(&mut engine, RATE as usize / 2, 2);
        let status = controller.status();
        assert!(status.master.peak > 1.0, "la suma tenía que pasar de 1");
        assert!(status.master.limited_frames > 0);
        (out, ())
    };
    let peak = controller_out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak <= 1.0, "pico {peak}");
}

#[test]
fn knobs_follow_ramps_and_lfos() {
    let sample = dc();
    // La densidad sube de 5 a 45 en el compás: nacen más al final.
    let params = Granular {
        size_ms: fixed(5.0),
        density: Knob::Moving(Curve::Ramp {
            from: 5.0,
            to: 45.0,
            steps: 16.0,
        }),
        ..Granular::new(1)
    };
    let (out, grains) = render(
        vec![granular(&sample, WHOLE_BAR, params)],
        2 * RATE as usize,
    );
    let half = RATE as usize;
    let (first, second) = (bumps(&out[..half]), bumps(&out[half..]));
    assert!(first < second, "{first} granos y luego {second}");
    // La densidad se interpola en escala logarítmica: de media
    // (45 − 5) / ln 9 ≈ 18,2 por segundo.
    assert!((34..=38).contains(&grains), "{grains}");
    // Un LFO en el pitch: el tono cambia (determinista).
    let params = Granular {
        size_ms: fixed(40.0),
        density: fixed(50.0),
        speed: fixed(0.0),
        position: fixed(0.25),
        pitch: Knob::Moving(Curve::Lfo {
            min: 0.0,
            max: 12.0,
            rate: LfoRate::Hz(0.5),
        }),
        ..Granular::new(1)
    };
    let (out, _) = render(
        vec![granular(&two_tones(), WHOLE_BAR, params)],
        RATE as usize,
    );
    let low = frequency(&out[RATE as usize / 20..RATE as usize / 8]);
    let high = frequency(&out[RATE as usize * 7 / 8..]);
    assert!(low < 360.0 && high > 500.0, "{low} → {high}");
}

#[test]
fn stop_and_choke_cut_the_cloud() {
    let params = Granular {
        size_ms: fixed(200.0),
        density: fixed(40.0),
        ..Granular::new(1)
    };
    let (mut controller, mut engine) = new_engine(RATE);
    controller
        .set_track_pattern(0, Some(granular(&dc(), WHOLE_BAR, params)))
        .unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, RATE as usize / 2, 2);
    controller.stop().unwrap();
    let tail = offline::render(&mut engine, RATE as usize / 10, 2);
    // Tras el fundido mínimo (1,5 ms), silencio.
    assert!(tail[2 * 100..].iter().all(|v| *v == 0.0));
    controller.collect_garbage();
}

#[test]
fn ranges_are_checked() {
    let sample = dc();
    let slices = Slices::grid(&sample, 1);
    let base = || Pattern::from_steps(sample.clone(), &slices, vec![None; 16]).unwrap();
    let cases = [
        (
            Granular {
                size_ms: fixed(2.0),
                ..Granular::new(0)
            },
            "size",
        ),
        (
            Granular {
                density: fixed(500.0),
                ..Granular::new(0)
            },
            "density",
        ),
        (
            Granular {
                position: fixed(1.5),
                ..Granular::new(0)
            },
            "pos",
        ),
        (
            Granular {
                pitch: Knob::Moving(Curve::Ramp {
                    from: 0.0,
                    to: 30.0,
                    steps: 16.0,
                }),
                ..Granular::new(0)
            },
            "pitch",
        ),
        (
            Granular {
                size_ms: fixed(500.0),
                density: fixed(100.0),
                ..Granular::new(0)
            },
            "32 granos",
        ),
    ];
    for (params, message) in cases {
        match base().with_granular(Some(params)) {
            Err(SendError::InvalidGranular(reason)) => {
                assert!(reason.contains(message), "{reason}")
            }
            other => panic!("{message}: {:?}", other.err()),
        }
    }
    // Instrumento y granos no van juntos.
    let both = base()
        .with_granular(Some(Granular::new(0)))
        .unwrap()
        .with_instrument(Some(engine::Instrument::new(4)));
    assert!(matches!(both, Err(SendError::InvalidGranular(_))));
}

/// Coste de una nube: `cargo test -p engine --release --test granular --
/// --ignored --nocapture`.
#[test]
#[ignore]
fn medida_de_coste() {
    let sample = two_tones();
    for (size, density, pitch) in [(80.0, 20.0, 0.0), (320.0, 100.0, 0.0), (320.0, 100.0, 12.0)] {
        let params = Granular {
            size_ms: fixed(size),
            density: fixed(density),
            jitter_ms: fixed(50.0),
            pitch: fixed(pitch),
            ..Granular::new(1)
        };
        let start = std::time::Instant::now();
        render(
            vec![granular(&sample, WHOLE_BAR, params)],
            2 * RATE as usize,
        );
        let share = start.elapsed().as_secs_f64() / 2.0;
        println!(
            "{size} ms × {density}/s (pitch {pitch}): {:.1} granos a la vez, {:.1} % de un núcleo",
            size * density / 1000.0,
            100.0 * share
        );
    }
}
