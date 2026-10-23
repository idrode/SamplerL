//! El tracker editable (fase N, parte 4): un efecto que se pone, cambia o
//! quita en un paso desde el tracker se escribe en el token de ese paso
//! del código, en un paso de deshacer y sin tocar nada más del texto; leer
//! ese texto y volver a cargarlo da lo mismo.

use script::{ScriptHost, Song};

use super::split::{editor_window, main_window, pump, pump_until};
use super::*;
use crate::palette::Action;

fn enter() -> Input {
    special(Key::Enter)
}

/// La canción de un texto, cargada aparte (como la cargaría otra vez el
/// editor).
fn song_of(path: &Path, text: &str) -> Song {
    let dir = path.parent().unwrap();
    let mut host = ScriptHost::new(RATE);
    host.set_seed(Some(1));
    host.eval(text, "song.lua", dir).unwrap()
}

/// Abre el cursor del tracker, baja a la fila `row` de la primera pista y
/// escribe `fx` en la línea de órdenes.
fn edit(app: &mut App, row: usize, fx: &str) {
    app.run_action(Action::EditTracker);
    for _ in 0..row {
        app.handle(special(Key::Down));
    }
    app.handle(enter());
    let prompt = app.view().prompt.clone().expect("la línea de órdenes");
    assert!(prompt.starts_with("efectos"), "{prompt}");
    // Lo que hubiera, fuera.
    for _ in 0..prompt.len() - "efectos ".len() {
        app.handle(special(Key::Backspace));
    }
    type_text(app, fx);
    app.handle(enter());
}

#[test]
fn an_effect_written_in_the_tracker_lands_in_its_token_and_reloads_the_same() {
    let (path, mut app, _engine) = open("tracker_edit_roundtrip", Some(SONG));
    edit(&mut app, 2, "lp=800,q=4 crush=6");
    let text = app.text_for_test();
    let expected = SONG.replace(
        "steps(\"1 . 2 .  1 . 2 .",
        "steps(\"1 . 2{lp=800,q=4}{crush=6} .  1 . 2 .",
    );
    // Solo cambian los caracteres de ese token.
    assert_eq!(text, expected);
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    assert!(
        app.view().message.contains("entra en el compás siguiente"),
        "{}",
        app.view().message
    );
    // Cargado: el modelo es el del texto, y el tracker lo enseña.
    let view = app.view();
    let drums = &view.tracks[0];
    assert_eq!(drums.cell_text(&drums.steps[2]), "02{lp,crush}");
    // Ida y vuelta: leer el texto y cargarlo otra vez da lo mismo, también
    // en el audio, que el texto escrito a mano.
    let again = song_of(&path, &text);
    let by_hand = song_of(&path, &expected);
    assert_eq!(app.tracks()[0].steps, again.tracks[0].steps);
    assert_eq!(again.tracks[0].steps, by_hand.tracks[0].steps);
    assert_eq!(
        again.render(RATE, 1).unwrap(),
        by_hand.render(RATE, 1).unwrap()
    );

    // Cambiar: Enter trae lo que tiene; se sustituye en su sitio.
    app.handle(enter());
    assert_eq!(
        app.view().prompt.as_deref(),
        Some("efectos lp=800,q=4 crush=6")
    );
    app.handle(esc());
    edit(&mut app, 0, "");
    app.run_action(Action::EditTracker);
    app.handle(special(Key::Down));
    app.handle(special(Key::Down));
    app.handle(enter());
    for _ in 0.."lp=800,q=4 crush=6".len() {
        app.handle(special(Key::Backspace));
    }
    type_text(&mut app, "{hp=300}");
    app.handle(enter());
    assert_eq!(
        app.text_for_test(),
        SONG.replace(
            "steps(\"1 . 2 .  1 . 2 .",
            "steps(\"1 . 2{hp=300} .  1 . 2 ."
        )
    );

    // Quitar con Supr, y deshacer cada cambio de una vez.
    app.handle(special(Key::Delete));
    assert_eq!(app.text_for_test(), SONG);
    app.handle(esc());
    app.handle(ctrl('z'));
    assert_eq!(
        app.text_for_test(),
        SONG.replace(
            "steps(\"1 . 2 .  1 . 2 .",
            "steps(\"1 . 2{hp=300} .  1 . 2 ."
        )
    );
    app.handle(ctrl('z'));
    assert_eq!(app.text_for_test(), expected);
    app.handle(ctrl('z'));
    assert_eq!(app.text_for_test(), SONG);
}

#[test]
fn the_tracker_cursor_moves_and_shows_in_the_view() {
    let (_, mut app, _engine) = open("tracker_edit_cursor", Some(SONG));
    app.run_action(Action::EditTracker);
    assert_eq!(app.view().tracker_cursor, Some((0, 0)));
    app.handle(special(Key::Right));
    app.handle(special(Key::Down));
    assert_eq!(app.view().tracker_cursor, Some((1, 1)));
    // La pista seleccionada sigue al cursor.
    assert_eq!(app.view().selected, Some(1));
    let buffer = draw(&mut app, 140, 50);
    // La celda de kick (la segunda «---» de la fila 1), invertida.
    let (x, y) = find(&buffer, "01 ---").unwrap();
    let kick = (x + 6..buffer.area.width)
        .find(|&cx| buffer[(cx, y)].symbol() == "-")
        .unwrap();
    let cell = (kick, y);
    assert!(
        buffer[cell].modifier.contains(Modifier::REVERSED),
        "{:?}",
        buffer[cell]
    );
    assert!(
        app.view()
            .message
            .starts_with("kick · paso 2: el paso 2 es un silencio"),
        "{}",
        app.view().message
    );
    app.handle(esc());
    assert_eq!(app.view().tracker_cursor, None);
}

#[test]
fn what_cannot_be_edited_says_why_and_leaves_the_text_alone() {
    let song = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps(euclid(5, 16, "2"))
track("hats", amen):steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
local i = sample("break.wav"):instrument()
track("bass", i):steps("1 . . .  . . . .  . . . .  . . . .")
"#;
    let (_, mut app, _engine) = open("tracker_edit_refused", Some(song));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    // Un paso generado: no está escrito tal cual.
    app.run_action(Action::EditTracker);
    app.handle(enter());
    assert!(app.view().prompt.is_none());
    assert!(
        app.view().message.contains("no está escrito tal cual"),
        "{}",
        app.view().message
    );
    // Un silencio.
    app.handle(special(Key::Right));
    app.handle(special(Key::Down));
    app.handle(enter());
    assert!(
        app.view().message.contains("es un silencio"),
        "{}",
        app.view().message
    );
    // Un instrumento.
    app.handle(special(Key::Right));
    app.handle(special(Key::Up));
    app.handle(enter());
    assert!(
        app.view().message.contains("un instrumento"),
        "{}",
        app.view().message
    );
    // Una errata en los efectos: no toca el texto.
    app.handle(special(Key::Left));
    app.handle(enter());
    type_text(&mut app, "zap=3");
    app.handle(enter());
    assert!(
        app.view().message.contains("{zap}: el efecto no existe"),
        "{}",
        app.view().message
    );
    assert_eq!(app.text_for_test(), song);
    // Con el texto cambiado sin cargar, tampoco.
    app.handle(esc());
    app.editor.move_cursor(ratatui_textarea::CursorMove::Bottom);
    type_text(&mut app, "-- nota");
    app.run_action(Action::EditTracker);
    app.handle(special(Key::Right));
    app.handle(enter());
    assert!(
        app.view().message.contains("no es el que suena"),
        "{}",
        app.view().message
    );
}

#[test]
fn an_edit_in_the_main_window_reaches_the_editor_window_as_one_undo() {
    let path = fixture("tracker_edit_split", Some(SONG));
    let (mut main, _engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    pump_until(&mut main, &mut editor, |m, _| m.view().tracks.len() == 2);
    edit(&mut main, 2, "crush=4");
    let expected = SONG.replace(
        "steps(\"1 . 2 .  1 . 2 .",
        "steps(\"1 . 2{crush=4} .  1 . 2 .",
    );
    assert_eq!(main.text_for_test(), expected);
    pump_until(&mut main, &mut editor, |_, e| e.text_for_test() == expected);
    editor.handle(ctrl('z'));
    assert_eq!(editor.text_for_test(), SONG);
}

#[test]
#[ignore]
fn pantalla_tracker_edit() {
    let (_, mut app, _engine) = open("tracker_edit_screen", Some(SONG));
    app.run_action(Action::EditTracker);
    app.handle(special(Key::Right));
    app.handle(special(Key::Down));
    println!("{}", text(&draw(&mut app, 140, 50)));
}
