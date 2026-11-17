//! Ajustar números con el ratón y el teclado: Ctrl y la rueda sobre un
//! número lo cambia (con el rango del registro si es un parámetro), la rueda
//! sola no; una ráfaga es un paso de deshacer y acaba tras una pausa o con
//! otro evento; Esc la cancela; la cola de la inercia no cuenta; Ctrl,
//! pulsar y arrastrar; Alt-A y Alt-X; Ctrl-A y Ctrl-X en vim; y lo que se ve.

use std::time::{Duration, Instant};

use super::*;
use crate::app::{Mods, MouseAction};
use crate::config::Config;

const NUMBERS: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
  :fx("filter", { type = "lowpass", cutoff = 3000, q = 0.7 })
  :gain(0.8):pan(-0.25) -- gain(2)
print(17, 0.25)
"#;

const CTRL: Mods = Mods {
    ctrl: true,
    alt: false,
};
const NONE: Mods = Mods {
    ctrl: false,
    alt: false,
};

fn open_numbers(test: &str) -> (App, Engine) {
    let (_, mut app, engine) = open(test, Some(NUMBERS));
    draw(&mut app, 120, 40);
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    (app, engine)
}

/// La celda de la terminal de la fila y columna del texto.
fn cell(app: &App, row: usize, col: usize) -> (u16, u16) {
    let area = app.editor_area.expect("el editor se dibujó");
    let (top, left) = app.editor.scroll_offset();
    let x = area.x + app.editor.line_number_width() + (col - left as usize) as u16;
    (x, area.y + (row - top as usize) as u16)
}

/// La celda del primer carácter de `needle` en la fila `row`.
fn on(app: &App, row: usize, needle: &str) -> (u16, u16) {
    let line = &app.editor.lines()[row];
    let byte = line
        .find(needle)
        .unwrap_or_else(|| panic!("{needle} en {line}"));
    cell(app, row, line[..byte].chars().count())
}

fn line(app: &App, row: usize) -> String {
    app.editor.lines()[row].clone()
}

/// Ctrl y la rueda (un paso), `ms` después de `t`.
fn ctrl_wheel(app: &mut App, up: bool, (x, y): (u16, u16), t: &mut Instant, ms: u64) {
    *t += Duration::from_millis(ms);
    let action = if up {
        MouseAction::ZoomIn
    } else {
        MouseAction::ZoomOut
    };
    app.mouse_at(action, x, y, CTRL, *t);
}

/// Deja pasar la pausa que acaba el gesto.
fn pause(app: &mut App, t: &mut Instant) {
    *t += Duration::from_millis(600);
    app.adjust_tick_for_test(*t);
}

#[test]
fn ctrl_and_the_wheel_change_the_number_under_the_pointer() {
    let (mut app, _engine) = open_numbers("adjust_wheel");
    let mut t = Instant::now();
    let at = on(&app, 0, "120");
    ctrl_wheel(&mut app, true, at, &mut t, 0);
    assert_eq!(line(&app, 0), "bpm(121)");
    ctrl_wheel(&mut app, true, at, &mut t, 50);
    ctrl_wheel(&mut app, false, at, &mut t, 50);
    ctrl_wheel(&mut app, false, at, &mut t, 50);
    ctrl_wheel(&mut app, false, at, &mut t, 50);
    assert_eq!(line(&app, 0), "bpm(119)");
    assert_eq!(app.editor.cursor(), (0, 0), "el cursor no se mueve");
    // Un parámetro del registro: el corte del filtro, en escala
    // logarítmica (un sesentavo de las tres décadas: unos 2 semitonos).
    pause(&mut app, &mut t);
    let at = on(&app, 3, "3000");
    ctrl_wheel(&mut app, true, at, &mut t, 0);
    let cutoff = line(&app, 3);
    assert!(cutoff.contains("cutoff = 3366,"), "{cutoff}");
    // Varios en una línea, y el negativo.
    pause(&mut app, &mut t);
    let at = on(&app, 4, "-0.25");
    ctrl_wheel(&mut app, true, at, &mut t, 0);
    assert_eq!(line(&app, 4), "  :gain(0.8):pan(-0.22) -- gain(2)");
    // Fuera de un número, o en el comentario: nada (ni desplaza).
    pause(&mut app, &mut t);
    let before = app.editor.lines().to_vec();
    for (row, needle) in [(2, "track"), (4, "gain(2)"), (2, "1 . 2")] {
        let at = on(&app, row, needle);
        ctrl_wheel(&mut app, true, at, &mut t, 700);
    }
    assert_eq!(app.editor.lines(), before);
    assert_eq!(app.editor.scroll_offset(), (0, 0));
    assert!(
        app.view().message.contains("cadena"),
        "{}",
        app.view().message
    );
}

#[test]
fn the_wheel_without_ctrl_does_not_change_numbers() {
    let (mut app, _engine) = open_numbers("adjust_plain_wheel");
    let at = on(&app, 0, "120");
    let mut t = Instant::now();
    for action in [MouseAction::ScrollUp, MouseAction::ScrollDown] {
        t += Duration::from_secs(1);
        app.mouse_at(action, at.0, at.1, NONE, t);
    }
    assert_eq!(line(&app, 0), "bpm(120)");
    // La primera vez sobre un número, lo dice (por si la terminal se queda
    // Ctrl y la rueda).
    assert!(
        app.view().message.contains("Ctrl y la rueda"),
        "{}",
        app.view().message
    );
    // Con Alt, tampoco (el modificador por defecto es Ctrl).
    t += Duration::from_secs(1);
    app.mouse_at(
        MouseAction::ScrollUp,
        at.0,
        at.1,
        Mods {
            ctrl: false,
            alt: true,
        },
        t,
    );
    assert_eq!(line(&app, 0), "bpm(120)");
}

#[test]
fn alt_can_be_the_modifier_and_the_direction_can_be_inverted() {
    let (mut app, _engine) = open_numbers("adjust_alt");
    let mut config = Config::default();
    config.editor.adjust.modifier = crate::config::AdjustModifier::Alt;
    config.editor.adjust.invert = true;
    app.set_config(&config);
    draw(&mut app, 120, 40);
    let at = on(&app, 0, "120");
    let alt = Mods {
        ctrl: false,
        alt: true,
    };
    let t = Instant::now();
    app.mouse_at(MouseAction::ScrollUp, at.0, at.1, alt, t);
    assert_eq!(line(&app, 0), "bpm(119)", "invertido: hacia arriba baja");
    // Ctrl y la rueda ya no cambian números.
    app.mouse_at(
        MouseAction::ZoomIn,
        at.0,
        at.1,
        CTRL,
        t + Duration::from_secs(2),
    );
    assert_eq!(line(&app, 0), "bpm(119)");
}

#[test]
fn a_long_burst_is_one_undo_step_and_loads_when_it_ends() {
    let (mut app, _engine) = open_numbers("adjust_undo");
    let loads = app.loads_for_test();
    let at = on(&app, 0, "120");
    let mut t = Instant::now();
    for _ in 0..40 {
        ctrl_wheel(&mut app, true, at, &mut t, 30);
    }
    assert_eq!(line(&app, 0), "bpm(160)");
    assert_eq!(app.loads_for_test(), loads, "no se carga a mitad del gesto");
    pause(&mut app, &mut t);
    assert_eq!(app.loads_for_test(), loads + 1, "al acabar, se carga");
    assert!(
        app.view().message.contains("compás siguiente"),
        "{}",
        app.view().message
    );
    // Después de acabar, la rueda empieza otro gesto (no sigue el de antes).
    ctrl_wheel(&mut app, true, at, &mut t, 0);
    assert_eq!(line(&app, 0), "bpm(161)");
    pause(&mut app, &mut t);
    // Ctrl-Z deshace el último gesto entero; otro, el primero.
    app.handle(ctrl('z'));
    assert_eq!(line(&app, 0), "bpm(160)");
    app.handle(ctrl('z'));
    assert_eq!(line(&app, 0), "bpm(120)");
    app.handle(ctrl('r'));
    assert_eq!(line(&app, 0), "bpm(160)");
}

#[test]
fn esc_cancels_the_gesture_and_stays_in_the_editor() {
    let (mut app, _engine) = open_numbers("adjust_esc");
    let loads = app.loads_for_test();
    let at = on(&app, 3, "0.7");
    let mut t = Instant::now();
    for _ in 0..5 {
        ctrl_wheel(&mut app, false, at, &mut t, 30);
    }
    assert!(!line(&app, 3).contains("q = 0.7 "), "{}", line(&app, 3));
    app.handle(Input {
        key: Key::Esc,
        ..Default::default()
    });
    assert!(line(&app, 3).contains("q = 0.7 })"), "{}", line(&app, 3));
    assert!(!app.view().playing_mode, "Esc solo cancela");
    assert!(
        app.view().message.contains("cancelado"),
        "{}",
        app.view().message
    );
    pause(&mut app, &mut t);
    assert_eq!(app.loads_for_test(), loads, "cancelado no carga");
    // Y no deja nada que deshacer del gesto.
    app.handle(ctrl('z'));
    assert!(line(&app, 3).contains("q = 0.7 })"));
}

#[test]
fn another_key_or_a_click_ends_the_gesture() {
    let (mut app, _engine) = open_numbers("adjust_other_event");
    let at = on(&app, 0, "120");
    let mut t = Instant::now();
    ctrl_wheel(&mut app, true, at, &mut t, 0);
    let loads = app.loads_for_test();
    app.handle(Input {
        key: Key::End,
        ..Default::default()
    });
    assert_eq!(app.loads_for_test(), loads + 1, "la tecla acaba el gesto");
    assert_eq!(app.editor.cursor(), (0, 8), "y hace lo suyo");
    ctrl_wheel(&mut app, true, at, &mut t, 500);
    assert_eq!(line(&app, 0), "bpm(122)");
    app.mouse_at(MouseAction::Down, at.0, at.1 + 1, NONE, t);
    assert_eq!(app.loads_for_test(), loads + 2, "el clic también");
}

#[test]
fn the_tail_of_the_inertia_and_the_pointer_on_another_number_do_not_change_it() {
    let (mut app, _engine) = open_numbers("adjust_tail");
    let mut t = Instant::now();
    let first = on(&app, 5, "17");
    let second = on(&app, 5, "0.25");
    ctrl_wheel(&mut app, true, first, &mut t, 0);
    ctrl_wheel(&mut app, true, first, &mut t, 20);
    assert_eq!(line(&app, 5), "print(19, 0.25)");
    // El puntero llega a otro número a mitad de la ráfaga: el gesto acaba
    // y lo que sigue de la ráfaga (la inercia) no cambia nada.
    let second = (second.0 + 1, second.1);
    for _ in 0..10 {
        ctrl_wheel(&mut app, true, second, &mut t, 20);
    }
    assert_eq!(line(&app, 5), "print(19, 0.25)");
    // Tras una pausa, otro gesto sí.
    ctrl_wheel(&mut app, true, second, &mut t, 300);
    assert_eq!(line(&app, 5), "print(19, 0.26)");
    // Moverse fuera del número también acaba el gesto.
    let loads = app.loads_for_test();
    assert!(app.pointer_moved(0, 0, CTRL, t));
    assert_eq!(app.loads_for_test(), loads + 1);
    // Y lo mismo tras Esc: la cola no empieza otro.
    ctrl_wheel(&mut app, true, second, &mut t, 300);
    app.handle(Input {
        key: Key::Esc,
        ..Default::default()
    });
    for _ in 0..5 {
        ctrl_wheel(&mut app, true, second, &mut t, 20);
    }
    assert_eq!(line(&app, 5), "print(19, 0.26)");
}

#[test]
fn the_rate_is_limited() {
    let (mut app, _engine) = open_numbers("adjust_rate");
    let at = on(&app, 0, "120");
    let mut t = Instant::now();
    // Cien pasos en 100 ms: caben la ráfaga y lo que se llena en ese
    // tiempo.
    for _ in 0..100 {
        ctrl_wheel(&mut app, true, at, &mut t, 1);
    }
    let bpm: f64 = line(&app, 0)[4..7].parse().unwrap();
    let most = 120.0 + crate::app::adjust_limits().1 + 0.1 * crate::app::adjust_limits().0 + 1.0;
    assert!(bpm > 125.0 && bpm <= most, "{bpm} (como mucho {most})");
}

#[test]
fn registry_limits_are_kept_and_never_produce_nan() {
    let (mut app, _engine) = open_numbers("adjust_limits");
    let at = on(&app, 4, "0.8");
    let mut t = Instant::now();
    for _ in 0..200 {
        ctrl_wheel(&mut app, true, at, &mut t, 30);
    }
    assert_eq!(
        line(&app, 4),
        "  :gain(4.0):pan(-0.25) -- gain(2)",
        "con su decimal"
    );
    let info = app.adjust_info().unwrap();
    assert_eq!(info.label, ":gain");
    assert_eq!(info.range, "0–4");
    assert_eq!(info.fraction, Some(1.0));
    pause(&mut app, &mut t);
    let at = on(&app, 4, "-0.25");
    for _ in 0..200 {
        ctrl_wheel(&mut app, false, at, &mut t, 30);
    }
    assert_eq!(line(&app, 4), "  :gain(4.0):pan(-1.00) -- gain(2)");
}

#[test]
fn ctrl_click_and_drag_changes_it_and_releasing_ends() {
    let (mut app, _engine) = open_numbers("adjust_drag");
    let (x, y) = on(&app, 0, "120");
    let t = Instant::now();
    app.mouse_at(MouseAction::Down, x, y, CTRL, t);
    assert_eq!(app.editor.cursor(), (0, 0), "no mueve el cursor");
    app.mouse_at(
        MouseAction::Drag,
        x + 5,
        y,
        CTRL,
        t + Duration::from_millis(100),
    );
    assert_eq!(line(&app, 0), "bpm(125)");
    // Hacia arriba, dos pasos por fila.
    app.mouse_at(
        MouseAction::Drag,
        x + 5,
        y - 1,
        CTRL,
        t + Duration::from_millis(200),
    );
    assert_eq!(line(&app, 0), "bpm(127)");
    let loads = app.loads_for_test();
    app.mouse_at(
        MouseAction::Up,
        x + 5,
        y - 1,
        CTRL,
        t + Duration::from_millis(300),
    );
    assert_eq!(app.loads_for_test(), loads + 1);
    assert!(app.editor.selection_range().is_none(), "no seleccionó");
    app.handle(ctrl('z'));
    assert_eq!(line(&app, 0), "bpm(120)");
    // Si el «soltar» se pierde, la pausa lo cierra igual.
    app.mouse_at(MouseAction::Down, x, y, CTRL, t + Duration::from_secs(2));
    app.mouse_at(
        MouseAction::Drag,
        x + 1,
        y,
        CTRL,
        t + Duration::from_secs(2),
    );
    app.adjust_tick_for_test(t + Duration::from_secs(3));
    assert!(!app.adjusting());
    // Sin Ctrl, pulsar y arrastrar selecciona como siempre.
    app.mouse_at(MouseAction::Down, x, y, NONE, t + Duration::from_secs(4));
    app.mouse_at(
        MouseAction::Drag,
        x + 2,
        y,
        NONE,
        t + Duration::from_secs(4),
    );
    assert!(app.editor.selection_range().is_some());
    assert_eq!(line(&app, 0), "bpm(121)");
}

#[test]
fn alt_a_and_alt_x_change_the_number_at_the_cursor() {
    let (mut app, _engine) = open_numbers("adjust_keys");
    let alt = |c: char, shift: bool| Input {
        key: Key::Char(c),
        alt: true,
        shift,
        ..Default::default()
    };
    // El cursor al principio de la línea: el primer número a su derecha.
    app.handle(alt('a', false));
    app.handle(alt('a', false));
    assert_eq!(line(&app, 0), "bpm(122)");
    app.handle(alt('x', false));
    assert_eq!(line(&app, 0), "bpm(121)");
    app.handle(alt('A', true));
    assert_eq!(line(&app, 0), "bpm(131)");
    // Todo seguido es un paso de deshacer.
    let mut t = Instant::now();
    pause(&mut app, &mut t);
    app.handle(ctrl('z'));
    assert_eq!(line(&app, 0), "bpm(120)");
    // Sin número en la línea, lo dice.
    app.editor
        .move_cursor(ratatui_textarea::CursorMove::Jump(2, 0));
    app.handle(alt('a', false));
    assert!(
        app.view().message.contains("no hay ningún número"),
        "{}",
        app.view().message
    );
}

#[test]
fn ctrl_a_and_ctrl_x_in_vim_normal() {
    let (mut app, _engine) = open_numbers("adjust_vim");
    let mut config = Config::default();
    config.editor.vim = true;
    app.set_config(&config);
    app.handle(ctrl('a'));
    assert_eq!(line(&app, 0), "bpm(121)");
    app.handle(ctrl('x'));
    app.handle(ctrl('x'));
    assert_eq!(line(&app, 0), "bpm(119)");
}

#[test]
fn the_number_is_highlighted_and_the_line_says_value_range_and_when() {
    let (mut app, _engine) = open_numbers("adjust_draw");
    draw(&mut app, 200, 60);
    let at = on(&app, 3, "3000");
    // Con Ctrl, el puntero sobre un número lo resalta; sin Ctrl, nada (ni
    // pide dibujar).
    let t = Instant::now();
    assert!(!app.pointer_moved(at.0, at.1, NONE, t));
    assert!(app.pointer_moved(at.0, at.1, CTRL, t));
    assert_eq!(app.adjust_mark(), Some((3, 45..49)));
    assert!(
        !app.pointer_moved(at.0 + 1, at.1, CTRL, t),
        "el mismo número"
    );
    assert!(app.pointer_moved(at.0, at.1, NONE, t));
    assert_eq!(app.adjust_mark(), None);
    // Durante el gesto, la línea de abajo.
    app.mouse_at(MouseAction::ZoomIn, at.0, at.1, CTRL, t);
    let buffer = draw(&mut app, 200, 60);
    let screen = text(&buffer);
    assert!(
        screen.contains("◆ fx(\"filter\") cutoff = 3366 Hz"),
        "{screen}"
    );
    assert!(screen.contains("20–20000 Hz · log"), "{screen}");
    assert!(screen.contains("se oirá en el compás siguiente · Esc cancela"));
    assert!(screen.contains('━') && screen.contains('─'), "el fader");
    let (x, y) = find(&buffer, "3366").unwrap();
    let style = buffer[(x, y)].style();
    assert_eq!(style.bg, Some(crate::ui::accent()), "resaltado");
    // En ASCII, sin Unicode.
    crate::theme::set_ascii(true);
    let screen = text(&draw(&mut app, 200, 60));
    crate::theme::set_ascii(false);
    assert!(
        screen.contains("# fx(\"filter\") cutoff = 3366 Hz"),
        "{screen}"
    );
    assert!(screen.contains("===>---"), "{screen}");
    // Estrecho: primero sobra el fader y después el rango.
    let narrow = text(&draw(&mut app, 120, 40));
    assert!(narrow.contains("cutoff = 3366 Hz  se oirá"), "{narrow}");
}

#[test]
fn the_idle_skip_still_works_without_a_gesture() {
    let (mut app, _engine) = open_numbers("adjust_idle");
    let key = app.idle_key();
    let t = Instant::now();
    assert!(!app.pointer_moved(10, 10, NONE, t));
    assert_eq!(app.idle_key(), key, "moverse sin Ctrl no cambia nada");
    let at = on(&app, 0, "120");
    app.mouse_at(MouseAction::ZoomIn, at.0, at.1, CTRL, t);
    assert_eq!(app.idle_key(), None, "con un gesto, se dibuja");
    app.adjust_tick_for_test(t + Duration::from_secs(1));
    assert!(!app.adjusting(), "la pausa lo acaba");
}

#[test]
fn coordinates_out_of_range_are_ignored() {
    let (mut app, _engine) = open_numbers("adjust_out_of_range");
    let t = Instant::now();
    for (x, y) in [(u16::MAX, u16::MAX), (0, u16::MAX), (u16::MAX, 0)] {
        for action in [
            MouseAction::ZoomIn,
            MouseAction::ZoomOut,
            MouseAction::Down,
            MouseAction::Drag,
            MouseAction::Up,
        ] {
            app.mouse_at(action, x, y, CTRL, t);
        }
        app.pointer_moved(x, y, CTRL, t);
    }
    assert_eq!(app.editor.lines()[0], "bpm(120)");
}

#[test]
fn in_the_editor_window_the_gesture_edits_its_text_and_the_main_window_loads_it() {
    use super::split::{editor_window, main_window, pump, pump_until, ui_editor};
    let path = fixture("adjust_split", Some(NUMBERS));
    let (mut main, _engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    let loads = main.loads_for_test();
    ui_editor(&mut editor, 120, 30);
    let at = on(&editor, 0, "120");
    let mut t = Instant::now();
    for _ in 0..3 {
        ctrl_wheel(&mut editor, true, at, &mut t, 30);
    }
    assert_eq!(line(&editor, 0), "bpm(123)");
    // La principal no tiene editor: no hay gesto que hacer en ella.
    assert!(!main.adjusting());
    pause(&mut editor, &mut t);
    pump_until(&mut main, &mut editor, |m, _| {
        m.text_for_test().starts_with("bpm(123)")
    });
    pump_until(&mut main, &mut editor, |m, _| m.loads_for_test() > loads);
    assert!(main.view().error.is_none());
    // Un Ctrl-Z en la ventana del editor lo deshace entero.
    editor.handle(ctrl('z'));
    assert_eq!(line(&editor, 0), "bpm(120)");
}
