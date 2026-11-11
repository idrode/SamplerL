//! Añadir samples al proyecto desde el explorador: copiar a `samples/`
//! junto al script (nunca mover ni sobrescribir), escribir `sample("...")`
//! relativo, marcar varios, carpetas enteras, arrastrar al editor, solo
//! referenciar, y recoger los de un script de antes.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use ratatui_textarea::{CursorMove, Input, Key};

use super::{SONG, ctrl, draw, key, open};
use crate::app::{App, MouseAction};
use crate::config::Config;
use crate::palette::Action;

fn alt(c: char) -> Input {
    Input {
        key: Key::Char(c),
        alt: true,
        ..Input::default()
    }
}

/// Un proyecto con una carpeta global al lado (con `think.wav`, una copia
/// del break que carga) puesta en `[samples] carpeta`.
fn project(test: &str) -> (PathBuf, PathBuf, App, engine::Engine) {
    let (path, mut app, engine) = open(test, Some(SONG));
    let dir = path.parent().unwrap().to_path_buf();
    let global = dir.with_file_name(format!("tui_{test}_global"));
    let _ = std::fs::remove_dir_all(&global);
    std::fs::create_dir_all(&global).unwrap();
    std::fs::copy(dir.join("break.wav"), global.join("think.wav")).unwrap();
    app.set_config(&Config {
        samples_dir: Some(global.clone()),
        ..Default::default()
    });
    app.editor.move_cursor(CursorMove::Bottom);
    draw(&mut app, 120, 40);
    (path, global, app, engine)
}

/// Elige en el árbol la fila de `path` (con el foco en el explorador).
fn select(app: &mut App, path: &Path) {
    draw(app, 120, 40);
    app.handle(alt('0'));
    let full = path.canonicalize().unwrap();
    let rows = app.view_files_rows();
    let i = rows
        .iter()
        .position(|r| r.path == full && !r.note)
        .unwrap_or_else(|| panic!("no está {}: {rows:?}", full.display()));
    app.select_file_for_test(i);
}

/// Hasta que acabe la copia.
fn wait(app: &mut App) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !app.adding_done() {
        assert!(Instant::now() < deadline, "la copia no acaba");
        app.tick();
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn bytes(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

#[test]
fn a_global_sample_is_copied_into_the_project_and_written_relative() {
    let (path, global, mut app, _engine) = project("add_copy");
    let dir = path.parent().unwrap();
    let before = app.text_for_test();
    select(&mut app, &global.join("think.wav"));
    app.handle(key('a'));
    wait(&mut app);
    let copy = dir.join("samples/think.wav");
    assert_eq!(bytes(&copy), bytes(&global.join("think.wav")));
    assert!(global.join("think.wav").exists(), "el original sigue");
    assert!(
        app.text_for_test().contains("sample(\"think.wav\")"),
        "{}",
        app.text_for_test()
    );
    let message = app.view().message;
    assert!(message.contains("think.wav copiado"), "{message}");
    // Sin temporales a la vista.
    let names: Vec<String> = std::fs::read_dir(dir.join("samples"))
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["think.wav"]);
    // El árbol tiene ahora la raíz del proyecto, la primera.
    draw(&mut app, 120, 40);
    let rows = app.view_files_rows();
    assert_eq!(rows[0].label.as_deref(), Some("proyecto"));
    // Escribirlo es un paso de deshacer.
    app.handle(alt('1'));
    app.handle(ctrl('z'));
    assert_eq!(app.text_for_test(), before);
}

#[test]
fn the_same_name_is_reused_if_equal_and_renamed_if_not_never_overwritten() {
    let (path, global, mut app, _engine) = project("add_duplicates");
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir.join("samples")).unwrap();
    // Igual: se reutiliza.
    std::fs::copy(global.join("think.wav"), dir.join("samples/think.wav")).unwrap();
    app.run_action(Action::RefreshFiles);
    select(&mut app, &global.join("think.wav"));
    app.handle(key('a'));
    wait(&mut app);
    assert!(
        app.view().message.contains("ya estaba (igual)"),
        "{}",
        app.view().message
    );
    assert!(!dir.join("samples/think-2.wav").exists());
    // Distinto: se copia como think-2.wav y el de dentro no se toca.
    let mut other = bytes(&global.join("think.wav"));
    let n = other.len();
    other[n - 1] ^= 0x01;
    std::fs::write(dir.join("samples/think.wav"), &other).unwrap();
    select(&mut app, &global.join("think.wav"));
    app.handle(key('a'));
    wait(&mut app);
    assert_eq!(bytes(&dir.join("samples/think.wav")), other);
    assert_eq!(
        bytes(&dir.join("samples/think-2.wav")),
        bytes(&global.join("think.wav"))
    );
    assert!(app.text_for_test().contains("sample(\"think-2.wav\")"));
    assert!(
        app.view().message.contains("copiado como think-2.wav"),
        "{}",
        app.view().message
    );
}

#[test]
fn folders_and_marked_samples_are_copied_together() {
    let (path, global, mut app, _engine) = project("add_folder");
    let dir = path.parent().unwrap();
    let wav = global.join("think.wav");
    std::fs::create_dir_all(global.join("breaks/sub")).unwrap();
    std::fs::copy(&wav, global.join("breaks/a.wav")).unwrap();
    std::fs::copy(&wav, global.join("breaks/sub/b.wav")).unwrap();
    std::fs::write(global.join("breaks/notas.txt"), "x").unwrap();
    // Un enlace que sale de la carpeta: no se sigue.
    let outside = dir.with_file_name("tui_add_folder_fuera");
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::copy(&wav, outside.join("c.wav")).unwrap();
    std::os::unix::fs::symlink(&outside, global.join("breaks/fuera")).unwrap();
    app.run_action(Action::RefreshFiles);
    let before = app.text_for_test();
    select(&mut app, &global.join("breaks"));
    app.handle(key('a'));
    wait(&mut app);
    assert!(dir.join("samples/breaks/a.wav").exists());
    assert!(dir.join("samples/breaks/sub/b.wav").exists());
    assert!(!dir.join("samples/breaks/notas.txt").exists());
    assert!(!dir.join("samples/breaks/fuera").exists());
    let message = app.view().message;
    assert!(message.contains("enlace fuera de la carpeta"), "{message}");
    // Una carpeta no escribe nada en el código.
    assert_eq!(app.text_for_test(), before);

    // Varios marcados con m, de una vez.
    std::fs::copy(&wav, global.join("uno.wav")).unwrap();
    std::fs::copy(&wav, global.join("dos.wav")).unwrap();
    app.run_action(Action::RefreshFiles);
    select(&mut app, &global.join("uno.wav"));
    app.handle(key('m'));
    select(&mut app, &global.join("dos.wav"));
    app.handle(key('m'));
    let screen = super::text(&draw(&mut app, 120, 40));
    assert!(screen.contains("● ♪ dos.wav"), "{screen}");
    app.handle(key('a'));
    wait(&mut app);
    assert!(dir.join("samples/uno.wav").exists());
    assert!(dir.join("samples/dos.wav").exists());
    assert_eq!(app.text_for_test(), before);
}

#[test]
fn a_broken_or_unsupported_file_is_not_copied_and_says_why() {
    let (path, global, mut app, _engine) = project("add_broken");
    let dir = path.parent().unwrap();
    std::fs::write(global.join("roto.wav"), b"RIFF\x10\0\0\0WAVEbasura").unwrap();
    std::fs::write(global.join("loop.m4a"), b"x").unwrap();
    app.run_action(Action::RefreshFiles);
    select(&mut app, &global.join("roto.wav"));
    app.handle(key('a'));
    wait(&mut app);
    let message = app.view().message;
    assert!(message.contains("no se puede cargar"), "{message}");
    assert!(!dir.join("samples/roto.wav").exists());
    select(&mut app, &global.join("loop.m4a"));
    app.handle(key('a'));
    assert!(app.adding_done());
    assert!(
        app.view().message.contains("formato no soportado: m4a"),
        "{}",
        app.view().message
    );
}

#[test]
fn reference_only_writes_the_name_without_copying_and_warns() {
    let (path, global, mut app, _engine) = project("add_reference");
    let dir = path.parent().unwrap();
    select(&mut app, &global.join("think.wav"));
    app.handle(key('A'));
    assert!(!dir.join("samples").exists(), "no copia");
    assert!(app.text_for_test().contains("sample(\"think.wav\")"));
    let message = app.view().message;
    assert!(message.contains("carpeta global"), "{message}");
    // Uno que ya está en el proyecto: `a` solo lo escribe.
    select(&mut app, &dir.join("break.wav"));
    app.handle(key('a'));
    assert!(app.adding_done());
    assert!(app.text_for_test().contains("sample(\"break.wav\")"));
}

#[test]
fn dragging_a_sample_onto_the_editor_adds_it() {
    let (path, global, mut app, _engine) = project("add_drag");
    let dir = path.parent().unwrap();
    select(&mut app, &global.join("think.wav"));
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = super::find(&buffer, "think.wav").unwrap();
    let editor = app.editor_area.unwrap();
    app.mouse(MouseAction::Down, x, y);
    app.mouse(MouseAction::Drag, editor.x + 5, editor.y + 2);
    assert!(app.view().message.contains("suéltalo en el editor"));
    app.mouse(MouseAction::Up, editor.x + 5, editor.y + 2);
    wait(&mut app);
    assert!(dir.join("samples/think.wav").exists());
    assert!(app.text_for_test().contains("sample(\"think.wav\")"));
}

/// Un script de antes con un sample por su ruta entera y otro de la
/// carpeta global: «Recoger» los copia a `samples/`, cambia su ruta en
/// el código (un paso de deshacer) y el proyecto se puede mover entero.
#[test]
fn gathering_copies_outside_samples_and_rewrites_their_names() {
    let (path, global, mut app, _engine) = project("add_gather");
    let dir = path.parent().unwrap().to_path_buf();
    let elsewhere = dir.with_file_name("tui_add_gather_lejos");
    let _ = std::fs::remove_dir_all(&elsewhere);
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::copy(global.join("think.wav"), elsewhere.join("lejos.wav")).unwrap();
    let song = format!(
        "bpm(120)\n\
         local a = sample(\"{}\"):slice(2)\n\
         local b = sample(\"think.wav\"):slice(2) -- de la global\n\
         local c = sample(\"break.wav\"):slice(2)\n\
         track(\"a\", a):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n\
         track(\"b\", b):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n\
         track(\"c\", c):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n",
        elsewhere.join("lejos.wav").display()
    );
    app.editor = ratatui_textarea::TextArea::new(song.lines().map(String::from).collect());
    app.load();
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    app.run_action(Action::GatherSamples);
    wait(&mut app);
    let text = app.text_for_test();
    assert!(text.contains("sample(\"lejos.wav\")"), "{text}");
    assert!(text.contains("sample(\"think.wav\")"), "{text}");
    assert!(text.contains("-- de la global"), "{text}");
    assert!(dir.join("samples/lejos.wav").exists());
    assert!(dir.join("samples/think.wav").exists());
    assert!(!dir.join("samples/break.wav").exists(), "ya estaba dentro");
    assert!(elsewhere.join("lejos.wav").exists(), "el original sigue");
    let message = app.view().message;
    assert!(message.contains("recogidos"), "{message}");
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    // Un paso de deshacer.
    app.handle(alt('1'));
    app.handle(ctrl('z'));
    assert!(
        app.text_for_test()
            .contains(&elsewhere.display().to_string())
    );
    app.run_action(Action::Redo);
    assert!(app.text_for_test().contains("sample(\"lejos.wav\")"));
    // El proyecto, movido entero a otro sitio y sin la carpeta global:
    // carga igual y sin avisos de samples que faltan.
    app.handle(ctrl('s'));
    let moved = dir.with_file_name("tui_add_gather_movido");
    let _ = std::fs::remove_dir_all(&moved);
    copy_dir(&dir, &moved);
    std::fs::remove_dir_all(&elsewhere).unwrap();
    let (controller, _engine) = engine::new_engine(super::RATE);
    let mut moved_app = App::open(&moved.join("song.lua"), controller, super::RATE).unwrap();
    moved_app.load();
    let view = moved_app.view();
    assert!(view.error.is_none(), "{:?}", view.error);
    assert_eq!(view.tracks.len(), 3);
    let warnings = moved_app.host_warnings_for_test();
    assert!(
        !warnings.iter().any(|w| w.contains("no está")),
        "{warnings:?}"
    );
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}
