//! `cortes:note(i)`: la nota de cada corte de una melodía, en cifrado
//! americano, desde Lua.

use std::f64::consts::TAU;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

/// `melodia.wav`: do, mi, sol y la (25 cents alto), 300 ms cada una, con
/// armónicos y caída; `ruido.wav`: ruido blanco.
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("notes");
        std::fs::create_dir_all(&dir).unwrap();
        let note = (0.3 * RATE as f64) as usize;
        let melody: Vec<f32> = [60.0, 64.0, 67.0, 69.25]
            .iter()
            .flat_map(|&midi: &f64| {
                let f = 440.0 * 2f64.powf((midi - 69.0) / 12.0);
                (0..note).map(move |i| {
                    let t = i as f64 / RATE as f64;
                    let wave: f64 = (1..=4)
                        .map(|k| (TAU * f * k as f64 * t).sin() / k as f64)
                        .sum();
                    (0.4 * (t / 0.005).min(1.0) * (-4.0 * t).exp() * wave) as f32
                })
            })
            .collect();
        offline::write_wav(dir.join("melodia.wav"), &melody, 1, RATE).unwrap();
        let mut s = 1u64;
        let noise: Vec<f32> = (0..RATE as usize / 2)
            .map(|_| {
                s = s.wrapping_mul(6364136223846793005).wrapping_add(1);
                ((s >> 40) as f32 / (1u64 << 24) as f32) - 0.5
            })
            .collect();
        offline::write_wav(dir.join("ruido.wav"), &noise, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(&mut controller, source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

/// Lo que el script deja en `error(...)`.
fn said(source: &str) -> String {
    song(source).err().expect("el script tenía que fallar")
}

#[test]
fn each_slice_says_its_note_and_cents() {
    let text = said(
        "local m = sample(\"melodia.wav\"):slice(4)\n\
         local out = {}\n\
         for i = 1, m:count() do\n\
           local nota, cents = m:note(i)\n\
           out[#out + 1] = nota .. \"/\" .. cents\n\
         end\n\
         error(\"[\" .. table.concat(out, \" \") .. \"]\")\n",
    );
    let list = &text[text.find('[').unwrap() + 1..text.find(']').unwrap()];
    let got: Vec<(&str, i32)> = list
        .split(' ')
        .map(|p| {
            let (n, c) = p.split_once('/').unwrap();
            (n, c.parse().unwrap())
        })
        .collect();
    let names: Vec<&str> = got.iter().map(|g| g.0).collect();
    assert_eq!(names, ["C4", "E4", "G4", "A4"], "{text}");
    for (i, &(_, cents)) in got.iter().enumerate() {
        let expected = if i == 3 { 25 } else { 0 };
        assert!((cents - expected).abs() <= 3, "{text}");
    }
}

#[test]
fn the_note_feeds_melody_and_instrument_roots() {
    // La nota del corte vale tal cual en `melody` y en `root`.
    let song = song(
        "bpm(120)\n\
         local m = sample(\"melodia.wav\"):slice(4)\n\
         local raiz = m:note(1)\n\
         track(\"voz\", sample(\"melodia.wav\"):instrument({ root = raiz }))\n  \
           :melody(m:note(2) .. \" . \" .. m:note(3) .. \" .  . . . .  . . . .  . . . .\")\n",
    )
    .unwrap();
    let steps = &song.tracks[0].steps;
    assert_eq!(steps[0].map(|s| s.semitones), Some(4.0));
    assert_eq!(steps[2].map(|s| s.semitones), Some(7.0));
}

#[test]
fn noise_has_no_note_and_errors_say_the_line() {
    let text = said(
        "local r = sample(\"ruido.wav\"):slice(1)\n\
         error(\"[\" .. tostring(r:note(1)) .. \"]\")\n",
    );
    assert!(text.contains("[nil]"), "{text}");
    let text = said(
        "local m = sample(\"melodia.wav\"):slice(4)\n\
         m:note(9)\n",
    );
    assert!(
        text.contains("note(9): el corte no existe (hay 4)"),
        "{text}"
    );
    assert!(text.contains(":2:"), "{text}");
}
