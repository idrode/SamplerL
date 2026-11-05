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

/// Alto del panel del sample (con sus bordes).
fn sample_height(screen: &str) -> usize {
    let lines: Vec<&str> = screen.lines().collect();
    let top = lines.iter().position(|l| l.contains("┌ sample")).unwrap();
    lines[top + 1..]
        .iter()
        .position(|l| l.contains('└'))
        .map_or(0, |n| n + 2)
}

#[test]
fn the_16_rows_show_as_soon_as_they_fit_in_both_windows() {
    let path = fixture("tracker_rows_main", Some(SONG));
    let (mut main, _engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    let (_, mut single, _engine) = open("tracker_rows_single", Some(SONG));
    let all: Vec<usize> = (0..16).collect();
    for (app, sizes) in [
        (&mut main, &[(120u16, 40u16), (120, 44), (180, 50), (250, 60)][..]),
        // Con el editor abajo hace falta más alto.
        (&mut single, &[(180, 56), (250, 60)][..]),
    ] {
        // Mediano y ancho con sitio: las 16 filas y al sample lo suyo.
        for &(width, height) in sizes {
            let screen = text(&draw(app, width, height));
            assert_eq!(tracker_rows(&screen), all, "{width}×{height}\n{screen}");
            assert!(sample_height(&screen) >= 6, "{width}×{height}\n{screen}");
        }
        // Mediano sin alto: se desplaza (parado, desde la primera fila).
        let screen = text(&draw(app, 100, 30));
        let rows = tracker_rows(&screen);
        assert!(
            !rows.is_empty() && rows.len() < 16 && rows[0] == 0,
            "{rows:?}\n{screen}"
        );
        // Pequeño: el tracker es la primera pestaña, y se desplaza.
        let screen = text(&draw(app, 80, 24));
        let rows = tracker_rows(&screen);
        assert!(!rows.is_empty() && rows.len() < 16, "{rows:?}\n{screen}");
    }
}
