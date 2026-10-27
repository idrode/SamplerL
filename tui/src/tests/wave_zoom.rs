//! La vista del sample: zoom (con `+`/`-`, Shift-←→ y la rueda del ratón)
//! y crear y quitar cortes (`n` y Supr en el modo tocar, clic derecho en la
//! onda o en una marca).

use super::*;
use crate::app::MouseAction;

/// Abre SONG como proyecto (para que las ediciones de cortes se guarden),
/// en el modo tocar.
fn open_playing(test: &str) -> (PathBuf, App, Engine) {
    let path = fixture(test, Some(SONG));
    let project = path.parent().unwrap().join("p.sampler");
    std::fs::write(&project, "script = song.lua\n").unwrap();
    let (controller, engine) = new_engine(RATE);
    let mut app = App::open_project(&project, controller, RATE).unwrap();
    app.handle(Input {
        key: Key::Esc,
        ..Input::default()
    });
    (project, app, engine)
}

fn shift(key: Key) -> Input {
    Input {
        key,
        shift: true,
        ..Input::default()
    }
}

/// Columna de la terminal del frame `frame` con lo que se ve ahora.
fn column_of(app: &App, frame: usize) -> u16 {
    let area = app.sample_area.expect("se dibujó el sample");
    let window = app.view().tracks[0].window.clone().unwrap_or(0..6000);
    let fraction = (frame - window.start) as f64 / window.len() as f64;
    area.x + (fraction * area.width as f64).floor() as u16
}

#[test]
fn the_keys_zoom_around_the_selected_slice_and_pan() {
    let (_, mut app, _engine) = open_playing("zoom_keys");
    assert_eq!(app.view().tracks[0].window, None);
    app.handle(key('+'));
    // La mitad, con el corte 1 (en 0) lo más centrado que se puede.
    assert_eq!(app.view().tracks[0].window, Some(0..3000));
    assert!(
        app.view().message.starts_with("zoom ×2"),
        "{}",
        app.view().message
    );
    // Sigue al corte seleccionado.
    app.handle(Input {
        key: Key::Right,
        ..Input::default()
    });
    assert_eq!(app.view().tracks[0].window, Some(1500..4500));
    app.handle(key('+'));
    assert_eq!(app.view().tracks[0].window, Some(2250..3750));
    // El título lo dice (break.wav: 48 kHz, 6000 frames).
    let screen = text(&draw(&mut app, 160, 40));
    assert!(screen.contains("zoom ×4 47-78 ms"), "{screen}");
    // Shift-→ desplaza un cuarto.
    app.handle(shift(Key::Right));
    assert_eq!(app.view().tracks[0].window, Some(2625..4125));
    app.handle(shift(Key::Left));
    app.handle(shift(Key::Left));
    assert_eq!(app.view().tracks[0].window, Some(1875..3375));
    // Hasta el máximo (256 frames) y de vuelta al sample entero.
    for _ in 0..10 {
        app.handle(key('+'));
    }
    assert_eq!(
        app.view().tracks[0].window.clone().map(|w| w.len()),
        Some(256)
    );
    assert_eq!(app.view().message, "zoom al máximo");
    for _ in 0..10 {
        app.handle(key('-'));
    }
    assert_eq!(app.view().tracks[0].window, None);
    // En otra pista se ve entera.
    app.handle(key('+'));
    app.next_track();
    let view = app.view();
    assert_eq!(
        view.tracks
            .iter()
            .map(|t| t.window.clone())
            .collect::<Vec<_>>(),
        [None, None]
    );
}

#[test]
fn the_zoomed_wave_draws_the_marks_where_they_are() {
    let (_, mut app, _engine) = open_playing("zoom_draw");
    app.handle(Input {
        key: Key::Right,
        ..Input::default()
    });
    app.handle(key('+'));
    // De 1500 a 4500: la marca del corte 2 (en 3000), en el centro.
    let buffer = draw(&mut app, 160, 40);
    let area = app.sample_area.unwrap();
    let middle = area.x + area.width / 2;
    assert_eq!(buffer[(middle, area.y)].symbol(), "2");
    // La del corte 1 (en 0) no se ve.
    let first_row: String = (area.x..area.right())
        .map(|x| buffer[(x, area.y)].symbol().to_string())
        .collect();
    assert!(!first_row.contains('1'), "{first_row}");
    // El resumen es del trozo: con su resolución entera.
    assert_eq!(app.view().tracks[0].waveform.buckets.len(), 2048);
}

#[test]
fn the_wheel_zooms_around_the_pointer_and_the_view_stays_there() {
    let (_, mut app, _engine) = open_playing("zoom_wheel");
    draw(&mut app, 160, 40);
    let area = app.sample_area.unwrap();
    let y = area.y + 1;
    let x = column_of(&app, 4500);
    app.mouse(MouseAction::ScrollUp, x, y);
    let window = app.view().tracks[0].window.clone().expect("zoom");
    assert_eq!(window.len(), 3000);
    // El punto apuntado se queda (casi) bajo el ratón.
    draw(&mut app, 160, 40);
    assert!(column_of(&app, 4500).abs_diff(x) <= 1, "{window:?}");
    // Un clic en la onda selecciona sin mover la vista.
    app.mouse(MouseAction::Down, area.x + 2, y);
    app.mouse(MouseAction::Up, area.x + 2, y);
    assert_eq!(app.view().tracks[0].window, Some(window.clone()));
    // Shift y la rueda (o la rueda horizontal) desplazan.
    app.mouse(MouseAction::ScrollLeft, x, y);
    assert_eq!(
        app.view().tracks[0].window,
        Some(window.start - 750..window.end - 750)
    );
    // Hacia abajo, aleja hasta verlo entero.
    app.mouse(MouseAction::ScrollDown, x, y);
    assert_eq!(app.view().tracks[0].window, None);
    // Con la paleta abierta, la rueda no hace nada.
    app.open_palette();
    app.mouse(MouseAction::ScrollUp, x, y);
    assert_eq!(app.view().tracks[0].window, None);
}

#[test]
fn a_right_click_adds_a_slice_there_and_on_a_mark_removes_it() {
    let (project, mut app, _engine) = open_playing("zoom_right_click");
    // Con zoom, para ponerlo con más precisión.
    app.handle(Input {
        key: Key::Right,
        ..Input::default()
    });
    app.handle(key('+'));
    app.handle(key('+'));
    draw(&mut app, 160, 40);
    let area = app.sample_area.unwrap();
    let y = area.y + 1;
    // 2250..3750 en 158 columnas: unos 9,5 frames cada una.
    let x = column_of(&app, 3600);
    app.mouse(MouseAction::RightDown, x, y);
    let view = app.view();
    assert_eq!(view.tracks[0].slices.len(), 3);
    let new = view.tracks[0].slices[2].start;
    assert!(new.abs_diff(3600) <= 10, "{new}");
    assert_eq!(view.selected_slice, Some(2));
    assert!(
        view.message
            .starts_with("corte nuevo en drums: el 3 empieza en"),
        "{}",
        view.message
    );
    // La vista no salta.
    assert_eq!(view.tracks[0].window, Some(2250..3750));
    let saved = std::fs::read_to_string(&project).unwrap();
    assert!(saved.contains("partir = break.wav 2 "), "{saved}");
    // Clic derecho en su marca: lo quita.
    draw(&mut app, 160, 40);
    app.mouse(MouseAction::RightDown, column_of(&app, new), y);
    let view = app.view();
    assert_eq!(view.tracks[0].slices, [0..3000, 3000..6000]);
    assert!(view.message.contains("unido al 2"), "{}", view.message);
    let saved = std::fs::read_to_string(&project).unwrap();
    assert!(saved.contains("unir = break.wav 2 3"), "{saved}");
    // Antes del primer corte (que empieza en 0) no cabe nada; la marca
    // del 1 no se quita.
    app.handle(key('-'));
    app.handle(key('-'));
    draw(&mut app, 160, 40);
    app.mouse(MouseAction::RightDown, area.x, y);
    assert!(
        app.view().message.contains("no tiene anterior"),
        "{}",
        app.view().message
    );
    assert_eq!(app.view().tracks[0].slices.len(), 2);
}

#[test]
fn n_splits_the_selected_slice_and_delete_removes_it() {
    let (_, mut app, _engine) = open_playing("zoom_keys_edit");
    app.handle(key('n'));
    let view = app.view();
    assert_eq!(view.tracks[0].slices, [0..1500, 1500..3000, 3000..6000]);
    assert_eq!(view.selected_slice, Some(1));
    for key in [Key::Delete, Key::Backspace] {
        app.handle(Input {
            key,
            ..Input::default()
        });
    }
    // Supr quita el 2 (el nuevo); Retroceso, con el 1 seleccionado, no
    // puede.
    let view = app.view();
    assert_eq!(view.tracks[0].slices, [0..3000, 3000..6000]);
    assert!(
        view.message.contains("no tiene anterior"),
        "{}",
        view.message
    );
}
