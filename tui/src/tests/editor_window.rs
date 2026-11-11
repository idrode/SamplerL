//! La ventana aparte del editor abre el mismo script (o proyecto) que la
//! principal, con lo que haya en su editor, guardado o no; aunque la
//! principal se abriera con una ruta relativa y la terminal nueva arranque
//! en otra carpeta, y aunque haya otra principal abierta a la vez. Si algo
//! falla, lo dice: nunca abre la plantilla ni otro archivo en silencio.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use engine::{Engine, new_engine};
use ratatui_textarea::CursorMove;

use super::split::{main_window, pump, pump_until, socket_dir};
use super::{RATE, SONG, ctrl, fixture, type_text};
use crate::app::App;
use crate::link::{self, Server, ToEditor};
use crate::palette::Action;

/// Una «terminal» que apunta sus argumentos en `out`, uno por línea (con
/// `extra` delante, como las opciones de una terminal de verdad).
fn recorder(out: &Path, extra: &[&str]) -> Vec<String> {
    let mut words = vec![
        "sh".to_string(),
        "-c".into(),
        format!("printf '%s\\n' \"$@\" > '{}'", out.display()),
        "sh".into(),
    ];
    // Las opciones de la terminal van antes del programa; aquí son
    // argumentos del `sh` que no se usan.
    words.extend(extra.iter().map(|w| w.to_string()));
    words
}

/// La ruta con que la principal lanzó la ventana del editor.
fn launched(out: &Path) -> String {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(text) = std::fs::read_to_string(out)
            && let Some(at) = text.lines().position(|l| l == "--window-editor")
            && let Some(target) = text.lines().nth(at + 1)
        {
            return target.to_string();
        }
        assert!(Instant::now() < deadline, "no se lanzó");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `path` relativa a la carpeta actual del proceso (la del crate en los
/// tests), con `..` donde haga falta.
fn relative_to_cwd(path: &Path) -> PathBuf {
    let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
    let full = path.canonicalize().unwrap();
    let common = cwd
        .components()
        .zip(full.components())
        .take_while(|(a, b)| a == b)
        .count();
    let mut relative = PathBuf::new();
    for _ in cwd.components().skip(common) {
        relative.push("..");
    }
    for part in full.components().skip(common) {
        relative.push(part);
    }
    assert!(relative.is_relative());
    relative
}

/// La ventana del editor arrancada en `dir` (la carpeta de la terminal
/// nueva) con el argumento que le pasó la principal.
fn editor_in(dir: &Path, target: &str, sockets: &Path) -> Result<(App, Engine), String> {
    let (controller, engine) = new_engine(RATE);
    App::open_editor(&dir.join(target), controller, RATE, sockets)
        .map(|app| (app, engine))
        .map_err(|e| e.to_string())
}

/// Una carpeta que no es la del proyecto ni la del proceso: la de una
/// terminal que arranca en otro sitio.
fn elsewhere() -> PathBuf {
    std::env::temp_dir()
}

/// Abre un script o su proyecto en una sola ventana, escribe (o no) sin
/// guardar, saca el editor a su ventana desde el menú y arranca esa
/// ventana en otra carpeta. Comprueba que muestra lo mismo que la
/// principal, que lo que se escribe en una se ve en la otra, que guardar
/// desde allí guarda el mismo archivo y que al volver no se pierde nada.
fn same_text_in_the_editor_window(test: &str, project: bool, relative: bool, unsaved: bool) {
    let script = fixture(test, Some(SONG));
    let sockets = socket_dir(&script);
    let target = if project {
        let project = script.with_file_name("cancion.sampler");
        std::fs::write(&project, "script = song.lua\n").unwrap();
        project
    } else {
        script.clone()
    };
    let opened = if relative {
        relative_to_cwd(&target)
    } else {
        target.clone()
    };
    let (controller, _engine) = new_engine(RATE);
    let mut main = if project {
        App::open_project(&opened, controller, RATE).unwrap()
    } else {
        App::open(&opened, controller, RATE).unwrap()
    };
    main.set_socket_dir(sockets.clone());
    let out = script.with_file_name("lanzado.txt");
    main.set_terminals(Some(recorder(&out, &[])), None);
    if unsaved {
        main.editor.move_cursor(CursorMove::Bottom);
        type_text(&mut main, "-- sin guardar todavía\n");
    }
    let text = main.text_for_test();
    assert_eq!(main.dirty(), unsaved);

    main.run_action(Action::EditorWindow);
    let arg = launched(&out);
    let (mut editor, _idle) = editor_in(&elsewhere(), &arg, &sockets)
        .unwrap_or_else(|e| panic!("la ventana no abre «{arg}»: {e}"));
    pump_until(&mut main, &mut editor, |_, e| {
        e.view().message.contains("historial")
    });
    assert_eq!(editor.text_for_test(), text, "con «{arg}»");

    // Lo que se escribe en la ventana aparte llega a la principal.
    editor.editor.move_cursor(CursorMove::Bottom);
    type_text(&mut editor, "-- desde la ventana\n");
    let written = editor.text_for_test();
    pump_until(&mut main, &mut editor, |m, _| m.text_for_test() == written);

    // Guardar desde allí guarda el mismo archivo.
    editor.handle(ctrl('s'));
    assert_eq!(std::fs::read_to_string(&script).unwrap(), written);

    // Y al volver, la principal tiene el texto intacto.
    editor.editor.move_cursor(CursorMove::Bottom);
    type_text(&mut editor, "-- sin guardar al volver\n");
    let back = editor.text_for_test();
    editor.run_action(Action::EditorBack);
    drop(editor);
    for _ in 0..4 {
        main.tick();
    }
    assert!(!main.view().editor_outside);
    assert_eq!(main.text_for_test(), back);
    assert!(main.dirty());
}

#[test]
fn a_relative_script_with_unsaved_text_opens_the_same_in_its_window() {
    same_text_in_the_editor_window("ew_rel_lua", false, true, true);
}

#[test]
fn a_relative_saved_project_opens_the_same_in_its_window() {
    same_text_in_the_editor_window("ew_rel_project", true, true, false);
}

#[test]
fn a_relative_project_with_unsaved_text_opens_the_same_in_its_window() {
    same_text_in_the_editor_window("ew_rel_project_unsaved", true, true, true);
}

#[test]
fn an_absolute_project_with_unsaved_text_opens_the_same_in_its_window() {
    same_text_in_the_editor_window("ew_abs_project", true, false, true);
}

#[test]
fn the_window_is_launched_with_the_full_canonical_path() {
    let script = fixture("ew_canonical", Some(SONG));
    let (controller, _engine) = new_engine(RATE);
    let mut main = App::open(&relative_to_cwd(&script), controller, RATE).unwrap();
    main.set_socket_dir(socket_dir(&script));
    let out = script.with_file_name("lanzado.txt");
    main.set_terminals(Some(recorder(&out, &[])), None);
    main.run_action(Action::EditorWindow);
    assert_eq!(
        launched(&out),
        script.canonicalize().unwrap().display().to_string()
    );
}

#[test]
fn the_window_never_opens_the_template_for_a_missing_file() {
    let script = fixture("ew_missing", None);
    let sockets = socket_dir(&script);
    let error = editor_in(&elsewhere(), &script.display().to_string(), &sockets)
        .err()
        .expect("abrió un archivo que no existe");
    assert!(error.contains("no existe"), "{error}");
    assert!(error.contains("song.lua"), "{error}");

    // Un proyecto cuyo script no está tampoco abre nada.
    let project = script.with_file_name("cancion.sampler");
    std::fs::write(&project, "script = song.lua\n").unwrap();
    let error = editor_in(&elsewhere(), &project.display().to_string(), &sockets)
        .err()
        .expect("abrió el proyecto sin su script");
    assert!(error.contains("no existe"), "{error}");
    assert!(error.contains("song.lua"), "{error}");

    // Ni una ruta relativa que, desde la carpeta de la terminal, no está.
    let error = editor_in(&elsewhere(), "no-hay-tal/song.lua", &sockets)
        .err()
        .expect("abrió una ruta relativa que no existe");
    assert!(error.contains("no existe"), "{error}");
}

#[test]
fn a_new_script_not_yet_on_disk_is_saved_before_opening_its_window() {
    let script = fixture("ew_new", None);
    let (controller, _engine) = new_engine(RATE);
    let mut main = App::open(&script, controller, RATE).unwrap();
    main.set_socket_dir(socket_dir(&script));
    let out = script.with_file_name("lanzado.txt");
    main.set_terminals(Some(recorder(&out, &[])), None);
    let text = main.text_for_test();
    main.run_action(Action::EditorWindow);
    assert!(
        main.view().message.contains("guardado"),
        "{}",
        main.view().message
    );
    assert_eq!(std::fs::read_to_string(&script).unwrap(), text);
    let (mut editor, _idle) =
        editor_in(&elsewhere(), &launched(&out), &socket_dir(&script)).unwrap();
    pump_until(&mut main, &mut editor, |_, e| {
        e.view().message.contains("historial")
    });
    assert_eq!(editor.text_for_test(), text);
}

/// Dos principales a la vez, con scripts que se llaman igual en carpetas
/// distintas y los sockets en la misma carpeta (como de verdad): cada
/// ventana del editor se une a la suya y muestra su texto.
#[test]
fn two_projects_open_at_once_do_not_mix_their_editor_windows() {
    let one = fixture("ew_two_a", Some(SONG));
    let two = fixture("ew_two_b", Some(SONG));
    let sockets = socket_dir(&one);
    let open = |script: &Path, note: &str| {
        let (controller, engine) = new_engine(RATE);
        let mut main = App::open(&relative_to_cwd(script), controller, RATE).unwrap();
        main.set_socket_dir(sockets.clone());
        let out = script.with_file_name("lanzado.txt");
        main.set_terminals(Some(recorder(&out, &[])), None);
        main.editor.move_cursor(CursorMove::Bottom);
        type_text(&mut main, note);
        main.run_action(Action::EditorWindow);
        (main, engine, launched(&out))
    };
    let (mut main_one, _e1, arg_one) = open(&one, "-- uno\n");
    let (mut main_two, _e2, arg_two) = open(&two, "-- dos\n");
    assert_ne!(arg_one, arg_two);
    let (mut editor_two, _i2) = editor_in(&elsewhere(), &arg_two, &sockets).unwrap();
    let (mut editor_one, _i1) = editor_in(&elsewhere(), &arg_one, &sockets).unwrap();
    pump_until(&mut main_two, &mut editor_two, |_, e| {
        e.view().message.contains("historial")
    });
    pump_until(&mut main_one, &mut editor_one, |_, e| {
        e.view().message.contains("historial")
    });
    assert!(editor_one.text_for_test().contains("-- uno\n"));
    assert!(!editor_one.text_for_test().contains("-- dos"));
    assert!(editor_two.text_for_test().contains("-- dos\n"));
    assert!(!editor_two.text_for_test().contains("-- uno"));
    assert_eq!(editor_one.text_for_test(), main_one.text_for_test());
    assert_eq!(editor_two.text_for_test(), main_two.text_for_test());
}

/// Si lo que contesta en el socket es la principal de otro script (un
/// choque de nombres, o una principal de antes que se abrió con otra
/// ruta), el editor no se queda con su texto: lo dice y no se une.
#[test]
fn the_editor_refuses_a_main_window_of_another_script() {
    let script = fixture("ew_other_main", Some(SONG));
    let other = fixture("ew_other_main_b", Some("bpm(90)\n"));
    let sockets = socket_dir(&script);
    let socket = link::socket_path(&sockets, &script);
    let mut server = Server::bind(&socket).unwrap();
    let (mut editor, _idle) =
        editor_in(&elsewhere(), &script.display().to_string(), &sockets).expect("abre el script");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !server.connected() {
        assert!(Instant::now() < deadline, "no se conectó");
        editor.tick();
        server.accept();
        std::thread::sleep(Duration::from_millis(10));
    }
    let welcome = ToEditor::Welcome {
        version: link::VERSION,
        path: other.canonicalize().unwrap(),
        samples: other.parent().unwrap().to_path_buf(),
        rev: 0,
        text: "bpm(90)\n".into(),
    };
    server.client().unwrap().send(&welcome).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !editor.view().message.contains("otro script") {
        assert!(
            Instant::now() < deadline,
            "no lo rechazó: {}",
            editor.view().message
        );
        editor.tick();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(editor.text_for_test(), SONG);
}

/// Lo escrito en la ventana del editor antes de que la principal conteste
/// no pisa lo que la principal tenía sin guardar: se queda el texto de la
/// principal y Ctrl-Z en el editor recupera lo de aquí.
#[test]
fn text_typed_before_connecting_does_not_overwrite_unsaved_text_in_the_main_window() {
    let script = fixture("ew_early", Some(SONG));
    let (mut editor, _idle) = editor_in(
        &elsewhere(),
        &script.display().to_string(),
        &socket_dir(&script),
    )
    .unwrap();
    editor.editor.move_cursor(CursorMove::Bottom);
    type_text(&mut editor, "-- aquí\n");
    let here = editor.text_for_test();

    let (mut main, _engine) = main_window(&script, None);
    // (`main_window` ya escucha: se escribe en su texto como lo habría
    // escrito antes de sacar el editor.)
    main.editor.move_cursor(CursorMove::Bottom);
    for c in "-- allí".chars() {
        main.editor.insert_char(c);
    }
    main.editor.insert_newline();
    let there = main.text_for_test();
    pump_until(&mut main, &mut editor, |_, e| {
        e.view().message.contains("Ctrl-Z")
    });
    pump(&mut main, &mut editor);
    assert_eq!(main.text_for_test(), there);
    assert_eq!(editor.text_for_test(), there);
    editor.handle(ctrl('z'));
    assert_eq!(editor.text_for_test(), here);
}

#[test]
fn a_terminal_directory_that_does_not_exist_is_reported_instead_of_launching() {
    let script = fixture("ew_directory", Some(SONG));
    let out = script.with_file_name("lanzado.txt");
    let (controller, _engine) = new_engine(RATE);
    let mut main = App::open(&relative_to_cwd(&script), controller, RATE).unwrap();
    main.set_socket_dir(socket_dir(&script));
    main.set_terminals(
        Some(recorder(
            &out,
            &["--directory", "/no/existe/esta/carpeta", "-e"],
        )),
        None,
    );
    main.run_action(Action::EditorWindow);
    let message = main.view().message;
    assert!(message.contains("/no/existe/esta/carpeta"), "{message}");
    assert!(message.contains("no existe"), "{message}");
    std::thread::sleep(Duration::from_millis(200));
    assert!(!out.exists(), "se lanzó igual");

    // Con una carpeta que existe (y otra que no es la del proyecto), se
    // lanza con la ruta completa: la carpeta de la terminal no importa.
    let script = fixture("ew_directory_ok", Some(SONG));
    let out = script.with_file_name("lanzado.txt");
    let (controller, _engine) = new_engine(RATE);
    let mut main = App::open(&relative_to_cwd(&script), controller, RATE).unwrap();
    main.set_socket_dir(socket_dir(&script));
    let home = elsewhere().display().to_string();
    let word = format!("--directory={home}");
    main.set_terminals(Some(recorder(&out, &[&word, "-e"])), None);
    main.run_action(Action::EditorWindow);
    let arg = launched(&out);
    assert!(Path::new(&arg).is_absolute(), "{arg}");
    let (mut editor, _idle) = editor_in(Path::new(&home), &arg, &socket_dir(&script)).unwrap();
    pump_until(&mut main, &mut editor, |_, e| {
        e.view().message.contains("historial")
    });
    assert_eq!(editor.text_for_test(), main.text_for_test());
}
