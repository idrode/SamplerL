//! El linter en el editor: se lanza tras la pausa al escribir (no en cada
//! tecla), solo si el texto cambió, sin cargar nada; marca en el margen,
//! subraya, dice el mensaje en la línea ⓘ y cuenta en la cabecera; Alt-J y
//! Alt-K (y `]d` `[d` en vim) recorren los diagnósticos, y Ctrl-G va al
//! error de la carga o, sin él, al primer error del texto.

use std::time::{Duration, Instant};

use super::*;
use crate::config::Config;

/// Dibuja (la vista toma la huella del texto) y lanza el análisis como si
/// hubiera pasado la pausa; espera el resultado.
fn lint_now(app: &mut App) {
    draw(app, 120, 40);
    app.poll_lint_at(Instant::now() + Duration::from_secs(1));
    app.wait_lint();
}

fn alt(c: char) -> Input {
    Input {
        key: Key::Char(c),
        alt: true,
        ..Default::default()
    }
}

const BROKEN: &str = "bpm(120)\n\
local amen = sample(\"break.wav\"):slice(2)\n\
track(\"drums\", amen):steps(\"1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .\")\n\
track(\"drums\"):fx(\"filter\", { cutof = 800 })\n\
trak(\"kick\")\n";

#[test]
fn the_linter_runs_after_a_pause_not_on_every_key() {
    let (_, mut app, _engine) = open("lint_debounce", Some(SONG));
    draw(&mut app, 120, 40);
    app.poll_lint_at(Instant::now() + Duration::from_secs(1));
    app.wait_lint();
    let first = app.lint.launched;
    assert_eq!(first, 1, "el texto al abrir se mira una vez");
    // Diez teclas seguidas, con su cuadro cada una: ningún análisis.
    app.editor.move_cursor(ratatui_textarea::CursorMove::Bottom);
    for c in "trak(\"a\")\n".chars() {
        type_text(&mut app, &c.to_string());
        draw(&mut app, 120, 40);
        app.poll_lint_at(Instant::now());
    }
    assert_eq!(app.lint.launched, first, "en cada tecla, nada");
    // Pasada la pausa (300 ms), uno.
    app.poll_lint_at(Instant::now() + Duration::from_millis(350));
    app.wait_lint();
    assert_eq!(app.lint.launched, first + 1);
    assert_eq!(app.view().problems.len(), 1, "{:?}", app.view().problems);
    // Sin cambios, aunque pase el tiempo y se dibuje: ninguno más.
    for _ in 0..5 {
        draw(&mut app, 120, 40);
        app.poll_lint_at(Instant::now() + Duration::from_secs(5));
    }
    assert_eq!(app.lint.launched, first + 1);
    // Mover el cursor no es cambiar el texto.
    app.handle(special(Key::Up));
    draw(&mut app, 120, 40);
    app.poll_lint_at(Instant::now() + Duration::from_secs(5));
    assert_eq!(app.lint.launched, first + 1);
}

#[test]
fn the_pause_is_configurable_and_the_linter_can_be_off() {
    let (_, mut app, _engine) = open("lint_config", Some(SONG));
    app.set_config(&Config::parse(
        "[editor]\nlinter_ms = 2000\n",
        true,
        "c.toml",
    ));
    draw(&mut app, 120, 40);
    app.poll_lint_at(Instant::now() + Duration::from_millis(1000));
    assert_eq!(app.lint.launched, 0);
    app.poll_lint_at(Instant::now() + Duration::from_millis(2100));
    assert_eq!(app.lint.launched, 1);
    app.wait_lint();
    let (_, mut app, _engine) = open("lint_off", Some(BROKEN));
    app.set_config(&Config::parse("[editor]\nlinter = false\n", true, "c.toml"));
    lint_now(&mut app);
    assert_eq!(app.lint.launched, 0);
    assert!(app.view().problems.is_empty());
}

#[test]
fn problems_show_in_the_margin_the_info_line_and_the_header() {
    let (_, mut app, _engine) = open("lint_show", Some(BROKEN));
    // El script no carga (al abrirlo): ese error es el de la carga.
    let loaded = app.view().error.map(|e| e.message);
    assert!(loaded.is_some());
    lint_now(&mut app);
    let problems = app.view().problems;
    assert_eq!(problems.len(), 2, "{problems:#?}");
    assert!(problems[0].message.contains("¿quisiste decir «cutoff»?"));
    assert!(problems[1].message.contains("¿quisiste decir «track»?"));
    let buffer = draw(&mut app, 120, 40);
    let screen = text(&buffer);
    // La cabecera: dos errores.
    assert!(screen.contains("●2"), "{screen}");
    // El margen: la marca en la primera columna de su línea.
    let area = app.editor_area.unwrap();
    let (_, y) = find(&buffer, "trak(").unwrap();
    assert_eq!(buffer[(area.x, y)].symbol(), "●");
    // Subrayado, en el color del error.
    let (x, _) = find(&buffer, "trak(").unwrap();
    assert!(buffer[(x, y)].modifier.contains(Modifier::UNDERLINED));
    assert_eq!(buffer[(x, y)].fg, crate::ui::error_color());
    // El mensaje entero en la línea ⓘ, con el cursor en su línea.
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(4, 0));
    let screen = text(&draw(&mut app, 120, 40));
    assert!(
        screen.contains("● error: «trak» no es una función de la API"),
        "{screen}"
    );
    // Nada de esto carga el script ni toca lo que suena.
    assert_eq!(app.view().error.map(|e| e.message), loaded);
    // En ASCII, letras.
    crate::theme::set_ascii(true);
    let buffer = draw(&mut app, 120, 40);
    crate::theme::set_ascii(false);
    assert_eq!(buffer[(area.x, y)].symbol(), "E");
    assert!(text(&buffer).contains("E2"));
}

#[test]
fn a_syntax_error_is_found_without_running_anything() {
    // Un bucle infinito arriba del todo: el linter no lo ejecuta.
    let song = "while true do end\nbpm(120\n";
    let (_, mut app, _engine) = open("lint_syntax", Some(song));
    let start = Instant::now();
    lint_now(&mut app);
    assert!(start.elapsed() < Duration::from_secs(5));
    let problems = app.view().problems;
    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert_eq!(problems[0].row, 1);
    assert!(
        problems[0].message.starts_with("sintaxis: falta «)»"),
        "{}",
        problems[0].message
    );
}

#[test]
fn alt_n_alt_p_ctrl_g_and_vim_walk_the_problems() {
    let (_, mut app, _engine) = open("lint_walk", Some(BROKEN));
    lint_now(&mut app);
    assert_eq!(app.editor.cursor(), (0, 0));
    app.handle(alt('j'));
    assert_eq!(app.editor.cursor().0, 3);
    assert!(
        app.view().message.starts_with("error 1 de 2, línea 4"),
        "{}",
        app.view().message
    );
    app.handle(alt('j'));
    assert_eq!(app.editor.cursor(), (4, 0));
    // Da la vuelta.
    app.handle(alt('j'));
    assert_eq!(app.editor.cursor().0, 3);
    app.handle(alt('k'));
    assert_eq!(app.editor.cursor(), (4, 0));
    // Ctrl-G sin error de carga: al primer error tras el cursor.
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(0, 0));
    app.handle(ctrl('g'));
    assert_eq!(app.editor.cursor().0, 3);
    // En vim, ]d y [d.
    app.set_config(&Config::parse("[editor]\nmodo = \"vim\"\n", true, "c.toml"));
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(0, 0));
    app.handle(key(']'));
    app.handle(key('d'));
    assert_eq!(app.editor.cursor().0, 3);
    app.handle(key('['));
    app.handle(key('d'));
    assert_eq!(app.editor.cursor(), (4, 0));
    assert_eq!(app.editor.lines()[0], "bpm(120)", "vim no escribe nada");
}

#[test]
fn notation_warnings_join_the_walk_and_the_count() {
    let song = "bpm(120)\nlocal amen = sample(\"break.wav\"):slice(2)\n\
                track(\"drums\", amen):steps(\"1 . 5^ .  1 . 2 .  1 . 2 .  1 . 2 .\")\n";
    let (_, mut app, _engine) = open("lint_notation", Some(song));
    lint_now(&mut app);
    let problems = app.view().problems;
    assert_eq!(problems.len(), 1, "{problems:#?}");
    assert_eq!(problems[0].severity, script::lint::Severity::Warning);
    assert!(text(&draw(&mut app, 120, 40)).contains("▲1"));
    app.handle(alt('j'));
    assert_eq!(app.editor.cursor().0, 2);
}
