//! Mute y solo por pista: en vivo (inmediatos, con fundido corto) y desde
//! el patrón (al compás). Las voces siguen su curso aunque no se oigan.

use std::sync::Arc;

use engine::{Controller, Engine, Pattern, Sample, Slices, new_engine, offline};

const RATE: u32 = 48_000;
/// 120 BPM: 6000 frames por paso, 96000 por compás.
const BAR: usize = 96_000;

fn sample() -> (Arc<Sample>, Slices) {
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    (sample, slices)
}

fn pattern(steps: &str) -> Pattern {
    let (sample, slices) = sample();
    let steps: Vec<Option<usize>> = steps.split_whitespace().map(|s| s.parse().ok()).collect();
    let steps: [Option<usize>; engine::STEPS] = steps.try_into().unwrap();
    Pattern::new(sample, &slices, steps).unwrap()
}

const A: &str = "0 . . .  0 . . .  0 . . .  0 . . .";
const B: &str = ". . 1 .  . . 1 .  . . 1 .  . . 1 .";

/// Dos pistas (A en la 0, B en la 1), en marcha.
fn two_tracks(a: Pattern, b: Pattern) -> (Controller, Engine) {
    let (mut controller, engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_track_pattern(0, Some(a)).unwrap();
    controller.set_track_pattern(1, Some(b)).unwrap();
    controller.start().unwrap();
    (controller, engine)
}

fn only(track: usize, steps: &str, bars: usize) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller
        .set_track_pattern(track, Some(pattern(steps)))
        .unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, bars * BAR, 2)
}

#[test]
fn live_mute_and_solo_silence_exactly_from_the_start() {
    let (mut c, mut e) = two_tracks(pattern(A), pattern(B));
    c.set_mute(1, true).unwrap();
    assert_eq!(offline::render(&mut e, BAR, 2), only(0, A, 1), "mute de B");

    let (mut c, mut e) = two_tracks(pattern(A), pattern(B));
    c.set_solo(1, true).unwrap();
    assert_eq!(offline::render(&mut e, BAR, 2), only(1, B, 1), "solo de B");

    // Un mute gana a un solo; dos solos suenan los dos.
    let (mut c, mut e) = two_tracks(pattern(A), pattern(B));
    c.set_solo(0, true).unwrap();
    c.set_solo(1, true).unwrap();
    c.set_mute(0, true).unwrap();
    assert_eq!(offline::render(&mut e, BAR, 2), only(1, B, 1));
}

#[test]
fn muting_while_sounding_fades_out_and_unmuting_resumes() {
    let (mut c, mut e) = two_tracks(pattern(A), pattern(". . . .  . . . .  . . . .  . . . ."));
    // El golpe del paso 0 dura 800 frames: se silencia a mitad.
    let head = offline::render(&mut e, 300, 2);
    assert!(head.iter().any(|s| s.abs() > 0.4));
    c.set_mute(0, true).unwrap();
    let tail = offline::render(&mut e, 600, 2);
    // Fundido de ~5 ms (240 frames): a los 500 frames ya no se oye.
    assert!(tail[..20].iter().any(|s| s.abs() > 0.3), "sin corte seco");
    assert!(tail[2 * 500..].iter().all(|s| s.abs() < 1e-3), "silenciada");
    // Las voces siguen: al quitar el mute en el paso 4, suena su golpe.
    offline::render(&mut e, 4 * 6000 - 900, 2);
    c.set_mute(0, false).unwrap();
    let back = offline::render(&mut e, 600, 2);
    assert!(back.iter().any(|s| s.abs() > 0.4));
}

#[test]
fn pattern_mute_and_solo_enter_at_the_bar() {
    let (mut c, mut e) = two_tracks(pattern(A), pattern(B));
    offline::render(&mut e, BAR / 2, 2);
    c.set_track_pattern(1, Some(pattern(B).with_mute(true)))
        .unwrap();
    let rest = offline::render(&mut e, BAR / 2 + BAR, 2);
    let both = {
        let (_c, mut e) = two_tracks(pattern(A), pattern(B));
        offline::render(&mut e, 2 * BAR, 2)
    };
    // (Índices en muestras estéreo: 2 por frame.) Medio compás más con las
    // dos; desde el compás siguiente, solo A.
    assert_eq!(rest[..BAR], both[BAR..2 * BAR]);
    assert_eq!(rest[BAR..], only(0, A, 2)[2 * BAR..]);

    let (mut c, mut e) = two_tracks(pattern(A).with_solo(true), pattern(B));
    assert_eq!(offline::render(&mut e, BAR, 2), only(0, A, 1));
    // El solo en vivo de otra pista se suma: suenan las dos.
    c.set_solo(1, true).unwrap();
    let next = offline::render(&mut e, BAR, 2);
    let both = {
        let (_c, mut e) = two_tracks(pattern(A), pattern(B));
        offline::render(&mut e, 2 * BAR, 2)
    };
    assert_eq!(next, both[2 * BAR..]);
}
