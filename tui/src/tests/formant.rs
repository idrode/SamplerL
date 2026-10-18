//! Formantes en la interfaz: el panel de efectos dice dónde quedan, y
//! `formant` se autocompleta y se explica.

use super::*;

const SONG_FORMANT: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("caja", amen):steps("1 . 2%7 .  . . . .  . . . .  . . . ."):formant(0)
local voz = sample("break.wav"):instrument({ keep = true, formant = -3 })
track("voz", voz):melody("c . g .  . . . .  . . . .  . . . .")
"#;

#[test]
fn the_panel_says_where_the_formants_go() {
    let (_, mut app, _engine) = open("formant_view", Some(SONG_FORMANT));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let view = app.view();
    assert_eq!(view.tracks[0].formant, Some(0.0));
    assert_eq!(view.tracks[1].formant, Some(-3.0));
    let screen = text(&draw(&mut app, 160, 44));
    assert!(screen.contains("caja · formant 0"), "{screen}");
    assert!(screen.contains("· formant -3"), "{screen}");
}

#[test]
fn formant_completes_and_explains_itself() {
    let (mut app, _engine) = open_at_end("formant_complete");
    type_text(&mut app, "track(\"drums\"):form");
    let items = completion_items(&app);
    assert_eq!(items, ["formant"]);
    type_text(&mut app, "ant(");
    let help = app.view().help.unwrap();
    assert_eq!(help.signature, "pista:formant(semitonos) → pista");
}
