//! Análisis de choques de frecuencia de una canción (`Song::analyze_masking`)
//! con pistas sintéticas: dos senos a 300 Hz chocan en esa banda; uno a
//! 100 Hz y otro a 5 kHz, no; y se analiza el ciclo entero de la canción.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::masking::{BAND_NAMES, MaskingOptions};
use engine::{new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

/// Carpeta con senos de 2 s (un compás a 120 BPM) a 100, 300 y 5000 Hz.
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("masking");
        std::fs::create_dir_all(&dir).unwrap();
        for freq in [100, 300, 5000] {
            let mono: Vec<f32> = (0..2 * RATE as usize)
                .map(|i| 0.3 * (std::f32::consts::TAU * freq as f32 * i as f32 / RATE as f32).sin())
                .collect();
            offline::write_wav(dir.join(format!("sine{freq}.wav")), &mono, 1, RATE).unwrap();
        }
        dir
    })
    .clone()
}

fn song(source: &str) -> Song {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(&mut controller, source, "song.lua", &dir())
        .unwrap()
        .clone()
}

const ONE: &str = "1 . . .  . . . .  . . . .  . . . .";

fn two(a: &str, b: &str) -> String {
    format!(
        "bpm(120)\n\
         track(\"a\", sample(\"{a}\"):slice_grid(1)):steps(\"{ONE}\")\n\
         track(\"b\", sample(\"{b}\"):slice_grid(1)):steps(\"{ONE}\"):gain(0.8)\n"
    )
}

fn band(name: &str) -> usize {
    BAND_NAMES.iter().position(|b| *b == name).unwrap()
}

#[test]
fn two_sines_at_300_hz_clash_in_that_band() {
    let report = song(&two("sine300.wav", "sine300.wav"))
        .analyze_masking(RATE, &MaskingOptions::default())
        .unwrap();
    assert_eq!(report.tracks, ["a", "b"]);
    assert_eq!(report.bars, 1);
    let score = report.score("a", "b", band("315")).unwrap();
    assert!(score > 0.7, "{score}");
    let clashes = report.clashes();
    assert!(!clashes.is_empty());
    assert!(
        clashes
            .iter()
            .all(|c| ["250", "315", "400"].contains(&BAND_NAMES[c.band])),
        "{}",
        report.text()
    );
    let text = report.text();
    assert!(text.contains("a × b") && text.contains("315 Hz"), "{text}");
    assert!(text.contains("a 2 dB más fuerte"), "{text}");
}

#[test]
fn a_sine_at_100_hz_and_another_at_5_khz_do_not() {
    let report = song(&two("sine100.wav", "sine5000.wav"))
        .analyze_masking(RATE, &MaskingOptions::default())
        .unwrap();
    assert!(report.clashes().is_empty(), "{}", report.text());
    assert!(report.text().contains("sin choques"));
    for b in ["100", "5k"] {
        assert!(report.score("a", "b", band(b)).is_none_or(|s| s < 0.01));
    }
}

#[test]
fn the_whole_cycle_is_analysed() {
    // b solo suena uno de cada tres compases (`every(3)`): el ciclo es de 3
    // compases y chocan en uno. Con la energía como peso, ese compás pesa
    // el doble que cada uno de los otros dos: 2 / (2 + 1 + 1) = 0,5. Con un
    // solo compás analizado saldría 0 o 1.
    let source = format!(
        "bpm(120)\n\
         track(\"a\", sample(\"sine300.wav\"):slice_grid(1)):steps(\"{ONE}\")\n\
         track(\"b\", sample(\"sine300.wav\"):slice_grid(1))\n  \
         :every(3, function(c) c:steps(\"{ONE}\") end)\n"
    );
    let s = song(&source);
    assert_eq!(s.length_bars(1), 3);
    let report = s.analyze_masking(RATE, &MaskingOptions::default()).unwrap();
    assert_eq!(report.bars, 3);
    let score = report.score("a", "b", band("315")).unwrap();
    assert!((score - 0.5).abs() < 0.08, "{score}");
}
