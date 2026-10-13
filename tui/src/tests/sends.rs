//! Envíos y buses en la interfaz: el panel de efectos, el autocompletado y
//! la ayuda de `bus(...)` y `send(...)`.

use super::*;

#[test]
fn the_fx_panel_shows_the_sends() {
    let song = format!(
        "{SONG}track(\"drums\"):send(\"delay\", 0.3):send(\"reverb\", 0.15)\n\
         bus(\"delay\", {{ time = \"1/16\" }})\n"
    );
    let (_, mut app, _engine) = open("sends_panel", Some(&song));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    assert_eq!(app.view().tracks[0].sends, [0.3, 0.15]);
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("→ d0.30 r0.15"), "{screen}");
}

#[test]
fn bus_and_send_complete_their_names_and_parameters() {
    let (mut app, _engine) = open_at_end("sends_complete");
    // La plantilla de bus(): el nombre marcado y su tabla.
    type_text(&mut app, "bus");
    app.handle(special(Key::Tab));
    assert_eq!(
        current_line(&app),
        "bus(\"delay\", { time = \"1/8\", feedback = 0.4 })"
    );
    assert_eq!(selection_text(&app), "delay");
    // Sobre el nombre, los buses; aceptar otro rehace la tabla.
    type_text(&mut app, "r");
    assert_eq!(completion_items(&app), ["reverb"]);
    app.handle(special(Key::Tab));
    assert_eq!(
        current_line(&app),
        "bus(\"reverb\", { size = 0.7, damp = 0.5 })"
    );
    // Vuelta al delay para mirar los valores de time.
    replace_all(
        &mut app,
        "bus(\"delay\", { time = \"1/8\", feedback = 0.4 })",
    );
    // Dentro de time = "...", sus valores.
    app.editor.cancel_selection();
    let line = current_line(&app);
    let col = line.find("1/8").unwrap() as u16;
    let row = app.editor.cursor().0 as u16;
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(row, col));
    app.editor.delete_str(3);
    app.handle(f(4));
    assert_eq!(completion_items(&app), ["1/4", "3/16", "1/8", "1/16"]);
    app.handle(esc());

    // La ayuda en la tabla del bus.
    app.editor.move_cursor(ratatui_textarea::CursorMove::End);
    app.editor.move_cursor(ratatui_textarea::CursorMove::Back);
    app.editor.move_cursor(ratatui_textarea::CursorMove::Back);
    let help = app.view().help.unwrap();
    assert_eq!(help.signature, "bus(\"delay\", { ... })");

    // send("..."): los dos buses.
    type_text(&mut app, "");
    app.editor.move_cursor(ratatui_textarea::CursorMove::Bottom);
    app.editor.move_cursor(ratatui_textarea::CursorMove::End);
    type_text(&mut app, "\ntrack(\"drums\"):send(\"r");
    assert_eq!(completion_items(&app), ["reverb"]);
}
