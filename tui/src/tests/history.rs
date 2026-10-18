//! Historial de versiones en la interfaz: una versión por carga buena (sin
//! duplicar), lista con fecha y vista previa, restaurar sin pisar el
//! archivo (Ctrl-Z lo deshace) y un disco roto que solo se avisa.

use super::*;

fn root(test: &str) -> PathBuf {
    let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../target/tmp"))
        .join(format!("tui_historial_{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

fn versions(app: &App) -> usize {
    app.view().versions.map_or(0, |v| v.items.len())
}

#[test]
fn every_good_load_is_a_version_and_restoring_does_not_touch_the_file() {
    let (path, mut app, mut engine) = open("history", Some(SONG));
    app.set_history_root(Some(root("versions")));
    // Cargar, y que el motor lea el lote (si no, la cola se llena).
    let mut load = |app: &mut App| {
        app.handle(ctrl('l'));
        offline::render(&mut engine, 256, 2);
    };
    load(&mut app);
    // La misma otra vez: no se repite.
    load(&mut app);
    replace_all(&mut app, &SONG.replace("bpm(120)", "bpm(130)"));
    load(&mut app);
    // Con errores no se guarda.
    replace_all(&mut app, "bpm(\n");
    load(&mut app);
    assert!(app.view().error.is_some());
    replace_all(&mut app, &SONG.replace("bpm(120)", "bpm(140)"));
    load(&mut app);
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    assert!(
        !app.view().message.contains("historial"),
        "{}",
        app.view().message
    );

    palette(&mut app, "versiones");
    assert_eq!(versions(&app), 3);
    let view = app.view().versions.unwrap();
    assert!(view.items[0].0.ends_with(" UTC"), "{:?}", view.items[0]);
    assert!(view.preview.iter().any(|l| l.contains("bpm(140)")));
    assert_eq!(view.description, "igual que el texto del editor");
    let screen = text(&draw(&mut app, 140, 44));
    assert!(screen.contains("versiones del script"), "{screen}");
    // Filtrar por el texto y restaurar la de 130.
    type_text(&mut app, "130");
    assert_eq!(versions(&app), 1);
    app.handle(special(Key::Enter));
    assert!(app.view().versions.is_none());
    assert!(
        app.view().message.contains("sin guardar"),
        "{}",
        app.view().message
    );
    assert_eq!(app.editor.lines()[0], "bpm(130)");
    // El archivo sigue como estaba (lo último guardado no se ha tocado).
    assert_eq!(std::fs::read_to_string(&path).unwrap(), SONG);
    assert!(app.view().dirty);
    // Un Ctrl-Z lo deshace entero.
    app.handle(ctrl('z'));
    assert_eq!(app.editor.lines()[0], "bpm(140)");
}

#[test]
fn a_broken_disk_only_warns() {
    let (_, mut app, _engine) = open("history_broken", Some(SONG));
    let broken = root("broken");
    std::fs::create_dir_all(broken.parent().unwrap()).unwrap();
    std::fs::write(&broken, "no soy una carpeta").unwrap();
    app.set_history_root(Some(broken.clone()));
    app.handle(ctrl('l'));
    assert!(app.view().error.is_none());
    assert!(
        app.view().message.contains("⚠ historial"),
        "{}",
        app.view().message
    );
    palette(&mut app, "versiones");
    assert!(app.view().versions.is_none());
    assert!(app.view().message.contains("⚠ historial"));
    std::fs::remove_file(broken).unwrap();
}

#[test]
fn without_history_it_says_so() {
    let (_, mut app, _engine) = open("history_none", Some(SONG));
    palette(&mut app, "versiones");
    assert!(app.view().message.contains("sin historial"));
    app.set_history_root(Some(root("empty")));
    palette(&mut app, "versiones");
    assert!(app.view().message.contains("está vacío"));
}
