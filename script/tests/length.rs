//! `pista:length(n)`: patrones de longitud variable y polirritmia. Cada
//! caso se compara, muestra a muestra, con el mismo ritmo escrito «a mano»
//! en un patrón largo.

use std::path::{Path, PathBuf};

use engine::{new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

fn dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("length");
    std::fs::create_dir_all(&dir).unwrap();
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
    dir
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.set_seed(Some(3));
    let source = format!("bpm(120)\nlocal b = sample(\"break.wav\"):slice(2)\n{source}");
    host.load(&mut controller, &source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

fn render(source: &str, bars: u64) -> Vec<f32> {
    song(source).unwrap().render(RATE, bars).unwrap()
}

/// `n` pasos en mini-notación: `x` en los que cumplen `hit`.
fn unrolled(n: usize, hit: impl Fn(usize) -> &'static str) -> String {
    (0..n).map(hit).collect::<Vec<_>>().join(" ")
}

#[test]
fn length_16_is_the_default() {
    let base = "track(\"d\", b):steps(\"1 . 2 .  1 . 2 2'  . 1^7 . 2r3  1 . . 2@0.5\")";
    let explicit = base.replace(":steps", ":length(16):steps");
    assert_eq!(render(&explicit, 4), render(base, 4));
}

#[test]
fn a_three_step_track_against_the_bar_is_its_unrolled_pattern() {
    let short = "track(\"d\", b):length(3):steps(\"1 . 2\")";
    let long = format!(
        "track(\"d\", b):length(48):steps(\"{}\")",
        unrolled(48, |g| ["1", ".", "2"][g % 3])
    );
    assert_eq!(render(short, 6), render(&long, 6));
    // Polirritmia: 3 contra 4 en dos pistas = las dos desenrolladas.
    let poly = "track(\"tres\", b):length(3):steps(\"1 . .\")\n\
                track(\"cuatro\", b):length(4):steps(\"2 . . .\")";
    let poly_long = format!(
        "track(\"tres\", b):steps(\"{}\")\ntrack(\"cuatro\", b):steps(\"{}\")",
        unrolled(16, |g| if g % 3 == 0 { "1" } else { "." }),
        unrolled(16, |g| if g % 4 == 0 { "2" } else { "." }),
    );
    // El de 3 no cabe en 16: se compara su primer compás.
    let bar = (2 * engine::bar_frames(120.0, RATE)) as usize;
    assert_eq!(render(poly, 1)[..bar], render(&poly_long, 1)[..bar]);
    assert_eq!(song(poly).unwrap().length_bars(1), 3);
}

#[test]
fn every_fill_and_swing_count_laps() {
    // every(2) en una pista de 12: vueltas pares invertidas.
    let laps = "track(\"d\", b):length(12)\n  :steps(\"1 . 2 .  1 . . .  2 . . .\")\n  \
                :every(2, function(c) c:steps(\"2 . 1 .  . . . .  . . . 1\") end)";
    let a = ["1", ".", "2", ".", "1", ".", ".", ".", "2", ".", ".", "."];
    let b = ["2", ".", "1", ".", ".", ".", ".", ".", ".", ".", ".", "1"];
    let long = format!(
        "track(\"d\", b):length(48):steps(\"{}\")",
        unrolled(48, |g| if (g / 12) % 2 == 0 {
            b[g % 12]
        } else {
            a[g % 12]
        })
    );
    assert_eq!(render(laps, 6), render(&long, 6));

    // fill con más pasos que la vuelta: la vuelta entera.
    let fill = "track(\"d\", b):length(3):steps(\"1 . .\"):fill(\"mute\", 2, 8)";
    let long = format!(
        "track(\"d\", b):length(48):steps(\"{}\")",
        unrolled(48, |g| if g % 3 == 0 && (g / 3) % 2 == 0 {
            "1"
        } else {
            "."
        })
    );
    assert_eq!(render(fill, 6), render(&long, 6));

    // Swing con longitud impar: los contratiempos son los de la canción.
    let swing = "track(\"d\", b):length(3):steps(\"1 2 1\"):swing(66)";
    let long = format!(
        "track(\"d\", b):length(48):steps(\"{}\"):swing(66)",
        unrolled(48, |g| ["1", "2", "1"][g % 3])
    );
    assert_eq!(render(swing, 6), render(&long, 6));
}

#[test]
fn the_song_length_covers_every_track_cycle() {
    let bars = |source: &str| song(source).unwrap().length_bars(1);
    assert_eq!(
        bars("track(\"d\", b):length(12):steps(\"1 . . .  . . . .  . . . .\")"),
        3
    );
    assert_eq!(
        bars(&format!(
            "track(\"d\", b):length(32):steps(\"{}\")",
            unrolled(32, |_| ".")
        )),
        2
    );
    assert_eq!(
        bars("track(\"d\", b):length(7):steps(\"1 . . . . . .\")"),
        7
    );
    // 61 y 63 pasos con un ciclo de 16 vueltas: el tope.
    let huge = format!(
        "track(\"a\", b):length(61):steps(\"{}\"):chance(0.5, \"mute\")\n\
         track(\"c\", b):length(63):steps(\"{}\")",
        unrolled(61, |_| "1"),
        unrolled(63, |_| "2")
    );
    assert_eq!(bars(&huge), script::MAX_SONG_BARS);
}

#[test]
fn length_errors_point_at_the_line() {
    let error = |source: &str| song(source).err().expect("debería fallar");
    assert_eq!(
        error("track(\"d\", b):length(0)"),
        "song.lua:3: length(0): la longitud va de 1 a 64 pasos"
    );
    assert!(error("track(\"d\", b):length(65)").contains("de 1 a 64"));
    assert_eq!(
        error("track(\"d\", b):steps(\"1 . . .  . . . .  . . . .  . . . .\"):length(12)"),
        "song.lua:3: length(12): steps ya tiene 16 pasos; pon :length(12) antes de :steps()"
    );
    assert_eq!(
        error("track(\"d\", b):length(12):steps(\"1 . . .  . . . .  . . . .  . . . .\")"),
        "song.lua:3: steps: el patrón necesita 12 pasos, tiene 16"
    );
    assert!(
        error(
            "track(\"d\", b):length(6):steps(\"1 . . . . .\")\n  \
             :every(2, function(c) c:steps(\"1 . .\") end)"
        )
        .contains("steps: el patrón necesita 6 pasos, tiene 3")
    );
    // La misma longitud otra vez, o antes de steps, vale.
    assert!(song("track(\"d\", b):length(6):steps(\"1 . . . . .\"):length(6)").is_ok());
}
