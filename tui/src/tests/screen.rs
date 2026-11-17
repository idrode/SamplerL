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
    // La línea del compás, parada: vacía y a todo el ancho.
    assert_eq!(line(&screen, 1), "─".repeat(80), "{screen}");
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
    // La columna de la derecha: el mezclador y la referencia de teclas;
    // el scope y el espectro del master, sin pedirlos, no están.
    let (rx, ry) = find(&buffer, "┌ referencia").expect(&screen);
    assert!(rx == mx && my < ry, "{screen}");
    assert!(
        !screen.contains("scope") && !screen.contains("espectro"),
        "{screen}"
    );
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
    // Con el scope y el espectro pedidos: más pestañas que no caben.
    app.toggle_master_viz();
    app.show_tab(crate::layout::Tab::None);
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

#[test]
fn no_size_breaks_the_drawing_with_the_explorer_on_top_floating_or_maximized() {
    let (_, mut app, mut engine) = open("screen_sizes_new", Some(SONG));
    offline::render(&mut engine, 3 * STEP, 2);
    app.tick();
    let alt = |c| Input {
        key: Key::Char(c),
        alt: true,
        ..Input::default()
    };
    // Explorador encima (a 80), editor flotante y maximizado, en ASCII y no.
    let mut terminal = Terminal::new(TestBackend::new(80, 24)).unwrap();
    crate::safe_draw::draw(&mut terminal, |frame| ui::draw(frame, &mut app));
    app.handle(alt('e'));
    for setup in [None, Some('w'), Some('z'), Some('z')] {
        if let Some(c) = setup {
            app.handle(alt(c));
        }
        for ascii in [false, true] {
            theme::set_ascii(ascii);
            for (width, height) in [
                (1u16, 1u16),
                (39, 9),
                (40, 10),
                (60, 16),
                (80, 24),
                (99, 29),
                (100, 30),
                (120, 40),
                (180, 50),
                (300, 90),
            ] {
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                let outcome =
                    crate::safe_draw::draw(&mut terminal, |frame| ui::draw(frame, &mut app));
                assert_eq!(
                    outcome,
                    crate::safe_draw::Outcome::Drawn,
                    "{width}×{height} {setup:?} ascii {ascii}"
                );
                if ascii && width >= 40 && height >= 10 {
                    let screen = text(terminal.backend().buffer());
                    assert!(
                        !screen
                            .chars()
                            .any(|c| ('\u{2800}'..='\u{28ff}').contains(&c)),
                        "{width}×{height}: {screen}"
                    );
                }
            }
        }
        theme::set_ascii(false);
    }
}

#[test]
fn shrinking_takes_away_the_explorer_first_then_the_tracker() {
    let (_, mut app, _engine) = open("screen_shrink", Some(SONG));
    app.toggle_master_viz();
    let at = |app: &mut App, w, h| {
        draw(app, w, h);
        app.last_grid().unwrap().clone()
    };
    let g = at(&mut app, 120, 40);
    assert!(g.explorer.is_some() && g.tracker.is_some());
    // Más estrecha: fuera el explorador; lo demás sigue.
    let g = at(&mut app, 119, 40);
    assert!(g.explorer.is_none() && g.tracker.is_some());
    // Más baja: el tracker, a las pestañas (el espectro siempre lo está).
    let g = at(&mut app, 99, 29);
    assert!(g.area_of(crate::focus::Panel::Spectrum).is_none() && g.tracker.is_none());
    let dock = g.dock.unwrap();
    assert_eq!(
        dock.tabs[0],
        crate::layout::Tab::Panel(crate::focus::Panel::Tracker)
    );
    assert!(
        dock.tabs
            .contains(&crate::layout::Tab::Panel(crate::focus::Panel::Spectrum))
    );
    // Diminuta: solo la línea que dice que el audio sigue.
    let g = at(&mut app, 39, 9);
    assert!(g.editor.is_none() && g.wave.is_none() && g.explorer.is_none());
    // Al volver a ensanchar, el explorador vuelve a su columna.
    let g = at(&mut app, 120, 40);
    assert!(g.explorer.is_some() && !g.explorer_overlay);
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
fn the_master_scope_and_spectrum_are_off_until_asked_and_need_nothing_in_the_script() {
    let (_, mut app, mut engine) = open("screen_master_viz", Some(SONG));
    assert!(app.view().viz.is_empty(), "el script no pide ninguna");
    // Sin pedirlos, ni pestañas ni paneles: ni un lote al motor, ni cola
    // del master, ni FFT, aunque suene.
    let screen = text(&draw(&mut app, 180, 50));
    assert!(
        !screen.contains("7 scope") && !screen.contains("8 espectro"),
        "{screen}"
    );
    play(&mut app, &mut engine, 20);
    draw(&mut app, 180, 50);
    assert_eq!(app.viz_cost(), (0, [false; engine::VIZ_SOURCES], 0));
    assert!(app.view().master_views.is_empty());
    // Desde el menú Ver: el espectro en su pestaña, elegida. La interfaz
    // enciende la cola del master (entra al compás).
    app.handle(f(10));
    app.handle(Input {
        key: Key::Char(crate::menu::Menu::View.letter()),
        alt: true,
        ..Input::default()
    });
    let items = app.view().menu.unwrap().items;
    let at = items
        .iter()
        .position(|i| i.name == "Scope y espectro del master")
        .expect("en el menú Ver");
    for _ in 0..at {
        app.handle(Input {
            key: Key::Down,
            ..Input::default()
        });
    }
    app.handle(Input {
        key: Key::Enter,
        ..Input::default()
    });
    assert!(app.master_viz_on());
    draw(&mut app, 180, 50);
    play(&mut app, &mut engine, 20);
    let screen = text(&draw(&mut app, 180, 50));
    assert_eq!(app.view().master_views.len(), 2, "scope y espectro");
    assert!(
        screen.contains(" 8 espectro · master · relleno "),
        "{screen}"
    );
    let (batches, sources, ffts) = app.viz_cost();
    assert!(batches == 1 && sources[engine::VIZ_MASTER] && ffts > 0);
    // El scope, en la suya.
    app.show_tab(crate::layout::Tab::Panel(crate::focus::Panel::Scope));
    draw(&mut app, 180, 50);
    let screen = text(&draw(&mut app, 180, 50));
    assert!(screen.contains("7 scope · master · RMS"), "{screen}");
    assert!(
        screen
            .chars()
            .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)),
        "el scope en Braille: {screen}"
    );
    // Alt-M los quita: el coste vuelve a cero (otro lote que apaga la
    // cola; ni una FFT más).
    app.handle(Input {
        key: Key::Char('m'),
        alt: true,
        ..Input::default()
    });
    assert!(!app.master_viz_on());
    draw(&mut app, 180, 50);
    play(&mut app, &mut engine, 1);
    let (batches, sources, ffts) = app.viz_cost();
    assert!(batches == 2 && !sources[engine::VIZ_MASTER]);
    let screen = text(&draw(&mut app, 180, 50));
    play(&mut app, &mut engine, 20);
    draw(&mut app, 180, 50);
    assert_eq!(app.viz_cost(), (2, [false; engine::VIZ_SOURCES], ffts));
    assert!(app.view().master_views.is_empty());
    assert!(
        !screen.contains("7 scope") && !screen.contains("8 espectro"),
        "{screen}"
    );
}

#[test]
fn the_master_spectrum_panel_draws_dots_an_axis_and_a_spectrogram() {
    use crate::config::Config;
    let (_, mut app, mut engine) = open("screen_master_spectrum", Some(SONG));
    // A 60 cuadros por segundo (un cuadro cada 800 muestras), uno por
    // cada sexto de paso (1000).
    app.set_config(&Config::parse("[interfaz]\nvisuales_hz = 60\n", true, "c"));
    app.toggle_master_viz();
    draw(&mut app, 120, 40);
    play(&mut app, &mut engine, 16);
    // Un cuadro dibujado cada sexto de paso (los golpes son cortos): lo
    // que suena sube y cae despacio.
    for _ in 0..32 {
        offline::render(&mut engine, STEP / 6, 2);
        app.tick();
        draw(&mut app, 120, 40);
    }
    let buffer = draw(&mut app, 120, 40);
    let screen = text(&buffer);
    // En su pestaña: el dock sin la fila de las pestañas.
    let dock = app.last_grid().unwrap().dock.clone().unwrap().area;
    let area = ratatui::layout::Rect::new(dock.x, dock.y + 1, dock.width, dock.height - 1);
    assert!(
        find(&buffer, "┌ 8 espectro").is_some_and(|p| p == (area.x, area.y)),
        "{screen}"
    );
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
        "[interfaz]\nespectro = \"barras\"\nvisuales_hz = 60\n",
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
        "[interfaz]\nespectro = \"espectrograma\"\nvisuales_hz = 60\n",
        true,
        "c",
    ));
    for _ in 0..30 {
        offline::render(&mut engine, STEP / 6, 2);
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
    app.toggle_master_viz();
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
    app.toggle_master_viz();
    // Sin dibujar (o sin su panel a la vista), el motor no copia el master.
    play(&mut app, &mut engine, 20);
    assert!(app.view().master_views.is_empty());
    // Pedidos, el scope y el espectro son pestañas: con otra elegida no se
    // ven, ni se calcula la FFT.
    app.show_tab(crate::layout::Tab::Panel(crate::focus::Panel::Mixer));
    draw(&mut app, 80, 24);
    play(&mut app, &mut engine, 20);
    draw(&mut app, 80, 24);
    assert!(app.view().master_views.is_empty());
    assert_eq!(app.viz_cost().2, 0, "ni una FFT");
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
    // Con el scope y el espectro pedidos, a 40 columnas no caben todas.
    app.toggle_master_viz();
    app.show_tab(crate::layout::Tab::None);
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

/// La fila de la línea del compás de una pantalla de `width` columnas.
fn bar_row(app: &mut App, width: u16, height: u16) -> String {
    let screen = text(&draw(app, width, height));
    let area = ratatui::layout::Rect::new(0, 0, width, height);
    let y = crate::screen::grid_of(&app.view(), area).ruler.unwrap().y;
    line(&screen, y as usize)
}

/// Celdas llenas de la línea (las gruesas, las del frente y las marcas en
/// lo lleno).
fn filled(row: &str) -> usize {
    row.chars()
        .filter(|c| matches!(c, '━' | '╾' | '┷' | '=' | '>'))
        .count()
}

#[test]
fn the_bar_line_grows_through_the_bar_and_starts_again() {
    let (_, mut app, mut engine) = open("screen_bar_line", Some(SONG));
    // Medio compás (8 pasos de 16): la mitad de la línea.
    offline::render(&mut engine, 8 * STEP, 2);
    app.tick();
    let row = bar_row(&mut app, 80, 24);
    assert!(row.chars().all(|c| matches!(c, '━' | '╾' | '─')), "{row}");
    let half = filled(&row);
    assert!((39..=41).contains(&half), "{half}: {row}");
    // Lo lleno, en el acento; lo vacío, atenuado.
    let buffer = draw(&mut app, 80, 24);
    assert_eq!(buffer[(0, 1)].fg, theme::get().accent);
    assert_eq!(buffer[(79, 1)].fg, theme::get().dim);
    // Casi al final del compás: casi llena.
    offline::render(&mut engine, 7 * STEP + STEP / 2, 2);
    app.tick();
    let full = filled(&bar_row(&mut app, 80, 24));
    assert!(full >= 76, "{full}");
    // El compás siguiente: vuelve a empezar.
    offline::render(&mut engine, STEP, 2);
    app.tick();
    let row = bar_row(&mut app, 80, 24);
    assert!(filled(&row) <= 4, "{row}");
}

#[test]
fn the_bar_line_follows_three_four_and_seven_eight() {
    for (meter, steps) in [("meter(3, 4)", 12), ("meter(7, 8)", 14)] {
        let song = format!("{SONG}{meter}\n");
        let (_, mut app, mut engine) = open("screen_bar_meter", Some(&song));
        assert!(app.view().error.is_none(), "{:?}", app.view().error);
        offline::render(&mut engine, steps / 2 * STEP, 2);
        app.tick();
        let half = filled(&bar_row(&mut app, 100, 30));
        let view = app.view();
        assert_eq!(view.bar.bar_steps, steps, "{meter}");
        let f = view.bar.fraction.unwrap();
        assert!((f - 0.5).abs() < 0.05, "{meter}: {f}");
        assert!((48..=52).contains(&half), "{meter}: {half}");
    }
}

#[test]
fn the_bar_line_fills_the_width_and_leaves_room_for_the_timeline() {
    let (_, mut app, mut engine) = open("screen_bar_width", Some(SONG));
    offline::render(&mut engine, 4 * STEP, 2);
    app.tick();
    // Sin línea de tiempo, todo el ancho, al redimensionar también.
    for width in [60, 80, 200] {
        let row = bar_row(&mut app, width, 30);
        assert_eq!(row.chars().count(), width as usize);
        assert!(!row.contains(' '), "{width}: {row}");
        let quarter = filled(&row) as f64 / width as f64;
        assert!((quarter - 0.25).abs() < 0.03, "{width}: {row}");
    }
    // Con secciones, la línea de tiempo detrás, y la línea no cambia de
    // largo de un compás a otro.
    let (_, mut app, mut engine) = open(
        "screen_bar_timeline",
        Some(crate::tests::sections::SONG_SECTIONS),
    );
    offline::render(&mut engine, 4 * STEP, 2);
    app.tick();
    let is_line = |c: char| matches!(c, '━' | '╾' | '─');
    let row = bar_row(&mut app, 120, 30);
    let width = row.chars().take_while(|&c| is_line(c)).count();
    assert!(width >= 16, "{row}");
    let rest: String = row.chars().skip(width).collect();
    assert!(rest.starts_with("   ▶ intro 1/2 │ drop 4"), "{row}");
    offline::render(&mut engine, 16 * STEP, 2);
    app.tick();
    let row = bar_row(&mut app, 120, 30);
    assert_eq!(
        row.chars().take_while(|&c| is_line(c)).count(),
        width,
        "{row}"
    );
    assert!(row.contains("intro 2/2"), "{row}");
    // Estrecha: la línea con lo mínimo y la línea de tiempo recortada.
    let row = bar_row(&mut app, 40, 30);
    assert_eq!(row.chars().take_while(|&c| is_line(c)).count(), 16, "{row}");
    assert_eq!(row.chars().nth(16), Some(' '), "{row}");
    assert!(row.contains("drop"), "{row}");
}

#[test]
fn the_bar_line_in_ascii() {
    let (_, mut app, mut engine) = open("screen_bar_ascii", Some(SONG));
    offline::render(&mut engine, 8 * STEP + STEP / 3, 2);
    app.tick();
    theme::set_ascii(true);
    let row = bar_row(&mut app, 60, 24);
    theme::set_ascii(false);
    assert!(row.starts_with("=============================="), "{row}");
    assert!(row.contains('>') && row.ends_with('-'), "{row}");
    assert!(row.is_ascii(), "{row}");
}

#[test]
fn no_screen_draws_squares_on_the_ruler() {
    let (_, mut app, mut engine) = open("screen_bar_squares", Some(SONG));
    for ascii in [false, true] {
        theme::set_ascii(ascii);
        for (w, h) in [(40, 12), (80, 24), (120, 40), (200, 50), (300, 90)] {
            offline::render(&mut engine, 3 * STEP, 2);
            app.tick();
            let screen = text(&draw(&mut app, w, h));
            assert!(!screen.contains('▰') && !screen.contains('▱'), "{screen}");
        }
    }
    theme::set_ascii(false);
}
