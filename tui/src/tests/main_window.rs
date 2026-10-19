//! La ventana principal con el editor en la suya (fase L, parte 2): las
//! vistas ocupan toda la ventana. Con una sola ventana, nada cambia.

use ratatui::style::Color;

use super::split::{editor_window, main_window, pump};
use super::{SONG, draw, draw_widget, find, fixture, open, text, tracker_view, waveform_view};
use crate::ui;

#[test]
fn with_the_editor_outside_the_views_take_the_whole_window() {
    let path = fixture("main_window_layout", Some(SONG));
    let (mut main, _engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    let buffer = draw(&mut main, 120, 40);
    let screen = text(&buffer);
    // Arriba, el transporte y el enlace; ni rastro del texto del script.
    assert!(screen.starts_with("■ parado   120.0 BPM"), "{screen}");
    assert!(
        screen
            .lines()
            .next()
            .unwrap()
            .contains("song.lua · ⇄ editor en su ventana")
    );
    assert!(!screen.contains("bpm(120)"), "{screen}");
    // El sample, de lado a lado.
    assert_eq!(find(&buffer, "┌ sample · drums"), Some((0, 1)), "{screen}");
    assert_eq!(buffer[(119, 1)].symbol(), "┐");

    // Con una sola ventana, el layout de siempre: el editor a la izquierda
    // y el sample a la derecha.
    let (_, mut single, _engine) = open("main_window_single", Some(SONG));
    let screen = text(&draw(&mut single, 120, 40));
    assert!(screen.starts_with("┌ song.lua"), "{screen}");
    assert!(screen.contains("bpm(120)"), "{screen}");
    let at = find(&draw(&mut single, 120, 40), "┌ sample · drums").unwrap();
    assert_eq!(at.0, 60);
}

#[test]
fn the_big_waveform_numbers_the_slices_on_their_own_row() {
    let mut view = waveform_view(Some(1));
    view.selected_slice = Some(0);
    // 80 de ancho útil y 10 de alto: números, 8 filas de onda y los ms.
    let buffer = draw_widget(ui::BigSamplePanel(&view), 82, 12);
    let screen = text(&buffer);
    assert!(
        screen.contains("sample · drums · break.wav · 0.12 s · 2 cortes"),
        "{screen}"
    );
    // Fila de números: el 1 al principio (elegido: negro sobre amarillo) y
    // el 2 en la mitad (sonando: negro sobre el acento).
    assert_eq!(buffer[(1, 1)].symbol(), "1");
    assert_eq!(buffer[(1, 1)].bg, Color::Yellow);
    assert_eq!(buffer[(41, 1)].symbol(), "2");
    assert_eq!(buffer[(41, 1)].bg, Color::Cyan);
    // La onda va debajo: en su primera fila no hay números, solo marcas.
    assert_eq!(buffer[(1, 2)].symbol(), "│", "{screen}");
    assert_eq!(buffer[(41, 2)].symbol(), "│", "{screen}");
    // Bloques de la onda en las 8 filas de en medio.
    let wave_rows = 2..10u16;
    assert!(
        (2..10u16)
            .flat_map(|x| wave_rows.clone().map(move |y| (x, y)))
            .any(|p| buffer[p].symbol() == "█"),
        "{screen}"
    );
    // Al pie, dónde empieza cada corte en ms.
    let times: String = (1..81u16).map(|x| buffer[(x, 10)].symbol()).collect();
    assert!(times.starts_with("0 "), "{times:?}");
    assert_eq!(&times[40..42], "62", "{times:?}");

    // Sin corte elegido ni sonando, los números van en amarillo sin fondo.
    let buffer = draw_widget(ui::BigSamplePanel(&waveform_view(None)), 82, 12);
    assert_eq!(buffer[(1, 1)].fg, Color::Yellow);
    assert_ne!(buffer[(1, 1)].bg, Color::Yellow);
    // Muy bajo, como el panel pequeño (números encima de la onda).
    let buffer = draw_widget(ui::BigSamplePanel(&waveform_view(None)), 82, 5);
    assert_eq!(buffer[(41, 1)].symbol(), "2");
}

/// Una vista de tracker con `n` pistas de 16 pasos (golpe en el paso 0).
fn many_tracks(n: usize) -> crate::model::ViewModel {
    let mut view = tracker_view(Some(4), true);
    let template = view.tracks[0].clone();
    view.tracks = (0..n)
        .map(|i| crate::model::TrackView {
            name: format!("pista{}", i + 1),
            ..template.clone()
        })
        .collect();
    view.selected = Some(0);
    view
}

#[test]
fn the_full_tracker_fits_every_track_widening_or_narrowing_the_columns() {
    // Dos pistas en 100 de ancho: columnas de 16, el máximo.
    assert_eq!(ui::FullTrackerPanel::columns(2, 100), (16, 2));
    // Ocho en 70: se estrechan a 8 y caben todas.
    assert_eq!(ui::FullTrackerPanel::columns(8, 70), (8, 8));
    // Ocho en 40: 6 como mucho, caben 6.
    assert_eq!(ui::FullTrackerPanel::columns(8, 40), (6, 6));
    assert_eq!(ui::FullTrackerPanel::columns(0, 40).1, 0);

    let view = many_tracks(8);
    let buffer = draw_widget(ui::FullTrackerPanel(&view), 72, 20);
    let screen = text(&buffer);
    assert!(
        screen.contains("tracker · compás 3 · 8 pistas "),
        "{screen}"
    );
    // Las ocho cabeceras, cada una en su columna de 8 (tras los 3 del
    // número de paso).
    let header = screen.lines().nth(1).unwrap();
    for i in 0..8 {
        let at = 1 + 3 + i * 8;
        let name: String = header.chars().skip(at).take(6).collect();
        assert_eq!(name, format!("pista{}", i + 1), "{header}");
    }
    // El paso 0 con su golpe en todas, y la fila del paso 4 resaltada
    // entera (todas van por ahí).
    let row0 = row(&screen, "00 ");
    assert_eq!(row0.matches("01").count(), 8, "{row0}");
    let y = screen
        .lines()
        .position(|l| l[3..].starts_with("04 "))
        .unwrap() as u16;
    assert_eq!(buffer[(30, y)].bg, Color::Blue);
    assert_ne!(buffer[(30, y + 1)].bg, Color::Blue);

    // Sin sitio para todas, el título dice cuántas se ven.
    let buffer = draw_widget(ui::FullTrackerPanel(&many_tracks(8)), 42, 20);
    let screen = text(&buffer);
    assert!(screen.contains("compás 3 · 6 de 8 pistas"), "{screen}");
    // Seis columnas de 6 (los nombres, recortados).
    assert_eq!(
        screen.lines().nth(1).unwrap().matches("pista ").count(),
        6,
        "{screen}"
    );

    // El tracker de una sola ventana no cambia: columnas de 11.
    let buffer = draw_widget(ui::TrackerPanel(&tracker_view(None, false)), 40, 20);
    let header = text(&buffer).lines().nth(1).unwrap().to_string();
    assert_eq!(
        header.chars().skip(4 + 11).take(4).collect::<String>(),
        "kick"
    );
}

/// La línea que empieza (tras el borde) por `prefix`.
fn row(screen: &str, prefix: &str) -> String {
    screen
        .lines()
        .find(|l| l.chars().skip(1).collect::<String>().starts_with(prefix))
        .unwrap()
        .to_string()
}

#[test]
fn the_main_window_shows_the_full_tracker() {
    let path = fixture("main_window_tracker", Some(SONG));
    let (mut main, _engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    let screen = text(&draw(&mut main, 120, 40));
    assert!(screen.contains("tracker · compás 0 · 2 pistas"), "{screen}");
    // En una sola ventana, el título de siempre.
    let (_, mut single, _engine) = open("main_window_tracker_single", Some(SONG));
    let screen = text(&draw(&mut single, 120, 40));
    assert!(screen.contains("tracker · compás 0 ─"), "{screen}");
}
