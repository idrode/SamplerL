//! La ventana principal en cada tamaño (ver `crate::layout`): qué se ve
//! dónde, las pestañas, la degradación a ASCII y que ningún tamaño rompe el
//! dibujo.

use super::*;
use crate::theme;

/// La fila `y` de la pantalla.
fn line(screen: &str, y: usize) -> String {
    screen.lines().nth(y).unwrap_or_default().to_string()
}

#[test]
fn small_has_the_wave_on_top_and_the_rest_in_tabs() {
    let (_, mut app, _engine) = open("screen_small", Some(SONG));
    let buffer = draw(&mut app, 80, 24);
    let screen = text(&buffer);
    // Cabecera y regla arriba; teclas abajo.
    assert!(line(&screen, 0).starts_with("■ parado"), "{screen}");
    assert!(line(&screen, 1).starts_with("▱▱▱▱"), "{screen}");
    assert!(line(&screen, 23).starts_with("EDITOR"), "{screen}");
    // El editor a la izquierda, la onda a la derecha y debajo las pestañas
    // con el tracker elegido (el primero).
    assert_eq!(find(&buffer, "┌ song.lua"), Some((0, 2)), "{screen}");
    assert_eq!(find(&buffer, "┌ sample").map(|p| p.0), Some(40), "{screen}");
    let (_, strip) = find(&buffer, " 3 trk ").expect(&screen);
    let (x, _) = find(&buffer, " 4 mez").expect(&screen);
    assert_eq!(buffer[(x + 1, strip)].fg, theme::get().dim);
    let (tx, ty) = find(&buffer, "┌ tracker").expect(&screen);
    assert_eq!((tx, ty), (40, strip + 1), "{screen}");
    assert!(!screen.contains("┌ mezclador"), "{screen}");
}

#[test]
fn medium_has_the_tracker_beside_the_mixer_and_the_effects_in_tabs() {
    let (_, mut app, _engine) = open("screen_medium", Some(SONG));
    let buffer = draw(&mut app, 120, 40);
    let screen = text(&buffer);
    let (_, tracker) = find(&buffer, "┌ tracker").expect(&screen);
    let (_, mixer) = find(&buffer, "┌ mezclador").expect(&screen);
    assert_eq!(tracker, mixer, "{screen}");
    let (_, strip) = find(&buffer, " 5 efectos ").expect(&screen);
    assert!(strip > tracker, "{screen}");
    assert!(find(&buffer, "┌ efectos por pista").is_some_and(|p| p.1 == strip + 1));
    assert!(!screen.contains("referencia"), "{screen}");
}

#[test]
fn wide_has_three_columns_the_reference_and_the_effects_below() {
    let (_, mut app, _engine) = open("screen_wide", Some(SONG));
    let buffer = draw(&mut app, 180, 50);
    let screen = text(&buffer);
    let (ex, _) = find(&buffer, "┌ song.lua").expect(&screen);
    let (sx, _) = find(&buffer, "┌ sample").expect(&screen);
    let (mx, my) = find(&buffer, "┌ mezclador").expect(&screen);
    assert!(ex < sx && sx < mx, "{screen}");
    // La referencia, con las teclas del editor una por fila.
    let (rx, ry) = find(&buffer, "┌ referencia · EDITOR").expect(&screen);
    assert_eq!(rx, mx);
    assert!(ry > my);
    assert!(
        screen.contains("Ctrl-S        guardar y cargar"),
        "{screen}"
    );
    // Los efectos, a lo ancho abajo.
    assert_eq!(
        find(&buffer, "┌ efectos por pista").map(|p| p.0),
        Some(0),
        "{screen}"
    );
}

#[test]
fn a_tiny_terminal_says_the_audio_goes_on() {
    let (_, mut app, _engine) = open("screen_tiny", Some(SONG));
    let screen = text(&draw(&mut app, 39, 9));
    assert!(line(&screen, 0).starts_with("■ parado"), "{screen}");
    assert!(screen.contains("demasiado"), "{screen}");
    assert!(screen.contains("audio sigue"), "{screen}");
}

#[test]
fn ascii_draws_the_ruler_and_the_tabs_without_unicode_blocks() {
    let (_, mut app, _engine) = open("screen_ascii", Some(SONG));
    theme::set_ascii(true);
    let screen = text(&draw(&mut app, 60, 20));
    theme::set_ascii(false);
    assert!(line(&screen, 1).starts_with("----------------"), "{screen}");
    assert!(!screen.contains('▰') && !screen.contains('▱'), "{screen}");
    // A 60 de ancho, en la mitad derecha, las pestañas van con el nombre
    // corto.
    assert!(screen.contains(" 3 trk "), "{screen}");
    assert!(screen.contains(" 6 lan "), "{screen}");
}

#[test]
fn no_size_breaks_the_drawing() {
    let (_, mut app, mut engine) = open("screen_sizes", Some(SONG));
    offline::render(&mut engine, 3 * STEP, 2);
    app.tick();
    for width in [
        1u16, 2, 10, 39, 40, 59, 60, 79, 80, 99, 100, 120, 159, 160, 200, 300,
    ] {
        for height in [
            1u16, 2, 9, 10, 15, 16, 19, 20, 24, 29, 30, 40, 44, 45, 60, 90,
        ] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let outcome = crate::safe_draw::draw(&mut terminal, |frame| ui::draw(frame, &mut app));
            assert_eq!(
                outcome,
                crate::safe_draw::Outcome::Drawn,
                "{width}×{height}"
            );
        }
    }
}

/// Para ver la ventana en sus tres tamaños sin terminal:
/// `cargo test -p tui pantalla_tamanos -- --ignored --nocapture`.
#[test]
#[ignore = "solo para ver la pantalla"]
fn pantalla_tamanos() {
    let (_, mut app, mut engine) = open("pantalla_tamanos", Some(SONG));
    offline::render(&mut engine, 5 * STEP + 100, 2);
    app.tick();
    for (w, h) in [(80, 24), (120, 40), (180, 50), (60, 16)] {
        println!("{w}×{h}\n{}\n", text(&draw(&mut app, w, h)));
    }
}

/// Mueve el motor `steps` pasos de 1/8 en 1/8, con un `tick` en cada uno.
fn play(app: &mut App, engine: &mut Engine, steps: usize) {
    for _ in 0..8 * steps {
        offline::render(engine, STEP / 8, 2);
        app.tick();
    }
}

#[test]
fn the_master_scope_and_spectrum_need_nothing_in_the_script() {
    let (_, mut app, mut engine) = open("screen_master_viz", Some(SONG));
    assert!(app.view().viz.is_empty(), "el script no pide ninguna");
    assert!(app.view().master_views.is_empty());
    // La interfaz enciende la cola del master: entra al compás.
    play(&mut app, &mut engine, 20);
    let views = app.view().master_views;
    assert_eq!(views.len(), 2, "scope y espectro");
    let screen = text(&draw(&mut app, 180, 50));
    assert!(screen.contains("7 scope · master · RMS"), "{screen}");
    assert!(screen.contains("8 espectro · master"), "{screen}");
    assert!(
        screen
            .chars()
            .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)),
        "el scope en Braille: {screen}"
    );
    // En el tamaño mediano, en sus pestañas.
    let screen = text(&draw(&mut app, 120, 40));
    assert!(
        screen.contains(" 7 scope ") && screen.contains(" 8 espectro "),
        "{screen}"
    );
}

#[test]
fn a_reload_keeps_the_master_for_its_panels() {
    let (_, mut app, mut engine) = open("screen_master_reload", Some(SONG));
    play(&mut app, &mut engine, 20);
    // Otra carga con una visualización de pista: el lote del script solo
    // trae la suya; la interfaz vuelve a pedir la del master.
    let song = format!("{SONG}\ntrack(\"drums\"):scope()\n");
    replace_all(&mut app, &song);
    app.load();
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    play(&mut app, &mut engine, 20);
    // El patrón se repite cada 4 pasos: un paso después, otra ventana.
    let before = app.view().master_views;
    play(&mut app, &mut engine, 1);
    let after = app.view().master_views;
    assert_eq!(after.len(), 2);
    assert!(before != after, "el scope del master sigue moviéndose");
}

#[test]
fn a_scrolled_tracker_says_how_many_rows_are_hidden() {
    let screen = text(&draw_widget(
        ui::FullTrackerPanel(&tracker_view(Some(8), true)),
        40,
        8,
    ));
    let bottom = screen.lines().last().unwrap().to_string();
    assert!(bottom.contains('▲') && bottom.contains('▼'), "{screen}");
}
