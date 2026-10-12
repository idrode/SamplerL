//! Preescucha: una voz aparte que no corta ni pasa por ninguna pista.

use std::sync::Arc;

use engine::{Fx, Pattern, Sample, Slices, new_engine, offline};

const RATE: u32 = 48_000;
const BAR: usize = 96_000;

fn sample() -> (Arc<Sample>, Slices) {
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    (sample, slices)
}

fn pattern() -> Pattern {
    let (sample, slices) = sample();
    let steps = [0, 4, 8, 12].map(Some);
    let mut all = [None; 16];
    for s in steps.into_iter().flatten() {
        all[s] = Some(0);
    }
    Pattern::new(sample, &slices, all).unwrap()
}

#[test]
fn preview_sounds_like_the_slice_even_when_stopped() {
    let (sample, slices) = sample();
    let (mut c, mut e) = new_engine(RATE);
    c.preview_slice(&sample, &slices, 1).unwrap();
    let preview = offline::render(&mut e, 4000, 2);
    // Igual que tocar el corte en una pista sin volumen ni efectos.
    let (mut c2, mut e2) = new_engine(RATE);
    c2.play_slice_on(3, &sample, &slices, 1).unwrap();
    assert_eq!(preview, offline::render(&mut e2, 4000, 2));
    assert!(preview.iter().any(|s| (s.abs() - 0.25).abs() < 1e-6));
    assert!(c.preview_slice(&sample, &slices, 5).is_err());
}

#[test]
fn preview_does_not_touch_the_tracks() {
    let (sample, slices) = sample();
    // La pista suena con bitcrush y en mute: la preescucha no pasa por ahí.
    let track = || pattern().with_fx(&[Fx::bitcrush(2, 4)]).unwrap();
    let (mut c, mut e) = new_engine(RATE);
    c.set_bpm(120.0).unwrap();
    c.set_track_pattern(0, Some(track())).unwrap();
    c.start().unwrap();
    // A los 100 frames, con el golpe de la pista sonando, se preescucha.
    let head = offline::render(&mut e, 100, 2);
    c.preview_slice(&sample, &slices, 1).unwrap();
    let mut both = head;
    both.extend(offline::render(&mut e, BAR - 100, 2));

    let alone = {
        let (mut c, mut e) = new_engine(RATE);
        c.set_bpm(120.0).unwrap();
        c.set_track_pattern(0, Some(track())).unwrap();
        c.start().unwrap();
        offline::render(&mut e, BAR, 2)
    };
    let preview = {
        let (mut c, mut e) = new_engine(RATE);
        offline::render(&mut e, 100, 2);
        c.preview_slice(&sample, &slices, 1).unwrap();
        let mut out = vec![0.0; 200];
        out.extend(offline::render(&mut e, BAR - 100, 2));
        out
    };
    // La suma exacta: la pista sigue igual (no se corta) y la preescucha
    // suena limpia encima.
    let sum: Vec<f32> = alone.iter().zip(&preview).map(|(a, b)| a + b).collect();
    assert_eq!(both, sum);

    // Con la pista en mute, solo queda la preescucha.
    let (mut c, mut e) = new_engine(RATE);
    c.set_bpm(120.0).unwrap();
    c.set_track_pattern(0, Some(track())).unwrap();
    c.set_mute(0, true).unwrap();
    c.start().unwrap();
    offline::render(&mut e, 100, 2);
    c.preview_slice(&sample, &slices, 1).unwrap();
    assert_eq!(offline::render(&mut e, BAR - 100, 2), preview[200..]);
}

#[test]
fn a_new_preview_crossfades_and_stop_fades_out() {
    let (sample, slices) = sample();
    let (mut c, mut e) = new_engine(RATE);
    c.preview_slice(&sample, &slices, 0).unwrap();
    offline::render(&mut e, 400, 2);
    c.preview_slice(&sample, &slices, 1).unwrap();
    let swap = offline::render(&mut e, 400, 2);
    // Al principio aún se oye el corte anterior (0,5) bajando.
    assert!(swap[0].abs() > 0.3, "{}", swap[0]);
    assert!((swap[2 * 300].abs() - 0.25).abs() < 1e-6);
    c.stop_preview().unwrap();
    let tail = offline::render(&mut e, 400, 2);
    assert!(tail[0].abs() > 0.1);
    assert!(tail[2 * 300..].iter().all(|s| *s == 0.0));
    offline::render(&mut e, 400, 2);
    assert!(!c.is_playing());
    c.collect_garbage();
}
