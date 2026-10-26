//! La nota de cada corte en la interfaz: la vista del sample la escribe en
//! cada marca y «Notas de los cortes» de la paleta las lista.

use super::*;

/// `melodia.wav` en la carpeta del test: do, mi, sol# y la (30 cents
/// bajo), 300 ms cada una.
fn melody(path: &Path) {
    let note = (0.3 * RATE as f32) as usize;
    let audio: Vec<f32> = [60.0f32, 64.0, 68.0, 68.7]
        .iter()
        .flat_map(|&midi| {
            let f = 440.0 * 2f32.powf((midi - 69.0) / 12.0);
            (0..note).map(move |i| {
                let t = i as f32 / RATE as f32;
                let wave: f32 = (1..=4)
                    .map(|k| (std::f32::consts::TAU * f * k as f32 * t).sin() / k as f32)
                    .sum();
                0.4 * (t / 0.005).min(1.0) * (-4.0 * t).exp() * wave
            })
        })
        .collect();
    offline::write_wav(path.parent().unwrap().join("melodia.wav"), &audio, 1, RATE).unwrap();
}

#[test]
fn the_sample_view_and_the_palette_say_each_slice_note() {
    let song = r#"bpm(120)
local m = sample("melodia.wav"):slice(4):name(2, "mi")
track("voz", m):steps("1 . 2 .  3 . 4 .  . . . .  . . . .")
track("drums", sample("break.wav"):slice(2)):steps("1 . 2 .  . . . .  . . . .  . . . .")
"#;
    let path = fixture("slice_notes", Some(song));
    melody(&path);
    let (controller, _engine) = new_engine(RATE);
    let mut app = App::open(&path, controller, RATE).unwrap();
    let view = app.view();
    let names: Vec<Option<String>> = (0..4).map(|i| view.tracks[0].note(i)).collect();
    assert_eq!(
        names,
        [Some("C4"), Some("E4"), Some("G#4"), Some("A4")].map(|n| n.map(String::from))
    );
    // El break de prueba (escalones de continua) no tiene nota.
    assert!(view.tracks[1].notes.iter().all(Option::is_none));

    let screen = text(&draw(&mut app, 160, 40));
    assert!(screen.contains("1 C4"), "{screen}");
    assert!(screen.contains("2 mi E4"), "{screen}");

    palette(&mut app, "notas de los cortes");
    assert_eq!(app.view().message, "voz: 1 C4 · 2 E4 · 3 G#4 · 4 A4 -30¢");
    app.handle(esc());
    app.next_track();
    palette(&mut app, "notas de los cortes");
    assert_eq!(
        app.view().message,
        "drums: ningún corte tiene un tono claro"
    );
}

#[test]
fn moving_a_slice_recomputes_its_note() {
    // Con un corte de dos notas, al mover su inicio a la segunda, cambia.
    let song = r#"bpm(120)
local m = sample("melodia.wav"):slice(1)
track("voz", m):steps("1 . . .  . . . .  . . . .  . . . .")
"#;
    let path = fixture("slice_notes_move", Some(song));
    melody(&path);
    let (controller, _engine) = new_engine(RATE);
    let mut app = App::open(&path, controller, RATE).unwrap();
    let first = app.view().tracks[0].note(0);
    assert!(first.is_some());
    let text = app
        .text_for_test()
        .replace("slice(1)", "slice(1):move(1, 905)");
    replace_all(&mut app, &text);
    app.handle(ctrl('s'));
    assert_eq!(app.view().tracks[0].note(0).as_deref(), Some("A4"));
}
