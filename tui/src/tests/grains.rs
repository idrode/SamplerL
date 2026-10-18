//! Granos en la interfaz: el panel de efectos los resume, el `%` sale en
//! el tracker y la tabla de `grains` se autocompleta y se explica.

use super::*;

const SONG_GRAINS: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("nube", amen):steps("1 _ _ _  _ _ _ _  2%-12 _ _ _  _ _ _ _"):grains({ size = 80, density = lfo(1, 5, 30) })
track("hielo", amen):steps("2 _ _ _  _ _ _ _  _ _ _ _  _ _ _ _"):freeze(0.4, { size = 120, density = 25 })
"#;

#[test]
fn the_panel_sums_up_the_grains() {
    let (_, mut app, _engine) = open("grains_view", Some(SONG_GRAINS));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let view = app.view();
    assert_eq!(view.tracks[0].grains.as_deref(), Some("⁘ 80 ms · ∿"));
    assert_eq!(
        view.tracks[1].grains.as_deref(),
        Some("❄ 0.4 · 120 ms · 25/s")
    );
    let screen = text(&draw(&mut app, 160, 44));
    assert!(screen.contains("nube · ⁘ 80 ms · ∿"), "{screen}");
    assert!(screen.contains("hielo · ❄ 0.4 · 120 ms · 25/s"), "{screen}");
    // El pitch que no cambia la duración, en el tracker.
    assert!(screen.contains("02%-12"), "{screen}");
}

#[test]
fn grains_options_complete_and_explain_themselves() {
    let (mut app, _engine) = open_at_end("grains_complete");
    type_text(&mut app, "track(\"drums\"):grains({ ");
    app.handle(f(4));
    assert_eq!(
        completion_items(&app),
        [
            "size", "density", "pos", "speed", "jitter", "pitch", "spread", "seed"
        ]
    );
    app.handle(esc());
    assert_eq!(app.view().help.unwrap().signature, "pista:grains({ ... })");
    type_text(&mut app, "density = ");
    let help = app.view().help.unwrap();
    assert!(
        help.description.contains("granos que nacen por segundo"),
        "{help:?}"
    );
}
