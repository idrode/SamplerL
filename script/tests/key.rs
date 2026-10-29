//! `key()` y `transpose()`: la tonalidad de la canción en `scale()` y en los
//! grados de `melody()`; `transpose()` mueve las notas de instrumentos,
//! sintetizadores y `melody()`, no los `^` de una pista de cortes; entra
//! al compás; errores con la línea. Sin ellas, todo igual que siempre (lo
//! comprueba también `golden.rs`).

use std::f64::consts::TAU;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

/// `c4.wav`: un seno de un segundo a 261,63 Hz.
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("key");
        std::fs::create_dir_all(&dir).unwrap();
        let mono: Vec<f32> = (0..RATE as usize)
            .map(|i| (0.5 * (TAU * 261.625_565 * i as f64 / RATE as f64).sin()) as f32)
            .collect();
        offline::write_wav(dir.join("c4.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.set_seed(Some(7));
    let source = format!("bpm(120)\n{source}");
    host.load(&mut controller, &source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

fn render(source: &str) -> Vec<f32> {
    song(source).unwrap().render(RATE, 2).unwrap()
}

const PIANO: &str =
    "local piano = sample(\"c4.wav\"):instrument({ root = \"C4\", adsr = { 5, 0, 1, 30 } })\n";

#[test]
fn the_key_is_in_the_song() {
    let s = song("key(\"F#\", \"dorian\")").unwrap();
    let key = s.key.unwrap();
    assert_eq!((key.tonic, key.scale.name), (6, "dorian"));
    assert_eq!(key.call(), "key(\"F#\", \"dorian\")");
    assert_eq!(s.transpose, 0);
    // Sin modo, mayor; bemoles como en scale().
    assert_eq!(
        song("key(\"Bb\")").unwrap().key.unwrap().call(),
        "key(\"A#\", \"major\")"
    );
    assert_eq!(song("").unwrap().key, None);
    assert_eq!(song("transpose(-3)").unwrap().transpose, -3);
}

#[test]
fn scale_takes_what_is_missing_from_the_key() {
    let pitches = |source: &str| -> Vec<f64> {
        let s = song(&format!(
            "{source}\nTRACE = {{}}\nfor g = 1, 8 do TRACE[g] = ESC:pitch(g) end\n\
             track(\"t\", sample(\"c4.wav\"):slice(1)):steps(\"1^\" .. TRACE[3] .. \" . . .  . . . .  . . . .  . . . .\")"
        ))
        .unwrap();
        s.tracks[0]
            .steps
            .iter()
            .flatten()
            .map(|st| st.semitones as f64)
            .collect()
    };
    let explicit = pitches("local ESC = scale(\"minor\", \"A\")");
    assert_eq!(explicit, pitches("key(\"A\", \"minor\")\nESC = scale()"));
    assert_eq!(
        explicit,
        pitches("key(\"A\", \"minor\")\nESC = scale(\"minor\")")
    );
    // Con nombre y sin tónica: la tónica de la canción.
    assert_eq!(
        pitches("ESC = scale(\"dorian\", \"D\")"),
        pitches("key(\"D\", \"minor\")\nESC = scale(\"dorian\")")
    );
    // transpose() no mueve scale(): sus grados son semitonos exactos.
    assert_eq!(
        explicit,
        pitches("key(\"A\", \"minor\")\ntranspose(5)\nESC = scale()")
    );
}

/// Los grados de `melody()` suenan como las notas escritas a mano.
#[test]
fn melody_degrees_sound_like_their_notes() {
    let by_notes = render(&format!(
        "{PIANO}track(\"l\", piano):melody(\"a3 . c4 .  e4 . a4 .  g3 . b3+d4 _  . . a3 .\")"
    ));
    let by_degrees = render(&format!(
        "key(\"A\", \"minor\")\n{PIANO}track(\"l\", piano):melody(\"1 . 3 .  5 . 8 .  -1 . 2+4 _  . . 1 .\")"
    ));
    assert_eq!(by_notes, by_degrees);
    // También en every.
    let every = render(&format!(
        "key(\"A\", \"minor\")\n{PIANO}track(\"l\", piano):melody(\"1 . . .  . . . .  . . . .  . . . .\")\n\
         :every(2, function(c) c:melody(\"5 . . .  . . . .  . . . .  . . . .\") end)"
    ));
    let written = render(&format!(
        "{PIANO}track(\"l\", piano):melody(\"a3 . . .  . . . .  . . . .  . . . .\")\n\
         :every(2, function(c) c:melody(\"e4 . . .  . . . .  . . . .  . . . .\") end)"
    ));
    assert_eq!(every, written);
}

/// `transpose(2)` suena como escribirlo todo dos semitonos más arriba, en
/// los instrumentos (melody y steps con `^`, acordes) y en `melody()` de
/// una pista de cortes; los `^` de una pista de cortes no se mueven.
#[test]
fn transpose_moves_the_notes_but_not_the_drums() {
    let tracks = |shift: &str, notes: [&str; 3]| {
        format!(
            "{shift}{PIANO}\
             track(\"lead\", piano):melody(\"{} . . .  . . . .  {} _ . .  . . . .\")\n\
             track(\"chords\", piano):steps(\"1^0^4^7 . . .  . . . .  1^{} . . .  . . . .\")\n\
             track(\"drums\", sample(\"c4.wav\"):slice(1)):steps(\". . 1^5 .  . . . .  . . 1 .  . . . .\")\n\
             track(\"slices\", sample(\"c4.wav\"):slice(1)):melody(\". . . .  . c4 . .  . . . .  . e4 . .\"):gain(0.5)\n",
            notes[0], notes[1], notes[2]
        )
    };
    let moved = render(&tracks("transpose(2)\n", ["c4", "e4+g4", "5"]));
    let written = render(&format!(
        "{PIANO}\
         track(\"lead\", piano):melody(\"d4 . . .  . . . .  f#4+a4 _ . .  . . . .\")\n\
         track(\"chords\", piano):steps(\"1^2^6^9 . . .  . . . .  1^7 . . .  . . . .\")\n\
         track(\"drums\", sample(\"c4.wav\"):slice(1)):steps(\". . 1^5 .  . . . .  . . 1 .  . . . .\")\n\
         track(\"slices\", sample(\"c4.wav\"):slice(1)):melody(\". . . .  . d4 . .  . . . .  . f#4 . .\"):gain(0.5)\n"
    ));
    assert_eq!(moved.len(), written.len());
    let first = moved.iter().zip(&written).position(|(a, b)| a != b);
    assert_eq!(
        first,
        None,
        "distinto desde la muestra {first:?} (frame {:?})",
        first.map(|i| i / 2)
    );
    // transpose(0), igual que sin él.
    assert_eq!(
        render(&tracks("transpose(0)\n", ["c4", "e4+g4", "5"])),
        render(&tracks("", ["c4", "e4+g4", "5"]))
    );
}

/// Recargar con `transpose()` entra al empezar el compás, entero.
#[test]
fn transpose_enters_with_the_bar() {
    let base =
        format!("{PIANO}track(\"lead\", piano):melody(\"c4 _ _ _  _ _ _ _  _ _ _ _  _ _ _ _\")\n");
    let (mut controller, mut engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(
        &mut controller,
        &format!("bpm(120)\n{base}"),
        "song.lua",
        &dir(),
    )
    .unwrap();
    controller.start().unwrap();
    let bar = engine::bar_frames(120.0, RATE) as usize;
    let first = offline::render(&mut engine, bar / 2, 2);
    host.load(
        &mut controller,
        &format!("bpm(120)\ntranspose(12)\n{base}"),
        "song.lua",
        &dir(),
    )
    .unwrap();
    let rest = offline::render(&mut engine, bar / 2 + bar, 2);
    let alone = song(&base).unwrap().render(RATE, 2).unwrap();
    // Lo que queda del compás, sin cambiar.
    let half = bar / 2;
    assert_eq!(&first[..], &alone[..2 * half]);
    assert_eq!(&rest[..2 * half], &alone[2 * half..2 * bar]);
    // El compás siguiente, una octava arriba.
    let up = song(&format!("transpose(12)\n{base}"))
        .unwrap()
        .render(RATE, 2)
        .unwrap();
    assert_ne!(&rest[2 * half..], &alone[2 * bar..]);
    assert_eq!(&rest[2 * half + 2 * 2000..], &up[2 * bar + 2 * 2000..]);
}

#[test]
fn errors_say_where() {
    for (source, message) in [
        (
            "key(\"H\", \"minor\")",
            "song.lua:2: key(\"H\", \"minor\"): tónica no válida",
        ),
        (
            "key(\"A\", \"moll\")",
            "song.lua:2: key(\"A\", \"moll\"): el modo es una escala de scale()",
        ),
        (
            "key(\"A\")\nkey(\"B\")",
            "song.lua:3: key(\"B\", \"major\"): la tonalidad ya está en song.lua:2",
        ),
        (
            "local s = scale(\"minor\")\nkey(\"A\", \"minor\")",
            "song.lua:3: key(\"A\", \"minor\"): va antes de scale()",
        ),
        (
            "local s = scale()",
            "song.lua:2: scale(): sin nombre usa la tonalidad de la canción",
        ),
        (
            "transpose(30)",
            "song.lua:2: transpose(30): semitonos enteros de -24 a 24",
        ),
        ("transpose(1.5)", "transpose(1.5): semitonos enteros"),
        (
            "transpose(1)\ntranspose(2)",
            "song.lua:3: transpose(2): ya está en song.lua:2",
        ),
        ("section(\"a\", function() key(\"A\") end)", "key"),
        (
            &format!("{PIANO}track(\"l\", piano):melody(\"1 . 3 .  . . . .  . . . .  . . . .\")"),
            "el grado 1 necesita la tonalidad de la canción",
        ),
        (
            &format!(
                "transpose(24)\n{PIANO}track(\"l\", piano):melody(\"c6 . . .  . . . .  . . . .  . . . .\")"
            ),
            "pista \"l\": transpose(24): una nota queda a 48 semitonos de la raíz (máximo ±36)",
        ),
    ] {
        let Err(error) = song(source) else {
            panic!("{source}: sin error")
        };
        assert!(error.contains(message), "{source}\n{error}");
    }
}
