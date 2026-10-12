//! Detección de BPM sobre breaks sintéticos de tempo conocido.

#[path = "support/synth_break.rs"]
mod synth;

use engine::{Sample, detect_bpm, offline};
use synth::Hit;

const RATE: u32 = 48_000;

fn load(audio: &[f32], rate: u32) -> Sample {
    Sample::from_interleaved(audio, 1, rate, RATE).unwrap()
}

/// Un loop de `bars` compases a `bpm` con golpes en estas semicorcheas
/// (dentro de cada compás).
fn break_at(bpm: f64, bars: usize, sixteenths: &[(usize, Hit)]) -> Vec<f32> {
    let rate = 44_100;
    let step = rate as f64 * 60.0 / bpm / 4.0;
    let mut hits = Vec::new();
    for bar in 0..bars {
        for &(s, hit) in sixteenths {
            hits.push((hit, ((bar * 16 + s) as f64 * step).round() as usize, 1.0));
        }
    }
    let frames = (bars as f64 * 16.0 * step).round() as usize;
    synth::render_hits(rate, &hits, frames)
}

#[test]
fn the_synthetic_break_is_140_bpm_one_bar() {
    let (audio, _) = synth::synth_break();
    let guess = detect_bpm(&load(&audio, synth::RATE)).unwrap();
    assert!((guess.bpm - 140.0).abs() < 0.1, "{guess:?}");
    assert_eq!(guess.rounded(), 140.0);
    assert_eq!(guess.bars, 1);
    assert!(guess.score > 0.9, "{guess:?}");
    // También pasado por un WAV y cargado a otra frecuencia.
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("bpm");
    std::fs::create_dir_all(&dir).unwrap();
    offline::write_wav(dir.join("b.wav"), &audio, 1, synth::RATE).unwrap();
    let sample = Sample::load_wav(dir.join("b.wav"), 48_000).unwrap();
    assert_eq!(detect_bpm(&sample).unwrap().rounded(), 140.0);
}

#[test]
fn sixteenth_breaks_pick_the_fast_tempo_and_eighths_the_one_near_130() {
    // Golpes en semicorcheas impares: a 86 no encajan, a 172 sí.
    let busy = [
        (0, Hit::Kick),
        (3, Hit::Hat),
        (4, Hit::Snare),
        (7, Hit::Hat),
        (10, Hit::Kick),
        (11, Hit::Hat),
        (12, Hit::Snare),
        (15, Hit::Hat),
    ];
    let guess = detect_bpm(&load(&break_at(172.0, 2, &busy), 44_100)).unwrap();
    assert_eq!((guess.rounded(), guess.bars), (172.0, 2), "{guess:?}");
    assert_eq!(guess.alternative, None);

    // Solo corcheas en 4 compases a 96: encaja igual a 192 (8 compases); se
    // propone el más cercano a 130 y el otro como alternativa.
    let eighths = [
        (0, Hit::Kick),
        (2, Hit::Hat),
        (4, Hit::Snare),
        (6, Hit::Hat),
        (8, Hit::Kick),
        (10, Hit::Hat),
        (12, Hit::Snare),
        (14, Hit::Hat),
    ];
    let guess = detect_bpm(&load(&break_at(96.0, 4, &eighths), 44_100)).unwrap();
    assert_eq!((guess.rounded(), guess.bars), (96.0, 4), "{guess:?}");
    let (alt, alt_bars) = guess.alternative.unwrap();
    assert_eq!((engine::bpm::round_bpm(alt), alt_bars), (192.0, 8));
}

#[test]
fn silence_has_no_tempo() {
    assert_eq!(detect_bpm(&load(&vec![0.0; 48_000], 48_000)), None);
}
