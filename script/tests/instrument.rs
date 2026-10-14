//! `sample:instrument()` y `pista:melody()`: las notas suenan a su
//! frecuencia respecto a la raíz, el script suena igual que el motor con lo
//! mismo, el bucle no hace clic y los errores dicen la línea.

use std::f64::consts::TAU;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{Adsr, Instrument, Pattern, Sample, Slices, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;
/// 120 BPM: 6000 frames por paso.
const STEP: usize = 6000;

/// Senos de un segundo: `c4.wav` a 261,63 Hz y `a3.wav` a 220 Hz.
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("instrument");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, freq) in [("c4.wav", 261.625_565), ("a3.wav", 220.0)] {
            let mono: Vec<f32> = (0..RATE as usize)
                .map(|i| (0.5 * (TAU * freq * i as f64 / RATE as f64).sin()) as f32)
                .collect();
            offline::write_wav(dir.join(name), &mono, 1, RATE).unwrap();
        }
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!("bpm(120)\n{source}");
    host.load(&mut controller, &source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

/// Frecuencia por los cruces por cero (interpolados) de `x`.
fn frequency(x: &[f32]) -> f64 {
    let crossings: Vec<f64> = x
        .windows(2)
        .enumerate()
        .filter(|(_, w)| (w[0] < 0.0) != (w[1] < 0.0))
        .map(|(i, w)| i as f64 + w[0] as f64 / (w[0] - w[1]) as f64)
        .collect();
    let (first, last) = (crossings[0], crossings[crossings.len() - 1]);
    (crossings.len() - 1) as f64 / 2.0 / ((last - first) / RATE as f64)
}

/// Amplitud de la componente a `freq` (con ventana de Hann).
fn level(x: &[f32], freq: f64) -> f64 {
    let w = TAU * freq / RATE as f64;
    let n = x.len() as f64;
    let (mut c, mut s, mut sum) = (0.0, 0.0, 0.0);
    for (k, &v) in x.iter().enumerate() {
        let hann = 0.5 - 0.5 * (TAU * k as f64 / n).cos();
        c += hann * v as f64 * (w * k as f64).cos();
        s += hann * v as f64 * (w * k as f64).sin();
        sum += hann;
    }
    2.0 * (c * c + s * s).sqrt() / sum
}

fn hz(midi: i32) -> f64 {
    440.0 * 2f64.powf((midi - 69) as f64 / 12.0)
}

/// Cada nota de la melodía suena a su frecuencia: la del sample en la raíz
/// y cromáticamente desde ahí, con la raíz que se diga.
#[test]
fn each_note_sounds_at_its_frequency_from_the_root() {
    // (sample, raíz, melodía de 4 notas de 4 pasos, sus notas MIDI)
    let cases = [
        (
            "c4.wav",
            "C4",
            "c _ _ _  e _ _ _  g _ _ _  c5 _ _ _",
            [60, 64, 67, 72],
        ),
        (
            "a3.wav",
            "A3",
            "a _ _ _  c4 _ _ _  e4 _ _ _  a2 _ _ _",
            [57, 60, 64, 45],
        ),
        // La raíz en número MIDI, y notas por debajo con alteraciones.
        (
            "c4.wav",
            "60",
            "bb3 _ _ _  f#3 _ _ _  eb4 _ _ _  c3 _ _ _",
            [58, 54, 63, 48],
        ),
    ];
    for (wav, root, melody, midis) in cases {
        let root = if root.chars().all(|c| c.is_ascii_digit()) {
            root.to_string()
        } else {
            format!("\"{root}\"")
        };
        let song = song(&format!(
            "local i = sample(\"{wav}\"):instrument({{ root = {root}, adsr = {{ 5, 0, 1, 20 }} }})\n\
             track(\"lead\", i):melody(\"{melody}\")"
        ))
        .unwrap();
        let audio = left(&song.render(RATE, 1).unwrap());
        for (k, midi) in midis.into_iter().enumerate() {
            let note = &audio[k * 4 * STEP + 1000..(k + 1) * 4 * STEP - 1000];
            let f = frequency(note);
            assert!(
                (f / hz(midi) - 1.0).abs() < 0.001,
                "{wav} {root}: nota {k} {f:.2} Hz, se esperaba {:.2}",
                hz(midi)
            );
        }
    }
}

/// Lo mismo escrito con el motor: el instrumento y `melody` dan los mismos
/// pasos y el mismo audio, muestra a muestra.
#[test]
fn the_script_sounds_like_the_engine() {
    let song = song(
        "local i = sample(\"c4.wav\"):instrument({\n\
           root = \"C3\", loop = { 100, 600 }, voices = 3, adsr = { 4, 50, 0.6, 120 }, crossfade = 15,\n\
         })\n\
         track(\"lead\", i):melody(\"c+e+g _ . e  g _ _ c4  . . b2 _  c2 . d# .\"):gain(0.8)",
    )
    .unwrap();
    let sample = Arc::new(Sample::load_wav(dir().join("c4.wav"), RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    let steps =
        engine::notation::parse_steps("1^0^4^7 _ . 1^4  1^7 _ _ 1^12  . . 1^-1 _  1^-12 . 1^3 .")
            .unwrap();
    let instrument = Instrument {
        voices: 3,
        adsr: Adsr {
            attack_ms: 4.0,
            decay_ms: 50.0,
            sustain: 0.6,
            release_ms: 120.0,
        },
        sustain_loop: Some((4800, 28_800)),
        crossfade: 720,
    };
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_gain(0.8)
        .unwrap()
        .with_instrument(Some(instrument))
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_track_pattern(0, Some(pattern)).unwrap();
    controller.start().unwrap();
    let frames = 2 * engine::bar_frames(120.0, RATE) as usize;
    let direct = offline::render(&mut engine, frames, 2);
    let scripted = song.render(RATE, 2).unwrap();
    assert_eq!(scripted.len(), direct.len());
    assert!(scripted == direct, "el script no suena como el motor");
    assert_eq!(song.tracks[0].instrument.unwrap().root, 48);
}

/// Una nota sostenida más larga que el sample sigue en bucle sin clic.
#[test]
fn a_held_note_loops_without_clicks() {
    let song = song(
        "local i = sample(\"a3.wav\"):instrument({ loop = { 250.3, 731.7 }, adsr = { 10, 0, 1, 50 } })\n\
         track(\"pad\", i):length(32):steps(\"1 _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ .\")",
    )
    .unwrap();
    let audio = left(&song.render(RATE, 2).unwrap());
    // 31 pasos de nota: casi 4 s con un sample de 1 s.
    let held = &audio[2000..31 * STEP];
    let jump = held
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    // Un clic sería un salto muy mayor que la pendiente del seno. El fundido
    // lineal entre dos trozos con la fase distinta añade, como mucho,
    // 2·amplitud/fundido por muestra (10 ms a 220 Hz: un 14 % de la
    // pendiente).
    let slope = (0.5 * TAU * 220.0 / RATE as f64) as f32;
    let crossfade = 0.010 * RATE as f32;
    assert!(
        jump <= slope + 2.0 * 0.5 / crossfade,
        "salto {jump} (seno {slope})"
    );
    for chunk in held.chunks(4800) {
        let peak = chunk.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.42, "pico {peak}");
    }
}

/// Con dos voces, un acorde de tres notas deja sonar las dos últimas (la
/// primera se roba al llegar la tercera).
#[test]
fn chords_steal_when_there_are_not_enough_voices() {
    let song = song(
        "local i = sample(\"c4.wav\"):instrument({ voices = 2 })\n\
         track(\"lead\", i):melody(\"c+e+g _ _ _  . . . .  . . . .  . . . .\")",
    )
    .unwrap();
    let audio = left(&song.render(RATE, 1).unwrap());
    let chord = &audio[1000..4 * STEP - 1000];
    assert!(level(chord, hz(60)) < 0.002, "{}", level(chord, hz(60)));
    assert!(level(chord, hz(64)) > 0.4);
    assert!(level(chord, hz(67)) > 0.4);
}

/// En una pista de cortes, `melody` toca el corte 1 con C4 como su tono
/// original: igual que escribir los pasos con `^`.
#[test]
fn melody_on_a_sliced_track_is_pitch_on_slice_one() {
    let track = "track(\"m\", sample(\"c4.wav\"):slice_grid(2))";
    let notes = song(&format!(
        "{track}:melody(\"c . e .  g . c5 .  b3 . . .  c _ . .\")"
    ))
    .unwrap();
    let steps = song(&format!(
        "{track}:steps(\"1^0 . 1^4 .  1^7 . 1^12 .  1^-1 . . .  1^0 _ . .\")"
    ))
    .unwrap();
    assert!(notes.tracks[0].bars == steps.tracks[0].bars);
    assert!(notes.render(RATE, 1).unwrap() == steps.render(RATE, 1).unwrap());
    // `melody` también dentro de `every`, con la raíz del instrumento.
    let every = song(
        "local i = sample(\"c4.wav\"):instrument({ root = \"C3\" })\n\
         track(\"lead\", i):melody(\"c . . .  . . . .  . . . .  . . . .\")\n\
           :every(2, function(c) c:melody(\"e+g . . .  . . . .  . . . .  . . . .\") end)",
    )
    .unwrap();
    let first = every.tracks[0].bars[0][0].unwrap();
    assert_eq!(first.notes().collect::<Vec<_>>(), vec![4.0, 7.0]);
    assert_eq!(every.tracks[0].bars[1][0].unwrap().semitones, 0.0);
}

/// El fundido del bucle se acorta si no hay audio antes de su inicio, y se
/// avisa.
#[test]
fn a_crossfade_that_does_not_fit_is_shortened_with_a_warning() {
    let song = song(
        "local i = sample(\"c4.wav\"):instrument({ loop = { 5, 500 }, crossfade = 30 })\n\
         track(\"lead\", i):melody(\"c . . .  . . . .  . . . .  . . . .\")",
    )
    .unwrap();
    let instrument = song.tracks[0].instrument.unwrap();
    assert_eq!(instrument.engine.crossfade, 240);
    assert_eq!(instrument.engine.sustain_loop, Some((240, 24_000)));
    assert!(
        song.warnings
            .iter()
            .any(|w| w.contains("se queda en 5.0 ms") && w.contains("pedía 30")),
        "{:?}",
        song.warnings
    );
    let song = song_ok(
        "local i = sample(\"c4.wav\"):instrument({ loop = { 0, 500 } })\n\
                        track(\"lead\", i):melody(\"c . . .  . . . .  . . . .  . . . .\")",
    );
    assert!(song.warnings.iter().any(|w| w.contains("puede hacer clic")));
}

fn song_ok(source: &str) -> Song {
    song(source).unwrap()
}

#[test]
fn errors_say_the_line() {
    let cases = [
        (
            "sample(\"c4.wav\"):instrument({ root = \"H2\" })",
            "no es una nota",
        ),
        (
            "sample(\"c4.wav\"):instrument({ root = 200 })",
            "número MIDI",
        ),
        (
            "sample(\"c4.wav\"):instrument({ voices = 0 })",
            "voices va de 1 a 16",
        ),
        (
            "sample(\"c4.wav\"):instrument({ voices = 17 })",
            "voices va de 1 a 16",
        ),
        (
            "sample(\"c4.wav\"):instrument({ loop = { 500, 100 } })",
            "dentro del sample",
        ),
        (
            "sample(\"c4.wav\"):instrument({ loop = { 0, 5000 } })",
            "dentro del sample",
        ),
        (
            "sample(\"c4.wav\"):instrument({ loop = 3 })",
            "{inicio, fin}",
        ),
        (
            "sample(\"c4.wav\"):instrument({ adsr = { 1, 2, 3 } })",
            "ataque, decay, sustain, release",
        ),
        (
            "sample(\"c4.wav\"):instrument({ adsr = { 1, 2, 1.5, 3 } })",
            "sustain de adsr",
        ),
        (
            "sample(\"c4.wav\"):instrument({ adsr = { -1, 2, 1, 3 } })",
            "de 0 a 10000",
        ),
        (
            "sample(\"c4.wav\"):instrument({ crossfade = 20 })",
            "falta loop",
        ),
        (
            "sample(\"c4.wav\"):instrument({ voice = 2 })",
            "clave desconocida \"voice\"",
        ),
        (
            "sample(\"c4.wav\"):stretch(120):instrument()",
            "está estirado",
        ),
        (
            "track(\"t\", sample(\"c4.wav\"):slice(1)):steps(\"1^0^4 . . .  . . . .  . . . .  . . . .\")",
            "necesita una pista con instrumento",
        ),
        (
            "track(\"t\", sample(\"c4.wav\"):slice(1)):melody(\"c+e . . .  . . . .  . . . .  . . . .\")",
            "necesita una pista con instrumento",
        ),
        (
            "track(\"t\", sample(\"c4.wav\"):instrument()):melody(\"c h . .  . . . .  . . . .  . . . .\")",
            "paso 2 (\"h\")",
        ),
        (
            "track(\"t\", sample(\"c4.wav\"):instrument()):melody(\"c . . .\")",
            "16 pasos",
        ),
        (
            "track(\"t\", sample(\"c4.wav\"):instrument()):steps(\"2 . . .  . . . .  . . . .  . . . .\")",
            "el corte 2 no existe",
        ),
        (
            "track(\"t\", sample(\"c4.wav\"):instrument()):midi(1)",
            "aún no está hecha",
        ),
        (
            "track(\"t\", scale(\"minor\"))",
            "cortes (sample(...):slice(n)) o un instrumento",
        ),
    ];
    for (line, expected) in cases {
        let Err(err) = song(&format!("local x = 1\n{line}")) else {
            panic!("{line}: debería fallar");
        };
        assert!(err.contains("song.lua:3:"), "{line}: {err}");
        assert!(err.contains(expected), "{line}: {err}");
    }
}
