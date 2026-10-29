//! MIDI learn desde la interfaz: elegir el parámetro en la paleta y mover un
//! mando lo asigna; el mando mueve el sonido (la tabla entra al compás); la
//! lista de mandos borra, cambia la curva, otro mando y el rango; se guarda
//! en el proyecto, se exporta a `config.toml` y un proyecto sin mandos usa
//! los de la configuración; el panel de dispositivos los enseña.

use super::devices::with_devices;
use super::*;
use crate::config::Config;
use crate::midi::{MidiEvent, TimedEvent};

const SONG_LEARN: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen)
  :steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
  :fx("lowpass", { cutoff = 6000 })
"#;

fn cc(channel: u8, number: u8, value: u8) -> TimedEvent {
    TimedEvent {
        micros: 0,
        event: MidiEvent::Controller {
            channel,
            number,
            value,
        },
    }
}

/// Elige en «MIDI learn» lo que filtra `query` y mueve el CC `number`.
fn learn(app: &mut App, query: &str, number: u8) {
    palette(app, "midi learn");
    type_query(app, query);
    app.handle(special(Key::Enter));
    app.midi_event(cc(1, number, 64));
}

#[test]
fn choosing_a_parameter_and_moving_a_knob_assigns_it() {
    let (path, mut app, mut engine) = open("learn_assign", Some(SONG_LEARN));
    palette(&mut app, "midi learn");
    let picker = app.view().knob_picker.unwrap();
    let names: Vec<&str> = picker.items.iter().map(|(n, _, _)| n.as_str()).collect();
    assert!(names.contains(&"drums · lowpass.cutoff"), "{names:?}");
    assert!(names.contains(&"drums · gain"), "{names:?}");
    assert!(names.contains(&"bus · reverb.size"), "{names:?}");
    for (width, height) in [(120, 40), (80, 24)] {
        let screen = text(&draw(&mut app, width, height));
        assert!(
            screen.contains("MIDI learn: elige el parámetro"),
            "{screen}"
        );
    }
    type_query(&mut app, "lowpass cutoff");
    assert_eq!(app.view().knob_picker.unwrap().items.len(), 1);
    app.handle(special(Key::Enter));
    assert!(app.view().knob_picker.is_none());
    assert!(
        app.view()
            .message
            .contains("mueve el mando para drums · lowpass.cutoff")
    );
    // Un CC: queda asignado, con su rango entero y curva logarítmica.
    app.midi_event(cc(1, 21, 127));
    assert_eq!(
        app.learned()
            .iter()
            .map(|l| l.to_line())
            .collect::<Vec<_>>(),
        ["1 21 drums lowpass.cutoff 20 20000 log"]
    );
    assert!(
        app.view()
            .message
            .starts_with("mando asignado: CC 21 · canal 1 → drums · lowpass.cutoff"),
        "{}",
        app.view().message
    );
    // Sin proyecto, se guarda con «guardar».
    command(&mut app, "guardar");
    let project = std::fs::read_to_string(path.with_extension("sampler")).unwrap();
    assert!(
        project.contains("mando = 1 21 drums lowpass.cutoff 20 20000 log\n"),
        "{project}"
    );

    // El mando mueve el filtro (la tabla entra en el compás siguiente).
    let highs = |x: &[f32]| {
        x.windows(4)
            .step_by(2)
            .map(|w| (w[2] - w[0]).powi(2))
            .sum::<f32>()
    };
    offline::render(&mut engine, 16 * STEP, 2);
    let open = highs(&offline::render(&mut engine, 8 * STEP, 2));
    app.midi_event(cc(1, 21, 0));
    offline::render(&mut engine, 8 * STEP, 2);
    let closed = highs(&offline::render(&mut engine, 8 * STEP, 2));
    assert!(closed < 0.2 * open, "{closed} {open}");
}

#[test]
fn a_cc_moves_one_thing_and_learning_can_be_cancelled() {
    let (_, mut app, _engine) = open("learn_one_cc", Some(SONG_LEARN));
    learn(&mut app, "drums gain", 7);
    learn(&mut app, "drums pan", 7);
    // El CC 7 pasa al pan.
    assert_eq!(
        app.learned().iter().map(|l| l.target()).collect::<Vec<_>>(),
        ["drums · pan"]
    );
    palette(&mut app, "midi learn");
    app.handle(special(Key::Enter));
    app.handle(esc());
    assert!(app.view().message.contains("cancelado"));
    app.midi_event(cc(1, 9, 1));
    assert_eq!(app.learned().len(), 1);
}

#[test]
fn the_knob_list_deletes_changes_the_curve_and_the_range() {
    let (_, mut app, _engine) = open("learn_list", Some(SONG_LEARN));
    learn(&mut app, "lowpass cutoff", 21);
    learn(&mut app, "drums gain", 22);
    palette(&mut app, "mandos midi");
    let view = app.view().knobs.unwrap();
    assert_eq!(view.rows.len(), 2);
    assert_eq!(view.rows[1].1, "CC 22 · canal 1");
    for (width, height) in [(120, 40), (70, 20)] {
        let screen = text(&draw(&mut app, width, height));
        assert!(screen.contains("drums · lowpass.cutoff"), "{screen}");
        assert!(screen.contains("20–20000 log"), "{screen}");
    }
    // c: la curva siguiente.
    app.handle(key('c'));
    assert_eq!(app.learned()[0].curve, engine::KnobCurve::Square);
    // Enter: el rango, en la línea de órdenes.
    app.handle(special(Key::Enter));
    assert_eq!(app.view().prompt.as_deref(), Some("mando 1 20 20000 cuad"));
    app.handle(esc());
    command(&mut app, "mando 1 200 8000 log");
    assert_eq!(
        app.learned()[0].to_line(),
        "1 21 drums lowpass.cutoff 200 8000 log"
    );
    // l: otro mando para el mismo, con su rango.
    palette(&mut app, "mandos midi");
    app.handle(key('l'));
    app.midi_event(cc(2, 30, 0));
    assert_eq!(
        app.learned()[0].to_line(),
        "2 30 drums lowpass.cutoff 200 8000 log"
    );
    // Supr: lo borra.
    palette(&mut app, "mandos midi");
    app.handle(special(Key::Down));
    app.handle(special(Key::Delete));
    assert_eq!(app.learned().len(), 1);
    assert!(
        app.view()
            .message
            .starts_with("mando borrado: drums · gain")
    );
}

#[test]
fn the_project_keeps_them_and_the_config_is_the_default() {
    let (path, mut app, _engine) = open("learn_project", Some(SONG_LEARN));
    learn(&mut app, "lowpass cutoff", 21);
    command(&mut app, "guardar");
    let project = path.with_extension("sampler");
    // Otra vez abierto: los mismos mandos.
    let (controller, _engine) = new_engine(RATE);
    let reopened = App::open_project(&project, controller, RATE).unwrap();
    assert_eq!(reopened.learned(), app.learned());

    // La configuración, en un proyecto sin los suyos.
    let config = Config::parse(
        "[midi]\nmandos = [\"* 74 drums gain 0 1 lin\"]\n",
        true,
        "c.toml",
    );
    assert!(config.warnings.is_empty(), "{:?}", config.warnings);
    let (_, mut fresh, _engine) = open("learn_config", Some(SONG_LEARN));
    fresh.set_config(&config);
    assert_eq!(fresh.learned()[0].to_line(), "* 74 drums gain 0 1 lin");
    // Con los suyos, los del proyecto.
    let (controller, _engine) = new_engine(RATE);
    let mut own = App::open_project(&project, controller, RATE).unwrap();
    own.set_config(&config);
    assert_eq!(own.learned()[0].cc, 21);

    // Exportar: a [midi] mandos de config.toml, sin tocar lo demás.
    let toml = path.with_file_name("config.toml");
    std::fs::write(&toml, "# mío\n[midi]\nnota_base = 60\n").unwrap();
    own.set_devices_config(Some(toml.clone()));
    palette(&mut own, "exportar los mandos");
    let written = std::fs::read_to_string(&toml).unwrap();
    assert!(
        written.starts_with("# mío\n[midi]\nnota_base = 60\nmandos = [\n"),
        "{written}"
    );
    let back = Config::load(&toml, true);
    assert!(back.warnings.is_empty(), "{:?}", back.warnings);
    assert_eq!(back.learned, own.learned());
    assert_eq!(back.midi.base_note, 60);
}

#[test]
fn the_devices_panel_shows_the_knobs() {
    let (mut app, _audio, _midi) = with_devices("learn_devices");
    palette(&mut app, "dispositivos");
    let line = app.view().devices.unwrap().view.knobs;
    assert_eq!(line, "sin mandos");
    app.midi_event(cc(3, 21, 64));
    assert_eq!(
        app.view().devices.unwrap().view.knobs,
        "sin mandos · último CC 21 (canal 3) = 64"
    );
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("⊙ sin mandos · último CC 21"), "{screen}");
}
