//! Nube de granos en vivo (`Controller::grain_on` / `grain_off`): una tecla
//! la abre y la mantiene hasta soltarla, con el secuenciador parado o en
//! marcha; los parámetros son los de la pista y entran al compás; el azar
//! sale de la semilla y no de dónde esté la canción.

use std::f64::consts::TAU;
use std::sync::Arc;

use engine::notation::parse_pattern_labeled;
use engine::{
    Batch, Controller, Engine, Granular, Knob, Pattern, Sample, SendError, Slices, new_engine,
    offline,
};

const RATE: u32 = 48_000;
/// Un paso a 120 BPM.
const STEP: usize = 6000;
/// Un compás a 120 BPM.
const BAR: usize = 16 * STEP;

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

fn fixed(v: f32) -> Knob {
    Knob::Fixed(v)
}

/// Pista con granos y sin un solo golpe: solo suena lo que se toca en vivo.
fn silent(sample: &Arc<Sample>, params: Granular) -> Pattern {
    let slices = Slices::grid(sample, 1);
    let steps = parse_pattern_labeled(". . . .  . . . .  . . . .  . . . .", 16, &[]).unwrap();
    Pattern::from_steps(sample.clone(), &slices, steps)
        .unwrap()
        .with_granular(Some(params))
        .unwrap()
}

/// Granos de 5 ms, sin azar, sobre continua: cada uno es una campana.
fn bumps(x: &[f32]) -> usize {
    (1..x.len())
        .filter(|&i| x[i - 1] <= 0.25 && x[i] > 0.25)
        .count()
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

fn frequency(x: &[f32]) -> f64 {
    let crossings: Vec<f64> = (1..x.len())
        .filter(|&i| x[i - 1] < 0.0 && x[i] >= 0.0)
        .map(|i| (i - 1) as f64 + (-x[i - 1] / (x[i] - x[i - 1])) as f64)
        .collect();
    RATE as f64 * (crossings.len() - 1) as f64 / (crossings[crossings.len() - 1] - crossings[0])
}

/// Un motor con esas pistas, ya con su patrón dentro; `stopped`: con el
/// secuenciador parado (los patrones se quedan puestos).
fn rig(patterns: Vec<Pattern>, stopped: bool) -> (Controller, Engine) {
    let (mut controller, mut engine) = new_engine(RATE);
    for (t, p) in patterns.into_iter().enumerate() {
        controller.set_track_pattern(t, Some(p)).unwrap();
    }
    controller.start().unwrap();
    offline::render(&mut engine, 100, 2);
    if stopped {
        controller.stop().unwrap();
        offline::render(&mut engine, 100, 2);
    }
    (controller, engine)
}

fn sparse(density: f32) -> Granular {
    Granular {
        size_ms: fixed(5.0),
        density: fixed(density),
        ..Granular::new(1)
    }
}

#[test]
fn a_held_key_emits_until_it_is_released() {
    let sample = dc();
    for stopped in [true, false] {
        let (mut controller, mut engine) = rig(vec![silent(&sample, sparse(12.0))], stopped);
        controller
            .grain_on(0, &sample, 0..sample.frames(), 0.0, 1.0, 130)
            .unwrap();
        // Un segundo mantenida: 12 granos por segundo, a pasos de 1/12 s.
        let held = left(&offline::render(&mut engine, RATE as usize, 2));
        assert_eq!(bumps(&held), 12, "parado: {stopped}");
        assert_eq!(controller.status().tracks[0].grains, 12);
        // Soltada: no nacen más (y lo que suena acaba su ventana de 5 ms).
        controller.grain_off(0, 130).unwrap();
        let after = left(&offline::render(&mut engine, RATE as usize, 2));
        assert_eq!(bumps(&after[300..]), 0, "parado: {stopped}");
        assert!(after[300..].iter().all(|v| *v == 0.0));
        controller.collect_garbage();
    }
}

#[test]
fn the_cloud_keeps_the_stopped_engine_playing_for_the_ui() {
    let sample = dc();
    let (mut controller, mut engine) = rig(vec![silent(&sample, sparse(12.0))], true);
    assert!(!controller.is_playing());
    controller
        .grain_on(0, &sample, 0..sample.frames(), 0.0, 1.0, 130)
        .unwrap();
    offline::render(&mut engine, 4800, 2);
    assert!(controller.is_playing(), "con la nube sonando, suena");
    controller.grain_off(0, 130).unwrap();
    offline::render(&mut engine, 4800, 2);
    assert!(!controller.is_playing());
}

#[test]
fn another_key_replaces_the_cloud_and_releasing_the_old_one_does_nothing() {
    let sample = two_tones();
    let params = Granular {
        size_ms: fixed(40.0),
        density: fixed(50.0),
        speed: fixed(0.0),
        ..Granular::new(1)
    };
    let (mut controller, mut engine) = rig(vec![silent(&sample, params)], true);
    let half = sample.frames() / 2;
    controller
        .grain_on(0, &sample, 0..half, 0.0, 1.0, 130)
        .unwrap();
    offline::render(&mut engine, 4800, 2);
    // Otra tecla: ahora lee la segunda mitad.
    controller
        .grain_on(0, &sample, half..sample.frames(), 0.0, 1.0, 131)
        .unwrap();
    offline::render(&mut engine, 4800, 2);
    // Soltar la primera no corta la segunda.
    controller.grain_off(0, 130).unwrap();
    let out = left(&offline::render(&mut engine, RATE as usize / 2, 2));
    let hz = frequency(&out[RATE as usize / 10..]);
    assert!((hz / 900.0 - 1.0).abs() < 0.01, "{hz}");
    controller.grain_off(0, 131).unwrap();
    let tail = left(&offline::render(&mut engine, RATE as usize / 2, 2));
    assert!(tail[RATE as usize / 10..].iter().all(|v| *v == 0.0));
}

#[test]
fn the_key_chooses_the_slice_and_the_pitch() {
    let sample = two_tones();
    let params = Granular {
        size_ms: fixed(40.0),
        density: fixed(50.0),
        speed: fixed(0.0),
        ..Granular::new(1)
    };
    let half = sample.frames() / 2;
    for (range, semitones, expected) in [
        (0..half, 0.0, 300.0),
        (half..sample.frames(), 0.0, 900.0),
        (half..sample.frames(), 12.0, 1800.0),
        (0..half, -12.0, 150.0),
        (0..half, 7.0, 300.0 * 2f64.powf(7.0 / 12.0)),
    ] {
        let (mut controller, mut engine) = rig(vec![silent(&sample, params)], true);
        controller
            .grain_on(0, &sample, range.clone(), semitones, 1.0, 130)
            .unwrap();
        let out = left(&offline::render(&mut engine, RATE as usize / 2, 2));
        let hz = frequency(&out[RATE as usize / 10..]);
        assert!(
            (hz / expected - 1.0).abs() < 0.01,
            "{range:?} {semitones:+}: {hz:.1} Hz en vez de {expected:.1}"
        );
    }
}

#[test]
fn the_pitch_adds_to_the_one_of_the_cloud() {
    let sample = two_tones();
    let params = Granular {
        size_ms: fixed(40.0),
        density: fixed(50.0),
        speed: fixed(0.0),
        pitch: fixed(-12.0),
        ..Granular::new(1)
    };
    let (mut controller, mut engine) = rig(vec![silent(&sample, params)], true);
    controller
        .grain_on(0, &sample, 0..sample.frames() / 2, 12.0, 1.0, 130)
        .unwrap();
    let out = left(&offline::render(&mut engine, RATE as usize / 2, 2));
    let hz = frequency(&out[RATE as usize / 10..]);
    assert!((hz / 300.0 - 1.0).abs() < 0.01, "{hz}");
}

/// Lo que suena `frames` frames tras apretar la tecla, con jitter y spread
/// (azar en cada grano).
fn random_cloud(seed: u64, stopped: bool, warmup: usize) -> Vec<f32> {
    let sample = two_tones();
    let params = Granular {
        size_ms: fixed(30.0),
        density: fixed(60.0),
        jitter_ms: fixed(200.0),
        spread: fixed(5.0),
        ..Granular::new(seed)
    };
    let (mut controller, mut engine) = rig(vec![silent(&sample, params)], stopped);
    offline::render(&mut engine, warmup, 2);
    controller
        .grain_on(0, &sample, 0..sample.frames(), 0.0, 1.0, 130)
        .unwrap();
    offline::render(&mut engine, RATE as usize, 2)
}

#[test]
fn the_randomness_comes_from_the_seed_not_from_the_song_position() {
    let base = random_cloud(7, true, 0);
    assert!(base.iter().any(|v| *v != 0.0));
    // Misma semilla: igual, esté la canción parada o en marcha y por donde
    // vaya (a mitad de un paso, tras varios compases).
    assert_eq!(random_cloud(7, true, 0), base);
    assert_eq!(random_cloud(7, false, 0), base);
    assert_eq!(random_cloud(7, false, 7777), base);
    assert_eq!(random_cloud(7, false, 3 * BAR + 1234), base);
    // Otra semilla: otra nube.
    assert_ne!(random_cloud(8, true, 0), base);
}

#[test]
fn two_presses_of_the_same_key_sound_different_but_repeatably() {
    let sample = two_tones();
    let params = Granular {
        size_ms: fixed(30.0),
        density: fixed(60.0),
        jitter_ms: fixed(200.0),
        ..Granular::new(3)
    };
    let play = || {
        let (mut controller, mut engine) = rig(vec![silent(&sample, params)], true);
        let mut presses = Vec::new();
        for _ in 0..2 {
            controller
                .grain_on(0, &sample, 0..sample.frames(), 0.0, 1.0, 130)
                .unwrap();
            presses.push(offline::render(&mut engine, RATE as usize / 2, 2));
            controller.grain_off(0, 130).unwrap();
            offline::render(&mut engine, RATE as usize / 4, 2);
            controller.collect_garbage();
        }
        presses
    };
    let a = play();
    assert_ne!(a[0], a[1], "cada pulsación tiene su azar");
    assert_eq!(a, play(), "pero tocar lo mismo suena igual");
}

#[test]
fn releasing_lets_the_grains_finish_without_a_click() {
    let sample = dc();
    let params = Granular {
        size_ms: fixed(200.0),
        density: fixed(20.0),
        ..Granular::new(1)
    };
    let (mut controller, mut engine) = rig(vec![silent(&sample, params)], true);
    controller
        .grain_on(0, &sample, 0..sample.frames(), 0.0, 1.0, 130)
        .unwrap();
    let mut out = left(&offline::render(&mut engine, RATE as usize / 2, 2));
    let at = out.len();
    controller.grain_off(0, 130).unwrap();
    out.extend(left(&offline::render(&mut engine, RATE as usize / 2, 2)));
    // Nada salta en el instante de soltar ni después (ventanas de Hann).
    let jump = out[at - 200..]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max);
    assert!(jump < 0.01, "{jump}");
    // Y acaba tras lo que dura un grano.
    assert!(
        out[at + RATE as usize / 5 + 300..]
            .iter()
            .all(|v| *v == 0.0)
    );
    assert!(out[at + 1000] != 0.0, "los granos que sonaban siguen");
}

#[test]
fn script_changes_enter_at_the_bar_while_the_key_is_held() {
    let sample = dc();
    let (mut controller, mut engine) = rig(vec![silent(&sample, sparse(10.0))], false);
    controller
        .grain_on(0, &sample, 0..sample.frames(), 0.0, 1.0, 130)
        .unwrap();
    // Otro patrón (40 granos por segundo): entra en el compás siguiente.
    let mut batch = Batch::new();
    batch.set_pattern(0, Some(silent(&sample, sparse(40.0))));
    controller.apply(batch).unwrap();
    let out = left(&offline::render(&mut engine, 2 * BAR - 100 - 100, 2));
    // El primer compás (menos lo ya renderizado al montarlo) sigue a 10/s;
    // el segundo, a 40/s.
    let first = bumps(&out[..BAR - 200]);
    let second = bumps(&out[BAR - 200..]);
    assert!((19..=20).contains(&first), "{first}");
    assert!((79..=80).contains(&second), "{second}");
}

#[test]
fn before_the_first_start_the_waiting_pattern_gives_the_parameters() {
    let sample = dc();
    let (mut controller, mut engine) = new_engine(RATE);
    controller
        .set_track_pattern(0, Some(silent(&sample, sparse(12.0))))
        .unwrap();
    // Sin arrancar: el patrón espera, y la tecla ya hace su nube.
    controller
        .grain_on(0, &sample, 0..sample.frames(), 0.0, 1.0, 130)
        .unwrap();
    let before = left(&offline::render(&mut engine, RATE as usize, 2));
    assert_eq!(bumps(&before), 12);
    // Al arrancar entra el patrón (con los mismos parámetros) y la tecla
    // mantenida sigue.
    controller.start().unwrap();
    let after = left(&offline::render(&mut engine, RATE as usize, 2));
    assert_eq!(bumps(&after), 12);
    controller.grain_off(0, 130).unwrap();
    let tail = left(&offline::render(&mut engine, 4800, 2));
    assert!(tail[300..].iter().all(|v| *v == 0.0));
    // Y si el patrón que espera cambia de nube al entrar, manda el nuevo.
    let (mut controller, mut engine) = new_engine(RATE);
    controller
        .set_track_pattern(0, Some(silent(&sample, sparse(12.0))))
        .unwrap();
    controller
        .grain_on(0, &sample, 0..sample.frames(), 0.0, 1.0, 130)
        .unwrap();
    offline::render(&mut engine, 4000, 2);
    controller
        .set_track_pattern(0, Some(silent(&sample, sparse(48.0))))
        .unwrap();
    controller.start().unwrap();
    let out = left(&offline::render(&mut engine, 2 * RATE as usize, 2));
    assert!(bumps(&out) > 80, "{}", bumps(&out));
}

#[test]
fn a_track_without_grains_plays_the_slice_as_it_is() {
    let sample = dc();
    let slices = Slices::grid(&sample, 1);
    let plain = Pattern::from_steps(sample.clone(), &slices, vec![None; 16]).unwrap();
    let (mut controller, mut engine) = rig(vec![plain], true);
    controller
        .grain_on(0, &sample, 0..4800, 0.0, 1.0, 130)
        .unwrap();
    let out = left(&offline::render(&mut engine, 9600, 2));
    assert!(out[100..4700].iter().all(|v| (*v - 0.5).abs() < 1e-6));
    assert!(out[5000..].iter().all(|v| *v == 0.0));
    // Y soltar la tecla no hace nada.
    controller.grain_off(0, 130).unwrap();
    offline::render(&mut engine, 256, 2);
}

#[test]
fn stop_and_the_steps_of_the_track_cut_a_live_cloud() {
    let sample = dc();
    let params = Granular {
        size_ms: fixed(200.0),
        density: fixed(40.0),
        ..Granular::new(1)
    };
    let (mut controller, mut engine) = rig(vec![silent(&sample, params)], true);
    controller
        .grain_on(0, &sample, 0..sample.frames(), 0.0, 1.0, 130)
        .unwrap();
    offline::render(&mut engine, RATE as usize / 4, 2);
    controller.stop().unwrap();
    let tail = offline::render(&mut engine, RATE as usize / 10, 2);
    assert!(tail[2 * 100..].iter().all(|v| *v == 0.0));
    controller.collect_garbage();
}

#[test]
fn eight_dense_live_clouds_never_pass_one() {
    let sample = dc();
    let params = Granular {
        size_ms: fixed(60.0),
        density: fixed(100.0),
        jitter_ms: fixed(30.0),
        spread: fixed(2.0),
        ..Granular::new(1)
    };
    let patterns = (0..8).map(|t| {
        silent(&sample, Granular { seed: t, ..params })
            .with_gain(4.0)
            .unwrap()
    });
    let (mut controller, mut engine) = rig(patterns.collect(), true);
    for t in 0..8 {
        controller
            .grain_on(t, &sample, 0..sample.frames(), 0.0, 4.0, 130 + t as u8)
            .unwrap();
    }
    let out = offline::render(&mut engine, RATE as usize, 2);
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak <= 1.0, "pico {peak}");
    assert!(peak > 0.9, "tenía que actuar el limitador: {peak}");
}

#[test]
fn what_is_wrong_is_refused() {
    let sample = dc();
    let (mut controller, _engine) = new_engine(RATE);
    let range = 0..sample.frames();
    assert!(matches!(
        controller.grain_on(8, &sample, range.clone(), 0.0, 1.0, 130),
        Err(SendError::NoSuchTrack(8))
    ));
    assert!(matches!(
        controller.grain_off(9, 130),
        Err(SendError::NoSuchTrack(9))
    ));
    for (semitones, gain) in [(0.0, 4.5), (0.0, -0.1), (37.0, 1.0), (-37.0, 1.0)] {
        assert!(
            matches!(
                controller.grain_on(0, &sample, range.clone(), semitones, gain, 130),
                Err(SendError::InvalidStep { .. })
            ),
            "{semitones} {gain}"
        );
    }
}
