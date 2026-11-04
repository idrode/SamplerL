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
    let (x, _) = find(&buffer, " 4 mezcla ").expect(&screen);
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
