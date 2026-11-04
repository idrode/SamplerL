//! Visualizaciones en la interfaz: las de rejilla salen del patrón con el
//! cursor de lo que suena; las de audio, de las muestras del motor. Cada
//! una, dibujada con el backend de test.

use super::*;
use crate::viz::{VizKind, VizSource, VizSpec, VizView};

fn spec(kind: VizKind, source: VizSource, size: u16) -> VizSpec {
    VizSpec {
        kind,
        source,
        size,
        bands: 32,
    }
}

fn drums(kind: VizKind) -> VizSpec {
    spec(kind, VizSource::Track("drums".into()), 3)
}

/// Mueve el motor `steps` pasos de 1/8 en 1/8, con un `tick` en cada uno.
fn play(app: &mut App, engine: &mut Engine, steps: usize) {
    for _ in 0..8 * steps {
        offline::render(engine, STEP / 8, 2);
        app.tick();
    }
}

#[test]
fn punchcard_shows_the_hits_of_the_pattern_and_the_playhead() {
    let (_, mut app, mut engine) = open("viz_punchcard", Some(SONG));
    app.set_viz(vec![drums(VizKind::Punchcard)]);
    let VizView::Grid(grid) = &app.view().viz[0] else {
        panic!("rejilla")
    };
    // Dos filas: los dos cortes que toca (`1 . 2 .` cuatro veces).
    assert_eq!(grid.rows.len(), 2);
    assert_eq!(grid.steps, 16);
    use crate::viz::grid::Cell::{Empty, Head};
    let (_, low) = &grid.rows[1];
    assert_eq!(&low[..4], &[Head, Empty, Empty, Empty]);
    let (_, high) = &grid.rows[0];
    assert_eq!(&high[..4], &[Empty, Empty, Head, Empty]);
    play(&mut app, &mut engine, 5);
    let VizView::Grid(grid) = &app.view().viz[0] else {
        panic!()
    };
    assert!(grid.playhead.is_some());
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("punchcard · drums"), "{screen}");
    assert!(screen.contains('█'), "{screen}");
}

#[test]
fn pianoroll_has_every_row_of_the_range_with_its_name() {
    let song = r#"bpm(120)
track("bajo", synth("acid", {})):melody("c2 . e2 .  g2 . . .  c2 . . .  . . . .")
"#;
    let (_, mut app, _engine) = open("viz_pianoroll", Some(song));
    app.set_viz(vec![spec(
        VizKind::Pianoroll,
        VizSource::Track("bajo".into()),
        8,
    )]);
    let VizView::Grid(grid) = &app.view().viz[0] else {
        panic!()
    };
    let names: Vec<&str> = grid.rows.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names.first(), Some(&"G2"), "{names:?}");
    assert_eq!(names.last(), Some(&"C2"), "{names:?}");
    assert_eq!(names.len(), 8);
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("pianoroll · bajo"), "{screen}");
    assert!(screen.contains("E2"), "{screen}");
}

#[test]
fn ascii_draws_the_grid_without_blocks() {
    let (_, mut app, _engine) = open("viz_ascii", Some(SONG));
    app.set_viz(vec![drums(VizKind::Punchcard)]);
    crate::theme::set_ascii(true);
    let screen = text(&draw(&mut app, 140, 40));
    crate::theme::set_ascii(false);
    assert!(screen.contains('#'), "{screen}");
}

/// Para verlas sin terminal: `cargo test -p tui pantalla_viz -- --ignored
/// --nocapture`.
#[test]
#[ignore]
fn pantalla_viz() {
    let (_, mut app, mut engine) = open("viz_pantalla", Some(SONG));
    app.set_viz(vec![
        drums(VizKind::Punchcard),
        spec(VizKind::Scope, VizSource::Master, 6),
        spec(VizKind::Spectrum, VizSource::Master, 6),
    ]);
    play(&mut app, &mut engine, 20);
    println!("{}", text(&draw(&mut app, 140, 45)));
}
