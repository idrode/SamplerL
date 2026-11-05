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
    // La onda arriba, debajo las pestañas con el tracker elegido (el
    // primero) y el editor abajo, a lo ancho (sin explorador: no cabe).
    assert_eq!(find(&buffer, "┌ sample"), Some((0, 2)), "{screen}");
    let (_, strip) = find(&buffer, " 3 tracker ").expect(&screen);
    let (x, _) = find(&buffer, " 4 mezcla").expect(&screen);
    assert_eq!(buffer[(x + 1, strip)].fg, theme::get().dim);
    let (tx, ty) = find(&buffer, "┌ tracker").expect(&screen);
    assert_eq!((tx, ty), (0, strip + 1), "{screen}");
    assert!(!screen.contains("┌ mezclador"), "{screen}");
    let (ex, ey) = find(&buffer, "┌ song.lua").expect(&screen);
    assert!(ex == 0 && ey > ty, "{screen}");
    assert!(!screen.contains("archivos"), "{screen}");
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
fn wide_has_the_sound_a_column_for_the_master_and_the_editor_below() {
    let (_, mut app, _engine) = open("screen_wide", Some(SONG));
    let buffer = draw(&mut app, 180, 50);
    let screen = text(&buffer);
    let (sx, sy) = find(&buffer, "┌ sample").expect(&screen);
    let (mx, my) = find(&buffer, "┌ mezclador").expect(&screen);
    let (ex, ey) = find(&buffer, "┌ song.lua").expect(&screen);
    assert!(sx < mx && sy == my, "{screen}");
    // El editor abajo, a todo el ancho del centro.
    assert!(ex == sx && ey > my, "{screen}");
    // La columna de la derecha: mezclador, scope y espectro del master.
    let (cx, cy) = find(&buffer, "┌ 7 scope").expect(&screen);
    let (px, py) = find(&buffer, "┌ 8 espectro").expect(&screen);
    assert!(cx == mx && px == mx && my < cy && cy < py, "{screen}");
    // Los efectos, en su pestaña (la primera).
    let (_, strip) = find(&buffer, " 5 efectos ").expect(&screen);
    assert_eq!(
        find(&buffer, "┌ efectos por pista").map(|p| p.1),
        Some(strip + 1),
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
    assert!(line(&screen, 1).starts_with("----------------"), "{screen}");
    assert!(!screen.contains('▰') && !screen.contains('▱'), "{screen}");
    // Ni cajas ni bloques ni flechas.
    for c in ['┌', '─', '│', '█', '▶', '■', '·'] {
        assert!(!screen.contains(c), "{c}: {screen}");
    }
    assert!(screen.starts_with("# parado"), "{screen}");
    // A 60 de ancho, en la mitad derecha, las pestañas van con el nombre
    // corto.
    assert!(screen.contains(" 3 trk "), "{screen}");
    // Las que no caben, con la marca (en ASCII, `>`).
    let screen = text(&draw(&mut app, 44, 20));
    theme::set_ascii(false);
    let strip = screen.lines().find(|l| l.contains(" 3 trk ")).unwrap();
    assert!(strip.ends_with('>'), "{screen}");
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
    // Al verse sus paneles, la interfaz enciende la cola del master: entra
    // al compás.
    draw(&mut app, 180, 50);
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
    // En el tamaño mediano, el scope en su pestaña y el espectro en su
    // panel, junto a las pestañas.
    let screen = text(&draw(&mut app, 120, 40));
    assert!(
        screen.contains(" 7 scope ") && screen.contains("┌ 8 espectro · master · relleno"),
        "{screen}"
    );
}

#[test]
fn the_master_spectrum_panel_draws_dots_an_axis_and_a_spectrogram() {
    use crate::config::Config;
    let (_, mut app, mut engine) = open("screen_master_spectrum", Some(SONG));
    draw(&mut app, 120, 40);
    play(&mut app, &mut engine, 16);
    // Un cuadro dibujado cada octavo de paso (los golpes son cortos): lo
    // que suena sube y cae despacio.
    for _ in 0..32 {
        offline::render(&mut engine, STEP / 8, 2);
        app.tick();
        draw(&mut app, 120, 40);
    }
    let buffer = draw(&mut app, 120, 40);
    let screen = text(&buffer);
    let area = app.last_grid().unwrap().spectrum.unwrap();
    let inner = |buffer: &Buffer| -> String {
        (area.y + 1..area.bottom() - 1)
            .map(|y| {
                (area.x + 1..area.right() - 1)
                    .map(|x| buffer[(x, y)].symbol().to_string())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    let plot = inner(&buffer);
    // Puntos, y el eje de frecuencias en la última fila.
    assert!(
        plot.chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)),
        "{screen}"
    );
    let axis = plot.lines().last().unwrap();
    assert!(
        axis.contains("100") && axis.contains("1k") && axis.contains("10k"),
        "{axis}"
    );
    // En barras (lo que menos gasta), bloques y no puntos.
    app.set_config(&Config::parse(
        "[interfaz]\nespectro = \"barras\"\n",
        true,
        "c",
    ));
    let plot = inner(&draw(&mut app, 120, 40));
    assert!(
        !plot.chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)),
        "{plot}"
    );
    assert!(plot.contains('█') || plot.contains('▄'), "{plot}");
    // En espectrograma, el historial crece con cada cuadro dibujado.
    app.set_config(&Config::parse(
        "[interfaz]\nespectro = \"espectrograma\"\n",
        true,
        "c",
    ));
    for _ in 0..30 {
        offline::render(&mut engine, STEP / 8, 2);
        app.tick();
        draw(&mut app, 120, 40);
    }
    let view = app.view().master_spectrum.unwrap();
    assert_eq!(view.history.len(), 30);
    let buffer = draw(&mut app, 120, 40);
    let screen = text(&buffer);
    assert!(screen.contains("espectro · master · espectrog"), "{screen}");
    let plot = inner(&buffer);
    // Lo más nuevo, a la derecha: la columna de la izquierda, vacía.
    let left: String = plot.lines().map(|l| l.chars().next().unwrap()).collect();
    assert!(left.trim().is_empty(), "{plot}");
    assert!(
        plot.chars().any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)),
        "{plot}"
    );
    // En ASCII, sin Braille.
    theme::set_ascii(true);
    let plot = inner(&draw(&mut app, 120, 40));
    theme::set_ascii(false);
    assert!(plot.is_ascii(), "{plot}");
}

#[test]
fn a_reload_keeps_the_master_for_its_panels() {
    let (_, mut app, mut engine) = open("screen_master_reload", Some(SONG));
    draw(&mut app, 180, 50);
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
fn the_master_is_only_copied_while_its_panels_are_seen() {
    let (_, mut app, mut engine) = open("screen_master_hidden", Some(SONG));
    // Sin dibujar (o sin su panel a la vista), el motor no copia el master.
    play(&mut app, &mut engine, 20);
    assert!(app.view().master_views.is_empty());
    // En el tamaño pequeño, el scope y el espectro son pestañas: con otra
    // elegida no se ven.
    app.show_tab(crate::layout::Tab::Panel(crate::focus::Panel::Mixer));
    draw(&mut app, 80, 24);
    play(&mut app, &mut engine, 20);
    assert!(app.view().master_views.is_empty());
    // Con la del espectro, sí.
    app.show_tab(crate::layout::Tab::Panel(crate::focus::Panel::Spectrum));
    draw(&mut app, 80, 24);
    play(&mut app, &mut engine, 20);
    assert_eq!(app.view().master_views.len(), 2);
    // Al dejar de verse se apaga y se olvida lo que había.
    app.show_tab(crate::layout::Tab::Panel(crate::focus::Panel::Mixer));
    draw(&mut app, 80, 24);
    play(&mut app, &mut engine, 1);
    assert!(app.view().master_views.is_empty());
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

/// Para ver la degradación a ASCII:
/// `cargo test -p tui pantalla_ascii -- --ignored --nocapture`.
#[test]
#[ignore = "solo para ver la pantalla"]
fn pantalla_ascii() {
    let (_, mut app, mut engine) = open("pantalla_ascii", Some(SONG));
    offline::render(&mut engine, 5 * STEP + 100, 2);
    app.tick();
    theme::set_ascii(true);
    for (w, h) in [(60, 20), (120, 40)] {
        println!("{w}×{h}\n{}\n", text(&draw(&mut app, w, h)));
    }
    theme::set_ascii(false);
}

#[test]
fn the_tab_strip_scrolls_to_show_the_chosen_tab() {
    let (_, mut app, _engine) = open("screen_tabs_scroll", Some(SONG));
    let screen = text(&draw(&mut app, 40, 24));
    assert!(screen.contains('›'), "hay más a la derecha: {screen}");
    // Alt-8 (espectro): su pestaña se ve, elegida.
    app.handle(Input {
        key: Key::Char('8'),
        alt: true,
        ..Input::default()
    });
    let buffer = draw(&mut app, 40, 24);
    let screen = text(&buffer);
    let (x, y) = find(&buffer, " 8 esp").unwrap_or_else(|| panic!("{screen}"));
    assert_eq!(buffer[(x + 1, y)].bg, theme::get().accent, "{screen}");
    assert!(screen.contains('‹'), "hay más a la izquierda: {screen}");
}
