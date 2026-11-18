//! Solapes de frecuencia en vivo en la pestaña «choques»: cerrado no cuesta
//! nada (ni colas en el motor ni FFT), abierto marca el choque de dos
//! pistas graves en su banda y no el de una aguda, se elige con el teclado
//! y el ratón, se ve en ASCII, y da lo mismo que el análisis offline de
//! `--analyze-masking`.

use super::masking::write_sines;
use super::*;
use crate::app::MouseAction;

const W: u16 = 140;
const H: u16 = 44;

/// Tres pistas: `a` y `b` a 100 Hz, `c` a 5 kHz; cada una toca su seno de
/// 2 s una vez por compás (a 120, el compás entero).
fn three() -> String {
    let one = super::masking::ONE;
    format!(
        "bpm(120)\n\
         track(\"a\", sample(\"sine100.wav\"):slice_grid(1)):steps(\"{one}\")\n\
         track(\"b\", sample(\"sine100.wav\"):slice_grid(1)):steps(\"{one}\"):gain(0.8)\n\
         track(\"c\", sample(\"sine5000.wav\"):slice_grid(1)):steps(\"{one}\")\n"
    )
}

fn song(test: &str, text: &str) -> (PathBuf, App, Engine) {
    let (path, mut app, engine) = open(test, Some(text));
    write_sines(path.parent().unwrap());
    app.load();
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    (path, app, engine)
}

/// `seconds` de audio, a cuadros de 1/30 s: render, `tick` y dibujo.
fn run(app: &mut App, engine: &mut Engine, seconds: f32) -> Buffer {
    let frames = (seconds * 30.0) as usize;
    let mut last = draw(app, W, H);
    for _ in 0..frames {
        offline::render(engine, RATE as usize / 30, 2);
        app.tick();
        last = draw(app, W, H);
    }
    last
}

fn sources(app: &App) -> [bool; engine::VIZ_SOURCES] {
    app.viz_cost().1
}

#[test]
fn closed_it_costs_nothing_and_open_it_marks_the_low_clash() {
    let (_, mut app, mut engine) = song("clashes_live", &three());
    // Cerrado: ni una cola encendida ni un análisis, aunque suene.
    run(&mut app, &mut engine, 2.0);
    assert_eq!(sources(&app), [false; engine::VIZ_SOURCES]);
    assert!(app.view().clash_table.is_none());
    // Desde la paleta: la pestaña «choques», elegida, en vivo.
    palette(&mut app, "solapes de frecuencia en vivo");
    let screen = text(&draw(&mut app, W, H));
    assert!(screen.contains("┌ choques · en vivo"), "{screen}");
    // Las colas de las tres pistas entran al compás; tras unos segundos,
    // el choque de las dos graves, y nada con la aguda.
    let buffer = run(&mut app, &mut engine, 6.0);
    let on = sources(&app);
    assert!(on[0] && on[1] && on[2] && !on[engine::VIZ_MASTER], "{on:?}");
    let table = app.view().clash_table.unwrap();
    let matrix = table.matrix.unwrap();
    assert!(table.live);
    assert_eq!(matrix.tracks, ["a", "b", "c"]);
    assert!(!matrix.groups.is_empty(), "{:?}", matrix.groups);
    let (first, score) = &matrix.groups[0];
    assert!(first.starts_with("a × b: "), "{first}");
    assert!(*score > 0.6, "{score}");
    assert!(
        matrix
            .groups
            .iter()
            .all(|(g, _)| !g.split(':').next().unwrap().contains('c')),
        "{:?}",
        matrix.groups
    );
    let band = engine::masking::BAND_NAMES
        .iter()
        .position(|b| *b == "100")
        .unwrap();
    assert!(matrix.hot[0][band] > 0.6 && matrix.hot[1][band] > 0.6);
    assert_eq!(matrix.hot[2][band], 0.0);
    // En pantalla: la celda del choque en rojo y la línea en la lista.
    let screen = text(&buffer);
    assert!(screen.contains("a × b: "), "{screen}");
    let layout = app.clash.layout.expect("se ve");
    let x = layout.cells_x + band as u16 * layout.cell;
    assert_eq!(buffer[(x, layout.first_track)].fg, crate::ui::error_color());
    // La celda de la aguda en 100 Hz, sin choque.
    assert_ne!(
        buffer[(x, layout.first_track + 2)].fg,
        crate::ui::error_color()
    );
    // Otra vez: apagado; las colas se apagan (otro lote) y no se analiza más.
    palette(&mut app, "solapes de frecuencia en vivo");
    run(&mut app, &mut engine, 3.0);
    assert_eq!(sources(&app), [false; engine::VIZ_SOURCES]);
    assert!(app.view().clash_table.is_none());
}

#[test]
fn with_another_tab_in_front_the_engine_copies_nothing() {
    let (_, mut app, mut engine) = song("clashes_hidden", &three());
    palette(&mut app, "solapes de frecuencia en vivo");
    run(&mut app, &mut engine, 3.0);
    assert!(sources(&app)[0]);
    // Otra pestaña delante: el panel no se ve, las colas se apagan.
    app.show_tab(crate::layout::Tab::Panel(crate::focus::Panel::Fx));
    run(&mut app, &mut engine, 3.0);
    assert_eq!(sources(&app), [false; engine::VIZ_SOURCES]);
    // Y al volver, empieza de cero (nada de lo de antes).
    app.show_tab(crate::layout::Tab::Clash);
    draw(&mut app, W, H);
    let matrix = app.view().clash_table.unwrap().matrix.unwrap();
    assert!(matrix.tracks.is_empty(), "{:?}", matrix.tracks);
}

#[test]
fn the_cursor_and_the_mouse_choose_a_clash() {
    // Tres graves: tres choques (a × b, a × c, b × c).
    let one = super::masking::ONE;
    let text_ = format!(
        "bpm(120)\n\
         track(\"a\", sample(\"sine100.wav\"):slice_grid(1)):steps(\"{one}\")\n\
         track(\"b\", sample(\"sine100.wav\"):slice_grid(1)):steps(\"{one}\"):gain(0.8)\n\
         track(\"c\", sample(\"sine100.wav\"):slice_grid(1)):steps(\"{one}\"):gain(0.5)\n"
    );
    let (_, mut app, mut engine) = song("clashes_cursor", &text_);
    palette(&mut app, "solapes de frecuencia en vivo");
    let buffer = run(&mut app, &mut engine, 6.0);
    let matrix = app.view().clash_table.unwrap().matrix.unwrap();
    assert_eq!(matrix.groups.len(), 3, "{:?}", matrix.groups);
    assert_eq!(matrix.selected, Some(0));
    // ↓ elige el siguiente, y la línea de estado lo dice.
    app.handle(special(Key::Down));
    let view = app.view();
    let matrix = view.clash_table.unwrap().matrix.unwrap();
    assert_eq!(matrix.selected, Some(1));
    assert_eq!(view.message, matrix.groups[1].0);
    // ↑ dos veces da la vuelta.
    app.handle(special(Key::Up));
    app.handle(special(Key::Up));
    let matrix = app.view().clash_table.unwrap().matrix.unwrap();
    assert_eq!(matrix.selected, Some(2));
    // Las celdas del elegido, invertidas.
    let buffer2 = draw(&mut app, W, H);
    let layout = app.clash.layout.unwrap();
    let band = engine::masking::BAND_NAMES
        .iter()
        .position(|b| *b == "100")
        .unwrap();
    let x = layout.cells_x + band as u16 * layout.cell;
    let selected = &matrix.selected_cells;
    for t in 0..3u16 {
        let reversed = buffer2[(x, layout.first_track + t)]
            .modifier
            .contains(ratatui::style::Modifier::REVERSED);
        assert_eq!(
            reversed,
            selected.iter().any(|&(st, _)| st == t as usize),
            "pista {t}"
        );
    }
    // Esc suelta el panel: las flechas vuelven al editor.
    app.handle(esc());
    assert_eq!(
        app.view().clash_table.unwrap().matrix.unwrap().selected,
        None
    );
    // Un clic en el primer choque de la lista lo elige y vuelve a poner
    // el cursor en el panel.
    let _ = buffer;
    app.mouse(MouseAction::Down, layout.inner.x + 4, layout.first_group);
    app.mouse(MouseAction::Up, layout.inner.x + 4, layout.first_group);
    let matrix = app.view().clash_table.unwrap().matrix.unwrap();
    assert_eq!(matrix.selected, Some(0));
    // Un clic en una celda de `c` en 100 Hz: el choque más fuerte de `c`.
    app.mouse(MouseAction::Down, x, layout.first_track + 2);
    let view = app.view();
    let matrix = view.clash_table.unwrap().matrix.unwrap();
    let chosen = &matrix.groups[matrix.selected.unwrap()].0;
    assert!(chosen.contains("c"), "{chosen}");
}

#[test]
fn in_ascii_the_matrix_has_no_blocks() {
    let (_, mut app, mut engine) = song("clashes_ascii", &three());
    palette(&mut app, "solapes de frecuencia en vivo");
    run(&mut app, &mut engine, 5.0);
    crate::theme::set_ascii(true);
    let buffer = draw(&mut app, W, H);
    crate::theme::set_ascii(false);
    let layout = app.clash.layout.unwrap();
    let mut cells = String::new();
    for t in 0..3 {
        for x in layout.cells_x..layout.cells_x + 30 * layout.cell {
            cells.push_str(buffer[(x, layout.first_track + t)].symbol());
        }
    }
    assert!(cells.is_ascii(), "{cells}");
    assert!(cells.contains('#'), "{cells}");
    let list = text(&buffer);
    assert!(list.contains("> a x b: "), "{list}");
}

#[test]
fn live_and_offline_give_the_same_clashes() {
    let (path, mut app, mut engine) = song("clashes_offline", &three());
    palette(&mut app, "solapes de frecuencia en vivo");
    run(&mut app, &mut engine, 6.0);
    let live: Vec<(String, f32)> = app.view().clash_table.unwrap().matrix.unwrap().groups;
    // Lo que imprime `--analyze-masking` de lo mismo: sus choques por
    // gravedad, las mismas líneas (la pareja y el rango iguales; la
    // puntuación, a menos de 0,1: en vivo son los últimos segundos con una
    // ventana por cuadro, offline el ciclo entero cada 25 ms).
    let printed = crate::masking_report(&path).unwrap();
    let offline: Vec<&str> = printed
        .lines()
        .skip_while(|l| *l != "por gravedad:")
        .skip(1)
        .take_while(|l| l.starts_with("  "))
        .map(str::trim)
        .collect();
    assert_eq!(offline.len(), live.len(), "{printed}\n{live:?}");
    let head = |l: &str| l.split(" · ").next().unwrap().to_string();
    let score = |l: &str| -> f32 {
        let field = l.split(" · ").nth(1).unwrap();
        field.rsplit(' ').next().unwrap().parse().unwrap()
    };
    for (o, (l, s)) in offline.iter().zip(&live) {
        assert_eq!(head(o), head(l), "{printed}");
        assert!((score(o) - s).abs() < 0.1, "{o} contra {l}");
    }
}

#[test]
#[ignore = "solo para ver la pantalla"]
fn pantalla_solapes() {
    let (_, mut app, mut engine) = song("pantalla_solapes", &three());
    palette(&mut app, "solapes de frecuencia en vivo");
    run(&mut app, &mut engine, 6.0);
    for (w, h) in [(140, 44), (100, 30), (80, 24)] {
        println!("{w}×{h}\n{}\n", text(&draw(&mut app, w, h)));
    }
    crate::theme::set_ascii(true);
    println!("ASCII\n{}\n", text(&draw(&mut app, 140, 44)));
    crate::theme::set_ascii(false);
}
