//! Efectos por paso en la interfaz: el tracker los enseña, el diagnóstico
//! en vivo subraya el paso con un efecto mal escrito, el resaltado de la
//! fase M marca el token entero (con sus llaves y los espacios de dentro)
//! y el autocompletado ofrece efectos y parámetros dentro de las llaves.

use super::*;

const SONG: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1{lp = 800} . 2{crush,downsample=2}{lp=200>900:2} .  1 . 2 .  1 . 2 .  1 . 2 .")
"#;

#[test]
fn the_tracker_shows_the_effects_of_each_step() {
    let (_, mut app, _engine) = open("step_fx_tracker", Some(SONG));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let screen = text(&draw(&mut app, 160, 50));
    assert!(screen.contains("01{lp}"), "{screen}");
    // La columna corta lo que no cabe; la celda entera, en el modelo.
    assert!(screen.contains("02{crush"), "{screen}");
    let view = app.view();
    let drums = &view.tracks[0];
    assert_eq!(drums.cell_text(&drums.steps[2]), "02{crush,lp∿}");
}

#[test]
fn live_diagnostics_underline_a_wrong_step_effect() {
    let lines: Vec<String> = [
        "local amen = sample(\"break.wav\"):slice(2)",
        "track(\"d\", amen):steps(\"1 . 2{zap=3} .  . . . .  . . . .  . . . .\")",
    ]
    .iter()
    .map(|l| l.to_string())
    .collect();
    let d = crate::diagnose::steps_diagnostics(&lines, &[]);
    assert_eq!(d.len(), 1, "{d:?}");
    let underlined: String = lines[1]
        .chars()
        .skip(d[0].cols.start)
        .take(d[0].cols.len())
        .collect();
    assert_eq!(underlined, "2{zap=3}");
    assert!(
        d[0].message.contains("{zap}: el efecto no existe"),
        "{}",
        d[0].message
    );
    // Bien escrito (también con espacios dentro de las llaves), nada.
    let good = vec![
        lines[0].clone(),
        "track(\"d\", amen):steps(\"1 . 2{lp = 800, q=2} .  . . . .  . . . .  . . . .\")".into(),
    ];
    assert!(crate::diagnose::steps_diagnostics(&good, &[]).is_empty());
}

#[test]
fn the_sounding_token_lights_up_with_its_braces() {
    let (_, mut app, mut engine) = open("step_fx_highlight", Some(SONG));
    offline::render(&mut engine, 100, 2);
    app.tick();
    let line = SONG.lines().nth(2).unwrap();
    let start = line.find("1{").unwrap();
    let marks = app.view().highlights;
    assert_eq!(marks.len(), 1, "{marks:?}");
    assert_eq!(marks[0].row, 2);
    assert_eq!(marks[0].cols, start..start + "1{lp = 800}".len());
    // El paso 2, con dos efectos.
    offline::render(&mut engine, 2 * STEP, 2);
    app.tick();
    let start = line.find("2{crush").unwrap();
    let marks = app.view().highlights;
    assert_eq!(
        marks[0].cols,
        start..start + "2{crush,downsample=2}{lp=200>900:2}".len()
    );
}

#[test]
fn completion_inside_the_braces_offers_effects_and_parameters() {
    let (mut app, _engine) = open_at_end("step_fx_complete");
    type_text(&mut app, "track(\"drums\"):steps(\"1{cr");
    assert_eq!(completion_items(&app), ["crush"]);
    app.handle(special(Key::Tab));
    assert!(
        current_line(&app).ends_with("steps(\"1{crush=8"),
        "{}",
        current_line(&app)
    );
    type_text(&mut app, ",do");
    assert_eq!(completion_items(&app), ["downsample"]);
}
