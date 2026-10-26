//! El modo vim del editor (fase R, fase 1): Normal e Insert, `jk` para
//! salir de Insert (Esc sigue llevando al modo tocar), los movimientos,
//! las órdenes de borrar, copiar y pegar, deshacer, buscar y `:w`.

use super::*;
use crate::config::Config;
use crate::palette::Action;

fn vim_app(test: &str) -> App {
    let (_, mut app, _engine) = open(test, Some(SONG));
    let config = Config::parse("[editor]\nmodo = \"vim\"\n", true, "c.toml");
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    app.set_config(&config);
    app
}

fn keys(app: &mut App, text: &str) {
    for c in text.chars() {
        let input = if c == '\n' {
            Input {
                key: Key::Enter,
                ..Input::default()
            }
        } else {
            key(c)
        };
        app.handle(input);
    }
}

fn at(app: &App) -> (usize, usize) {
    let c = app.editor.cursor();
    (c.0, c.1)
}

fn line(app: &App, row: usize) -> String {
    app.editor.lines()[row].clone()
}

#[test]
fn normal_mode_moves_and_does_not_type() {
    let mut app = vim_app("vim_moves");
    let before = app.text_for_test();
    assert_eq!(app.view().vim, Some("NORMAL"));
    keys(&mut app, "l");
    assert_eq!(at(&app), (0, 1));
    keys(&mut app, "$");
    assert_eq!(at(&app), (0, 7), "sobre el último carácter");
    keys(&mut app, "l");
    assert_eq!(at(&app), (0, 7), "l no pasa del final");
    keys(&mut app, "0");
    assert_eq!(at(&app), (0, 0));
    keys(&mut app, "h");
    assert_eq!(at(&app), (0, 0), "h no pasa a la línea de antes");
    keys(&mut app, "jw");
    assert_eq!(at(&app), (1, 6), "w: «amen»");
    keys(&mut app, "e");
    let (row, col) = at(&app);
    assert_eq!(row, 1);
    assert_eq!(
        line(&app, 1).chars().nth(col),
        Some('n'),
        "e: final de «amen»"
    );
    keys(&mut app, "b");
    assert_eq!(at(&app), (1, 6));
    keys(&mut app, "G");
    assert_eq!(at(&app).0, 3);
    keys(&mut app, "gg");
    assert_eq!(at(&app), (0, 0));
    keys(&mut app, "k");
    assert_eq!(at(&app), (0, 0));
    assert_eq!(app.text_for_test(), before, "Normal no escribe");
    // El modo, en el borde del editor.
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains(" NORMAL "), "{screen}");
}

#[test]
fn insert_and_jk_back_to_normal() {
    let mut app = vim_app("vim_insert");
    keys(&mut app, "ix");
    assert_eq!(app.view().vim, Some("INSERT"));
    assert_eq!(line(&app, 0), "xbpm(120)");
    // jk: la j se borra y vuelve a Normal, con el cursor a la izquierda.
    keys(&mut app, "jk");
    assert_eq!(app.view().vim, Some("NORMAL"));
    assert_eq!(line(&app, 0), "xbpm(120)");
    assert_eq!(at(&app), (0, 0));
    // Una j sola se queda.
    keys(&mut app, "Ajx");
    assert_eq!(line(&app, 0), "xbpm(120)jx");
    keys(&mut app, "jk");
    assert_eq!(at(&app), (0, 10));
    // a: detrás del cursor.
    keys(&mut app, "0a-jk");
    assert_eq!(line(&app, 0), "x-bpm(120)jx");
    // o y O: una línea nueva debajo y encima.
    keys(&mut app, "oabajojk");
    assert_eq!(line(&app, 1), "abajo");
    keys(&mut app, "Oarribajk");
    assert_eq!(line(&app, 1), "arriba");
    assert_eq!(line(&app, 2), "abajo");
    // I: al principio de la línea (tras la sangría).
    keys(&mut app, "0i  jk");
    keys(&mut app, "I>jk");
    assert_eq!(line(&app, 1), "  >arriba");
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains(" NORMAL "), "{screen}");
}

#[test]
fn delete_yank_put_undo_and_redo() {
    let mut app = vim_app("vim_edit");
    let original: Vec<String> = app.editor.lines().to_vec();
    // dd en medio: se va la línea y se queda copiada.
    keys(&mut app, "jdd");
    assert_eq!(app.editor.lines().len(), 3);
    assert_eq!(line(&app, 1), original[2]);
    assert_eq!(at(&app), (1, 0));
    // p: debajo de la del cursor.
    keys(&mut app, "p");
    assert_eq!(line(&app, 2), original[1]);
    assert_eq!(at(&app), (2, 0));
    // u deshace el p; Ctrl-R lo rehace.
    keys(&mut app, "u");
    assert_eq!(app.editor.lines().len(), 3);
    app.handle(ctrl('r'));
    assert_eq!(line(&app, 2), original[1]);
    keys(&mut app, "uu");
    assert_eq!(app.editor.lines(), &original[..]);
    // dd en la última y P encima de la primera.
    keys(&mut app, "Gdd");
    assert_eq!(app.editor.lines().len(), 3);
    keys(&mut app, "ggP");
    assert_eq!(line(&app, 0), original[3]);
    assert_eq!(line(&app, 1), original[0]);
    // x: un carácter; p lo pone detrás del cursor.
    keys(&mut app, "jx");
    assert_eq!(line(&app, 1), "pm(120)");
    keys(&mut app, "p");
    assert_eq!(line(&app, 1), "pbm(120)");
    // yy y P: la línea entera, encima.
    keys(&mut app, "yyP");
    assert_eq!(line(&app, 1), "pbm(120)");
    assert_eq!(line(&app, 2), "pbm(120)");
    // La única línea: dd la vacía.
    let mut alone = vim_app("vim_alone");
    alone.editor.select_all();
    alone.editor.cut();
    keys(&mut alone, "iunajk");
    keys(&mut alone, "dd");
    assert_eq!(alone.editor.lines(), [""]);
}

#[test]
fn search_and_write() {
    let mut app = vim_app("vim_search");
    keys(&mut app, "/");
    assert_eq!(app.view().prompt.as_deref(), Some("buscar "));
    keys(&mut app, "track\n");
    assert_eq!(at(&app).0, 2);
    keys(&mut app, "n");
    assert_eq!(at(&app).0, 3);
    keys(&mut app, "N");
    assert_eq!(at(&app).0, 2);
    // :w guarda.
    keys(&mut app, "ggA -- vim");
    keys(&mut app, "jk");
    keys(&mut app, ":w\n");
    let saved = std::fs::read_to_string(app.path_for_test()).unwrap();
    assert!(saved.starts_with("bpm(120) -- vim\n"), "{saved}");
    keys(&mut app, ":q\n");
    assert!(
        app.view().message.contains("solo :w"),
        "{}",
        app.view().message
    );
}

#[test]
fn esc_goes_to_play_mode_from_normal_and_from_insert() {
    let mut app = vim_app("vim_esc");
    app.handle(esc());
    assert!(app.view().playing_mode);
    app.handle(esc());
    assert!(!app.view().playing_mode);
    assert_eq!(app.view().vim, Some("NORMAL"));
    // En Insert, Esc también va al modo tocar (y al volver sigue en Insert).
    keys(&mut app, "i");
    app.handle(esc());
    assert!(app.view().playing_mode);
    app.handle(esc());
    assert_eq!(app.view().vim, Some("INSERT"));
    keys(&mut app, "z");
    assert_eq!(line(&app, 0), "zbpm(120)");
    // Los atajos siguen en Normal (Ctrl-S guarda y carga).
    keys(&mut app, "jk");
    app.handle(ctrl('s'));
    let saved = std::fs::read_to_string(app.path_for_test()).unwrap();
    assert!(saved.starts_with("zbpm"), "{saved}");
}

#[test]
fn the_config_and_the_palette() {
    let config = Config::parse(
        "[editor]\nmodo = \"vim\"\nsalir_insert = \"fd\"\n",
        true,
        "c.toml",
    );
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    assert!(config.editor.vim);
    assert_eq!(config.editor.insert_exit, ['f', 'd']);
    let config = Config::parse(
        "[editor]\nmodo = \"emacs\"\nsalir_insert = \"j\"\n",
        true,
        "c.toml",
    );
    assert_eq!(config.warnings.len(), 2, "{:?}", config.warnings);
    assert!(!config.editor.vim);

    // Con otras teclas para salir de Insert.
    let (_, mut app, _engine) = open("vim_fd", Some(SONG));
    let config = Config::parse(
        "[editor]\nmodo = \"vim\"\nsalir_insert = \"fd\"\n",
        true,
        "c.toml",
    );
    app.set_config(&config);
    keys(&mut app, "iajkfd");
    assert_eq!(line(&app, 0), "ajkbpm(120)");
    assert_eq!(app.view().vim, Some("NORMAL"));

    // La paleta lo apaga y lo enciende.
    app.run_action(Action::ToggleVim);
    assert_eq!(app.view().vim, None);
    keys(&mut app, "q");
    assert_eq!(
        line(&app, 0),
        "ajqkbpm(120)",
        "el editor de siempre, en el cursor"
    );
    app.run_action(Action::ToggleVim);
    assert_eq!(app.view().vim, Some("NORMAL"));
}
