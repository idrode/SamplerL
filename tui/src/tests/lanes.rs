//! Panel de las lanes de automatización: se abre y se cierra desde la
//! paleta, enseña las lanes de la pista seleccionada con su curva, sus
//! puntos y el cursor de lo que suena (lo que publica el motor), en las
//! dos disposiciones.

use super::*;
use crate::app::MouseAction;

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
    // El gain: 0.3 la primera mitad y 1 la segunda (escalones), en la
    // escala del volumen (de 0 a 4).
    let row = &mut view.lanes[1];
    row.live = Some(engine::LaneLive {
        value: 1.0,
        phase: Some(24.0),
    });
    // Para ver la curva entera en cuatro filas, la escala de sus puntos.
    row.scale = (0.3, 1.0, false);
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
    // con su valor (1, lo más alto): el rombo; en la primera mitad, vacío.
    let top: Vec<char> = lines[2].chars().collect();
    assert_eq!(top[25], '◆', "{}", lines[2]);
    assert_eq!(top[5], ' ', "{}", lines[2]);
}

#[test]
fn the_live_reading_shows_the_value_its_bar_and_a_mark_on_the_curve() {
    let (_, mut app, mut engine) = open("lanes_live", Some(SONG_LANES));
    palette(&mut app, "lanes de automatización");
    // A mitad del segundo compás de la lane del gain: vale 1.
    play(&mut app, &mut engine, 24);
    let view = app.view().lanes.unwrap();
    let gain = &view.lanes[1];
    let live = gain.live.expect("publicada");
    assert_eq!(live.value, 1.0);
    assert_eq!(gain.reading().as_deref(), Some("1.00"));
    let single = crate::lanes_panel::LanesView {
        track: view.track.clone(),
        lanes: vec![gain.clone()],
    };
    let buffer = draw_widget(crate::lanes_panel::LanesPanel(&single), 60, 7);
    let lines: Vec<String> = text(&buffer).lines().map(str::to_string).collect();
    // El título lleva la lectura y su barra (1 de 0 a 4: un cuarto).
    assert!(
        lines[1].contains("▸ 1.00 en vivo ███░░░░░░░░░"),
        "{}",
        lines[1]
    );
    // El rombo, a un cuarto de la altura (la fila de abajo de las cuatro,
    // redondeado), en la columna del cursor.
    let x = gain.column(live.phase.unwrap() as f64, 58) as usize + 1;
    assert_eq!(lines[4].chars().nth(x), Some('◆'), "{}", lines[4]);
    // Fuera del tramo de una once: el valor fijo, sin cursor.
    let mut once = gain.clone();
    once.live = Some(engine::LaneLive {
        value: 0.8,
        phase: None,
    });
    assert_eq!(once.reading().as_deref(), Some("0.80 fijo"));
    assert_eq!(crate::lanes_panel::number(2345.6), "2346");
    assert_eq!(crate::lanes_panel::number(12.34), "12.3");
}

/// Una canción con una lane escrita tal cual y otra que no.
const SONG_EDIT: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
local p = { {0, 0}, {1, 1} }
track("drums", amen):steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
  :auto("gain", { {0, 0.5}, {2, 2} }, { len = 4 })
  :auto("pan", p)
"#;

/// El panel abierto y dibujado, con su área.
fn open_edit(test: &str) -> (App, Engine, ratatui::layout::Rect) {
    let (_, mut app, engine) = open(test, Some(SONG_EDIT));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    palette(&mut app, "lanes de automatización");
    draw(&mut app, 140, 40);
    let area = app.lanes_area_for_test().expect("dibujado");
    (app, engine, area)
}

/// La celda de la curva de la lane `lane` en la columna `x` (desde su
/// izquierda) y la fila `y` (desde arriba, de 0 a 3).
fn cell(area: ratatui::layout::Rect, lane: u16, x: u16, y: u16) -> (u16, u16) {
    (area.x + 1 + x, area.y + 1 + lane * 5 + 1 + y)
}

fn auto_line(app: &App, param: &str) -> String {
    app.editor
        .lines()
        .join("\n")
        .lines()
        .find(|l| l.contains(&format!("auto(\"{param}\"")))
        .unwrap_or_default()
        .to_string()
}

#[test]
fn a_click_adds_a_point_and_undo_takes_it_away() {
    let (mut app, _engine, area) = open_edit("lanes_add");
    let width = area.width - 2;
    // A tres cuartos de la lane (3 compases de 4), arriba del todo (4, el
    // máximo del volumen): el paso del centro de esa columna.
    let column = width * 3 / 4;
    let (x, y) = cell(area, 0, column, 0);
    app.mouse(MouseAction::Down, x, y);
    let at = ((column as f64 + 0.5) / width as f64 * 64.0).round() / 16.0;
    assert_eq!(
        auto_line(&app, "gain").trim(),
        format!(
            ":auto(\"gain\", {{ {{0, 0.5}}, {{2, 2}}, {{{}, 4}} }}, {{ len = 4 }})",
            script::number_text(at)
        ),
        "{}",
        app.view().message
    );
    assert!(app.view().error.is_none());
    // Un paso de deshacer.
    app.handle(ctrl('z'));
    assert_eq!(
        auto_line(&app, "gain").trim(),
        ":auto(\"gain\", { {0, 0.5}, {2, 2} }, { len = 4 })"
    );
}

#[test]
fn dragging_moves_a_point_and_writes_it_on_release() {
    let (mut app, _engine, area) = open_edit("lanes_drag");
    let width = area.width - 2;
    // El punto de 2 compases (la mitad), a 2 de 4: fila de en medio.
    let column = (width as f64 * 0.5) as u16;
    let (_, y) = cell(area, 0, 0, 1);
    app.mouse(MouseAction::Down, area.x + 1 + column, y);
    // Arrastrado a un cuarto de la lane (1 compás), abajo del todo (0).
    let (x2, y2) = cell(area, 0, width / 4, 3);
    app.mouse(MouseAction::Drag, x2, y2);
    // Mientras se arrastra, el texto no cambia; la vista, sí.
    assert!(auto_line(&app, "gain").contains("{2, 2}"));
    let shown = app.view().lanes.unwrap();
    assert_eq!(shown.lanes[0].lane.points()[1].value, 0.0);
    app.mouse(MouseAction::Up, x2, y2);
    assert_eq!(
        auto_line(&app, "gain").trim(),
        ":auto(\"gain\", { {0, 0.5}, {1, 0} }, { len = 4 })",
        "{}",
        app.view().message
    );
}

#[test]
fn right_click_deletes_and_keys_change_the_chosen_point() {
    let (mut app, mut engine, area) = open_edit("lanes_keys");
    let width = area.width - 2;
    // Elegir el punto del medio con un clic (y soltar sin moverlo).
    let column = (width as f64 * 0.5) as u16;
    let (x, y) = cell(area, 0, column, 1);
    app.mouse(MouseAction::Down, x, y);
    app.mouse(MouseAction::Up, x, y);
    // `c`: su curva pasa a exp. (Entre edición y edición, el motor vacía
    // la cola de comandos, como en vivo.)
    app.handle(key('c'));
    offline::render(&mut engine, 1024, 2);
    assert!(
        auto_line(&app, "gain").contains("{2, 2, \"exp\"}"),
        "{}",
        auto_line(&app, "gain")
    );
    // → un paso a la derecha; ↑ un cuarentavo de la escala (0.1).
    app.handle(Input {
        key: Key::Right,
        ..Input::default()
    });
    offline::render(&mut engine, 1024, 2);
    app.handle(Input {
        key: Key::Up,
        ..Input::default()
    });
    offline::render(&mut engine, 1024, 2);
    assert!(
        auto_line(&app, "gain").contains("{2.0625, 2.1, \"exp\"}"),
        "{}",
        auto_line(&app, "gain")
    );
    // Supr lo borra; Esc suelta.
    app.handle(Input {
        key: Key::Delete,
        ..Input::default()
    });
    offline::render(&mut engine, 1024, 2);
    assert_eq!(
        auto_line(&app, "gain").trim(),
        ":auto(\"gain\", { {0, 0.5} }, { len = 4 })"
    );
    // Clic derecho sobre el que queda: una lane se queda con uno.
    let (x0, y0) = cell(area, 0, 0, 2);
    app.mouse(MouseAction::RightDown, x0, y0);
    assert!(
        app.view().message.contains("se queda con un punto"),
        "{}",
        app.view().message
    );
}

#[test]
fn a_table_that_is_not_literal_is_read_only_and_says_why() {
    let (mut app, _engine, area) = open_edit("lanes_readonly");
    let view = app.view().lanes.unwrap();
    assert!(view.lanes[1].editable.is_err());
    let screen = text(&draw(&mut app, 140, 40));
    assert!(
        screen.contains("pan · 1 compás · loop · solo lectura"),
        "{screen}"
    );
    let before = app.editor.lines().join("\n");
    let (x, y) = cell(area, 1, 10, 1);
    app.mouse(MouseAction::Down, x, y);
    assert_eq!(app.editor.lines().join("\n"), before);
    let message = app.view().message;
    assert!(message.contains("no se puede editar"), "{message}");
    assert!(
        message.contains("escríbela tal cual dentro de la llamada"),
        "{message}"
    );
}

#[test]
fn after_typing_it_waits_for_a_load() {
    let (mut app, _engine, area) = open_edit("lanes_stale");
    // Se escribe algo al final sin cargar.
    app.editor.move_cursor(ratatui_textarea::CursorMove::Bottom);
    type_text(&mut app, "-- nada");
    let before = app.editor.lines().join("\n");
    let (x, y) = cell(area, 0, 5, 0);
    app.mouse(MouseAction::Down, x, y);
    assert_eq!(app.editor.lines().join("\n"), before);
    assert!(
        app.view().message.contains("carga antes"),
        "{}",
        app.view().message
    );
}

#[test]
fn a_table_in_several_lines_keeps_its_shape() {
    let song = "bpm(120)\nlocal amen = sample(\"break.wav\"):slice(2)\n\
                track(\"drums\", amen):steps(\"1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .\")\n  \
                :auto(\"gain\", {\n    {0, 0.5},\n    {2, 2},\n  }, { len = 4 })\n";
    let (_, mut app, _engine) = open("lanes_multi", Some(song));
    palette(&mut app, "lanes de automatización");
    draw(&mut app, 140, 40);
    let area = app.lanes_area_for_test().unwrap();
    // El primer punto está en la columna 0.
    let (x, y) = cell(area, 0, 0, 3);
    app.mouse(MouseAction::RightDown, x, y);
    let text = app.editor.lines().join("\n");
    assert!(
        text.contains("  :auto(\"gain\", {\n    {2, 2},\n  }, { len = 4 })"),
        "{text}"
    );
}

/// Mueve el motor `steps` pasos con un `tick` cada octavo de paso.
fn advance(app: &mut App, engine: &mut Engine, steps: usize) {
    play(app, engine, steps);
}

#[test]
fn touch_with_the_mouse_records_over_the_stretch_it_played() {
    let (mut app, mut engine, area) = open_edit("lanes_touch_mouse");
    palette(&mut app, "grabar lanes");
    assert!(
        app.view().message.contains("armado"),
        "{}",
        app.view().message
    );
    advance(&mut app, &mut engine, 4);
    // Arriba del todo (4) un rato y luego abajo (0), donde sea en x: manda
    // el cursor.
    let (x, top) = cell(area, 0, 10, 0);
    let (_, bottom) = cell(area, 0, 10, 3);
    app.mouse(MouseAction::Down, x, top);
    for _ in 0..16 {
        advance(&mut app, &mut engine, 1);
        app.mouse(MouseAction::Drag, x, top);
    }
    for _ in 0..16 {
        advance(&mut app, &mut engine, 1);
        app.mouse(MouseAction::Drag, x, bottom);
    }
    let before = auto_line(&app, "gain");
    app.mouse(MouseAction::Up, x, bottom);
    let after = auto_line(&app, "gain");
    assert_ne!(after, before, "{}", app.view().message);
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    assert!(
        app.view().message.contains("toma grabada"),
        "{}",
        app.view().message
    );
    // Lo tocado: un tramo a 4 y otro a 0, y el primer punto de antes (el
    // del paso 0, fuera de la toma) se queda.
    assert!(after.contains("{0, 0.5}"), "{after}");
    assert!(after.contains(", 4}"), "{after}");
    assert!(after.contains(", 0}"), "{after}");
    // Desarmar.
    palette(&mut app, "grabar lanes");
    assert!(app.view().message.contains("desarmado"));
}

#[test]
fn touch_with_a_midi_knob_records_when_it_rests() {
    use crate::midi::{MidiEvent, TimedEvent};
    let (mut app, mut engine, _area) = open_edit("lanes_touch_midi");
    let cc = |value: u8| TimedEvent {
        micros: 0,
        event: MidiEvent::Controller {
            channel: 1,
            number: 21,
            value,
        },
    };
    // El CC 21 al volumen de la pista.
    palette(&mut app, "midi learn");
    type_query(&mut app, "drums gain");
    app.handle(special(Key::Enter));
    app.midi_event(cc(0));
    assert!(
        app.view().message.contains("mando asignado"),
        "{}",
        app.view().message
    );
    palette(&mut app, "grabar lanes");
    advance(&mut app, &mut engine, 2);
    for i in 0..24u8 {
        app.midi_event(cc(i * 5));
        advance(&mut app, &mut engine, 1);
    }
    let before = auto_line(&app, "gain");
    // Quieto más de 0,6 s: la toma se cierra en el siguiente tick.
    std::thread::sleep(std::time::Duration::from_millis(700));
    app.tick();
    let after = auto_line(&app, "gain");
    assert_ne!(after, before, "{}", app.view().message);
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
}

#[test]
fn the_fx_panel_shows_the_live_value_next_to_each_parameter_with_a_lane() {
    let (_, mut app, mut engine) = open("lanes_fx_live", Some(SONG_LANES));
    play(&mut app, &mut engine, 6);
    let view = app.view();
    let drums = &view.tracks[0];
    // Lo que vale ahora, de lo que publica el motor: el corte y el
    // volumen.
    let cutoff = drums
        .live
        .iter()
        .find(|l| l.name == "lowpass.cutoff")
        .expect("en vivo");
    assert!((200.0..=8000.0).contains(&cutoff.value), "{}", cutoff.value);
    let gain = drums.live.iter().find(|l| l.name == "gain").unwrap();
    assert_eq!(gain.value, 0.3);
    let screen = text(&draw(&mut app, 140, 40));
    let shown = crate::units::with_unit("lowpass.cutoff", cutoff.value);
    assert!(screen.contains(&format!("∿cutoff ▸{shown}")), "{screen}");
    assert!(screen.contains("vol ▸0.30∿"), "{screen}");
    // Sin lanes, como siempre: ni `▸` ni barras de más.
    let (_, mut plain, mut engine) = open("lanes_fx_plain", Some(SONG));
    play(&mut plain, &mut engine, 2);
    assert!(plain.view().tracks[0].live.is_empty());
    assert!(!text(&draw(&mut plain, 140, 40)).contains('▸'));
}

#[test]
fn a_synth_lane_reads_in_its_own_units() {
    let song = r#"bpm(120)
track("acid", synth("acid", { envmod = 0.5 })):melody("c2 . . .  . . . .  . . . .  . . . .")
  :auto("envmod", { {0, 0.2}, {1, 0.8} })
"#;
    let (_, mut app, mut engine) = open("lanes_synth_units", Some(song));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    play(&mut app, &mut engine, 8);
    let live = app.view().tracks[0]
        .live
        .iter()
        .find(|l| l.name == "synth.envmod")
        .cloned()
        .expect("en vivo");
    // A mitad de la lane: 0.5, en las unidades de Lua (no las del núcleo).
    assert!((0.2..=0.8).contains(&live.value), "{}", live.value);
    palette(&mut app, "lanes de automatización");
    let lanes = app.view().lanes.unwrap();
    let points: Vec<f32> = lanes.lanes[0]
        .lane
        .points()
        .iter()
        .map(|p| p.value)
        .collect();
    assert_eq!(points, [0.2, 0.8]);
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
