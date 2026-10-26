//! Detección de la nota de cada corte, sobre notas sintéticas de altura
//! conocida.

#[path = "support/synth_break.rs"]
mod synth;

use engine::{Sample, Slices, detect_note, detect_notes, note_name};

const RATE: u32 = 48_000;

fn hz(midi: i32) -> f32 {
    440.0 * 2f32.powf((midi - 69) as f32 / 12.0)
}

fn load(audio: &[f32], rate: u32) -> Sample {
    Sample::from_interleaved(audio, 1, rate, rate).unwrap()
}

/// Una nota de `seconds` con armónicos (`harmonics` parciales a 1/k), un
/// ataque de 5 ms y una caída exponencial.
fn tone(rate: u32, f: f32, seconds: f32, harmonics: usize) -> Vec<f32> {
    let n = (seconds * rate as f32) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            let attack = (t / 0.005).min(1.0);
            let decay = (-3.0 * t / seconds).exp();
            let wave: f32 = (1..=harmonics)
                .map(|k| (std::f32::consts::TAU * f * k as f32 * t).sin() / k as f32)
                .sum();
            0.5 * attack * decay * wave
        })
        .collect()
}

/// Ruido blanco determinista.
fn noise(n: usize, seed: u64) -> Vec<f32> {
    let mut s = seed;
    (0..n)
        .map(|_| {
            s = s
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((s >> 40) as f32 / (1u64 << 24) as f32) * 2.0 - 1.0
        })
        .collect()
}

#[test]
fn pure_and_rich_tones_give_their_note_from_e1_to_c7() {
    for rate in [44_100, 48_000] {
        for midi in [28, 33, 40, 48, 57, 60, 64, 69, 76, 84, 96] {
            for harmonics in [1, 8] {
                let audio = tone(rate, hz(midi), 0.5, harmonics);
                let s = load(&audio, rate);
                let g = detect_note(&s, 0..s.frames())
                    .unwrap_or_else(|| panic!("{midi} a {rate} Hz con {harmonics}: sin nota"));
                assert_eq!(g.note(), midi, "{rate} Hz, {harmonics} armónicos: {g:?}");
                assert!(g.cents().abs() <= 3, "{midi}: {g:?}");
                assert!(g.clarity > 0.8, "{midi}: {g:?}");
            }
        }
    }
}

#[test]
fn a_detuned_tone_says_how_far_it_is() {
    // 25 cents por encima de A3.
    let f = 220.0 * 2f32.powf(0.25 / 12.0);
    let s = load(&tone(RATE, f, 0.4, 6), RATE);
    let g = detect_note(&s, 0..s.frames()).unwrap();
    assert_eq!(g.name(), "A3");
    assert!((g.cents() - 25).abs() <= 2, "{g:?}");
    assert!((g.hz - f).abs() < 0.5, "{g:?}");
}

#[test]
fn noise_silence_and_drums_have_no_note() {
    let s = load(&noise(RATE as usize / 2, 7), RATE);
    assert_eq!(detect_note(&s, 0..s.frames()), None);
    let s = load(&vec![0.0; RATE as usize / 2], RATE);
    assert_eq!(detect_note(&s, 0..s.frames()), None);
    // Un charles y una caja sueltos no tienen nota.
    for hit in [synth::Hit::Hat, synth::Hit::Snare] {
        let audio = synth::render_hits(synth::RATE, &[(hit, 0, 1.0)], synth::eighth());
        let s = load(&audio, synth::RATE);
        assert_eq!(detect_note(&s, 0..s.frames()), None, "{hit:?}");
    }
    // El bombo sí: baja de 120 a 45 Hz y lo que más suena es su cola, entre
    // F#1 y G1; esa cola sigue bajo el charles del corte siguiente.
    let (audio, _) = synth::synth_break();
    let s = load(&audio, synth::RATE);
    let notes = detect_notes(&s, &Slices::detect(&s, 8));
    for i in [0, 1, 4, 5] {
        let midi = notes[i].map_or(0.0, |n| n.midi);
        assert!((29.5..31.5).contains(&midi), "corte {}: {notes:?}", i + 1);
    }
}

#[test]
fn each_slice_of_a_melody_gets_its_note() {
    // Do, mi, sol, do, la grave, fa#: cada nota 250 ms.
    let melody = [60, 64, 67, 72, 45, 66];
    let audio: Vec<f32> = melody
        .iter()
        .flat_map(|&m| tone(RATE, hz(m), 0.25, 5))
        .collect();
    let s = load(&audio, RATE);
    let slices = Slices::detect(&s, melody.len());
    assert_eq!(slices.len(), melody.len());
    let names: Vec<String> = detect_notes(&s, &slices)
        .iter()
        .map(|n| n.map_or("-".into(), |n| n.name()))
        .collect();
    assert_eq!(names, ["C4", "E4", "G4", "C5", "A2", "F#4"]);
    // También en una rejilla que no cae en los ataques.
    let names: Vec<String> = detect_notes(&s, &Slices::grid(&s, melody.len()))
        .iter()
        .map(|n| n.map_or("-".into(), |n| n.name()))
        .collect();
    assert_eq!(names, ["C4", "E4", "G4", "C5", "A2", "F#4"]);
}

#[test]
fn a_slice_with_two_notes_says_the_longer() {
    let mut audio = tone(RATE, hz(62), 0.1, 4);
    audio.extend(tone(RATE, hz(55), 0.4, 4));
    let s = load(&audio, RATE);
    assert_eq!(detect_note(&s, 0..s.frames()).unwrap().name(), "G3");
}

#[test]
fn a_very_short_slice_does_not_panic() {
    let s = load(&tone(RATE, 440.0, 0.5, 3), RATE);
    for len in [0, 1, 10, 50, 100, 500, 2000] {
        let _ = detect_note(&s, 100..100 + len);
    }
    // Fuera del sample, nada.
    assert_eq!(detect_note(&s, s.frames()..s.frames() + 10), None);
    // Corto, pero con un tono agudo, aún se ve.
    assert_eq!(
        detect_note(&s, 0..1500).map(|g| g.name()),
        Some("A4".into())
    );
    assert_eq!(note_name(69), "A4");
}
