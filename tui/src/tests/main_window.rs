//! La ventana principal con el editor en la suya (fase L, parte 2): las
//! vistas ocupan toda la ventana. Con una sola ventana, nada cambia.

use ratatui::style::Color;

use super::split::{editor_window, main_window, pump};
use super::{SONG, draw, draw_widget, find, fixture, open, text, waveform_view};
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
