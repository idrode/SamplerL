//! Guardar la edición de cortes hecha en la onda: arrastrar una marca con
//! el ratón, guardar con Ctrl-S, cerrar y volver a abrir da los mismos
//! cortes, con y sin proyecto `.sampler`.

use super::*;
use crate::app::MouseAction;

/// Los cortes (en frames) de la pista `name`.
fn slices_of(app: &App, name: &str) -> Vec<std::ops::Range<usize>> {
    let view = app.view();
    let track = view
        .tracks
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("no está la pista {name}"));
    track.slices.clone()
}

/// Arrastra la marca del corte que empieza en `start` `columns` columnas,
/// con las coordenadas que da la geometría de la onda.
fn drag_mark(app: &mut App, start: usize, columns: i32) {
    draw(app, 160, 40);
    let area = app.sample_area.unwrap();
    let (x, y) = (
        area.x + (start as f64 / 6000.0 * area.width as f64).floor() as u16,
        area.y + 1,
    );
    let to = (x as i32 + columns) as u16;
    app.mouse(MouseAction::Down, x, y);
    app.mouse(MouseAction::Drag, to, y);
    app.mouse(MouseAction::Up, to, y);
}

/// Abre `path` (un script o un proyecto) en una aplicación nueva, como al
/// cerrar y volver a abrir.
fn reopen(path: &Path) -> (App, Engine) {
    let (controller, engine) = new_engine(RATE);
    let app = if path.extension().is_some_and(|e| e == "sampler") {
        App::open_project(path, controller, RATE).unwrap()
    } else {
        App::open(path, controller, RATE).unwrap()
    };
    (app, engine)
}

#[test]
fn a_dragged_mark_survives_ctrl_s_and_reopening_the_script() {
    let (path, mut app, _engine) = open("slice_save_script", Some(SONG));
    drag_mark(&mut app, 3000, 6);
    let edited = slices_of(&app, "drums");
    assert_ne!(edited[1].start, 3000, "el arrastre movió el corte");
    app.handle(ctrl('s'));
    assert!(!app.dirty());
    let (again, _engine) = reopen(&path);
    assert_eq!(slices_of(&again, "drums"), edited);
    assert_eq!(slices_of(&again, "kick"), slices_of(&app, "kick"));
}

#[test]
fn a_dragged_mark_survives_ctrl_s_with_a_project_too() {
    let path = fixture("slice_save_project", Some(SONG));
    let project = path.parent().unwrap().join("p.sampler");
    std::fs::write(&project, "script = song.lua\n").unwrap();
    let (mut app, _engine) = reopen(&project);
    drag_mark(&mut app, 3000, 6);
    let edited = slices_of(&app, "drums");
    assert_ne!(edited[1].start, 3000);
    app.handle(ctrl('s'));
    // Abrir el proyecto o solo su script da lo mismo: una sola fuente.
    for target in [&project, &path] {
        let (again, _engine) = reopen(target);
        assert_eq!(slices_of(&again, "drums"), edited, "{}", target.display());
    }
}

#[test]
fn several_edit_save_reopen_cycles_keep_the_slices() {
    let (path, mut app, _engine) = open("slice_save_cycles", Some(SONG));
    let mut expected = slices_of(&app, "drums");
    for round in 0..3 {
        let start = expected[1].start;
        drag_mark(&mut app, start, 3 + round);
        expected = slices_of(&app, "drums");
        app.handle(ctrl('s'));
        let (again, engine) = reopen(&path);
        assert_eq!(slices_of(&again, "drums"), expected, "vuelta {round}");
        drop(engine);
        app = again;
    }
}

#[test]
fn an_old_project_adjustment_moves_into_the_code_and_leaves_the_project() {
    let path = fixture("slice_save_old_project", Some(SONG));
    let project = path.parent().unwrap().join("p.sampler");
    // Un proyecto de antes: el corte 2 movido a 70 ms, guardado en él.
    std::fs::write(
        &project,
        "script = song.lua\ncorte = break.wav 2 2 70\netiqueta = break.wav 2 1 bombo\n",
    )
    .unwrap();
    let (mut app, _engine) = reopen(&project);
    assert_eq!(slices_of(&app, "drums")[1].start, 3360);
    // Al editar otro corte, todo pasa al código (con el ajuste y la
    // etiqueta del proyecto) y lo dice.
    app.handle(esc());
    app.handle(key('.'));
    let message = app.view().message;
    assert!(
        message.contains("slice(2) pasa a cortes explícitos en el código"),
        "{message}"
    );
    let text = app.text_for_test();
    assert!(
        text.contains("slice({ {5, 70, \"bombo\"}, {70, 125} })"),
        "{text}"
    );
    app.handle(ctrl('s'));
    let saved = std::fs::read_to_string(&project).unwrap();
    assert!(!saved.contains("corte ="), "{saved}");
    assert!(!saved.contains("etiqueta ="), "{saved}");
    for target in [&project, &path] {
        let (again, _engine) = reopen(target);
        assert_eq!(slices_of(&again, "drums"), [240..3360, 3360..6000]);
        assert_eq!(again.view().tracks[0].labels[0].as_deref(), Some("bombo"));
    }
}

#[test]
fn slices_from_code_are_not_lost_in_silence() {
    let song = SONG.replace(
        "sample(\"break.wav\"):slice(2)",
        "sample(\"break.wav\"):slice(1 + 1)",
    );
    let (_, mut app, _engine) = open("slice_save_generated", Some(&song));
    drag_mark(&mut app, 3000, 6);
    // Se mueve (en memoria, como antes) y avisa de que no se guarda.
    assert_ne!(slices_of(&app, "drums")[1].start, 3000);
    let message = app.view().message;
    assert!(
        message.contains("no se puede guardar en el código")
            && message.contains("no está escrito tal cual")
            && message.contains("slice(8)"),
        "{message}"
    );
    assert_eq!(app.text_for_test(), song);
    // Unos cortes explícitos que salen de una variable: no se cambian.
    let song = SONG.replace(
        "local amen = sample(\"break.wav\"):slice(2)",
        "local t = { {0, 60}, {60, 125} }\nlocal amen = sample(\"break.wav\"):slice(t)",
    );
    let (_, mut app, _engine) = open("slice_save_generated_table", Some(&song));
    drag_mark(&mut app, 2880, 6);
    assert_eq!(slices_of(&app, "drums")[1].start, 2880);
    let message = app.view().message;
    assert!(message.contains("no está escrita tal cual"), "{message}");
}

#[test]
fn an_edit_waits_for_the_text_to_be_loaded_and_ctrl_z_undoes_it() {
    let (_, mut app, _engine) = open("slice_save_unloaded", Some(SONG));
    // Texto escrito sin cargar: la posición de la llamada no vale.
    type_text(&mut app, "-- algo\n");
    drag_mark(&mut app, 3000, 6);
    assert_eq!(slices_of(&app, "drums")[1].start, 3000);
    assert!(
        app.view().message.contains("carga antes (Ctrl-S)"),
        "{}",
        app.view().message
    );
    app.handle(ctrl('s'));
    drag_mark(&mut app, 3000, 6);
    assert_ne!(slices_of(&app, "drums")[1].start, 3000);
    assert!(app.text_for_test().contains("slice({"));
    // Ctrl-Z: vuelve slice(2), y suena como antes.
    app.handle(esc());
    app.handle(ctrl('z'));
    assert!(app.text_for_test().contains("slice(2)"));
    app.load();
    assert_eq!(slices_of(&app, "drums")[1].start, 3000);
}

#[test]
fn with_the_editor_in_its_window_the_edit_reaches_it() {
    let path = fixture("slice_save_split", Some(SONG));
    let (mut main, _engine) = super::split::main_window(&path, None);
    let (mut editor, _e) = super::split::editor_window(&path);
    super::split::pump(&mut main, &mut editor);
    drag_mark(&mut main, 3000, 6);
    let edited = slices_of(&main, "drums");
    assert_ne!(edited[1].start, 3000);
    super::split::pump_until(&mut main, &mut editor, |_, e| {
        e.text_for_test().contains("slice({")
    });
    // Se guarda desde la ventana del editor, que es la del archivo.
    editor.handle(ctrl('s'));
    assert!(std::fs::read_to_string(&path).unwrap().contains("slice({"));
    let (again, _engine) = reopen(&path);
    assert_eq!(slices_of(&again, "drums"), edited);
}
