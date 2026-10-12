//! Fase C: funciones de patrones, escalas, swing, humanize y fill. Render
//! offline con semilla fija: cada función suena igual que el patrón que
//! dice generar.

use std::path::{Path, PathBuf};

use engine::offline;
use script::ScriptHost;

const RATE: u32 = 48_000;

/// `break8.wav`: ocho cortes de 300 frames, cada 3000, de volumen distinto.
fn fixture(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("patterns_{test}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut eight = vec![0.0f32; 24_000];
    for k in 0..8 {
        eight[k * 3000..k * 3000 + 300].fill(0.2 + 0.05 * k as f32);
    }
    offline::write_wav(dir.join("break8.wav"), &eight, 1, RATE).unwrap();
    dir
}

fn host() -> ScriptHost {
    let mut host = ScriptHost::new(RATE);
    host.set_seed(Some(11));
    host
}

const HEAD: &str = "bpm(120)\nlocal amen = sample(\"break8.wav\"):slice(8)\n";

/// Evalúa y renderiza `bars` compases (en un motor aparte).
fn render(dir: &Path, body: &str, bars: u64) -> Vec<f32> {
    let source = format!("{HEAD}{body}");
    let song = host()
        .eval(&source, "song.lua", dir)
        .unwrap_or_else(|e| panic!("{e}\n{source}"));
    song.render(RATE, bars).unwrap()
}

/// Valor de una expresión de Lua que devuelve texto (vía un error, que es
/// lo único que sale del script además del modelo).
fn value(dir: &Path, expr: &str) -> String {
    let source = format!("{HEAD}error(\"=\" .. {expr}, 0)");
    match host().eval(&source, "song.lua", dir) {
        Ok(_) => panic!("debería devolver el valor"),
        Err(e) => e
            .message
            .split_once('=')
            .map(|(_, v)| v.lines().next().unwrap_or("").to_string())
            .unwrap_or(e.message),
    }
}

/// Mismo audio, muestra a muestra (sin imprimir megas si falla).
fn same(a: &[f32], b: &[f32]) {
    assert_eq!(a.len(), b.len());
    if let Some(i) = a.iter().zip(b).position(|(x, y)| x != y) {
        panic!("el audio difiere en la muestra {i}: {} != {}", a[i], b[i]);
    }
}

fn error(dir: &Path, body: &str) -> String {
    match host().eval(&format!("{HEAD}{body}"), "song.lua", dir) {
        Ok(_) => panic!("debería fallar: {body}"),
        Err(e) => e.message,
    }
}

#[test]
fn euclid_sounds_like_the_pattern_it_writes() {
    let dir = fixture("euclid");
    assert_eq!(
        value(&dir, "euclid(3, 8, \"1\")"),
        "1 . . 1  . . 1 .  1 . . 1  . . 1 ."
    );
    let a = render(&dir, "track(\"d\", amen):steps(euclid(3, 8, \"2^7\"))", 2);
    let b = render(
        &dir,
        "track(\"d\", amen):steps(\"2^7 . . 2^7  . . 2^7 .  2^7 . . 2^7  . . 2^7 .\")",
        2,
    );
    assert!(a.iter().any(|s| s.abs() > 0.1));
    same(&a, &b);
    // Dentro de every también.
    let a = render(
        &dir,
        "track(\"d\", amen):steps(euclid(4, 16)):every(2, function(c) c:steps(euclid(5, 8, \"3\")) end)",
        2,
    );
    let b = render(
        &dir,
        "track(\"d\", amen):steps(\"x . . .  x . . .  x . . .  x . . .\")\
         :every(2, function(c) c:steps(\"3 . 3 .  3 3 . 3  3 . 3 .  3 3 . 3\") end)",
        2,
    );
    same(&a, &b);
    let e = error(&dir, "track(\"d\", amen):steps(euclid(3, 20))");
    assert_eq!(e, "song.lua:3: euclid(3, 20): n va de 1 a 16");
}

#[test]
fn rotate_sounds_like_the_pattern_it_writes() {
    let dir = fixture("rotate");
    let p = "\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\"";
    assert_eq!(
        value(&dir, &format!("rotate({p}, -4)")),
        "3 . 4 .  5 6 2 .  7 . 8 .  1 . 2 ."
    );
    let a = render(
        &dir,
        &format!("track(\"d\", amen):steps(rotate({p}, 3))"),
        2,
    );
    let b = render(
        &dir,
        "track(\"d\", amen):steps(\". 8 . 1  . 2 . 3  . 4 . 5  6 2 . 7\")",
        2,
    );
    same(&a, &b);
    assert_eq!(
        error(&dir, "rotate(\"1 2\", 1)"),
        "song.lua:3: rotate(patrón, 1): el patrón necesita 16 pasos, tiene 2"
    );
}

#[test]
fn reverse_pattern_sounds_like_the_pattern_it_writes() {
    let dir = fixture("reverse_pattern");
    let p = "\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 3'\"";
    assert_eq!(
        value(&dir, &format!("reverse_pattern({p})")),
        "3' 8 . 7  . 2 6 5  . 4 . 3  . 2 . 1"
    );
    let a = render(
        &dir,
        &format!("track(\"d\", amen):steps(reverse_pattern({p}))"),
        1,
    );
    let b = render(
        &dir,
        "track(\"d\", amen):steps(\"3' 8 . 7  . 2 6 5  . 4 . 3  . 2 . 1\")",
        1,
    );
    same(&a, &b);
}

#[test]
fn mutate_follows_its_seed_or_the_load_seed() {
    let dir = fixture("mutate");
    let p = "\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\"";
    let fixed = value(&dir, &format!("mutate({p}, 0.4, 5)"));
    assert_ne!(fixed, "1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .");
    // Con semilla propia, igual aunque cambie la de la carga.
    let mut other = ScriptHost::new(RATE);
    other.set_seed(Some(999));
    let source = format!("{HEAD}error(\"=\" .. mutate({p}, 0.4, 5), 0)");
    let e = other.eval(&source, "song.lua", &dir).err().unwrap().message;
    assert!(e.starts_with(&format!("={fixed}")), "{e}");
    // Suena como el patrón que escribe.
    let a = render(
        &dir,
        &format!("track(\"d\", amen):steps(mutate({p}, 0.4, 5))"),
        1,
    );
    let b = render(&dir, &format!("track(\"d\", amen):steps(\"{fixed}\")"), 1);
    same(&a, &b);

    // Sin semilla: reproducible con la de la carga, distinta con otra.
    let free = value(&dir, &format!("mutate({p}, 0.4)"));
    assert_eq!(free, value(&dir, &format!("mutate({p}, 0.4)")));
    let e = other
        .eval(
            &format!("{HEAD}error(\"=\" .. mutate({p}, 0.4), 0)"),
            "song.lua",
            &dir,
        )
        .err()
        .unwrap()
        .message;
    assert!(
        !e.starts_with(&format!("={free}\n")),
        "otra semilla, otro resultado"
    );
    // En every, cada compás distinto; y el ciclo se repite igual.
    let source = format!(
        "{HEAD}track(\"d\", amen):steps({p}):every(1, function(c) c:steps(mutate({p}, 0.5)) end):chance(0, \"mute\")"
    );
    let song = host().eval(&source, "song.lua", &dir).unwrap();
    let bars = &song.tracks[0].bars;
    assert_eq!(bars.len(), 16);
    assert!(bars.windows(2).any(|w| w[0] != w[1]), "varía por compás");
    let again = host().eval(&source, "song.lua", &dir).unwrap();
    assert_eq!(again.tracks[0].bars, *bars);
}

#[test]
fn shuffle_follows_its_seed_and_sounds_like_its_pattern() {
    let dir = fixture("shuffle");
    let p = "\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\"";
    let fixed = value(&dir, &format!("shuffle({p}, 3)"));
    assert_ne!(fixed, "1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .");
    let mut a: Vec<&str> = fixed.split_whitespace().collect();
    let mut b: Vec<&str> = "1 . 2 . 3 . 4 . 5 6 2 . 7 . 8 ."
        .split_whitespace()
        .collect();
    a.sort();
    b.sort();
    assert_eq!(a, b, "los mismos pasos");
    let a = render(
        &dir,
        &format!("track(\"d\", amen):steps(shuffle({p}, 3))"),
        1,
    );
    let b = render(&dir, &format!("track(\"d\", amen):steps(\"{fixed}\")"), 1);
    same(&a, &b);
    // Sin semilla, reproducible con la de la carga.
    assert_eq!(
        value(&dir, &format!("shuffle({p})")),
        value(&dir, &format!("shuffle({p})"))
    );
}

#[test]
fn scale_melodies_sound_like_the_pitched_pattern() {
    let dir = fixture("scale");
    assert_eq!(
        value(
            &dir,
            "scale(\"minor\", \"C\"):melody(\"1 3 5 3  . . . .  . . . .  . . . .\")"
        ),
        "1^0 1^3 1^7 1^3  . . . .  . . . .  . . . ."
    );
    assert_eq!(
        value(&dir, "tostring(scale(\"major\", \"D\"):pitch(3))"),
        "6"
    );
    let a = render(
        &dir,
        "track(\"m\", amen):steps(scale(\"dorian\", \"Eb\"):melody(\"1 . 3 .  5 . 7@0.5 .  8 . -1 .  . . 2' .\", \"3\"))",
        1,
    );
    // dórica sobre mi bemol (3): 3 6 10 15 | 15 2 | 5
    let b = render(
        &dir,
        "track(\"m\", amen):steps(\"3^3 . 3^6 .  3^10 . 3^13@0.5 .  3^15 . 3^1 .  . . 3^5' .\")",
        1,
    );
    same(&a, &b);
    assert_eq!(
        error(&dir, "scale(\"menor\")"),
        "song.lua:3: scale(\"menor\"): no existe (major, minor, harmonic_minor, melodic_minor, \
         dorian, phrygian, lydian, mixolydian, locrian, major_pentatonic, minor_pentatonic, \
         blues, chromatic)"
    );
    assert!(error(&dir, "scale(\"minor\", \"H\")").contains("tónica no válida"));
    assert!(error(&dir, "scale(\"minor\"):pitch(0)").contains("pitch(0): el grado 0 no existe"));
}

/// Frames donde arranca un golpe en el canal izquierdo.
fn onsets(audio: &[f32]) -> Vec<usize> {
    let left: Vec<f32> = audio.iter().step_by(2).copied().collect();
    (1..left.len())
        .filter(|&i| left[i - 1] == 0.0 && left[i] != 0.0)
        .map(|i| i - 1)
        .collect()
}

/// A 120 BPM, 6000 frames por paso.
const STEP: usize = 6000;

#[test]
fn swing_delays_offbeats_by_exact_frames() {
    let dir = fixture("swing");
    let steps = "track(\"d\", amen):steps(\"1 2 1 2  1 2 1 2  1 2 1 2  1 2 1 2\")";
    for (percent, delay) in [(50, 0), (58, 960), (66, 1920), (75, 3000)] {
        let got = onsets(&render(&dir, &format!("{steps}:swing({percent})"), 1));
        let expected: Vec<usize> = (0..16)
            .map(|k| k * STEP + if k % 2 == 1 { delay } else { 0 })
            .collect();
        assert_eq!(got, expected, "swing({percent})");
    }
    // 50 es lo mismo que no ponerlo.
    same(
        &render(&dir, &format!("{steps}:swing(50)"), 2),
        &render(&dir, steps, 2),
    );
    // Se aplica también a los compases de every.
    let got = onsets(&render(
        &dir,
        "track(\"d\", amen):steps(\"1 . . .  . . . .  . . . .  . . . .\")\
         :every(2, function(c) c:steps(\". 2 . .  . . . .  . . . .  . . . .\") end):swing(66)",
        2,
    ));
    assert_eq!(got, vec![STEP + 1920, 16 * STEP]);
    assert_eq!(
        error(&dir, "track(\"d\", amen):swing(80)"),
        "song.lua:3: swing(80): va de 50 (recto) a 75"
    );
}
