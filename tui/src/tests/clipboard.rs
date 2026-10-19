//! Portapapeles del sistema con un ejecutor falso: copiar y cortar mandan
//! la selección por OSC 52 y `wl-copy` (si está), pegar lee `wl-paste` (si
//! está y contesta) y, si no, el portapapeles interno; sin programas, todo
//! sigue funcionando.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::*;
use crate::clipboard::{Clipboard, Runner, osc52};

#[derive(Default)]
struct Log {
    writes: Vec<(String, String)>,
    reads: Vec<String>,
}

/// Ejecutor falso: qué programas hay, qué devuelve `wl-paste` y lo que se
/// le pide.
struct Fake {
    programs: Vec<&'static str>,
    paste: Option<String>,
    log: Arc<Mutex<Log>>,
}

impl Runner for Fake {
    fn available(&self, program: &str) -> bool {
        self.programs.contains(&program)
    }
    fn write(&self, program: &str, _: &[&str], text: &str) {
        self.log
            .lock()
            .unwrap()
            .writes
            .push((program.into(), text.into()));
    }
    fn read(&self, program: &str, args: &[&str], timeout: Duration) -> Option<String> {
        assert!(timeout <= Duration::from_millis(500), "timeout corto");
        assert_eq!(args, ["--no-newline"]);
        self.log.lock().unwrap().reads.push(program.into());
        self.paste.clone()
    }
}

fn with_fake(
    test: &str,
    programs: Vec<&'static str>,
    paste: Option<&str>,
) -> (App, Engine, Arc<Mutex<Log>>) {
    let (mut app, engine) = open_at_end(test);
    let log = Arc::new(Mutex::new(Log::default()));
    app.clipboard = Clipboard::new(Box::new(Fake {
        programs,
        paste: paste.map(String::from),
        log: log.clone(),
    }));
    (app, engine, log)
}

/// Escribe `text` al final de la línea y la selecciona entera (End y
/// Shift-Home).
fn select_typed(app: &mut App, text: &str) {
    type_text(app, text);
    app.handle(special(Key::End));
    app.handle(Input {
        key: Key::Home,
        shift: true,
        ..Input::default()
    });
}

#[test]
fn copy_and_cut_go_to_osc52_and_wl_copy() {
    let (mut app, _engine, log) = with_fake("clip_copy", vec!["wl-copy", "wl-paste"], None);
    select_typed(&mut app, "bpm(90)");
    app.handle(ctrl('c'));
    assert_eq!(app.clipboard.take_osc(), [osc52("bpm(90)")]);
    select_typed(&mut app, "");
    app.handle(ctrl('x'));
    assert_eq!(app.clipboard.take_osc(), [osc52("bpm(90)")]);
    assert_eq!(current_line(&app), "");
    assert_eq!(
        log.lock().unwrap().writes,
        [
            ("wl-copy".to_string(), "bpm(90)".to_string()),
            ("wl-copy".to_string(), "bpm(90)".to_string())
        ]
    );
    // Sin selección, copiar no manda nada.
    app.handle(ctrl('c'));
    assert!(app.clipboard.take_osc().is_empty());
    assert_eq!(log.lock().unwrap().writes.len(), 2);
}

#[test]
fn paste_reads_wl_paste_and_undoes_at_once() {
    let pasted = "track(\"x\", amen)\n  :gain(0.5)";
    let (mut app, _engine, log) =
        with_fake("clip_paste", vec!["wl-copy", "wl-paste"], Some(pasted));
    let before = app.editor.lines().to_vec();
    app.handle(ctrl('y'));
    assert_eq!(log.lock().unwrap().reads, ["wl-paste"]);
    let n = app.editor.lines().len();
    assert_eq!(
        app.editor.lines()[n - 2..],
        ["track(\"x\", amen)", "  :gain(0.5)"]
    );
    app.handle(ctrl('z'));
    assert_eq!(app.editor.lines(), before);
}

#[test]
fn without_programs_or_an_answer_the_internal_clipboard_pastes() {
    // Sin wl-paste (o sin respuesta a tiempo): el interno; sin wl-copy,
    // solo OSC 52.
    for (programs, answer) in [(vec![], None), (vec!["wl-copy", "wl-paste"], None)] {
        let (mut app, _engine, log) = with_fake("clip_none", programs.clone(), answer);
        select_typed(&mut app, "bpm(140)");
        app.handle(ctrl('c'));
        assert_eq!(app.clipboard.take_osc(), [osc52("bpm(140)")]);
        app.handle(special(Key::End));
        app.handle(ctrl('y'));
        assert_eq!(current_line(&app), "bpm(140)bpm(140)", "{programs:?}");
        let log = log.lock().unwrap();
        assert_eq!(log.writes.is_empty(), programs.is_empty());
        assert_eq!(log.reads.len(), usize::from(!programs.is_empty()));
    }
}

#[test]
fn the_play_mode_does_not_touch_the_clipboard() {
    let (mut app, _engine, log) = with_fake("clip_play", vec!["wl-copy", "wl-paste"], Some("x"));
    app.handle(special(Key::Esc));
    app.handle(ctrl('y'));
    app.handle(ctrl('c'));
    assert!(log.lock().unwrap().reads.is_empty());
    assert!(app.clipboard.take_osc().is_empty());
}
