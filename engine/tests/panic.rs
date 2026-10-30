//! Botón de pánico: corta todas las voces que suenan sin tocar el
//! secuenciador; en pausa, no deja nacer ninguna hasta reanudar. Y el
//! límite de voces entre todas las pistas, que roba la más antigua.

use std::sync::Arc;

use engine::synth::{CUTOFF, ENV_AMOUNT, FilterMode, Patch, RESONANCE, Wave};
use engine::{Fx, Instrument, Pattern, Sample, SliceFx, Slices, new_engine, offline};

const RATE: u32 = 48_000;
/// A 120 BPM: 6000 frames por paso, 96 000 por compás.
const STEP: usize = 6000;
const BAR: usize = 16 * STEP;
/// 200 ms.
const MS200: usize = RATE as usize / 5;
/// Lo que dura el fundido mínimo (1,5 ms), con margen.
const FADE: usize = 100;

/// Un seno de 220 Hz de 2 s a media amplitud: suena todo el rato.
fn tone() -> Arc<Sample> {
    let mono: Vec<f32> = (0..2 * RATE as usize)
        .map(|i| 0.5 * (i as f32 * 220.0 * std::f32::consts::TAU / RATE as f32).sin())
        .collect();
    Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap())
}

/// El seno entero como un corte, en `steps`.
fn pattern(sample: &Arc<Sample>, steps: &[usize]) -> Pattern {
    let slices = Slices::grid(sample, 1);
    let mut all = [None; 16];
    for &s in steps {
        all[s] = Some(0);
    }
    Pattern::new(sample.clone(), &slices, all).unwrap()
}

/// Un seno sin filtro que oiga.
fn sine() -> Patch {
    let mut p = Patch::new()
        .with(CUTOFF, 20.0)
        .with(RESONANCE, 0.0)
        .with(ENV_AMOUNT, 0.0);
    p.wave = Wave::Sine;
    p.filter = FilterMode::Highpass;
    p.root = 69.0;
    p
}

fn silent(audio: &[f32]) -> bool {
    audio.iter().all(|s| s.abs() < 1e-6)
}

/// Todo lo que puede sonar a la vez: la voz del secuenciador (pista 0),
/// un pad (1), una nota de instrumento (2), una de sintetizador (3), una
/// nube de granos (4) y la preescucha.
fn everything(c: &mut engine::Controller, sample: &Arc<Sample>) {
    let slices = Slices::grid(sample, 1);
    c.set_bpm(120.0).unwrap();
    c.set_track_pattern(0, Some(pattern(sample, &[0]))).unwrap();
    c.start().unwrap();
    c.play_slice_on(1, sample, &slices, 0).unwrap();
    c.note_on(
        2,
        sample,
        0..sample.frames(),
        Instrument::new(4),
        0.0,
        1.0,
        60,
    )
    .unwrap();
    c.synth_note_on(3, sine(), 0.0, 1.0, 60).unwrap();
    c.preview_slice(sample, &slices, 0).unwrap();
}

#[test]
fn panic_silences_every_voice_at_once_and_lets_the_next_in() {
    let sample = tone();
    let (mut c, mut e) = new_engine(RATE);
    everything(&mut c, &sample);
    let before = offline::render(&mut e, 2000, 2);
    assert!(!silent(&before));
    assert!(e.sounding_voices() >= 5, "{}", e.sounding_voices());

    c.panic(false).unwrap();
    let after = offline::render(&mut e, MS200, 2);
    assert!(silent(&after[2 * FADE..]), "tras el fundido mínimo, nada");
    assert_eq!(e.sounding_voices(), 0, "ninguna voz en 200 ms");
    assert!(!c.status().hushed);
    // El secuenciador sigue: donde estaba más 200 ms.
    let steps = c.position().unwrap();
    let expected = (2000 + MS200) as f64 / STEP as f64;
    assert!((steps - expected).abs() < 1e-6, "{steps} vs {expected}");

    // Lo siguiente entra: un pad ya, y el golpe del compás siguiente.
    c.play_slice_on(1, &sample, &Slices::grid(&sample, 1), 0)
        .unwrap();
    assert!(!silent(&offline::render(&mut e, 1000, 2)));
    c.panic(false).unwrap();
    let rest = BAR - 2000 - MS200 - 1000;
    assert!(silent(&offline::render(&mut e, rest, 2)[2 * FADE..]));
    assert!(
        !silent(&offline::render(&mut e, 1000, 2)),
        "el compás 2 suena"
    );
}

#[test]
fn panic_cuts_the_tails_of_the_slice_effects() {
    // Un golpe con los efectos de su corte: al acabar la voz, su cola
    // seguiría sonando; el pánico la corta también.
    let sample = tone();
    let (mut c, mut e) = new_engine(RATE);
    let p = pattern(&sample, &[0])
        .with_slice_fx(&[Some(SliceFx::new(&[Fx::lowpass(800.0)], &[]).unwrap())])
        .unwrap();
    c.set_bpm(120.0).unwrap();
    c.set_track_pattern(0, Some(p)).unwrap();
    c.start().unwrap();
    assert!(!silent(&offline::render(&mut e, 2000, 2)));
    c.panic(false).unwrap();
    // La cola de los efectos de corte duraría 100 ms; cortada, el filtro
    // solo acaba de sonar en el trozo del fundido (256 frames).
    let after = offline::render(&mut e, 2 * 256, 2);
    assert_eq!(e.sounding_voices(), 0, "sin cola");
    assert!(silent(&after[2 * 256..]));
    assert!(silent(&offline::render(&mut e, MS200, 2)));
}

#[test]
fn a_paused_panic_lets_nothing_new_in_until_resumed() {
    let sample = tone();
    // Un golpe en cada paso, para que el secuenciador quiera sonar.
    let every: Vec<usize> = (0..16).collect();
    let (mut c, mut e) = new_engine(RATE);
    everything(&mut c, &sample);
    c.set_track_pattern(0, Some(pattern(&sample, &every)))
        .unwrap();
    let _ = offline::render(&mut e, BAR + 1000, 2);

    c.panic(true).unwrap();
    let paused = offline::render(&mut e, MS200, 2);
    assert!(silent(&paused[2 * FADE..]));
    assert_eq!(e.sounding_voices(), 0);
    assert!(c.status().hushed);
    assert!(e.is_hushed());
    // Nada de lo que se toca entra: ni pads, ni notas, ni granos, ni la
    // preescucha, ni los pasos del secuenciador (que sigue contando).
    let slices = Slices::grid(&sample, 1);
    c.play_slice_on(1, &sample, &slices, 0).unwrap();
    c.note_on(
        2,
        &sample,
        0..sample.frames(),
        Instrument::new(4),
        0.0,
        1.0,
        61,
    )
    .unwrap();
    c.synth_note_on(3, sine(), 0.0, 1.0, 61).unwrap();
    c.grain_on(4, &sample, 0..sample.frames(), 0.0, 1.0, 1)
        .unwrap();
    c.preview_slice(&sample, &slices, 0).unwrap();
    let held = offline::render(&mut e, BAR, 2);
    assert!(silent(&held), "en pausa no suena nada");
    assert_eq!(e.sounding_voices(), 0);
    let steps = c.position().unwrap();
    let expected = (2 * BAR + 1000 + MS200) as f64 / STEP as f64;
    assert!((steps - expected).abs() < 1e-6, "{steps} vs {expected}");

    // Un pánico sin pausa no la quita; reanudar, sí: el paso siguiente
    // suena.
    c.panic(false).unwrap();
    assert!(silent(&offline::render(&mut e, 2 * STEP, 2)));
    assert!(c.status().hushed);
    c.resume().unwrap();
    let back = offline::render(&mut e, 2 * STEP, 2);
    assert!(!silent(&back), "tras reanudar, el secuenciador suena");
    assert!(!c.status().hushed);
}

#[test]
fn the_voice_limit_steals_the_oldest_voice_of_any_track() {
    let sample = tone();
    let (mut c, mut e) = new_engine(RATE);
    c.set_voice_limit(2).unwrap();
    let range = 0..sample.frames();
    // Dos notas en la pista 0 y una en la 1: la primera (la más antigua)
    // se roba aunque su pista tenga sitio.
    c.note_on(0, &sample, range.clone(), Instrument::new(4), 0.0, 0.5, 60)
        .unwrap();
    let _ = offline::render(&mut e, 256, 2);
    c.note_on(0, &sample, range.clone(), Instrument::new(4), 7.0, 0.5, 67)
        .unwrap();
    let _ = offline::render(&mut e, 256, 2);
    c.synth_note_on(1, sine(), 0.0, 0.5, 60).unwrap();
    let _ = offline::render(&mut e, 256, 2);
    let _ = c.status();
    let _ = offline::render(&mut e, 256, 2);
    let status = c.status();
    assert_eq!(status.tracks[0].voices, 1);
    assert_eq!(status.tracks[1].voices, 1);
    // La que queda en la pista 0 es la 67: soltarla la calla entera.
    c.note_off(0, 67).unwrap();
    let _ = offline::render(&mut e, MS200, 2);
    let _ = c.status();
    let _ = offline::render(&mut e, 256, 2);
    assert_eq!(c.status().tracks[0].voices, 0, "la 60 ya no sonaba");

    // Bajar el límite roba ya lo que sobra.
    c.note_on(0, &sample, range.clone(), Instrument::new(4), 0.0, 0.5, 60)
        .unwrap();
    let _ = offline::render(&mut e, 256, 2);
    c.set_voice_limit(1).unwrap();
    let _ = offline::render(&mut e, 256, 2);
    let _ = c.status();
    let _ = offline::render(&mut e, 256, 2);
    let status = c.status();
    assert_eq!(status.tracks[0].voices + status.tracks[1].voices, 1);
    assert_eq!(
        status.tracks[0].voices, 1,
        "se fue el sintetizador, más antiguo"
    );

    assert!(c.set_voice_limit(0).is_err());
    assert!(c.set_voice_limit(engine::MAX_VOICE_LIMIT + 1).is_err());
    assert_eq!(engine::DEFAULT_VOICE_LIMIT, 64);
}
