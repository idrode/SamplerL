//! Automatización en la interfaz: `ramp`/`lfo` en el autocompletado de la
//! tabla de `fx`, el panel de efectos marca lo que se mueve y el tracker
//! enseña los cortes por paso.

use super::*;

#[test]
fn ramp_and_lfo_complete_after_an_automatable_key() {
    let (mut app, _engine) = open_at_end("auto_complete");
    type_text(&mut app, "track(\"drums\"):fx(\"lowpass\", { cutoff = ");
    app.handle(f(4));
    assert_eq!(completion_items(&app), ["ramp", "lfo"]);
    app.handle(special(Key::Tab));
    assert!(
        current_line(&app).ends_with("cutoff = ramp(200, 4000, \"4 compases\")"),
        "{}",
        current_line(&app)
    );
    // También tras las claves nuevas (`bits`) y en las tablas de `bus()`.
    replace_all(&mut app, "track(\"drums\"):fx(\"bitcrush\", { bits = ");
    app.handle(f(4));
    assert_eq!(completion_items(&app), ["ramp", "lfo"]);
    replace_all(&mut app, "bus(\"reverb\", { size = ");
    app.handle(f(4));
    assert_eq!(completion_items(&app), ["ramp", "lfo"]);
    // Una clave que no se mueve: nada.
    replace_all(&mut app, "bus(\"delay\", { time = ");
    app.handle(f(4));
    assert!(completion_items(&app).is_empty());
}

#[test]
fn the_panels_show_what_moves() {
    let song = format!(
        "{SONG}track(\"drums\")\n  \
         :steps(\"1~300 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .\")\n  \
         :fx(\"lowpass\", {{ cutoff = ramp(300, 3000, \"2 compases\") }})\n"
    );
    let (_, mut app, _engine) = open("auto_panels", Some(&song));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let screen = text(&draw(&mut app, 140, 45));
    assert!(screen.contains("› LP 300 Hz q0.71 ∿cutoff"), "{screen}");
    assert!(screen.contains("00 01~300"), "{screen}");
}

#[test]
fn the_fx_panel_shows_pan_and_moving_mix() {
    let song = format!(
        "{SONG}track(\"drums\"):steps(\"1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .\")\n  \
         :gain(ramp(0.5, 1, \"2 compases\")):pan(-0.3):send(\"reverb\", lfo(1, 0, 0.5))\n"
    );
    let (_, mut app, _engine) = open("auto_mix", Some(&song));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let screen = text(&draw(&mut app, 140, 45));
    assert!(screen.contains("vol 0.50∿"), "{screen}");
    assert!(screen.contains("pan -0.30"), "{screen}");
    assert!(screen.contains("→ r∿"), "{screen}");
}
