//! Ajustar números con secuencias de ratón de verdad: `tui --edit` en un
//! pseudoterminal, con lo que manda una terminal en SGR 1006 (la rueda
//! son los botones 64 y 65; Ctrl suma 16 y Alt 8; pulsar y arrastrar, 0 y
//! 32), sin tarjeta de sonido. Es lo que mandan kitty y tmux (con `mouse
//! on`) cuando les llega Ctrl y la rueda: si una terminal se lo queda (el
//! zoom de la letra), aquí no se puede ver; está en la guía.

mod common;

use std::path::Path;
use std::time::Duration;

use common::Term;

/// Ctrl (16) y Alt (8) sobre los botones.
const CTRL: u8 = 16;
const ALT: u8 = 8;
const WHEEL_UP: u8 = 64;
const WHEEL_DOWN: u8 = 65;

fn setup(name: &str, config: Option<&str>) -> std::path::PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("config/sampler")).unwrap();
    if let Some(config) = config {
        std::fs::write(dir.join("config/sampler/config.toml"), config).unwrap();
    }
    let song = "bpm(120)\n-- números: N1 17 N2 0.25\nprint(17, 0.25)\n";
    std::fs::write(dir.join("song.lua"), song).unwrap();
    dir
}

/// Abre y maximiza el editor (Alt-Z).
fn open(dir: &Path) -> Term {
    let term = Term::spawn(dir, "song.lua");
    term.wait_for("print(17, 0.25)");
    term.send(b"\x1bz");
    std::thread::sleep(Duration::from_millis(400));
    term
}

fn step(term: &Term, button: u8, at: (u16, u16)) {
    term.mouse(button, at.0, at.1, true);
    std::thread::sleep(Duration::from_millis(40));
}

#[test]
fn ctrl_and_the_real_wheel_sequences_change_the_number_and_undo_at_once() {
    let dir = setup("pty_adjust_ctrl", None);
    let term = open(&dir);
    let at = term.find("bpm(120)").map(|(x, y)| (x + 4, y)).unwrap();
    // La rueda sola (64) no cambia el número.
    step(&term, WHEEL_UP, at);
    std::thread::sleep(Duration::from_millis(200));
    assert!(term.text().contains("bpm(120)"), "{}", term.text());
    // Ctrl y la rueda hacia arriba (80), tres pasos; hacia abajo (81), uno.
    for _ in 0..3 {
        step(&term, WHEEL_UP + CTRL, at);
    }
    term.wait_for("bpm(123)");
    step(&term, WHEEL_DOWN + CTRL, at);
    term.wait_for("bpm(122)");
    // Tras la pausa, el gesto acaba y se carga; Ctrl-Z lo deshace entero.
    term.wait_for("entra en el compás siguiente");
    term.send(b"\x1a");
    term.wait_for("bpm(120)");
    // Esc a mitad del gesto lo cancela.
    let n = term.find("17, 0.25)").map(|(x, y)| (x + 4, y)).unwrap();
    step(&term, WHEEL_UP + CTRL, n);
    term.wait_for("print(17, 0.26)");
    term.send(b"\x1b");
    term.wait_for("cancelado");
    assert!(term.text().contains("print(17, 0.25)"));
    // Ctrl, pulsar y arrastrar (16, 48) y soltar.
    let n = term.find("print(17").map(|(x, y)| (x + 6, y)).unwrap();
    term.mouse(CTRL, n.0, n.1, true);
    term.mouse(32 + CTRL, n.0 + 3, n.1, true);
    term.mouse(CTRL, n.0 + 3, n.1, false);
    term.wait_for("print(20, 0.25)");
    // Secuencias rotas o fuera de la pantalla: nada (ni se cae).
    term.send(b"\x1b[<80;9999;9999M\x1b[<80;;M\x1b[<abcM");
    std::thread::sleep(Duration::from_millis(300));
    term.send(b"\x11\x11");
}

#[test]
fn with_ajuste_numeros_alt_the_alt_wheel_changes_it() {
    let dir = setup(
        "pty_adjust_alt",
        Some("[editor]\najuste_numeros = \"alt\"\n"),
    );
    let term = open(&dir);
    let at = term.find("bpm(120)").map(|(x, y)| (x + 4, y)).unwrap();
    // Ctrl ya no (el zoom del piano roll, como siempre)...
    step(&term, WHEEL_UP + CTRL, at);
    std::thread::sleep(Duration::from_millis(200));
    assert!(term.text().contains("bpm(120)"), "{}", term.text());
    // ... y Alt (72) sí.
    step(&term, WHEEL_UP + ALT, at);
    term.wait_for("bpm(121)");
    term.send(b"\x11\x11");
}
