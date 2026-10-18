//! Secciones en la interfaz: saltar, repetir y volver a la línea de tiempo
//! desde el modo tocar, la paleta y la línea de órdenes; las pistas que se
//! ven son las de la sección que suena.

use super::*;

pub const SONG_SECTIONS: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
section("intro", function() track("drums"):mute() end)
section("drop", function()
  track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
end)
section("break", function() end)
arrange({ {"intro", 2}, {"drop", 4}, {"break", 1} })
"#;

/// Un compás a 120 BPM.
const BAR: usize = 16 * STEP;

fn place(app: &App) -> (usize, u32, bool) {
    let s = app.view().timeline.unwrap();
    (s.current, s.bar, s.looping)
}

#[test]
fn jumps_loops_and_back_from_play_mode() {
    let (_, mut app, mut engine) = open("sections_live", Some(SONG_SECTIONS));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    offline::render(&mut engine, BAR / 2, 2);
    app.tick();
    assert_eq!(place(&app), (0, 0, false));
    // En la intro solo está «drums» (en mute); en el drop, también «kick».
    assert_eq!(app.view().tracks.len(), 1);
    app.handle(esc());
    assert!(app.view().playing_mode);
    // ]: la entrada siguiente, en el compás siguiente.
    app.handle(key(']'));
    assert!(
        app.view().message.contains("salta a «drop»"),
        "{}",
        app.view().message
    );
    offline::render(&mut engine, BAR, 2);
    app.tick();
    assert_eq!(place(&app), (1, 0, false));
    assert_eq!(app.view().tracks.len(), 2);
    // l: repetir el drop (4 compases) más allá de su final.
    app.handle(key('l'));
    assert!(place(&app).2);
    offline::render(&mut engine, 5 * BAR, 2);
    app.tick();
    assert_eq!(place(&app), (1, 1, true), "vuelve a empezar el drop");
    // l otra vez: vuelve a la línea de tiempo.
    app.handle(key('l'));
    assert!(app.view().message.contains("sigue la línea de tiempo"));
    offline::render(&mut engine, 3 * BAR, 2);
    app.tick();
    assert_eq!(place(&app), (2, 0, false), "tras el drop, el break");
    // [: la anterior.
    app.handle(key('['));
    offline::render(&mut engine, BAR, 2);
    app.tick();
    assert_eq!(place(&app).0, 1);
}

#[test]
fn jumps_from_the_palette_and_the_command_line() {
    let (_, mut app, mut engine) = open("sections_palette", Some(SONG_SECTIONS));
    offline::render(&mut engine, BAR / 2, 2);
    app.tick();
    command(&mut app, "seccion break");
    assert!(app.view().message.contains("salta a «break»"));
    offline::render(&mut engine, BAR, 2);
    app.tick();
    assert_eq!(place(&app).0, 2);
    command(&mut app, "seccion coda");
    assert!(
        app.view().message.contains("«coda» no está")
            && app.view().message.contains("intro, drop, break"),
        "{}",
        app.view().message
    );
    palette(&mut app, "repetir la seccion");
    assert!(place(&app).2);
    palette(&mut app, "volver a la linea de tiempo");
    assert!(!place(&app).2);
    palette(&mut app, "saltar a una seccion");
    assert_eq!(app.view().prompt.as_deref(), Some("seccion "));
}

#[test]
fn without_a_timeline_it_says_so() {
    let (_, mut app, _engine) = open("sections_none", Some(SONG));
    app.handle(esc());
    app.handle(key(']'));
    assert!(
        app.view().message.contains("no hay línea de tiempo"),
        "{}",
        app.view().message
    );
    assert!(app.view().timeline.is_none());
}

#[test]
fn the_timeline_bar_shows_where_the_song_is() {
    let (_, mut app, mut engine) = open("sections_view", Some(SONG_SECTIONS));
    offline::render(&mut engine, 2 * BAR + BAR / 2, 2);
    app.tick();
    let screen = text(&draw(&mut app, 170, 44));
    assert!(
        screen.contains("intro 2 │ ▶ drop 1/4 │ break 1"),
        "{screen}"
    );
    // Resaltada la que suena.
    let buffer = draw(&mut app, 170, 44);
    let (x, y) = find(&buffer, "▶ drop").unwrap();
    assert_eq!(buffer[(x, y)].bg, ui::accent());
    assert_eq!(buffer[(x - 2, y)].bg, Color::Reset);
    // Repitiendo, ⟳.
    app.handle(esc());
    app.handle(key('l'));
    let screen = text(&draw(&mut app, 170, 44));
    assert!(screen.contains("▶ drop 1/4 ⟳"), "{screen}");
    // Sin sitio: empieza más adelante para que se vea la que suena.
    let view = crate::model::TimelineView {
        entries: (0..12).map(|i| (format!("parte{i}"), 4)).collect(),
        current: 9,
        bar: 2,
        looping: false,
    };
    let line = text(&draw_widget(ui::TimelineBar(&view), 40, 1));
    assert!(line.starts_with("… "), "{line}");
    assert!(line.contains("▶ parte9 3/4"), "{line}");
}
