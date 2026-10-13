//! Fase F: biblioteca de bloques y ranuras rápidas, con el backend de test.

use std::path::{Path, PathBuf};

use ratatui_textarea::{CursorMove, Key};

use super::*;

/// Abre la canción con una carpeta global de bloques propia del test.
fn open_with_blocks(test: &str) -> (PathBuf, App, Engine) {
    let (path, mut app, engine) = open(test, Some(SONG));
    app.set_global_blocks(Some(global_dir(&path)));
    (path, app, engine)
}

fn global_dir(song: &Path) -> PathBuf {
    song.parent().unwrap().join("config/sampler/blocks")
}

/// Selecciona las líneas `a` a `b` (desde 0) enteras.
fn select_lines(app: &mut App, a: usize, b: usize) {
    app.editor.cancel_selection();
    app.editor.move_cursor(CursorMove::Jump(a as u16, 0));
    app.editor.start_selection();
    let end = app.editor.lines()[b].chars().count();
    app.editor
        .move_cursor(CursorMove::Jump(b as u16, end as u16));
}

fn command_line(app: &mut App, text: &str) {
    type_query(app, text);
    app.handle(special(Key::Enter));
}

#[test]
fn the_selection_is_saved_as_a_block_from_the_palette() {
    let (path, mut app, _engine) = open_with_blocks("block_save");
    // La carpeta global se crea al abrir; la del proyecto, no.
    assert!(global_dir(&path).is_dir());
    assert!(!path.parent().unwrap().join("blocks").exists());

    // Sin selección no se abre la línea de órdenes.
    palette(&mut app, "guardar selec bloque");
    assert!(app.view().prompt.is_none());
    assert!(app.view().message.contains("selecciona"));

    select_lines(&mut app, 2, 3);
    palette(&mut app, "guardar selec bloque");
    assert_eq!(app.view().prompt.as_deref(), Some("bloque "));
    command_line(
        &mut app,
        "dos_pistas drums #kick -- Las dos pistas del break",
    );
    let file = global_dir(&path).join("dos_pistas.lua");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        format!(
            "-- nombre: dos_pistas\n-- descripción: Las dos pistas del break\n\
             -- etiquetas: drums, kick\n\n{}",
            SONG.lines()
                .skip(2)
                .map(|l| format!("{l}\n"))
                .collect::<String>()
        )
    );
    assert!(
        app.view().message.contains("guardado"),
        "{}",
        app.view().message
    );

    // Con el mismo nombre pide `!`; con `!` lo sustituye.
    select_lines(&mut app, 0, 0);
    app.handle(f(2));
    command_line(&mut app, "bloque dos_pistas");
    assert!(
        app.view().message.contains("bloque! "),
        "{}",
        app.view().message
    );
    app.handle(f(2));
    command_line(&mut app, "bloque! dos_pistas");
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .ends_with("\n\nbpm(120)\n")
    );

    // En la carpeta del proyecto (se crea al guardar), sin sangría común.
    app.editor.cancel_selection();
    app.editor.move_cursor(CursorMove::Bottom);
    app.editor.move_cursor(CursorMove::End);
    type_text(&mut app, "\n  :gain(0.5)\n    :swing(60)");
    select_lines(&mut app, 4, 5);
    app.handle(f(2));
    command_line(&mut app, "bloque-proyecto mezcla");
    let local = path.parent().unwrap().join("blocks/mezcla.lua");
    assert_eq!(
        std::fs::read_to_string(&local).unwrap(),
        "-- nombre: mezcla\n\n:gain(0.5)\n  :swing(60)\n"
    );

    // Nombre no válido.
    app.handle(f(2));
    command_line(&mut app, "bloque mal-nombre");
    assert!(
        app.view().message.contains("no válido"),
        "{}",
        app.view().message
    );
}
