//! Choques de frecuencia (`engine::masking`) con senos sintéticos: dos
//! iguales chocan en su banda, dos lejanos no, y la puntuación baja con la
//! diferencia de nivel.

use engine::masking::{BAND_NAMES, BANDS, BandSpectrum, MaskingOptions, band_center, clashes};

const RATE: u32 = 48_000;

/// Estéreo entrelazado: un seno de `freq` con amplitud `amp` durante
/// `seconds`.
fn sine(freq: f32, amp: f32, seconds: f32) -> Vec<f32> {
    let frames = (seconds * RATE as f32) as usize;
    (0..frames)
        .flat_map(|i| {
            let x = amp * (std::f32::consts::TAU * freq * i as f32 / RATE as f32).sin();
            [x, x]
        })
        .collect()
}

fn band_of(freq: f32) -> usize {
    (0..BANDS)
        .min_by(|&a, &b| {
            let d = |k: usize| (band_center(k) / freq).ln().abs();
            d(a).total_cmp(&d(b))
        })
        .unwrap()
}

fn scores(a: &[f32], b: &[f32]) -> [Option<f32>; BANDS] {
    let options = MaskingOptions::default();
    let a = BandSpectrum::analyze(a, RATE, &options);
    let b = BandSpectrum::analyze(b, RATE, &options);
    clashes(&a, &b, &options).map(|c| c.map(|c| c.score))
}

#[test]
fn the_bands_are_third_octaves_from_25_hz_to_20_khz() {
    assert_eq!(BAND_NAMES[band_of(300.0)], "315");
    assert_eq!(BAND_NAMES[band_of(1000.0)], "1k");
    assert!((band_center(16) - 1000.0).abs() < 1e-3);
    let ratio = band_center(11) / band_center(10);
    assert!((ratio - 2f32.powf(1.0 / 3.0)).abs() < 0.01);
    // Ventanas de 25 ms: un segundo son 40.
    let options = MaskingOptions::default();
    let s = BandSpectrum::analyze(&sine(300.0, 0.5, 1.0), RATE, &options);
    assert_eq!(s.windows(), 40);
    // Toda la energía de un seno, en su banda (y sus vecinas por la ventana).
    let band = band_of(300.0);
    let total: f32 = (0..BANDS).map(|b| s.band_energy(b)).sum();
    let near: f32 = (band - 1..=band + 1).map(|b| s.band_energy(b)).sum();
    assert!(near / total > 0.999, "{}", near / total);
}

#[test]
fn two_sines_at_300_hz_clash_in_that_band() {
    let s = scores(&sine(300.0, 0.5, 2.0), &sine(300.0, 0.4, 2.0));
    let band = band_of(300.0);
    let at = s[band].unwrap();
    // A 2 dB uno de otro: 1 − 2/9.
    assert!(at > 0.75, "{at}");
    // Es la banda que más choca (la vecina, por la ventana, casi igual: los
    // dos senos se derraman lo mismo en ella).
    let max = s.iter().flatten().fold(0.0f32, |a, &b| a.max(b));
    assert!(max - at < 1e-4, "{max} {at}");
    // Lejos de 300 Hz, nada (o banda vacía).
    for (k, score) in s.iter().enumerate() {
        if (k as i32 - band as i32).abs() > 2 {
            assert!(
                score.is_none_or(|x| x < 0.05),
                "{}: {score:?}",
                BAND_NAMES[k]
            );
        }
    }
}

#[test]
fn a_sine_at_100_hz_and_another_at_5_khz_do_not() {
    let s = scores(&sine(100.0, 0.5, 2.0), &sine(5000.0, 0.5, 2.0));
    for (k, score) in s.iter().enumerate() {
        assert!(
            score.is_none_or(|x| x < 0.01),
            "{}: {score:?}",
            BAND_NAMES[k]
        );
    }
}

#[test]
fn the_score_falls_with_the_level_gap_and_with_time_apart() {
    let band = band_of(300.0);
    let at =
        |gain: f32| scores(&sine(300.0, 0.5, 2.0), &sine(300.0, 0.5 * gain, 2.0))[band].unwrap();
    let same = at(1.0);
    assert!(same > 0.99, "{same}");
    // −4,5 dB: la mitad de los 9 dB.
    let half = at(10f32.powf(-4.5 / 20.0));
    assert!((half - 0.5).abs() < 0.03, "{half}");
    // −12 dB: ya no chocan.
    assert!(at(0.25) < 0.01);

    // Mismo seno, pero uno suena el primer segundo y el otro el segundo:
    // casi nunca a la vez.
    let first: Vec<f32> = sine(300.0, 0.5, 1.0)
        .into_iter()
        .chain(vec![0.0; 2 * RATE as usize])
        .collect();
    let second: Vec<f32> = vec![0.0; 2 * RATE as usize]
        .into_iter()
        .chain(sine(300.0, 0.5, 1.0))
        .collect();
    let apart = scores(&first, &second)[band].unwrap();
    assert!(apart < 0.05, "{apart}");
}

#[test]
fn a_silent_track_clashes_with_nothing_and_shorter_ones_are_padded() {
    let s = scores(&sine(300.0, 0.5, 1.0), &vec![0.0; 2 * RATE as usize]);
    assert!(s.iter().flatten().all(|&x| x == 0.0));
    // La segunda dura la mitad: chocan la mitad del tiempo, pero esas
    // ventanas tienen el doble de energía: 2/3.
    let s = scores(&sine(300.0, 0.5, 2.0), &sine(300.0, 0.5, 1.0));
    let half = s[band_of(300.0)].unwrap();
    assert!((half - 2.0 / 3.0).abs() < 0.03, "{half}");
}

#[test]
fn quiet_bands_are_ignored() {
    // Un seno fuerte a 300 Hz y, en las dos pistas, uno a 3 kHz 50 dB más
    // bajo: la banda de 3 kHz no se puntúa.
    let mix = |a: &[f32], b: &[f32]| -> Vec<f32> { a.iter().zip(b).map(|(x, y)| x + y).collect() };
    let quiet = sine(3000.0, 0.5 * 10f32.powf(-50.0 / 20.0), 2.0);
    let a = mix(&sine(300.0, 0.5, 2.0), &quiet);
    let b = mix(&sine(300.0, 0.5, 2.0), &quiet);
    let s = scores(&a, &b);
    assert!(s[band_of(3000.0)].is_none());
    assert!(s[band_of(300.0)].unwrap() > 0.99);
}
