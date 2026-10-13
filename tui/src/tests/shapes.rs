//! Forma de los cortes, gate y choke en la interfaz.

use super::*;

#[test]
fn the_panel_marks_gate_and_choke_and_set_completes_its_keys() {
    let song = format!("{SONG}track(\"drums\"):mode(\"gate\"):choke(4)\n");
    let (_, mut app, _engine) = open("shapes_panel", Some(&song));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("drums · gate · choke 4"), "{screen}");

    // Dentro de la tabla de set(), sus claves y su ayuda.
    app.editor.move_cursor(ratatui_textarea::CursorMove::Bottom);
    app.editor.move_cursor(ratatui_textarea::CursorMove::End);
    type_text(&mut app, "\namen:set(2, { ");
    app.handle(f(4));
    assert_eq!(
        completion_items(&app),
        ["attack", "release", "pan", "length", "play"]
    );
    app.handle(esc());
    // Los valores de play.
    type_text(&mut app, "play = \"");
    assert_eq!(completion_items(&app), ["normal", "loop", "pingpong"]);
    app.handle(esc());
    type_text(&mut app, "\" ");
    assert_eq!(
        app.view().help.unwrap().signature,
        "cortes:set([i,] { ... })"
    );
}

#[test]
fn pads_play_with_the_slice_shape() {
    // El corte 1 con 20 ms de ataque: el pad sale con él.
    let song = "bpm(120)\nlocal amen = sample(\"break.wav\"):slice(2)\n\
                amen:set(1, { attack = 10 })\n\
                track(\"drums\", amen):steps(\". . . .  . . . .  . . . .  . . . .\")\n";
    let (_, mut app, mut engine) = open("shapes_pad", Some(song));
    app.controller_status_for_test();
    offline::render(&mut engine, 100, 2);
    app.handle(esc());
    app.handle(key('1'));
    let audio = offline::render(&mut engine, 1000, 2);
    // A los 5 ms (240 frames), la mitad del nivel (0,5 · 0,5).
    assert!((audio[2 * 240] - 0.25).abs() < 0.01, "{}", audio[2 * 240]);
}
