//! Resaltado en vivo del código al sonar (fase M): los tokens de la
//! mini-notación que se oyen, sobre el texto del editor, con el motor
//! movido offline.

use super::*;
use crate::theme;

const LIVE: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
"#;

/// Celda de la pantalla del carácter `col` de la línea que empieza por
/// `line`.
fn cell(buffer: &Buffer, line: &str, col: usize) -> (u16, u16) {
    let (x, y) = find(buffer, line).unwrap();
    (x + col as u16, y)
}

/// Columna (en caracteres) de la `n`-ésima aparición de `token` entre las
/// comillas de la línea.
fn column(line: &str, token: &str, n: usize) -> usize {
    let open = line.find("(\"").unwrap() + 2;
    let open = line[open..].find('"').map_or(open, |q| open + q + 1);
    let byte = line[open..].match_indices(token).nth(n).unwrap().0 + open;
    line[..byte].chars().count()
}

fn bg(buffer: &Buffer, at: (u16, u16)) -> Color {
    buffer[at].bg
}

const DRUMS: &str = "track(\"drums\", amen):steps(";
const KICK: &str = "track(\"kick\", amen:slice_at(1)):steps(";

fn drums_line() -> &'static str {
    LIVE.lines().nth(2).unwrap()
}

fn kick_line() -> &'static str {
    LIVE.lines().nth(3).unwrap()
}

#[test]
fn the_tokens_that_sound_light_up_in_the_color_of_their_track() {
    let (_, mut app, mut engine) = open("highlight_live", Some(LIVE));
    let theme = theme::get();
    // Paso 0: el «1» de drums (la seleccionada, entera y en negrita) y la
    // «x» de kick (apagada).
    offline::render(&mut engine, 100, 2);
    app.tick();
    let buffer = draw(&mut app, 120, 40);
    let one = cell(&buffer, DRUMS, column(drums_line(), "1", 0));
    assert_eq!(bg(&buffer, one), theme.highlight[0]);
    assert_eq!(buffer[one].fg, Color::Black);
    assert!(buffer[one].modifier.contains(Modifier::BOLD));
    let x = cell(&buffer, KICK, column(kick_line(), "x", 0));
    assert_eq!(bg(&buffer, x), theme.highlight_dim[1]);
    // Solo el token: lo de al lado, sin fondo.
    let after = (one.0 + 1, one.1);
    assert_ne!(bg(&buffer, after), theme.highlight[0]);
    assert_eq!(app.view().highlights.len(), 2);

    // Paso 1: silencios.
    offline::render(&mut engine, STEP, 2);
    app.tick();
    assert!(app.view().highlights.is_empty());

    // Paso 6: el segundo «2» de drums; kick, nada.
    offline::render(&mut engine, 5 * STEP, 2);
    app.tick();
    let view = app.view();
    assert_eq!(view.highlights.len(), 1, "{:?}", view.highlights);
    let buffer = draw(&mut app, 120, 40);
    let two = cell(&buffer, DRUMS, column(drums_line(), "2", 1));
    assert_eq!(bg(&buffer, two), theme.highlight[0]);

    // Parado, nada.
    app.handle(ctrl('t'));
    offline::render(&mut engine, 100, 2);
    app.tick();
    assert!(app.view().highlights.is_empty());
}

#[test]
fn the_heard_step_lags_behind_by_the_output_delay() {
    let (_, mut app, mut engine) = open("highlight_delay", Some(LIVE));
    // El motor va por el paso 4 (golpe en las dos); con un paso de
    // retardo, se oye el 3 (silencio).
    engine.set_output_delay(STEP as u64);
    offline::render(&mut engine, 4 * STEP + 100, 2);
    app.tick();
    assert!(app.view().highlights.is_empty());
    // Medio paso después del 5, se oye el 4.
    offline::render(&mut engine, STEP, 2);
    app.tick();
    let rows: Vec<usize> = app.view().highlights.iter().map(|h| h.row).collect();
    assert_eq!(rows, [3, 2]);
}

#[test]
fn edited_text_and_patterns_of_an_older_load_are_not_highlighted() {
    let (_, mut app, mut engine) = open("highlight_edits", Some(LIVE));
    offline::render(&mut engine, 100, 2);
    app.tick();
    assert_eq!(app.view().highlights.len(), 2);
    // Una letra escrita: el texto ya no es el cargado.
    app.handle(key('-'));
    assert!(app.view().highlights.is_empty());
    // Al deshacerla, vuelve a serlo.
    app.handle(ctrl('z'));
    assert_eq!(app.view().highlights.len(), 2);

    // Otra carga (el mismo texto con kick movida): hasta el compás
    // siguiente suena el patrón viejo, que no es de esta carga.
    let moved = LIVE.replace("\"x . . .  x", "\". x . .  x");
    replace_all(&mut app, &moved);
    app.load();
    offline::render(&mut engine, 8 * STEP, 2);
    app.tick();
    assert!(app.view().highlights.is_empty());
    // Compás 1, paso 1: la «x» que se movió.
    offline::render(&mut engine, 9 * STEP, 2);
    app.tick();
    let view = app.view();
    assert_eq!(view.highlights.len(), 1, "{:?}", view.highlights);
    let kick = moved.lines().nth(3).unwrap();
    let h = &view.highlights[0];
    assert_eq!((h.row, h.cols.start), (3, column(kick, "x", 0)));
}

#[test]
fn the_selected_track_wins_where_two_tracks_share_a_token() {
    let shared = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
local p = "1 . . .  1 . . .  1 . . .  1 . . ."
track("a", amen):steps(p)
track("b", amen):steps(p)
"#;
    let (_, mut app, mut engine) = open("highlight_shared", Some(shared));
    let theme = theme::get();
    offline::render(&mut engine, 100, 2);
    app.tick();
    let one = |app: &mut App| {
        let buffer = draw(app, 120, 40);
        let line = shared.lines().nth(2).unwrap();
        let col = line.find('"').unwrap() + 1;
        bg(&buffer, cell(&buffer, "local p = ", col))
    };
    // «a» seleccionada: se dibuja la última y manda.
    let view = app.view();
    assert_eq!(view.highlights.len(), 2);
    assert!(view.highlights.last().unwrap().selected);
    assert_eq!(one(&mut app), theme.highlight[0]);
    // Con «b» seleccionada, su color.
    app.handle(ctrl('o'));
    assert_eq!(one(&mut app), theme.highlight[1]);
}

#[test]
fn several_tracks_on_a_line_keep_their_own_colors() {
    let line = r#"track("a", amen):length(4):steps("1 . . .") track("b", amen):length(4):steps(". . . .  ") track("c", amen):length(4):steps("2 . 2 .")"#;
    let song = format!("bpm(120)\nlocal amen = sample(\"break.wav\"):slice(2)\n{line}\n");
    let (_, mut app, mut engine) = open("highlight_one_line", Some(&song));
    offline::render(&mut engine, 100, 2);
    app.tick();
    let view = app.view();
    // Dos tokens en la misma fila, cada uno con el color de su pista; la
    // seleccionada («a»), la última.
    let lit: Vec<(usize, usize, bool)> = view
        .highlights
        .iter()
        .map(|h| (h.row, h.color, h.selected))
        .collect();
    assert_eq!(lit, [(2, 2, false), (2, 0, true)]);
}
