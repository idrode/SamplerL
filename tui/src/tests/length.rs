//! Pistas de distinta longitud (`:length(n)`) en el tracker, el panel del
//! motor y el diagnóstico en vivo.

use super::*;

const POLY: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 . . .  2 . . .  1 . . .  2 . . .")
track("tres", amen):length(12):steps("2 . . 1  . . 2 .  . . . .")
track("largo", amen:slice_at(1))
  :length(32)
  :steps("x . . .  . . . .  . . . .  . . . .  . . . .  . . . .  . . . .  . . . x")
"#;

#[test]
fn the_tracker_follows_each_track_at_its_own_length() {
    let (_, mut app, mut engine) = open("length_tracker", Some(POLY));
    // Compás 1, paso 2: global 18 → "tres" en su paso 6, "largo" en el 18.
    offline::render(&mut engine, 18 * STEP + 100, 2);
    app.tick();
    let view = app.view();
    let positions: Vec<Option<usize>> = view.tracks.iter().map(|t| t.position).collect();
    assert_eq!(positions, [Some(2), Some(6), Some(18)]);
    assert_eq!(view.tracks[1].length, 12);

    let screen = text(&draw(&mut app, 130, 50));
    assert!(screen.contains("12 pasos"), "{screen}");
    assert!(screen.contains("32 pasos"), "{screen}");
    // El tracker entero (en la interfaz se desplaza: tiene 16 filas).
    let buffer = draw_widget(ui::TrackerPanel(&view), 40, 40);
    let screen = text(&buffer);
    // 32 filas: la pista de 16 y la de 12 dejan en blanco las que no tienen.
    assert!(screen.contains("31                       01"), "{screen}");
    // Cada pista resalta su paso (no la fila entera).
    for (row, name_col) in [("02 ", 0), ("06 ", 1), ("18 ", 2)] {
        let (x, y) = find(&buffer, &format!("│{row}")).expect(row);
        let cell = x + 4 + 11 * name_col;
        assert_eq!(buffer[(cell, y)].bg, Color::Blue, "{row}\n{screen}");
        let other = x + 4 + 11 * ((name_col + 1) % 3);
        assert_ne!(buffer[(other, y)].bg, Color::Blue, "{row}");
    }
}

#[test]
fn live_diagnostics_count_the_steps_of_the_track_length() {
    let (_, mut app, _engine) = open("length_diagnose", Some(POLY));
    assert!(app.view().diagnostics.is_empty());
    let text = format!("{POLY}track(\"cinco\", amen):length(5):steps(\"1 . . .\")");
    app.editor = ratatui_textarea::TextArea::new(text.lines().map(String::from).collect());
    let d = app.view().diagnostics;
    assert_eq!(d.len(), 1, "{d:?}\n{}", app.editor.lines().join("\n"));
    assert_eq!(d[0].message, "steps: el patrón necesita 5 pasos, tiene 4");
}
