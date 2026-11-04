//! Panel de las lanes de automatización: se abre y se cierra desde la
//! paleta, enseña las lanes de la pista seleccionada con su curva, sus
//! puntos y el cursor de lo que suena (lo que publica el motor), en las
//! dos disposiciones.

use super::*;

const SONG_LANES: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
  :fx("lowpass", { cutoff = 3000 })
  :auto("cutoff", { {0, 200, "exp"}, {2, 8000, "exp"}, {4, 200} }, { len = "4 compases" })
  :auto("gain", { {0, 0.3, "step"}, {1, 1, "step"}, {2, 0.3} }, { len = 2 })
track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
"#;

/// Mueve el motor `steps` pasos de 1/8 en 1/8, con un `tick` en cada uno.
fn play(app: &mut App, engine: &mut Engine, steps: usize) {
    for _ in 0..8 * steps {
        offline::render(engine, STEP / 8, 2);
        app.tick();
    }
}

#[test]
fn the_palette_opens_and_closes_it_with_the_lanes_of_the_track() {
    let (_, mut app, mut engine) = open("lanes_panel", Some(SONG_LANES));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    assert!(app.view().lanes.is_none());
    palette(&mut app, "lanes de automatización");
    let view = app.view().lanes.expect("abierto");
    assert_eq!(view.track, "drums");
    let titles: Vec<String> = view.lanes.iter().map(|l| l.title()).collect();
    assert_eq!(
        titles,
        [
            "lowpass.cutoff · 4 compases · loop",
            "gain · 2 compases · loop"
        ]
    );
    // El cursor sigue al motor: tras 6 pasos y medio, la lane va por ahí.
    play(&mut app, &mut engine, 6);
    let view = app.view().lanes.unwrap();
    let phase = view.lanes[0].live.and_then(|l| l.phase).expect("suena");
    assert!((5.0..7.0).contains(&phase), "{phase}");
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("lanes · drums"), "{screen}");
    assert!(
        screen.contains("lowpass.cutoff · 4 compases · loop"),
        "{screen}"
    );
    assert!(screen.contains('●'), "{screen}");
    // Otra vez, se cierra.
    palette(&mut app, "lanes de automatización");
    assert!(app.view().lanes.is_none());
}

#[test]
fn the_curve_its_points_and_the_cursor_are_where_they_should_be() {
    let (_, mut app, _) = open("lanes_curve", Some(SONG_LANES));
    palette(&mut app, "lanes de automatización");
    let mut view = app.view().lanes.unwrap();
    // El gain: 0.3 la primera mitad y 1 la segunda (escalones).
    let row = &mut view.lanes[1];
    row.live = Some(engine::LaneLive {
        value: 1.0,
        phase: Some(24.0),
    });
    assert_eq!(row.level(0.3), 0.0);
    assert_eq!(row.level(1.0), 1.0);
    assert_eq!(row.column(24.0, 32), 24);
    let single = crate::lanes_panel::LanesView {
        track: view.track.clone(),
        lanes: vec![row.clone()],
    };
    let buffer = draw_widget(crate::lanes_panel::LanesPanel(&single), 34, 7);
    let lines: Vec<String> = text(&buffer).lines().map(str::to_string).collect();
    // Fila de abajo de la curva (la 5): llena en toda la segunda mitad, y
    // con un octavo... nada en la primera (0.3 es el mínimo).
    let bottom: Vec<char> = lines[5].chars().collect();
    assert_eq!(bottom[1], '●', "{}", lines[5]);
    assert!(
        bottom[17..33].iter().all(|&c| c == '█' || c == '●'),
        "{}",
        lines[5]
    );
    // Arriba, en la columna 24 de la curva (25 con el borde), el cursor
    // está sobre lo lleno; en la primera mitad, vacío.
    let top: Vec<char> = lines[2].chars().collect();
    assert_eq!(top[25], '█', "{}", lines[2]);
    assert_eq!(top[5], ' ', "{}", lines[2]);
}

/// Para ver el panel sin terminal: `cargo test -p tui pantalla_lanes --
/// --ignored --nocapture`.
#[test]
#[ignore]
fn pantalla_lanes() {
    let (_, mut app, mut engine) = open("lanes_pantalla", Some(SONG_LANES));
    palette(&mut app, "lanes de automatización");
    play(&mut app, &mut engine, 10);
    println!("{}", text(&draw(&mut app, 140, 45)));
}
