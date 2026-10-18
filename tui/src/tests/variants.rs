//! Variantes A/B en la interfaz: `v` en el modo tocar y la paleta
//! alternan, el panel del motor dice cuál suena y la línea de tiempo sigue.

use super::*;

const SONG_VARIANTS: &str = r#"bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
section("intro", function() end)
section("drop", function() end)
variant("drop", "B", function()
  track("kick", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . .")
end)
variant("drums", "B", function(t) t:gain(0.5) end)
arrange({ {"intro", 1}, {"drop", 4} })
"#;

const BAR: usize = 16 * STEP;

#[test]
fn v_toggles_and_the_panel_says_which_one_plays() {
    let (_, mut app, mut engine) = open("variants", Some(SONG_VARIANTS));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    assert_eq!(app.view().variant, Some('A'));
    assert!(text(&draw(&mut app, 170, 44)).contains("variante A"));
    offline::render(&mut engine, BAR + BAR / 2, 2);
    app.tick();
    // En el drop, A: solo «drums».
    assert_eq!(app.view().tracks.len(), 1);
    app.handle(esc());
    app.handle(key('v'));
    assert!(
        app.view()
            .message
            .contains("variante B: entra en el compás siguiente")
    );
    assert_eq!(app.view().variant, Some('B'));
    offline::render(&mut engine, BAR, 2);
    app.tick();
    // La B añade «kick» al drop; la línea de tiempo sigue por donde iba.
    assert_eq!(app.view().tracks.len(), 2);
    let timeline = app.view().timeline.unwrap();
    assert_eq!((timeline.current, timeline.bar), (1, 1));
    assert!(text(&draw(&mut app, 170, 44)).contains("variante B"));
    // Desde la paleta, vuelta a la A.
    palette(&mut app, "alternar variante");
    assert_eq!(app.view().variant, Some('A'));
}

#[test]
fn without_variants_it_says_so() {
    let (_, mut app, _engine) = open("variants_none", Some(SONG));
    app.handle(esc());
    app.handle(key('v'));
    assert!(
        app.view().message.contains("no hay variantes"),
        "{}",
        app.view().message
    );
    assert_eq!(app.view().variant, None);
    let screen = text(&draw(&mut app, 170, 44));
    assert!(!screen.contains("variante A") && !screen.contains("variante B"));
}
