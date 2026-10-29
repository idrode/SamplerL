//! «Tonalidad del sample»: el panel desde la paleta y desde el explorador,
//! en varios tamaños, el aviso de confianza baja y sus acciones (la raíz
//! del instrumento, insertar uno, proponer `key()`), que solo cambian el
//! texto y se deshacen de una vez.

use super::*;

/// Una cadencia en A menor (i-iv-V-i, con armónicos y el bajo), 2,4 s.
fn cadence(path: &Path) {
    let hz = |m: i32| 440.0 * 2f32.powf((m - 69) as f32 / 12.0);
    let chords: [&[i32]; 4] = [
        &[45, 57, 60, 64],
        &[62, 65, 69],
        &[64, 68, 71],
        &[45, 57, 60, 64],
    ];
    let len = RATE as usize * 6 / 10;
    let mut mono = vec![0.0f32; 4 * len];
    for (c, notes) in chords.iter().enumerate() {
        for &n in notes.iter() {
            for k in 1..=4 {
                let f = hz(n) * k as f32;
                for i in 0..len {
                    let t = i as f32 / RATE as f32;
                    let env = (t / 0.01).min(1.0) * (-2.0 * t).exp();
                    mono[c * len + i] +=
                        0.08 / k as f32 * env * (std::f32::consts::TAU * f * t).sin();
                }
            }
        }
    }
    offline::write_wav(path, &mono, 1, RATE).unwrap();
}

const SONG_KEY: &str = "bpm(120)\n\
    local piano = sample(\"cadencia.wav\"):instrument({ root = \"C4\", voices = 2 })\n\
    track(\"lead\", piano):melody(\"c . . .  . . . .  . . . .  . . . .\")\n\
    track(\"drums\", sample(\"break.wav\"):slice(2)):steps(\"1 . 2 .  . . . .  . . . .  . . . .\")\n";

fn open_key(test: &str) -> (App, Engine) {
    let path = fixture(test, Some(SONG_KEY));
    cadence(&path.with_file_name("cadencia.wav"));
    let (controller, engine) = new_engine(RATE);
    let app = App::open(&path, controller, RATE).unwrap();
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    (app, engine)
}

#[test]
fn the_palette_shows_the_three_keys_in_several_sizes() {
    let (mut app, _engine) = open_key("sample_key_palette");
    palette(&mut app, "tonalidad del sample");
    app.wait_key_analysis();
    let view = app.view().key.unwrap();
    assert_eq!(view.sample, "cadencia.wav");
    let crate::key_panel::KeyState::Done {
        candidates,
        low,
        root,
        ..
    } = view.state
    else {
        panic!("{:?}", view.state)
    };
    assert_eq!(candidates[0].0, "A menor");
    assert_eq!(candidates.len(), 3);
    assert!(!low);
    assert_eq!(root, "A3");
    for (width, height) in [(120, 40), (80, 24), (60, 18)] {
        let screen = text(&draw(&mut app, width, height));
        assert!(
            screen.contains("tonalidad · cadencia.wav"),
            "{width}x{height}\n{screen}"
        );
        assert!(screen.contains("A menor"), "{width}x{height}\n{screen}");
        assert!(screen.contains("raíz A3"), "{width}x{height}\n{screen}");
    }
    app.handle(special(Key::Up));
    let screen = text(&draw(&mut app, 120, 40));
    assert!(screen.contains("TONALIDAD:"), "{screen}");
    for action in [
        "usar como raíz",
        "insertar sample:instrument",
        "proponer como tonalidad",
    ] {
        assert!(screen.contains(action), "{screen}");
    }
    // ↓ elige la segunda; Esc cierra sin tocar nada.
    let before = app.text_for_test();
    app.handle(special(Key::Down));
    assert_eq!(app.view().key.unwrap().selected, 1);
    app.handle(esc());
    assert!(app.view().key.is_none());
    assert_eq!(app.text_for_test(), before);
}

#[test]
fn a_break_has_low_confidence() {
    let (mut app, _engine) = open_key("sample_key_break");
    app.handle(ctrl('o'));
    palette(&mut app, "tonalidad del sample");
    app.wait_key_analysis();
    let view = app.view().key.unwrap();
    assert_eq!(view.sample, "break.wav");
    let screen = text(&draw(&mut app, 100, 30));
    assert!(
        screen.contains("confianza baja") || screen.contains("nada que analizar"),
        "{screen}"
    );
}

#[test]
fn r_sets_the_instrument_root_and_undoes_at_once() {
    let (mut app, _engine) = open_key("sample_key_root");
    palette(&mut app, "tonalidad del sample");
    app.wait_key_analysis();
    let before = app.text_for_test();
    app.handle(key('r'));
    assert!(app.view().key.is_none());
    let text = app.text_for_test();
    assert!(
        text.contains("instrument({ root = \"A3\", voices = 2 })"),
        "{text}"
    );
    // No carga sola: lo cargado sigue con la raíz de antes.
    assert_eq!(app.view().tracks[0].instrument.unwrap().root, 60);
    app.handle(ctrl('s'));
    assert_eq!(app.view().tracks[0].instrument.unwrap().root, 57);
    app.handle(ctrl('z'));
    assert_eq!(app.text_for_test(), before);
}

#[test]
fn i_inserts_an_instrument_and_k_proposes_the_song_key() {
    let (mut app, _engine) = open_key("sample_key_insert");
    palette(&mut app, "tonalidad del sample");
    app.wait_key_analysis();
    app.handle(key('i'));
    assert!(
        app.text_for_test()
            .contains("sample(\"cadencia.wav\"):instrument({ root = \"A3\" })")
    );
    app.handle(ctrl('z'));

    palette(&mut app, "tonalidad del sample");
    app.wait_key_analysis();
    app.handle(key('k'));
    let text = app.text_for_test();
    assert!(
        text.starts_with("bpm(120)\nkey(\"A\", \"minor\")\n"),
        "{text}"
    );
    // Propuesta, no aplicada: la canción no tiene tonalidad hasta Ctrl-S.
    assert_eq!(app.host_song_key_for_test(), None);
    app.handle(ctrl('s'));
    assert_eq!(
        app.host_song_key_for_test().as_deref(),
        Some("key(\"A\", \"minor\")")
    );
    // Otra vez: sustituye la línea, no añade otra.
    palette(&mut app, "tonalidad del sample");
    app.wait_key_analysis();
    app.handle(special(Key::Down));
    app.handle(key('k'));
    let text = app.text_for_test();
    assert_eq!(text.matches("key(").count(), 1, "{text}");
}

#[test]
fn the_browser_opens_it_with_ctrl_k() {
    let (mut app, _engine) = open_key("sample_key_browser");
    palette(&mut app, "explorador");
    type_query(&mut app, "cadencia");
    app.handle(ctrl('k'));
    assert!(app.view().browser.is_none());
    app.wait_key_analysis();
    let view = app.view().key.unwrap();
    assert_eq!(view.sample, "cadencia.wav");
    let screen = text(&draw(&mut app, 90, 30));
    assert!(screen.contains("A menor"), "{screen}");
}
