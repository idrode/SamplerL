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

/// Escribe un bloque a mano en una carpeta.
fn write_block(dir: &Path, file: &str, text: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(file), text).unwrap();
}

const GHOST_BLOCK: &str = "-- nombre: fantasma\n\
                           -- descripción: Ghost notes suaves en contratiempos\n\
                           -- etiquetas: ghost, hats\n\
                           \n\
                           track(\"${ghost}\", amen:slice_at(${2}))\n  \
                           :steps(\". x@0.2 . .  . x@0.2 . .  . x@0.2 . .  . . x@0.2 .\")\n";

fn picker_names(app: &App) -> Vec<String> {
    app.view()
        .block_picker
        .map(|p| p.items.into_iter().map(|(n, _, _)| n).collect())
        .unwrap_or_default()
}

#[test]
fn blocks_are_inserted_from_a_filterable_list_with_preview() {
    let (path, mut app, _engine) = open_with_blocks("block_insert");
    // Sin bloques, lo dice y no abre nada.
    palette(&mut app, "insertar un bloque");
    assert!(app.view().block_picker.is_none());
    assert!(app.view().message.contains("no hay bloques"));

    write_block(&global_dir(&path), "fantasma.lua", GHOST_BLOCK);
    write_block(
        &path.parent().unwrap().join("blocks"),
        "lento.lua",
        "-- etiquetas: tempo\nbpm(${90})\n",
    );
    app.editor.move_cursor(CursorMove::Bottom);
    app.editor.move_cursor(CursorMove::End);
    type_text(&mut app, "\n");
    palette(&mut app, "insertar un bloque");
    let screen = text_of(&mut app);
    assert!(screen.contains("insertar un bloque"), "{screen}");
    assert!(screen.contains("#ghost #hats global"), "{screen}");
    assert!(screen.contains("#tempo proyecto"), "{screen}");
    assert!(screen.contains("BLOQUES: escribe para filtrar"), "{screen}");
    assert_eq!(picker_names(&app), ["fantasma", "lento"]);
    // Vista previa del elegido, sin las marcas de campo.
    assert!(
        screen.contains("Ghost notes suaves en contratiempos"),
        "{screen}"
    );
    assert!(
        screen.contains("track(\"ghost\", amen:slice_at(2))"),
        "{screen}"
    );

    // Filtrar por etiqueta; escribir no toca el texto.
    type_query(&mut app, "#tempo");
    assert_eq!(picker_names(&app), ["lento"]);
    assert!(text_of(&mut app).contains("bpm(90)"));
    type_query(&mut app, "zzz");
    assert!(text_of(&mut app).contains("ningún bloque coincide"));
    // Esc cierra sin entrar en modo tocar.
    app.handle(esc());
    assert!(app.view().block_picker.is_none());
    assert!(!app.view().playing_mode);
    assert_eq!(current_line(&app), "");

    // Insertar: campos con Tab, como los snippets.
    palette(&mut app, "insertar un bloque");
    type_query(&mut app, "ghost");
    app.handle(special(Key::Enter));
    assert!(app.view().block_picker.is_none());
    assert_eq!(selection_text(&app), "ghost");
    type_text(&mut app, "suave");
    app.handle(special(Key::Tab));
    assert_eq!(selection_text(&app), "2");
    let lines = app.editor.lines().to_vec();
    assert_eq!(lines[4], "track(\"suave\", amen:slice_at(2))");
    assert!(lines[5].starts_with("  :steps("));
    app.handle(ctrl('s'));
    let view = app.view();
    assert!(view.error.is_none(), "{:?}", view.error);
    assert_eq!(view.tracks[2].name, "suave");
    // Se deshace de una vez.
    let before = app.editor.lines().to_vec();
    palette(&mut app, "insertar un bloque");
    type_query(&mut app, "lento");
    app.handle(special(Key::Enter));
    assert_ne!(app.editor.lines(), before);
    assert_eq!(
        app.view().message,
        "bloque lento: Tab salta al siguiente campo"
    );
    app.handle(ctrl('z'));
    assert_eq!(app.editor.lines(), before);
}

/// Un bloque con Lua no válido se inserta como texto; al cargarlo da el
/// error de siempre y el audio no cambia.
#[test]
fn a_block_with_invalid_lua_keeps_the_previous_song_playing() {
    let audio = |broken: bool| {
        let (path, mut app, mut engine) = open_with_blocks(if broken {
            "block_broken"
        } else {
            "block_broken_ref"
        });
        if broken {
            write_block(
                &global_dir(&path),
                "roto.lua",
                "track(\"${x}\", :steps(\"1 . . .\"\nbpm(\n",
            );
            app.editor.move_cursor(CursorMove::Bottom);
            palette(&mut app, "insertar un bloque");
            app.handle(special(Key::Enter));
            app.handle(ctrl('s'));
            let screen = text_of(&mut app);
            assert!(screen.contains("✗"), "{screen}");
            assert!(
                screen.contains("error: sigue sonando lo anterior"),
                "{screen}"
            );
        }
        offline::render(&mut engine, 32 * STEP, 2)
    };
    assert_eq!(audio(true), audio(false));
}

/// `cargo test -p tui pantalla_bloques -- --ignored --nocapture`
#[test]
#[ignore]
fn pantalla_bloques() {
    let (path, mut app, _engine) = open_with_blocks("block_screen");
    write_block(&global_dir(&path), "fantasma.lua", GHOST_BLOCK);
    write_block(
        &path.parent().unwrap().join("blocks"),
        "lento.lua",
        "-- etiquetas: tempo\nbpm(${90})\n",
    );
    palette(&mut app, "insertar un bloque");
    println!("{}", text(&draw(&mut app, 120, 40)));
}
