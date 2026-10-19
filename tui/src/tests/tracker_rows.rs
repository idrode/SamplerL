//! El tracker enseña sus 16 filas sin desplazarse en cuanto caben dejando
//! al sample lo mínimo; si no caben, el reparto de siempre (se desplaza).

use super::split::{editor_window, main_window, pump};
use super::*;

/// Números de fila que se ven en el tracker de la pantalla.
fn tracker_rows(screen: &str) -> Vec<usize> {
    let lines: Vec<Vec<char>> = screen.lines().map(|l| l.chars().collect()).collect();
    let (top, x) = lines
        .iter()
        .enumerate()
        .find_map(|(i, l)| {
            let s: String = l.iter().collect();
            let byte = s.find("┌ tracker")?;
            Some((i, s[..byte].chars().count()))
        })
        .expect("sin tracker");
    // Debajo del título, la cabecera; luego las filas hasta el borde.
    lines[top + 2..]
        .iter()
        .take_while(|l| l.get(x) == Some(&'│'))
        .filter_map(|l| {
            let n: String = l[x + 1..x + 3].iter().collect();
            n.parse().ok()
        })
        .collect()
}

/// Alto del panel del sample (con sus bordes) en una sola ventana.
fn sample_height(screen: &str) -> usize {
    let lines: Vec<&str> = screen.lines().collect();
    let top = lines.iter().position(|l| l.contains("┌ sample")).unwrap();
    let bottom = lines.iter().position(|l| l.contains("┌ tracker")).unwrap();
    bottom - top
}

#[test]
fn a_single_window_shows_the_16_rows_as_soon_as_they_fit() {
    let (_, mut app, _engine) = open("tracker_rows_single", Some(SONG));
    let all: Vec<usize> = (0..16).collect();
    // Con sitio de sobra, o quitándole al sample hasta dejarle 4 filas.
    for height in [40u16, 42, 44, 50] {
        let screen = text(&draw(&mut app, 120, height));
        assert_eq!(tracker_rows(&screen), all, "alto {height}\n{screen}");
        assert!(sample_height(&screen) >= 4, "alto {height}");
    }
    assert_eq!(sample_height(&text(&draw(&mut app, 120, 40))), 5);
    // Sin sitio ni así: el sample conserva sus 6 filas y el tracker se
    // desplaza, como siempre.
    for height in [30u16, 34, 38] {
        let screen = text(&draw(&mut app, 120, height));
        let rows = tracker_rows(&screen);
        assert!(rows.len() < 16 && rows[0] == 0, "alto {height}: {rows:?}");
        assert_eq!(sample_height(&screen), 6, "alto {height}");
    }
}

#[test]
fn the_main_window_shows_the_16_rows_as_soon_as_they_fit() {
    let path = fixture("tracker_rows_main", Some(SONG));
    let (mut main, _engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    let all: Vec<usize> = (0..16).collect();
    for height in [36u16, 38, 40, 50] {
        let screen = text(&draw(&mut main, 120, height));
        assert_eq!(tracker_rows(&screen), all, "alto {height}\n{screen}");
    }
    for height in [28u16, 32, 34] {
        let rows = tracker_rows(&text(&draw(&mut main, 120, height)));
        assert!(rows.len() < 16, "alto {height}: {rows:?}");
    }
}
