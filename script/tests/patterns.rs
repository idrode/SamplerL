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
