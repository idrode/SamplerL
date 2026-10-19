//! Efectos por corte en la interfaz: el panel de efectos los dice con su
//! corte, `cortes:fx` se autocompleta y se explica, y una etiqueta o un
//! efecto mal escritos se subrayan mientras se escribe.

use super::*;

const SONG_SLICE_FX: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
amen:name(2, "snare")
amen:fx("snare", "bitcrush", { bits = 6 })
amen:fx("snare", "lowpass", { cutoff = ramp(800, 8000, "1 compás") })
amen:fx(1, "gain", { level = 0.5 })
track("drums", amen):steps("1 . snare .  . . . .  1 . snare .  . . . .")
"#;

#[test]
fn the_panel_lists_each_slice_with_its_effects() {
    let (_, mut app, _engine) = open("slice_fx_view", Some(SONG_SLICE_FX));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let view = app.view();
    let chains = &view.tracks[0].slice_fx;
    assert_eq!(chains.len(), 2);
    assert_eq!((chains[0].slice, chains[1].slice), (0, 1));
    assert_eq!(chains[1].automated, [(1, "cutoff")]);
    let screen = text(&draw(&mut app, 160, 44));
    assert!(screen.contains("◇ 1 › gain ×0.50"), "{screen}");
    assert!(
        screen.contains("◇ 2 snare › crush 6b ÷1 · LP 800 Hz q0.71 ∿cutoff"),
        "{screen}"
    );
    assert!(!screen.contains("sin efectos"), "{screen}");
}

#[test]
fn slice_fx_completes_explains_and_diagnoses_itself() {
    let (mut app, _engine) = open_at_end("slice_fx_complete");
    type_text(
        &mut app,
        "local c = sample(\"break.wav\"):slice(4)\nc:name(3, \"caja\")\nc:f",
    );
    assert_eq!(completion_items(&app), ["fx"]);
    type_text(&mut app, "x(\"");
    assert!(completion_items(&app).contains(&"caja".to_string()));
    let help = app.view().help.unwrap();
    assert_eq!(
        help.signature,
        "cortes:fx(cuál, nombre, parámetros) → cortes"
    );
    // Una etiqueta que no existe y un efecto mal escrito, subrayados.
    type_text(&mut app, "cja\", \"bitcrsh\", {})");
    let messages: Vec<String> = app
        .view()
        .diagnostics
        .iter()
        .map(|d| d.message.clone())
        .collect();
    assert_eq!(messages.len(), 2, "{messages:?}");
    assert!(messages[0].contains("ningún corte se llama \"cja\""));
    assert!(messages[1].contains("el efecto \"bitcrsh\" no existe"));
}
