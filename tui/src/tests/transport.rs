//! Transporte con atajos y cue (fase S): Ctrl-Espacio arranca y para desde
//! el editor y el modo tocar; Shift-Ctrl-Espacio (o F9) fija el cue, el
//! compás en que empieza el siguiente arranque; la paleta lo borra.

use super::sections::SONG_SECTIONS;
use super::*;
use crate::config::{Bound, KeyCombo, Keymap};
use crate::palette::Action;

/// Un compás a 120 BPM.
const BAR: usize = 16 * STEP;

fn ctrl_space() -> Input {
    Input {
        key: Key::Char(' '),
        ctrl: true,
        ..Input::default()
    }
}

/// Shift-Ctrl-Espacio, como llega con el protocolo de teclado de kitty o
/// las teclas extendidas de tmux.
fn shift_ctrl_space() -> Input {
    Input {
        key: Key::Char(' '),
        ctrl: true,
        shift: true,
        ..Input::default()
    }
}

fn special(key: Key) -> Input {
    Input {
        key,
        ..Input::default()
    }
}

/// El compás que suena (el del último paso disparado).
fn bar(app: &App) -> u64 {
    app.view().transport.bar
}

fn running(app: &App) -> bool {
    app.view().transport.running
}

/// Una tecla y un poco de audio para que el motor la vea.
fn press(app: &mut App, engine: &mut Engine, input: Input) {
    app.handle(input);
    offline::render(engine, 512, 2);
    app.tick();
}

#[test]
fn the_keys_tell_ctrl_space_from_shift_ctrl_space() {
    let keys = Keymap::default();
    assert_eq!(
        keys.lookup(&ctrl_space()),
        Some(Bound::Action(Action::Transport))
    );
    assert_eq!(
        keys.lookup(&shift_ctrl_space()),
        Some(Bound::Action(Action::Cue))
    );
    assert_eq!(
        keys.lookup(&special(Key::F(9))),
        Some(Bound::Action(Action::Cue))
    );
    // Ctrl-T con Shift (si la terminal lo dice) sigue siendo marcha.
    let shifted_t = Input {
        shift: true,
        ..ctrl('t')
    };
    assert_eq!(
        keys.lookup(&shifted_t),
        Some(Bound::Action(Action::Transport))
    );
    let combo = KeyCombo::parse("shift-ctrl-space").unwrap();
    assert_eq!(combo.text(), "Shift-Ctrl-Espacio");
    assert_eq!(
        KeyCombo::parse("ctrl-espacio").unwrap().text(),
        "Ctrl-Espacio"
    );
    assert!(KeyCombo::parse("space").is_err(), "un espacio solo escribe");
    assert_eq!(
        keys.describe(Bound::Action(Action::Transport)),
        "Ctrl-Espacio o Ctrl-T o F8"
    );
    assert_eq!(
        keys.describe(Bound::Action(Action::Cue)),
        "Shift-Ctrl-Espacio o F9"
    );
}

#[test]
fn ctrl_space_starts_and_stops_from_the_editor_and_play_mode() {
    let (_, mut app, mut engine) = open("ctrl_space", Some(SONG));
    let text_before = app.text_for_test();
    press(&mut app, &mut engine, Input::default());
    assert!(running(&app));
    press(&mut app, &mut engine, ctrl_space());
    assert!(!running(&app));
    // No escribe un espacio en el editor.
    assert_eq!(app.text_for_test(), text_before);
    press(&mut app, &mut engine, ctrl_space());
    assert!(running(&app));

    // En el modo tocar, igual.
    app.handle(esc());
    assert!(app.view().playing_mode);
    press(&mut app, &mut engine, ctrl_space());
    assert!(!running(&app));
    press(&mut app, &mut engine, ctrl_space());
    assert!(running(&app));
    assert!(app.view().playing_mode);
}

#[test]
fn the_cue_takes_the_bar_that_sounds_and_the_next_start_begins_there() {
    let (_, mut app, mut engine) = open("cue_running", Some(SONG));
    // Al compás 3 (0, 1, 2, 3) y un poco.
    offline::render(&mut engine, 3 * BAR + 2 * STEP, 2);
    app.tick();
    assert_eq!(bar(&app), 3);
    app.handle(shift_ctrl_space());
    assert!(
        app.view().message.contains("cue en compás 3"),
        "{}",
        app.view().message
    );
    // En la barra de estado.
    let screen = text(&draw(&mut app, 130, 40));
    assert!(screen.contains("⏮ cue compás 3"), "{screen}");

    // Sigue sonando; al parar y arrancar, empieza en el compás 3.
    offline::render(&mut engine, 2 * BAR, 2);
    app.tick();
    assert_eq!(bar(&app), 5);
    press(&mut app, &mut engine, ctrl_space());
    assert!(!running(&app));
    press(&mut app, &mut engine, ctrl_space());
    assert!(running(&app));
    assert_eq!(bar(&app), 3);
    assert!(app.view().message.contains("desde el cue"));
    // Y cada arranque vuelve a él.
    offline::render(&mut engine, 2 * BAR, 2);
    press(&mut app, &mut engine, ctrl_space());
    press(&mut app, &mut engine, ctrl_space());
    assert_eq!(bar(&app), 3);

    // Borrado desde la paleta: desde el principio.
    app.run_action(Action::ClearCue);
    assert!(app.view().message.contains("cue borrado"));
    assert!(app.view().cue.is_none());
    press(&mut app, &mut engine, ctrl_space());
    press(&mut app, &mut engine, ctrl_space());
    assert_eq!(bar(&app), 0);
    app.run_action(Action::ClearCue);
    assert_eq!(app.view().message, "no hay cue");
}

#[test]
fn stopped_the_cue_takes_the_bar_of_the_tracker_cursor() {
    const LONG: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("largo", amen)
  :length(32)
  :steps("1 . . .  . . . .  . . . .  . . . .  2 . . .  . . . .  . . . .  . . . .")
"#;
    let (_, mut app, mut engine) = open("cue_tracker", Some(LONG));
    // Dos vueltas de 2 compases y medio compás más: para en el compás 4.
    offline::render(&mut engine, 4 * BAR + 4 * STEP, 2);
    app.tick();
    press(&mut app, &mut engine, ctrl_space());
    assert!(!running(&app));
    assert_eq!(bar(&app), 4);
    // Sin cursor: donde se paró.
    app.handle(f(9));
    assert_eq!(app.view().cue.as_deref(), Some("compás 4"));
    // Con el cursor del tracker en el paso 20 de la vuelta que se ve (la
    // tercera, desde el compás 4): el compás 5.
    app.run_action(Action::EditTracker);
    for _ in 0..20 {
        app.handle(special(Key::Down));
    }
    assert_eq!(app.view().tracker_cursor, Some((0, 20)));
    app.handle(shift_ctrl_space());
    assert_eq!(app.view().cue.as_deref(), Some("compás 5"));
    press(&mut app, &mut engine, ctrl_space());
    assert!(running(&app));
    assert_eq!(bar(&app), 5);
    // Suena el paso que toca: el compás 5 es la segunda mitad de la vuelta.
    assert_eq!(app.view().tracks[0].position, Some(16));
}

#[test]
fn with_sections_the_cue_is_the_entry_and_shows_on_the_timeline() {
    let (_, mut app, mut engine) = open("cue_sections", Some(SONG_SECTIONS));
    // intro 2 · drop 4 · break 1: el compás 3 es el segundo del drop.
    offline::render(&mut engine, 3 * BAR + STEP, 2);
    app.tick();
    app.handle(shift_ctrl_space());
    assert_eq!(app.view().cue.as_deref(), Some("drop (entrada 2)"));
    assert_eq!(app.view().timeline.unwrap().cue, Some(1));
    let screen = text(&draw(&mut app, 170, 44));
    assert!(screen.contains("⏮ ▶ drop"), "{screen}");
    assert!(screen.contains("⏮ cue drop (entrada 2)"), "{screen}");

    // Hasta el break, parar y arrancar: el drop desde su principio.
    offline::render(&mut engine, 3 * BAR, 2);
    app.tick();
    assert_eq!(app.view().timeline.unwrap().current, 2);
    press(&mut app, &mut engine, ctrl_space());
    press(&mut app, &mut engine, ctrl_space());
    let timeline = app.view().timeline.unwrap();
    assert_eq!((timeline.current, timeline.bar), (1, 0));
    assert_eq!(bar(&app), 2);
    // Sus pistas: las del drop.
    assert_eq!(app.view().tracks.len(), 2);
}

fn f(n: u8) -> Input {
    special(Key::F(n))
}
