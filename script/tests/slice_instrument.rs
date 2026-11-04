//! Un corte como instrumento (`cortes:slice_at(i):instrument{...}`): la
//! raíz sale de su tono (YIN) con los cents corregidos, cada nota suena a
//! su frecuencia como sampler clásico y con `keep`, la duración cambia o no
//! según el modo, y los errores dicen la línea.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{detect_mono, new_engine};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;
/// Frames de cada corte (250 ms).
const SLICE: usize = 12_000;
/// El corte 2 es un do4 subido 30 cents.
const DETUNED: f32 = 261.625_58 * 1.017_478_6;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("slice_instrument");
        std::fs::create_dir_all(&dir).unwrap();
        // Tres cortes iguales: un la3 (220 Hz), un do4 desafinado y ruido.
        let mut seed = 0x1234_5678u32;
        let mono: Vec<f32> = (0..3 * SLICE)
            .map(|i| {
                let t = i as f32 / RATE as f32;
                match i / SLICE {
                    0 => 0.4 * (std::f32::consts::TAU * 220.0 * t).sin(),
                    1 => 0.4 * (std::f32::consts::TAU * DETUNED * t).sin(),
                    _ => {
                        seed ^= seed << 13;
                        seed ^= seed >> 17;
                        seed ^= seed << 5;
                        0.3 * (seed as f32 / u32::MAX as f32 * 2.0 - 1.0)
                    }
                }
            })
            .collect();
        engine::offline::write_wav(dir.join("tonos.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!("bpm(120)\nlocal t = sample(\"tonos.wav\"):slice_grid(3)\n{source}");
    host.load(&mut controller, &source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

/// Canal izquierdo de `bars` compases.
fn left(song: &Song, bars: u64) -> Vec<f32> {
    song.render(RATE, bars)
        .unwrap()
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

fn sounding(left: &[f32]) -> usize {
    left.iter()
        .rposition(|x| x.abs() > 1e-6)
        .map_or(0, |i| i + 1)
}

/// A 120 BPM, un paso son 6000 frames y un compás 96 000.
const STEP: usize = 6000;

#[test]
fn the_root_is_the_tone_of_the_slice_with_its_cents() {
    let song = song(
        "track(\"a\", t:slice_at(1):instrument()):length(1):melody(\"a3\")\n\
         track(\"c\", t:slice_at(2):instrument()):length(1):melody(\"c4\")",
    )
    .unwrap();
    let a = song.tracks[0].instrument.unwrap();
    assert_eq!(a.root, 57);
    assert!(a.engine.tune.abs() < 0.02, "{}", a.engine.tune);
    let c = song.tracks[1].instrument.unwrap();
    assert_eq!(c.root, 60);
    // `cortes:instrument(2)` es lo mismo que `slice_at(2):instrument()`.
    let same = song_ok("track(\"c\", t:instrument(2)):length(1):melody(\"c4\")");
    assert_eq!(same.tracks[0].instrument, song.tracks[1].instrument);
    assert_eq!(same.tracks[0].slices, song.tracks[1].slices);
    assert!((c.engine.tune - 0.3).abs() < 0.02, "{}", c.engine.tune);
    // `root` a mano manda (y no corrige nada); `"auto"` vale también en
    // el sample entero.
    let song =
        song_ok("track(\"c\", t:slice_at(2):instrument{ root = \"D4\" }):length(1):melody(\"d4\")");
    let c = song.tracks[0].instrument.unwrap();
    assert_eq!((c.root, c.engine.tune), (62, 0.0));
    let song = song_ok(
        "local s = sample(\"tonos.wav\"):instrument{ root = \"auto\" }\n\
         track(\"s\", s):length(1):melody(\"a3\")",
    );
    assert_eq!(song.tracks[0].instrument.unwrap().root, 57);
}

fn song_ok(source: &str) -> Song {
    song(source).unwrap()
}

/// La frecuencia que mide YIN entre `from` y `to` frames.
fn hz(left: &[f32], from: usize, to: usize) -> f32 {
    detect_mono(&left[from..to], RATE).expect("tiene tono").hz
}

fn midi_hz(note: i32) -> f32 {
    440.0 * 2f32.powf((note - 69) as f32 / 12.0)
}

#[test]
fn each_note_sounds_at_its_frequency_in_both_modes() {
    // Una nota por compás: do3, mi3, sol3 y do5 (de -12 a +12 del do4
    // desafinado).
    let notes = [("c3", 48), ("e3", 52), ("g3", 55), ("c5", 72)];
    let melody: Vec<String> = notes
        .iter()
        .map(|(n, _)| format!("{n} . . .  . . . .  . . . .  . . . ."))
        .collect();
    let mut worst = [0.0f32; 2];
    for (m, keep) in ["false", "true"].iter().enumerate() {
        let song = song_ok(&format!(
            "track(\"c\", t:slice_at(2):instrument{{ keep = {keep} }})\n  :length(64):melody(\"{}\")",
            melody.join("  ")
        ));
        let left = left(&song, notes.len() as u64);
        for (bar, (name, midi)) in notes.iter().enumerate() {
            let start = bar * 16 * STEP;
            // Lo que dura la nota más corta (do5 como sampler: 125 ms).
            let got = hz(&left, start + 600, start + 5400);
            let error = got / midi_hz(*midi) - 1.0;
            worst[m] = worst[m].max(error.abs());
            assert!(
                error.abs() < 0.003,
                "{name} (keep = {keep}): {got} Hz, error {:.3} %",
                100.0 * error
            );
        }
    }
    println!(
        "error máximo de frecuencia: resample {:.3} %, keep {:.3} %",
        100.0 * worst[0],
        100.0 * worst[1]
    );
}

#[test]
fn resample_changes_the_duration_and_keep_does_not() {
    // Como sampler, la corrección de los 30 cents también alarga un poco.
    let cents = 2f64.powf(0.3 / 12.0);
    let slice = SLICE as f64;
    for (keep, note, frames) in [
        ("false", "c3", (2.0 * slice * cents).round() as usize),
        ("false", "c5", (slice / 2.0 * cents).round() as usize),
        ("true", "c3", SLICE),
        ("true", "c5", SLICE),
    ] {
        let song = song_ok(&format!(
            "track(\"c\", t:slice_at(2):instrument{{ keep = {keep}, adsr = {{ 0, 0, 1, 10 }} }})\n  \
             :melody(\"{note} _ _ _  _ _ _ _  _ _ _ _  _ _ _ _\")"
        ));
        let got = sounding(&left(&song, 1));
        // Acaba con el corte (con el fundido mínimo y el redondeo de la
        // lectura a otra velocidad).
        assert!(
            got.abs_diff(frames) <= 4,
            "{note} keep = {keep}: {got} frames, el corte da {frames}"
        );
    }
}

#[test]
fn a_loop_in_the_slice_sustains_long_notes_and_chords_are_polyphonic() {
    let song = song_ok(
        "track(\"c\", t:slice_at(2):instrument{ voices = 3, loop = { 50, 200 } })\n  \
         :melody(\"c3+e3+g3 _ _ _  _ _ _ _  _ _ _ _  _ _ _ _\")",
    );
    let left = left(&song, 1);
    // El corte dura 250 ms, la nota (a -12, el doble) sigue el compás entero.
    for w in left[1000..90_000].chunks(4800) {
        assert!(w.iter().any(|x| x.abs() > 0.1));
    }
    let def = song.tracks[0].instrument.unwrap();
    // Los ms del bucle cuentan desde el inicio del corte.
    assert_eq!(def.engine.sustain_loop, Some((SLICE + 2400, SLICE + 9600)));
}

#[test]
fn errors_point_at_the_line() {
    let error = |source: &str| song(source).err().expect("debería fallar");
    assert_eq!(
        error("track(\"r\", t:slice_at(3):instrument())"),
        "song.lua:3: instrument(tonos.wav, corte): no se ve el tono del corte (YIN); pon la raíz a mano: root = \"C3\""
    );
    assert!(error("track(\"r\", t:instrument())").contains(
        "estos cortes son 3; elige uno: cortes:instrument(3, {...}) o cortes:slice_at(3):instrument"
    ));
    assert!(
        error("t:slice_at(1):instrument{ loop = { 10, 400 } }")
            .contains("tiene que ir dentro del corte (0 a 250.0 ms)")
    );
    assert!(
        error("t:instrument(4, { root = 60 })")
            .contains("instrument(4): el corte no existe (hay 3)")
    );
    // Con root a mano, el ruido se toca igual.
    assert!(
        song("track(\"r\", t:slice_at(3):instrument{ root = 60 }):length(1):melody(\"c4\")")
            .is_ok()
    );
}
