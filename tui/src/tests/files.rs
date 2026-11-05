//! El explorador de archivos: el árbol en su columna (o encima, en una
//! terminal estrecha), sus teclas con el foco (Alt-0), el ratón, y lo que
//! hace con cada archivo.

use super::*;
use crate::app::MouseAction;
use crate::files_panel::{FilesHit, FilesPanel, FilesView};
use crate::theme;

fn alt(c: char) -> Input {
    Input {
        key: Key::Char(c),
        alt: true,
        ..Input::default()
    }
}

/// SONG con una carpeta `scripts` con otro script.
fn open_tree(test: &str) -> (PathBuf, App, Engine) {
    let (path, app, engine) = open(test, Some(SONG));
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir.join("scripts")).unwrap();
    std::fs::write(
        dir.join("scripts/otro.lua"),
        "bpm(90)\nlocal amen = sample(\"../break.wav\"):slice(2)\ntrack(\"solo\", amen):steps(\"1 . . .  1 . . .  1 . . .  1 . . .\")\n",
    )
    .unwrap();
    (path, app, engine)
}

/// La fila `name` del árbol dibujado.
fn row_of(buffer: &Buffer, name: &str) -> (u16, u16) {
    find(buffer, name).unwrap_or_else(|| panic!("no se ve «{name}»\n{}", text(buffer)))
}

#[test]
fn the_tree_draws_folded_and_unfolded_with_the_selection() {
    let (path, mut app, _engine) = open_tree("files_draw");
    let buffer = draw(&mut app, 120, 40);
    let screen = text(&buffer);
    let files = app.last_grid().unwrap().explorer.unwrap();
    assert_eq!((files.x, files.y), (0, 3));
    assert!(screen.contains("┌ 0 archivos"), "{screen}");
    // La raíz abierta, la carpeta cerrada y el script del editor marcado.
    assert!(screen.contains("▾ tui_files_draw"), "{screen}");
    assert!(screen.contains("▸ scripts"), "{screen}");
    assert!(screen.contains("♪ break.wav"), "{screen}");
    assert!(screen.contains("λ song.lua ◂"), "{screen}");
    assert!(!screen.contains("otro.lua"), "{screen}");
    // Con el foco, la elegida en el acento; abrir la carpeta la despliega.
    app.handle(alt('0'));
    assert_eq!(app.focus(), crate::focus::Panel::Files);
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = row_of(&buffer, "song.lua");
    assert_eq!(buffer[(x, y)].bg, theme::get().accent);
    app.handle(key('k'));
    app.handle(key('k'));
    app.handle(special(Key::Right));
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("▾ scripts"), "{screen}");
    assert!(screen.contains("λ otro.lua"), "{screen}");
    // Widget suelto: sin filas, sin romperse; en ASCII, iconos ASCII.
    let empty = text(&draw_widget(FilesPanel(&FilesView::default()), 20, 5));
    assert!(empty.contains("archivos"), "{empty}");
    theme::set_ascii(true);
    let screen = text(&draw(&mut app, 120, 40));
    theme::set_ascii(false);
    assert!(
        screen.contains("v scripts") && screen.contains("s otro.lua"),
        "{screen}"
    );
    let _ = path;
}

#[test]
fn enter_on_a_script_opens_it_and_esc_gives_the_focus_back() {
    let (path, mut app, _engine) = open_tree("files_open");
    draw(&mut app, 120, 40);
    app.handle(alt('0'));
    // Arriba del todo, a scripts, abrirla y bajar a otro.lua.
    app.handle(special(Key::Home));
    app.handle(special(Key::Down));
    app.handle(special(Key::Enter));
    app.handle(special(Key::Down));
    app.handle(special(Key::Enter));
    let other = path.parent().unwrap().join("scripts/otro.lua");
    assert_eq!(app.path_for_test(), other.as_path());
    assert!(app.text_for_test().starts_with("bpm(90)"));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    assert_eq!(app.tracks()[0].name, "solo");
    // Esc devuelve el foco a donde estaba (el editor).
    app.handle(esc());
    assert_eq!(app.focus(), crate::focus::Panel::Code);
    // Con cambios sin guardar, hay que pedirlo dos veces.
    app.handle(key('x'));
    // song.lua en el árbol, con el ratón.
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = row_of(&buffer, "song.lua");
    app.mouse(MouseAction::Down, x, y);
    app.mouse(MouseAction::Up, x, y);
    app.handle(special(Key::Enter));
    assert!(
        app.view().message.contains("cambios sin guardar"),
        "{}",
        app.view().message
    );
    assert_eq!(app.path_for_test(), other.as_path());
    app.handle(special(Key::Enter));
    assert_eq!(app.path_for_test(), path.as_path());
}

#[test]
fn a_sample_previews_shows_in_the_wave_and_a_writes_it() {
    let (_, mut app, _engine) = open_tree("files_sample");
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = row_of(&buffer, "♪ break.wav");
    app.mouse(MouseAction::Down, x, y);
    app.mouse(MouseAction::Up, x, y);
    assert_eq!(app.focus(), crate::focus::Panel::Files);
    app.handle(key(' '));
    assert!(
        app.view().message.contains("break.wav · 0.12 s"),
        "{}",
        app.view().message
    );
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("vista previa · break.wav"), "{screen}");
    // Al pie del árbol, el mismo sample.
    assert!(screen.contains("break.wav · 0.12 s"), "{screen}");
    // `a` lo escribe en el cursor del editor.
    app.handle(key('a'));
    assert!(app.text_for_test().starts_with("sample(\"break.wav\")"));
    // Sin el foco en el árbol, la onda vuelve a ser la de la pista.
    app.handle(esc());
    let screen = text(&draw(&mut app, 120, 40));
    assert!(!screen.contains("vista previa"), "{screen}");
}

#[test]
fn the_mouse_toggles_folders_and_drags_the_edge() {
    let (_, mut app, _engine) = open_tree("files_mouse");
    let buffer = draw(&mut app, 120, 40);
    let layout = app.files_layout_for_test().unwrap();
    let (x, y) = row_of(&buffer, "▸ scripts");
    assert_eq!(layout.hit(x, y), Some(FilesHit::Row(1)));
    // Clic en el icono: la abre.
    app.mouse(MouseAction::Down, x, y);
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("otro.lua"), "{screen}");
    // Doble clic en el nombre: también (la cierra).
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = row_of(&buffer, "scripts");
    app.mouse(MouseAction::Down, x + 2, y);
    app.mouse(MouseAction::Down, x + 2, y);
    let screen = text(&draw(&mut app, 120, 40));
    assert!(!screen.contains("otro.lua"), "{screen}");
    // El borde derecho, arrastrado: más ancho, y el resto se aparta.
    let files = app.last_grid().unwrap().explorer.unwrap();
    let edge = files.right() - 1;
    assert_eq!(layout.hit(edge, files.y + 2), Some(FilesHit::Edge));
    app.mouse(MouseAction::Down, edge, files.y + 2);
    app.mouse(MouseAction::Drag, edge + 6, files.y + 2);
    app.mouse(MouseAction::Up, edge + 6, files.y + 2);
    draw(&mut app, 120, 40);
    let grid = app.last_grid().unwrap();
    assert_eq!(grid.explorer.unwrap().width, files.width + 6);
    assert_eq!(grid.wave.unwrap().x, files.right() + 6);
    // Y con las teclas (con el foco), más estrecho.
    app.handle(alt('0'));
    app.handle(key('<'));
    draw(&mut app, 120, 40);
    assert_eq!(
        app.last_grid().unwrap().explorer.unwrap().width,
        files.width + 4
    );
}

#[test]
fn alt_e_hides_it_and_a_narrow_terminal_opens_it_on_top() {
    let (_, mut app, _engine) = open_tree("files_toggle");
    draw(&mut app, 120, 40);
    app.handle(alt('e'));
    draw(&mut app, 120, 40);
    let grid = app.last_grid().unwrap();
    assert!(grid.explorer.is_none());
    assert_eq!(grid.wave.unwrap().x, 0);
    app.handle(alt('e'));
    draw(&mut app, 120, 40);
    assert!(app.last_grid().unwrap().explorer.is_some());
    // A 80 columnas no cabe: no se ve, y Alt-E lo abre encima con el foco.
    let screen = text(&draw(&mut app, 80, 24));
    assert!(!screen.contains("0 archivos"), "{screen}");
    app.handle(alt('e'));
    let buffer = draw(&mut app, 80, 24);
    let grid = app.last_grid().unwrap();
    assert!(grid.explorer_overlay);
    assert_eq!(app.focus(), crate::focus::Panel::Files);
    let files = grid.explorer.unwrap();
    // Tapa la onda: un clic ahí es del árbol, no de la onda.
    let wave = grid.wave.unwrap();
    assert!(files.intersects(wave));
    assert!(grid.covered(crate::focus::Panel::Wave, files.x + 2, wave.y + 1));
    assert!(text(&buffer).contains("┌ 0 archivos"));
    // Esc lo cierra.
    app.handle(esc());
    draw(&mut app, 80, 24);
    assert!(app.last_grid().unwrap().explorer.is_none());
    assert_ne!(app.focus(), crate::focus::Panel::Files);
}

#[test]
fn new_rename_and_delete_with_confirmation() {
    let (path, mut app, _engine) = open_tree("files_edit");
    let dir = path.parent().unwrap().to_path_buf();
    draw(&mut app, 120, 40);
    app.handle(alt('0'));
    // Nuevo, junto al elegido (song.lua): con .lua si no lleva extensión.
    app.handle(key('n'));
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("nuevo: "), "{screen}");
    for c in "drop".chars() {
        app.handle(key(c));
    }
    app.handle(special(Key::Enter));
    assert!(dir.join("drop.lua").exists());
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("λ drop.lua"), "{screen}");
    // Renombrar el elegido (el nuevo).
    app.handle(key('r'));
    for _ in 0.."drop.lua".len() {
        app.handle(special(Key::Backspace));
    }
    for c in "fin".chars() {
        app.handle(key(c));
    }
    app.handle(special(Key::Enter));
    assert!(!dir.join("drop.lua").exists() && dir.join("fin.lua").exists());
    // Borrar: pide confirmación; otra tecla no borra.
    app.handle(special(Key::Delete));
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("¿borrar fin.lua? s/n"), "{screen}");
    app.handle(key('n'));
    assert!(dir.join("fin.lua").exists());
    app.handle(special(Key::Delete));
    app.handle(key('s'));
    assert!(!dir.join("fin.lua").exists());
    // El script del editor no se borra desde aquí.
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = row_of(&buffer, "song.lua");
    app.mouse(MouseAction::Down, x, y);
    app.handle(special(Key::Delete));
    assert!(
        app.view().message.contains("no se borra"),
        "{}",
        app.view().message
    );
    assert!(path.exists());
}
