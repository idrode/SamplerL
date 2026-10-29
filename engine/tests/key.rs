//! Tonalidad de un sample (`detect_key`): senos y acordes sintéticos de
//! tonalidad conocida, en las 24 tonalidades; una nota sola; ruido y
//! golpes (poca confianza); silencio.

use engine::{Sample, detect_key};

const RATE: u32 = 48_000;

fn hz(midi: i32) -> f32 {
    440.0 * 2f32.powf((midi - 69) as f32 / 12.0)
}

/// Acordes de `seconds` cada uno, con armónicos (como una sierra suave)
/// y un ataque y una caída para que no sean senos fijos.
fn chords(chords: &[&[i32]], seconds: f32) -> Sample {
    let len = (seconds * RATE as f32) as usize;
    let mut mono = vec![0.0f32; len * chords.len()];
    for (c, notes) in chords.iter().enumerate() {
        for &note in notes.iter() {
            for k in 1..=5 {
                let f = hz(note) * k as f32;
                if f > 20_000.0 {
                    continue;
                }
                for i in 0..len {
                    let t = i as f32 / RATE as f32;
                    let env = (t / 0.01).min(1.0) * (-2.0 * t).exp();
                    mono[c * len + i] +=
                        0.1 / k as f32 * env * (std::f32::consts::TAU * f * t).sin();
                }
            }
        }
    }
    Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap()
}

/// I-IV-V-I en mayor, o i-iv-V-i en menor (con la sensible), sobre la
/// tónica `tonic` (nota MIDI en la octava 3).
fn cadence(tonic: i32, minor: bool) -> Sample {
    let third = if minor { 3 } else { 4 };
    let i: Vec<i32> = vec![tonic, tonic + third, tonic + 7];
    let iv: Vec<i32> = vec![tonic + 5, tonic + 5 + third, tonic + 12];
    // La dominante siempre mayor (la sensible de la menor armónica).
    let v: Vec<i32> = vec![tonic + 7, tonic + 11, tonic + 14];
    let bass: Vec<i32> = vec![tonic - 12];
    let mut first = i.clone();
    first.extend(&bass);
    chords(&[&first, &iv, &v, &first], 0.6)
}

#[test]
fn cadences_give_their_key_in_all_24() {
    for tonic in 0..12 {
        for minor in [false, true] {
            let midi = 48 + tonic;
            let analysis = detect_key(&cadence(midi, minor)).unwrap();
            let best = analysis.best();
            assert_eq!(
                (best.tonic as i32, best.minor),
                (tonic, minor),
                "{} {}: salió {} ({:?})",
                tonic,
                if minor { "menor" } else { "mayor" },
                best.label(),
                analysis.candidates
            );
            assert!(
                !analysis.low_confidence(),
                "{}: confianza {}",
                best.label(),
                analysis.confidence
            );
            // La raíz: la tónica en la octava donde más suena.
            assert_eq!(analysis.root.rem_euclid(12), tonic);
        }
    }
}

#[test]
fn a_scale_of_sines_gives_its_key() {
    // C mayor subiendo y bajando, con la tónica al principio y al final.
    let scale = [
        60, 62, 64, 65, 67, 69, 71, 72, 71, 69, 67, 65, 64, 62, 60, 60,
    ];
    let notes: Vec<Vec<i32>> = scale.iter().map(|&n| vec![n]).collect();
    let refs: Vec<&[i32]> = notes.iter().map(Vec::as_slice).collect();
    let analysis = detect_key(&chords(&refs, 0.25)).unwrap();
    assert_eq!(
        analysis.best().label(),
        "C mayor",
        "{:?}",
        analysis.candidates
    );
    assert_eq!(analysis.root, 60);
}

#[test]
fn a_single_note_gives_its_root_and_says_so() {
    let analysis = detect_key(&chords(&[&[57]], 2.0)).unwrap();
    // A3: la tónica de la primera es la, y la raíz, en su octava.
    assert_eq!(analysis.root, 57, "{:?}", analysis);
    assert!(
        analysis.candidates.iter().any(|g| g.tonic_name() == "A"),
        "{:?}",
        analysis.candidates
    );
}

/// Ruido blanco: ninguna tonalidad.
#[test]
fn noise_has_low_confidence() {
    let mut x = 0x1234_5678u32;
    let mono: Vec<f32> = (0..RATE as usize * 2)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x as f32 / u32::MAX as f32 - 0.5) * 0.5
        })
        .collect();
    let analysis = detect_key(&Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap()).unwrap();
    assert!(analysis.low_confidence(), "{:?}", analysis);
}

/// Golpes de ruido que caen (un break sin tono): poca confianza.
#[test]
fn drums_have_low_confidence() {
    let mut x = 0x9e37_79b9u32;
    let beat = RATE as usize / 4;
    let mono: Vec<f32> = (0..RATE as usize * 2)
        .map(|i| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let t = (i % beat) as f32 / RATE as f32;
            (x as f32 / u32::MAX as f32 - 0.5) * (-30.0 * t).exp()
        })
        .collect();
    let analysis = detect_key(&Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap()).unwrap();
    assert!(analysis.low_confidence(), "{:?}", analysis);
}

#[test]
fn silence_or_too_short_is_nothing() {
    let silence = Sample::from_interleaved(&vec![0.0; RATE as usize], 1, RATE, RATE).unwrap();
    assert_eq!(detect_key(&silence), None);
    let short = Sample::from_interleaved(&vec![0.1; 100], 1, RATE, RATE).unwrap();
    assert_eq!(detect_key(&short), None);
}
