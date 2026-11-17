//! La rueda de verdad: `tui --edit` en un pseudoterminal, con las
//! secuencias que manda una terminal (SGR 1006: botones 64-67 y Shift con
//! la rueda), sin tarjeta de sonido. Se lee la pantalla con un intérprete
//! mínimo de lo que escribe ratatui (mover el cursor y escribir) y se
//! comprueba que la vista se desplaza y que el cursor no se movió (lo que se
//! escribe después va a la línea 1).

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use common::{COLS, ROWS, Term};

#[test]
fn real_wheel_sequences_scroll_the_editor_without_moving_the_cursor() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("pty_wheel");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("config")).unwrap();
    let mut song = String::from("bpm(120)\n");
    for i in 1..=120 {
        if i == 70 {
            song.push_str(&format!("-- ancha {} FIN_DE_LA_ANCHA\n", "=".repeat(200)));
        } else {
            song.push_str(&format!("-- marca_{i:03}\n"));
        }
    }
    std::fs::write(dir.join("song.lua"), &song).unwrap();
    let term = Term::spawn(&dir, "song.lua");
    term.wait_for("marca_001");
    // El editor maximizado (Alt-Z): el centro de la pantalla es suyo.
    term.send(b"\x1bz");
    std::thread::sleep(Duration::from_millis(400));
    let (x, y) = (COLS / 2, ROWS / 2);
    assert!(!term.text().contains("marca_060"));
    // Botón 65: la rueda hacia abajo, 20 pasos de 3 líneas.
    term.wheel(65, x, y, 20);
    term.wait_for("marca_062");
    assert!(!term.text().contains("marca_001"), "{}", term.text());
    // Botón 64: hacia arriba.
    term.wheel(64, x, y, 2);
    term.wait_for("marca_056");
    // Botón 67: la rueda hacia la derecha, hasta el final de la línea ancha.
    assert!(!term.text().contains("FIN_DE_LA_ANCHA"));
    term.wheel(67, x, y, 40);
    term.wait_for("FIN_DE_LA_ANCHA");
    // Shift y la rueda hacia arriba (64 + 4): hacia la izquierda.
    term.wheel(68, x, y, 60);
    term.wait_for("marca_06");
    // El cursor sigue en la línea 1: lo que se escribe va ahí, y la vista
    // vuelve a él.
    term.send(b"Q");
    term.wait_for("Qbpm(120)");
    // Ctrl-S guarda; Ctrl-Q sale.
    term.send(b"\x13");
    let start = Instant::now();
    while !std::fs::read_to_string(dir.join("song.lua"))
        .unwrap()
        .starts_with("Qbpm(120)")
    {
        assert!(start.elapsed() < Duration::from_secs(10), "no se guardó");
        std::thread::sleep(Duration::from_millis(50));
    }
    term.send(b"\x11");
}
