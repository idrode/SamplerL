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

#[test]
fn blocks_are_updated_with_the_selection_and_deleted_after_confirming() {
    let (path, mut app, _engine) = open_with_blocks("block_update");
    write_block(&global_dir(&path), "fantasma.lua", GHOST_BLOCK);
    write_block(&global_dir(&path), "lento.lua", "bpm(90)\n");
    let local = path.parent().unwrap().join("blocks");
    write_block(
        &local,
        "lento.lua",
        "-- descripción: del proyecto\nbpm(80)\n",
    );

    // Actualizar pide una selección.
    palette(&mut app, "actualizar un bloque");
    assert!(app.view().block_picker.is_none());
    assert!(app.view().message.contains("selecciona"));

    select_lines(&mut app, 0, 0);
    palette(&mut app, "actualizar un bloque");
    type_query(&mut app, "fantasma");
    app.handle(special(Key::Enter));
    // El primer Enter solo pide confirmación.
    let screen = text_of(&mut app);
    assert!(
        screen.contains("Enter otra vez para sustituir el texto de «fantasma»"),
        "{screen}"
    );
    assert!(
        std::fs::read_to_string(global_dir(&path).join("fantasma.lua"))
            .unwrap()
            .contains("slice_at")
    );
    // Moverse la quita.
    app.handle(special(Key::Down));
    assert!(app.view().block_picker.unwrap().confirm.is_empty());
    app.handle(special(Key::Up));
    app.handle(special(Key::Enter));
    app.handle(special(Key::Enter));
    assert!(app.view().block_picker.is_none());
    assert!(
        app.view().message.contains("actualizado"),
        "{}",
        app.view().message
    );
    // Conserva la cabecera.
    assert_eq!(
        std::fs::read_to_string(global_dir(&path).join("fantasma.lua")).unwrap(),
        "-- nombre: fantasma\n-- descripción: Ghost notes suaves en contratiempos\n\
         -- etiquetas: ghost, hats\n\nbpm(120)\n"
    );

    // Borrar el del proyecto deja ver el global del mismo nombre.
    palette(&mut app, "borrar un bloque");
    type_query(&mut app, "lento");
    assert_eq!(app.view().block_picker.unwrap().description, "del proyecto");
    app.handle(special(Key::Enter));
    assert!(text_of(&mut app).contains("Enter otra vez para borrar «lento»"));
    app.handle(special(Key::Enter));
    assert!(!local.join("lento.lua").exists());
    let message = app.view().message;
    assert!(message.contains("queda el global"), "{message}");
    // Esc cancela sin borrar.
    palette(&mut app, "borrar un bloque");
    app.handle(special(Key::Enter));
    app.handle(esc());
    assert!(global_dir(&path).join("fantasma.lua").exists());
    assert!(!app.view().playing_mode);
}

#[test]
fn quick_slots_yank_and_paste_like_vim_registers() {
    let (_, mut app, _engine) = open_with_blocks("slots");
    // Todas vacías: pegar lo dice y no espera número.
    palette(&mut app, "pegar una ranura");
    assert!(app.view().slots.is_none());
    assert!(app.view().message.contains("vacías"));

    // Sin selección, la línea entera (con su salto de línea).
    app.editor.move_cursor(CursorMove::Jump(0, 3));
    palette(&mut app, "guardar en una ranura");
    let screen = text_of(&mut app);
    assert!(screen.contains("guardar en la ranura · 1-9"), "{screen}");
    assert!(screen.contains("RANURA: pulsa 1-9"), "{screen}");
    app.handle(key('1'));
    assert_eq!(app.view().message, "ranura 1: 1 línea");
    assert!(app.view().slots.is_none());
    assert_eq!(current_line(&app), "bpm(120)", "no escribe el número");

    // Una selección de varias líneas.
    select_lines(&mut app, 2, 3);
    palette(&mut app, "guardar en una ranura");
    app.handle(key('3'));
    assert_eq!(app.view().message, "ranura 3: 2 líneas");

    // La lista enseña lo que hay en cada una.
    palette(&mut app, "pegar una ranura");
    let screen = text_of(&mut app);
    assert!(screen.contains(" 1 bpm(120)"), "{screen}");
    assert!(screen.contains(" 2 vacía"), "{screen}");
    assert!(screen.contains(" 3 2 líneas · track(\"drums\""), "{screen}");
    // Otra tecla cancela sin escribir.
    app.handle(key('x'));
    assert_eq!(app.view().message, "ranura cancelada");
    assert!(
        !app.editor
            .lines()
            .iter()
            .any(|l| l.contains('x') && l.len() == 1)
    );

    // Una ranura vacía.
    palette(&mut app, "pegar una ranura");
    app.handle(key('2'));
    assert_eq!(app.view().message, "la ranura 2 está vacía");

    // Líneas enteras: encima de la del cursor, aunque esté en mitad.
    app.editor.cancel_selection();
    app.editor.move_cursor(CursorMove::Jump(1, 5));
    palette(&mut app, "pegar una ranura");
    app.handle(key('1'));
    let lines = app.editor.lines().to_vec();
    assert_eq!(
        lines[0..3],
        [
            "bpm(120)",
            "bpm(120)",
            "local amen = sample(\"break.wav\"):slice(2)"
        ]
    );
    // Se deshace de una vez.
    app.handle(ctrl('z'));
    assert_eq!(app.editor.lines().len(), 4);

    // Un trozo que no acaba en salto de línea va en el cursor.
    app.editor.move_cursor(CursorMove::Jump(0, 0));
    app.editor.start_selection();
    app.editor.move_cursor(CursorMove::Jump(0, 3));
    palette(&mut app, "guardar en una ranura");
    app.handle(key('9'));
    app.editor.move_cursor(CursorMove::Bottom);
    app.editor.move_cursor(CursorMove::End);
    type_text(&mut app, "\n-- ");
    palette(&mut app, "pegar una ranura");
    app.handle(key('9'));
    assert_eq!(current_line(&app), "-- bpm");
}

#[test]
fn blocks_show_up_in_the_completion_with_their_fields() {
    let (path, mut app, _engine) = open("block_complete", Some(SONG));
    write_block(&global_dir(&path), "fantasma.lua", GHOST_BLOCK);
    app.set_global_blocks(Some(global_dir(&path)));
    app.editor.move_cursor(CursorMove::Bottom);
    app.editor.move_cursor(CursorMove::End);
    type_text(&mut app, "\nfan");
    assert_eq!(completion_items(&app), ["fantasma"]);
    let screen = text_of(&mut app);
    assert!(
        screen.contains("bloque fantasma (global) #ghost #hats"),
        "{screen}"
    );
    app.handle(special(Key::Tab));
    let lines = app.editor.lines().to_vec();
    assert_eq!(lines[4], "track(\"ghost\", amen:slice_at(2))");
    assert_eq!(selection_text(&app), "ghost");
    app.handle(special(Key::Tab));
    assert_eq!(selection_text(&app), "2");
    // Tras el último campo, al final del bloque.
    app.handle(special(Key::Tab));
    assert_eq!(app.editor.selection_range(), None);
    assert_eq!(app.editor.cursor(), (6, 0));

    // Un bloque nuevo en disco aparece sin reiniciar (la biblioteca se
    // relee al abrir el selector y cada medio segundo).
    write_block(&global_dir(&path), "lento.lua", "bpm(${90})\n");
    palette(&mut app, "insertar un bloque");
    app.handle(esc());
    type_text(&mut app, "len");
    assert_eq!(completion_items(&app), ["lento"]);
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

#[test]
fn slots_persist_between_sessions() {
    let (path, mut app, _engine) = open_with_blocks("slots_persist");
    let dir = path.parent().unwrap().join("config/sampler/ranuras");
    app.set_slots_dir(Some(dir.clone()));
    // Guardar crea la carpeta y el archivo.
    app.editor.move_cursor(CursorMove::Jump(0, 0));
    palette(&mut app, "guardar en una ranura");
    app.handle(key('4'));
    assert_eq!(
        std::fs::read_to_string(dir.join("4.txt")).unwrap(),
        "bpm(120)\n"
    );

    // Otra sesión: la ranura 4 sigue ahí y se pega.
    let (_, mut other, _engine2) = open_with_blocks("slots_persist_2");
    other.set_slots_dir(Some(dir.clone()));
    other.editor.move_cursor(CursorMove::Bottom);
    palette(&mut other, "pegar una ranura");
    assert!(text_of(&mut other).contains(" 4 bpm(120)"));
    other.handle(key('4'));
    assert_eq!(other.text_for_test().matches("bpm(120)").count(), 2);

    // Un archivo que no se puede leer: aviso y la ranura vacía.
    std::fs::write(dir.join("7.txt"), [0xff, 0xfe]).unwrap();
    let (_, mut third, _engine3) = open_with_blocks("slots_persist_3");
    third.set_slots_dir(Some(dir));
    assert!(
        third.view().message.starts_with("⚠ ranuras: "),
        "{}",
        third.view().message
    );
    palette(&mut third, "pegar una ranura");
    third.handle(key('7'));
    assert_eq!(third.view().message, "la ranura 7 está vacía");
}
