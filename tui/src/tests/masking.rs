//! Choques de frecuencia en la interfaz: la acción de la paleta abre el
//! informe (pareja × banda, puntuación y resumen) sin tocar lo que suena, y
//! la orden `--analyze-masking` da el mismo informe en texto.

use super::*;

/// La carpeta del test con senos de 2 s a 100, 300 y 5000 Hz y `song.lua`.
fn sines(test: &str, song: &str) -> (PathBuf, App, Engine) {
    let (path, app, engine) = open(test, Some(song));
    write_sines(path.parent().unwrap());
    (path, app, engine)
}

/// Senos de 2 s a 100, 300 y 5000 Hz en `dir`.
pub fn write_sines(dir: &Path) {
    for freq in [100, 300, 5000] {
        let mono: Vec<f32> = (0..2 * RATE as usize)
            .map(|i| 0.3 * (std::f32::consts::TAU * freq as f32 * i as f32 / RATE as f32).sin())
            .collect();
        offline::write_wav(dir.join(format!("sine{freq}.wav")), &mono, 1, RATE).unwrap();
    }
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
    // El EQ sugerido para la que cede y la tabla pareja × banda, debajo.
    assert_eq!(view.preview[0], "sugerencia para b (cede en este choque):");
    assert!(
        view.preview[1].starts_with("  track(\"b\"):fx(\"eq\", { freq1 = "),
        "{:?}",
        view.preview
    );
    assert!(
        view.preview.iter().any(|l| l.starts_with("a × b")),
        "{:?}",
        view.preview
    );
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
        report.starts_with("choques de frecuencia: 2 pistas, 1 compás"),
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

#[test]
fn enter_twice_inserts_the_suggested_eq_after_the_track() {
    let song = format!("{}-- fin\n", two(300, 300));
    let (path, mut app, mut engine) = sines("masking_apply", &song);
    app.handle(ctrl('l'));
    offline::render(&mut engine, 256, 2);
    let before = app.editor.lines().to_vec();
    palette(&mut app, "choques");
    let first = app.view().masking.unwrap();
    let line = first.preview[1].trim().to_string();
    // El primer Enter solo propone: el texto no cambia.
    app.handle(special(Key::Enter));
    let view = app.view().masking.expect("sigue abierto");
    assert!(
        view.confirm.contains("Enter otra vez inserta el EQ de «b»"),
        "{}",
        view.confirm
    );
    assert_eq!(app.editor.lines(), before);
    // Esc cancela la propuesta (el informe sigue abierto).
    app.handle(esc());
    assert!(app.view().masking.unwrap().confirm.is_empty());
    assert_eq!(app.editor.lines(), before);
    // Moverse también la cancela.
    app.handle(special(Key::Enter));
    app.handle(special(Key::Down));
    assert!(app.view().masking.unwrap().confirm.is_empty());
    app.handle(special(Key::Up));
    // Enter y Enter: insertado justo después de la definición de b.
    app.handle(special(Key::Enter));
    app.handle(special(Key::Enter));
    assert!(app.view().masking.is_none());
    let lines = app.editor.lines().to_vec();
    assert_eq!(lines.len(), before.len() + 1);
    let b = lines
        .iter()
        .position(|l| l.starts_with("track(\"b\", "))
        .unwrap();
    assert_eq!(lines[b + 1], line);
    assert_eq!(lines.last().unwrap(), "-- fin");
    assert!(
        app.view()
            .message
            .contains("EQ de «b» insertado en la línea 4, sin guardar"),
        "{}",
        app.view().message
    );
    // El archivo no se ha tocado y nada se ha cargado.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), song);
    assert!(app.view().dirty);
    // Un Ctrl-Z lo quita entero.
    app.handle(ctrl('z'));
    assert_eq!(app.editor.lines(), before);
    app.handle(ctrl('r'));
    assert_eq!(app.editor.lines(), lines);

    // Cargado, el choque en 315 Hz baja.
    let score = |app: &mut App| {
        palette(app, "choques");
        let view = app.view().masking;
        app.handle(esc());
        view.and_then(|v| {
            v.items
                .iter()
                .find(|i| i.0 == "a × b  315 Hz")
                .map(|i| i.1.parse::<f32>().unwrap())
        })
        .unwrap_or(0.0)
    };
    app.handle(ctrl('z'));
    app.handle(ctrl('l'));
    offline::render(&mut engine, 256, 2);
    let high = score(&mut app);
    app.handle(ctrl('r'));
    app.handle(ctrl('l'));
    offline::render(&mut engine, 256, 2);
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let low = score(&mut app);
    assert!(low < high - 0.3, "{high} → {low}");
}

#[test]
fn a_track_defined_over_several_lines_gets_it_after_its_chain() {
    let song = format!(
        "bpm(120)\n\
         track(\"a\", sample(\"sine300.wav\"):slice_grid(1)):steps(\"{ONE}\")\n\
         if true then\n  \
           track(\"b\", sample(\"sine300.wav\"):slice_grid(1))\n    \
             :steps(\"{ONE}\")\n    \
             :gain(0.8)\n\
         end\n"
    );
    let (_, mut app, mut engine) = sines("masking_chain", &song);
    app.handle(ctrl('l'));
    offline::render(&mut engine, 256, 2);
    palette(&mut app, "choques");
    app.handle(special(Key::Enter));
    app.handle(special(Key::Enter));
    let lines = app.editor.lines().to_vec();
    assert!(
        lines[6].starts_with("  track(\"b\"):fx(\"eq\""),
        "{lines:?}"
    );
    assert_eq!(lines[7], "end");
    app.handle(ctrl('l'));
    offline::render(&mut engine, 256, 2);
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
}

/// El informe de choques en pantalla, con la propuesta de EQ:
/// `cargo test -p tui pantalla_choques -- --ignored --nocapture`
#[test]
#[ignore]
fn pantalla_choques() {
    let song = format!(
        "{}track(\"c\", sample(\"sine100.wav\"):slice_grid(1)):steps(\"{ONE}\")\npriority({{ \"b\" }})\n",
        two(300, 300)
    );
    let (_, mut app, mut engine) = sines("masking_screen", &song);
    app.handle(ctrl('l'));
    offline::render(&mut engine, 256, 2);
    palette(&mut app, "choques");
    app.handle(special(Key::Enter));
    println!("{}", text(&draw(&mut app, 120, 40)));
}
