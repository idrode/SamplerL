//! Choques de frecuencia en la interfaz: la acción de la paleta abre el
//! informe (pareja × banda, puntuación y resumen) sin tocar lo que suena, y
//! la orden `--analyze-masking` da el mismo informe en texto.

use super::*;

/// La carpeta del test con senos de 2 s a 100, 300 y 5000 Hz y `song.lua`.
fn sines(test: &str, song: &str) -> (PathBuf, App, Engine) {
    let (path, app, engine) = open(test, Some(song));
    let dir = path.parent().unwrap();
    for freq in [100, 300, 5000] {
        let mono: Vec<f32> = (0..2 * RATE as usize)
            .map(|i| 0.3 * (std::f32::consts::TAU * freq as f32 * i as f32 / RATE as f32).sin())
            .collect();
        offline::write_wav(dir.join(format!("sine{freq}.wav")), &mono, 1, RATE).unwrap();
    }
    (path, app, engine)
}

pub const ONE: &str = "1 . . .  . . . .  . . . .  . . . .";

pub fn two(a: u32, b: u32) -> String {
    format!(
        "bpm(120)\n\
         track(\"a\", sample(\"sine{a}.wav\"):slice_grid(1)):steps(\"{ONE}\")\n\
         track(\"b\", sample(\"sine{b}.wav\"):slice_grid(1)):steps(\"{ONE}\"):gain(0.8)\n"
    )
}

#[test]
fn the_palette_shows_where_two_tracks_clash() {
    let (_, mut app, mut engine) = sines("masking_palette", &two(300, 300));
    let (_, mut quiet, mut reference) = sines("masking_palette_ref", &two(300, 300));
    for (app, engine) in [(&mut app, &mut engine), (&mut quiet, &mut reference)] {
        app.handle(ctrl('l'));
        offline::render(engine, 256, 2);
        assert!(app.view().error.is_none(), "{:?}", app.view().error);
    }
    palette(&mut app, "choques");
    let view = app.view().masking.expect("informe abierto");
    assert!(!view.items.is_empty());
    assert_eq!(view.items[0].0, "a × b  315 Hz", "{:?}", view.items);
    assert!(view.items[0].1.starts_with("0."), "{:?}", view.items[0]);
    assert!(
        view.description.contains("a 2 dB más fuerte"),
        "{}",
        view.description
    );
    // La tabla pareja × banda, debajo.
    assert!(view.preview[0].contains("315"), "{:?}", view.preview);
    assert!(view.preview[1].starts_with("a × b"), "{:?}", view.preview);
    let screen = text(&draw(&mut app, 140, 44));
    assert!(screen.contains("choques de frecuencia"), "{screen}");
    assert!(screen.contains("a × b  315 Hz"), "{screen}");
    // Filtrar por la banda y cerrar.
    type_text(&mut app, "400");
    assert!(
        app.view()
            .masking
            .unwrap()
            .items
            .iter()
            .all(|i| i.0.contains("400 Hz"))
    );
    app.handle(esc());
    assert!(app.view().masking.is_none());
    // Lo que suena no se ha tocado: igual que la misma canción sin
    // analizar (el análisis va en motores aparte).
    assert_eq!(
        offline::render(&mut engine, 48_000, 2),
        offline::render(&mut reference, 48_000, 2)
    );
}

#[test]
fn without_clashes_it_only_says_so() {
    let (_, mut app, mut engine) = sines("masking_none", &two(100, 5000));
    app.handle(ctrl('l'));
    offline::render(&mut engine, 256, 2);
    palette(&mut app, "choques");
    assert!(app.view().masking.is_none());
    assert!(
        app.view()
            .message
            .contains("sin choques de frecuencia entre 2 pistas"),
        "{}",
        app.view().message
    );
    // Con una sola pista no hay nada que comparar.
    replace_all(
        &mut app,
        &format!(
            "bpm(120)\ntrack(\"a\", sample(\"sine300.wav\"):slice_grid(1)):steps(\"{ONE}\")\n"
        ),
    );
    app.handle(ctrl('l'));
    offline::render(&mut engine, 256, 2);
    palette(&mut app, "choques");
    assert!(
        app.view().message.contains("más de una pista"),
        "{}",
        app.view().message
    );
}

#[test]
fn the_command_line_prints_the_same_report() {
    let (path, _app, _engine) = sines("masking_cli", &two(300, 300));
    let report = crate::masking_report(&path).unwrap();
    assert!(
        report.starts_with("choques de frecuencia: 2 pistas, 1 compases"),
        "{report}"
    );
    assert!(report.contains("a × b  315 Hz"), "{report}");
    assert!(report.contains("fuerte"), "{report}");
    let (path, _app, _engine) = sines("masking_cli_none", &two(100, 5000));
    assert!(
        crate::masking_report(&path)
            .unwrap()
            .ends_with("sin choques\n")
    );
}
