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

/// Un seno de 220 Hz largo en un bucle, para las de audio.
fn open_tone(test: &str) -> (App, Engine) {
    let path = fixture(test, None);
    let dir = path.parent().unwrap();
    let tone: Vec<f32> = (0..96_000)
        .map(|i| 0.6 * (std::f32::consts::TAU * 220.0 * i as f32 / RATE as f32).sin())
        .collect();
    offline::write_wav(dir.join("tono.wav"), &tone, 1, RATE).unwrap();
    std::fs::write(
        &path,
        "bpm(120)\nlocal t = sample(\"tono.wav\"):slice_grid(1)\n\
         t:set(1, { play = \"loop\" })\n\
         track(\"tono\", t):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n",
    )
    .unwrap();
    let (controller, engine) = new_engine(RATE);
    let app = App::open(&path, controller, RATE).unwrap();
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    (app, engine)
}

#[test]
fn scope_triggers_and_stays_still_frame_after_frame() {
    let (mut app, mut engine) = open_tone("viz_scope");
    app.set_viz(vec![spec(VizKind::Scope, VizSource::Master, 6)]);
    // El encendido entra al compás: pasado el primero, ya llega.
    play(&mut app, &mut engine, 20);
    let mut firsts = Vec::new();
    for _ in 0..5 {
        play(&mut app, &mut engine, 1);
        let VizView::Scope(scope) = &app.view().viz[0] else {
            panic!("scope")
        };
        assert!(scope.triggered);
        assert_eq!(scope.samples.len(), crate::viz::scope::WINDOW);
        firsts.push(scope.samples[..6].to_vec());
    }
    for f in &firsts[1..] {
        for (a, b) in f.iter().zip(&firsts[0]) {
            assert!((a - b).abs() < 0.02, "{a} {b}");
        }
    }
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("scope · master"), "{screen}");
    assert!(
        screen
            .chars()
            .any(|c| ('\u{2801}'..='\u{28ff}').contains(&c)),
        "sin Braille: {screen}"
    );
    crate::theme::set_ascii(true);
    let screen = text(&draw(&mut app, 140, 40));
    crate::theme::set_ascii(false);
    assert!(screen.contains('*'), "{screen}");
}

#[test]
fn spectrum_peaks_where_the_tone_is_and_draws_bars() {
    let (mut app, mut engine) = open_tone("viz_spectrum");
    app.set_viz(vec![VizSpec {
        bands: 24,
        ..spec(VizKind::Spectrum, VizSource::Master, 5)
    }]);
    play(&mut app, &mut engine, 20);
    let VizView::Spectrum(spectrum) = &app.view().viz[0] else {
        panic!("spectrum")
    };
    assert_eq!(spectrum.levels.len(), 24);
    let peak = spectrum
        .levels
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .unwrap()
        .0;
    let edges = crate::viz::spectrum::edges(24, RATE);
    assert!(
        edges[peak] <= 220.0 && 220.0 < edges[peak + 1],
        "banda {peak}"
    );
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("spectrum · master"), "{screen}");
    assert!(screen.contains('█'), "{screen}");
}

#[test]
fn the_script_asks_for_them_and_the_load_switches_the_engine_on() {
    let (mut app, mut engine) = open_tone("viz_from_lua");
    let mut text = app.editor.lines().join("\n");
    text.push_str(
        "\ntrack(\"tono\"):scope{ size = 5 }:punchcard{ size = 2 }\nmaster():spectrum()\n",
    );
    let path = fixture("viz_from_lua_2", None);
    let dir = path.parent().unwrap();
    std::fs::copy(
        Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/tmp/tui_viz_from_lua/tono.wav"
        )),
        dir.join("tono.wav"),
    )
    .unwrap();
    std::fs::write(&path, &text).unwrap();
    let (controller, engine2) = new_engine(RATE);
    drop(std::mem::replace(&mut engine, engine2));
    app = App::open(&path, controller, RATE).unwrap();
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let kinds: Vec<&str> = app
        .view()
        .viz
        .iter()
        .map(|v| match v {
            VizView::Grid(_) => "grid",
            VizView::Scope(_) => "scope",
            VizView::Spectrum(_) => "spectrum",
            VizView::Wheel(_) => "pitchwheel",
            VizView::Spiral(_) => "spiral",
        })
        .collect();
    assert_eq!(kinds, ["scope", "grid", "spectrum"]);
    play(&mut app, &mut engine, 20);
    let VizView::Scope(scope) = &app.view().viz[0] else {
        panic!()
    };
    assert!(scope.triggered, "el lote de la carga encendió la cola");
}

#[test]
fn pitchwheel_and_spiral_from_the_audio() {
    let (mut app, mut engine) = open_tone("viz_wheel");
    app.set_viz(vec![
        spec(VizKind::Pitchwheel, VizSource::Master, 8),
        spec(VizKind::Spiral, VizSource::Master, 8),
    ]);
    play(&mut app, &mut engine, 20);
    let view = app.view();
    let VizView::Wheel(wheel) = &view.viz[0] else {
        panic!("rueda")
    };
    // 220 Hz es un la.
    assert_eq!(wheel.strongest(), Some(9));
    let VizView::Spiral(spiral) = &view.viz[1] else {
        panic!("espiral")
    };
    assert_eq!(spiral.note.as_deref(), Some("A3"));
    let screen = text(&draw(&mut app, 140, 45));
    assert!(screen.contains("pitchwheel · master"), "{screen}");
    assert!(screen.contains("spiral · master A3"), "{screen}");
    assert!(screen.contains('A'), "{screen}");
    crate::theme::set_ascii(true);
    let screen = text(&draw(&mut app, 140, 45));
    crate::theme::set_ascii(false);
    assert!(screen.contains("A####"), "{screen}");
    assert!(
        !screen
            .chars()
            .any(|c| ('\u{2800}'..='\u{28ff}').contains(&c)),
        "{screen}"
    );
}

/// Para verlas sin terminal: `cargo test -p tui pantalla_viz -- --ignored
/// --nocapture`.
#[test]
#[ignore]
fn pantalla_viz() {
    let (mut app, mut engine) = open_tone("viz_pantalla");
    app.set_viz(vec![
        spec(VizKind::Punchcard, VizSource::Track("tono".into()), 2),
        spec(VizKind::Scope, VizSource::Master, 6),
        spec(VizKind::Spectrum, VizSource::Master, 6),
        spec(VizKind::Pitchwheel, VizSource::Master, 8),
        spec(VizKind::Spiral, VizSource::Master, 8),
    ]);
    play(&mut app, &mut engine, 20);
    println!("{}", text(&draw(&mut app, 140, 45)));
}

/// Lo que cuesta cada visualización por cuadro (calcularla y dibujarla
/// en 140×45, en el hilo de la interfaz): `cargo test -p tui --release
/// coste_de_las_visualizaciones -- --ignored --nocapture`.
#[test]
#[ignore]
fn coste_de_las_visualizaciones() {
    for (name, kind, source) in [
        (
            "punchcard",
            VizKind::Punchcard,
            VizSource::Track("tono".into()),
        ),
        (
            "pianoroll",
            VizKind::Pianoroll,
            VizSource::Track("tono".into()),
        ),
        ("scope", VizKind::Scope, VizSource::Master),
        ("spectrum", VizKind::Spectrum, VizSource::Master),
        ("pitchwheel", VizKind::Pitchwheel, VizSource::Master),
        ("spiral", VizKind::Spiral, VizSource::Master),
    ] {
        let (mut app, mut engine) = open_tone(&format!("viz_coste_{name}"));
        app.set_viz(vec![spec(kind, source, 8)]);
        play(&mut app, &mut engine, 20);
        // Sin visualizaciones, para restar lo que cuesta el resto.
        let base = {
            let (mut plain, mut e) = open_tone(&format!("viz_coste_base_{name}"));
            play(&mut plain, &mut e, 20);
            let start = std::time::Instant::now();
            for _ in 0..100 {
                plain.tick();
                draw(&mut plain, 140, 45);
            }
            start.elapsed().as_secs_f64() / 100.0
        };
        let start = std::time::Instant::now();
        for _ in 0..100 {
            app.tick();
            draw(&mut app, 140, 45);
        }
        let frame = start.elapsed().as_secs_f64() / 100.0;
        let extra = (frame - base).max(0.0);
        println!(
            "{name}: {:.2} ms por cuadro ({:.1} % de un núcleo a 30 cuadros/s)",
            extra * 1000.0,
            extra * 30.0 * 100.0
        );
    }
}
