//! Instrumentos en la interfaz: la tabla de `instrument` en el
//! autocompletado y la ayuda, notas por su nombre en el tracker y las voces
//! que suenan en el panel de efectos.

use super::*;

const SONG_SYNTH: &str = r#"bpm(120)
local piano = sample("break.wav"):instrument({ root = "C3", loop = { 10, 100 }, voices = 4 })
track("lead", piano):melody("c+e+g _ _ _  e . c4 .  . . . .  . . . .")
"#;

#[test]
fn the_tracker_writes_notes_and_the_panel_counts_voices() {
    let (_, mut app, mut engine) = open("instrument_view", Some(SONG_SYNTH));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let view = app.view();
    let track = &view.tracks[0];
    assert_eq!(track.instrument.map(|i| (i.root, i.voices)), Some((48, 4)));
    assert_eq!(track.cell_text(&track.steps[0]), "C3+E3+G3");
    assert_eq!(track.cell_text(&track.steps[6]), "C4");
    assert!(track.held(1) && track.held(3) && !track.held(4) && !track.held(5));

    // El acorde suena: tres voces de cuatro.
    offline::render(&mut engine, STEP, 2);
    app.tick();
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("lead · ♪ C3 3/4"), "{screen}");
    assert!(screen.contains("C3+E3+G3"), "{screen}");
    assert!(screen.contains(" │"), "{screen}");
}

#[test]
fn instrument_options_complete_and_explain_themselves() {
    let (mut app, _engine) = open_at_end("instrument_complete");
    type_text(&mut app, "local i = sample(\"break.wav\"):instrument({ ");
    app.handle(f(4));
    assert_eq!(
        completion_items(&app),
        ["root", "loop", "voices", "adsr", "crossfade"]
    );
    app.handle(esc());
    assert_eq!(
        app.view().help.unwrap().signature,
        "sample:instrument({ ... })"
    );
    type_text(&mut app, "voices = 2 })\ntrack(\"lead\", i):mel");
    assert_eq!(completion_items(&app), ["melody"]);
    app.handle(special(Key::Tab));
    app.handle(ctrl('s'));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    assert_eq!(
        app.view().tracks.last().unwrap().instrument.unwrap().voices,
        2
    );
}
