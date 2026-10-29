//! Vista de ocupación por paso: qué pistas suenan en cada paso de la
//! página, leído de lo que toca el secuenciador (no del texto), con el
//! motor movido offline.

use super::*;
use crate::occupancy::Grid;
use crate::theme;

const SONG: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 . 2 .  1 . 2 .  1 . 2 .  . . . .")
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
"#;

fn grid(app: &App) -> Grid {
    let view = app.view();
    assert!(view.error.is_none(), "{:?}", view.error);
    view.occupancy.expect("la vista de ocupación")
}

/// Los caracteres de las celdas de una fila de la rejilla.
fn glyphs(grid: &Grid, row: usize) -> String {
    grid.rows[row].cells.iter().map(|c| c.glyph()).collect()
}

fn opened(test: &str, song: &str) -> (PathBuf, App, Engine) {
    let (path, mut app, engine) = open(test, Some(song));
    app.set_occupancy(true);
    (path, app, engine)
}

#[test]
fn hidden_by_default_and_counts_the_tracks_that_sound_in_each_step() {
    let (_, mut app, _engine) = open("occupancy_counts", Some(SONG));
    assert!(app.view().occupancy.is_none());
    app.set_occupancy(true);
    let grid = grid(&app);
    assert_eq!(grid.steps(), 16);
    assert_eq!(grid.rows.len(), 2);
    assert_eq!(glyphs(&grid, 0), "█·█·█·█·█·█·····");
    assert_eq!(glyphs(&grid, 1), "█···█···█···█···");
    assert_eq!(
        grid.counts,
        [2, 0, 1, 0, 2, 0, 1, 0, 2, 0, 1, 0, 1, 0, 0, 0]
    );
    assert_eq!(grid.gaps(), [1, 3, 5, 7, 9, 11, 13, 14, 15]);
}

#[test]
fn volume_retrigger_holds_and_lengths_show_what_the_sequencer_plays() {
    let song = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):length(12):steps("1@0.5 . 2r4 .  1@0.25 _ _ .  2@0.8 . . .")
track("kick", amen:slice_at(1)):steps(euclid(5, 16, "x"))
"#;
    let (_, app, _engine) = opened("occupancy_lengths", song);
    let grid = grid(&app);
    // 12 y 16 vuelven a coincidir a los 48 pasos.
    assert_eq!(grid.steps(), 48);
    let drums = glyphs(&grid, 0);
    assert_eq!(
        &drums[..],
        "▒·4·░──·▓···▒·4·░──·▓···▒·4·░──·▓···▒·4·░──·▓···"
    );
    // `euclid` no es texto literal, pero se ve lo que suena.
    let kick = glyphs(&grid, 1);
    assert_eq!(kick.chars().filter(|&c| c == '█').count(), 15, "{kick}");
    // Lo alargado cuenta como sonando.
    assert_eq!(grid.counts[5], 1);
    // El paso 12 de la canción es el 0 de la segunda vuelta de drums.
    assert_eq!(grid.rows[0].at[12], (0, 0));
}

#[test]
fn every_and_chance_show_each_lap_and_what_they_take_away() {
    let song = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
  :every(2, function(t) t:steps("1 . . .  . . . .  . . . .  . . . .") end)
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
  :chance(1, "mute")
"#;
    let (_, app, _engine) = opened("occupancy_every_chance", song);
    let grid = grid(&app);
    // drums da un ciclo de dos vueltas y kick, uno de 16 (chance): la
    // página es la de las longitudes, un compás, y la que suena es la del
    // `every`.
    assert_eq!(grid.steps(), 16);
    // drums: lo que el `every` quita del patrón base, `×`.
    assert_eq!(glyphs(&grid, 0), "█·×·×·×·×·×·×·×·");
    // kick: los golpes que chance quita, `×`, y no cuentan.
    assert_eq!(glyphs(&grid, 1), "×···×···×···×···");
    assert_eq!(grid.counts[4], 0);
}

#[test]
fn the_current_step_follows_what_is_heard() {
    let (_, mut app, mut engine) = opened("occupancy_heard", SONG);
    assert_eq!(grid(&app).current, None, "parado, sin paso");
    offline::render(&mut engine, 6 * STEP + 100, 2);
    app.tick();
    assert_eq!(grid(&app).current, Some(6));
    // Con un paso de retardo de salida, el de antes (como el resaltado).
    engine.set_output_delay(STEP as u64);
    offline::render(&mut engine, STEP, 2);
    app.tick();
    assert_eq!(grid(&app).current, Some(6));
    // En el compás siguiente, la página siguiente.
    offline::render(&mut engine, 10 * STEP, 2);
    app.tick();
    let grid = grid(&app);
    assert_eq!(grid.bar, 1);
    assert_eq!(grid.current, Some(0));
}

#[test]
fn a_muted_track_is_dim_and_does_not_count() {
    let (_, mut app, _engine) = opened("occupancy_mute", SONG);
    app.run_action(crate::palette::Action::NextTrack);
    app.run_action(crate::palette::Action::Mute);
    let grid = grid(&app);
    assert!(!grid.rows[1].audible);
    assert_eq!(grid.counts[0], 1);
    assert_eq!(grid.counts[12], 0);
}

#[test]
fn the_panel_draws_at_two_cells_one_cell_or_scrolled() {
    let (_, mut app, mut engine) = opened("occupancy_panel", SONG);
    offline::render(&mut engine, 2 * STEP + 100, 2);
    app.tick();
    let view = app.view();
    let theme = theme::get();
    // Ancho: dos celdas por paso.
    let buffer = draw_widget(ui::OccupancyPanel(&view), 60, 6);
    let screen = text(&buffer);
    assert!(
        screen.contains("ocupación · compás 0 · 9 huecos"),
        "{screen}"
    );
    assert!(
        screen.contains("drums █ · ▓".replace('▓', "█").as_str()),
        "{screen}"
    );
    assert!(screen.contains("kick  █ · · · █"), "{screen}");
    assert!(screen.contains("Σ     2 0 1 0 2"), "{screen}");
    let (x, y) = find(&buffer, "Σ").unwrap();
    // El hueco del paso 1, sobre su color; el paso que se oye (el 2),
    // marcado arriba y sobre el color del paso que suena.
    assert_eq!(buffer[(x + 8, y)].bg, theme.gap);
    assert_eq!(buffer[(x + 10, y)].bg, theme.playing);
    assert_eq!(buffer[(x + 10, 1)].symbol(), "▼");
    assert_eq!(buffer[(x + 6, 2)].fg, theme.highlight[0]);
    // Estrecho: una celda por paso.
    let buffer = draw_widget(ui::OccupancyPanel(&view), 26, 6);
    let screen = text(&buffer);
    assert!(screen.contains("drums █·█·█·█·█·█·····"), "{screen}");
    assert!(screen.contains("Σ     2010201020101000"), "{screen}");
    // Más estrecho: se desplaza siguiendo el paso que se oye.
    let buffer = draw_widget(ui::OccupancyPanel(&view), 16, 6);
    let screen = text(&buffer);
    assert!(screen.contains("drums █·█·█·█·"), "{screen}");
    offline::render(&mut engine, 11 * STEP, 2);
    app.tick();
    let later = app.view();
    let buffer = draw_widget(ui::OccupancyPanel(&later), 16, 6);
    let screen = text(&buffer);
    assert!(screen.contains("drums █·█·····"), "{screen}");
    assert!(screen.contains("Σ     20101000"), "{screen}");
    // Con sitio para el título, lo dice.
    let buffer = draw_widget(ui::OccupancyPanel(&later), 60, 6);
    assert!(!text(&buffer).contains("pasos"));
    // Bajo: las pistas de alrededor de la elegida, y lo dice.
    let buffer = draw_widget(ui::OccupancyPanel(&view), 60, 5);
    let screen = text(&buffer);
    assert!(screen.contains("pistas 1–1 de 2"), "{screen}");
    assert!(screen.contains("Σ"), "{screen}");
}

#[test]
fn the_palette_shows_and_hides_it_above_the_sample() {
    use crate::palette::Action;
    let (_, mut app, _engine) = open("occupancy_palette", Some(SONG));
    let screen = text(&draw(&mut app, 120, 40));
    assert!(!screen.contains("ocupación"), "{screen}");
    app.run_action(Action::Occupancy);
    assert!(app.view().message.contains("vista de ocupación"));
    for (width, height) in [(120, 40), (100, 30), (80, 24)] {
        let buffer = draw(&mut app, width, height);
        let screen = text(&buffer);
        // En la columna de la derecha, bajo el motor y encima del sample;
        // el editor y el tracker siguen ahí.
        let (x, y) = find(&buffer, "┌ ocupación").unwrap_or_else(|| panic!("{screen}"));
        assert_eq!(x, width / 2, "{screen}");
        assert!(screen.contains("Σ"), "{screen}");
        assert!(screen.contains("drums █"), "{screen}");
        assert!(screen.contains("┌ song.lua"), "{screen}");
        // A 80×24 no queda sitio para el tracker al lado.
        if height >= 30 {
            assert!(screen.contains("tracker"), "{screen}");
        }
        if let Some((_, sample)) = find(&buffer, "┌ sample") {
            assert!(sample > y, "{screen}");
        }
    }
    // A 120×40 caben las dos pistas y el sample.
    let buffer = draw(&mut app, 120, 40);
    assert!(find(&buffer, "┌ sample").is_some(), "{}", text(&buffer));
    assert!(find(&buffer, "drums █ · █").is_some(), "{}", text(&buffer));
    app.run_action(Action::Occupancy);
    assert!(app.view().occupancy.is_none());
    assert!(!text(&draw(&mut app, 120, 40)).contains("┌ ocupación"));
}

#[test]
fn the_config_shows_it_from_the_start_with_its_colors() {
    use crate::config::Config;
    let (_, mut app, _engine) = open("occupancy_config", Some(SONG));
    let config = Config::parse(
        "[ocupacion]\nvisible = true\n[colores]\nhueco = \"#400000\"\n",
        true,
        "c",
    );
    theme::set(config.theme);
    app.set_config(&config);
    let buffer = draw(&mut app, 120, 40);
    let (x, y) = find(&buffer, "Σ").expect("la vista");
    // El paso 1 es un hueco: sobre el color de la configuración.
    assert_eq!(buffer[(x + 8, y)].bg, Color::Rgb(0x40, 0, 0));
    theme::set(theme::Theme::default());
}

#[test]
fn with_the_editor_outside_the_main_window_shows_it_full_width() {
    use super::split::{editor_window, main_window, pump};
    use crate::palette::Action;
    let path = fixture("occupancy_main_window", Some(SONG));
    let (mut main, _engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    // Desde la ventana del editor, la acción va a la principal.
    editor.run_action(Action::Occupancy);
    pump(&mut main, &mut editor);
    assert!(main.view().occupancy.is_some());
    for (width, height) in [(120, 40), (80, 24)] {
        let buffer = draw(&mut main, width, height);
        let screen = text(&buffer);
        let (x, y) = find(&buffer, "┌ ocupación").unwrap_or_else(|| panic!("{screen}"));
        assert_eq!((x, y), (0, 1), "{screen}");
        assert_eq!(buffer[(width - 1, 1)].symbol(), "┐", "{screen}");
        assert!(screen.contains("tracker"), "{screen}");
        assert!(screen.contains("┌ sample"), "{screen}");
    }
}
