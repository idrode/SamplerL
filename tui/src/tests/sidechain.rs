//! Vista del sidechain: desde la paleta se abre y se cierra; por cada EQ
//! dinámico y compresor, quién lo dispara, cuánto reduce y sus curvas,
//! leídas del motor; en varios tamaños, y en la ventana principal con el
//! editor en la suya.

use super::split::{editor_window, main_window, pump};
use super::*;

/// Un bombo (un seno grave que cae) y un pad de 1 kHz que le deja sitio
/// con un EQ dinámico en 1 kHz que escucha al bombo; el bombo, con un
/// compresor que se escucha a sí mismo.
fn write_sounds(path: &Path) {
    let dir = path.parent().unwrap();
    let kick: Vec<f32> = (0..STEP)
        .map(|i| {
            let t = i as f32 / RATE as f32;
            0.9 * (-20.0 * t).exp() * (std::f32::consts::TAU * 1000.0 * t).sin()
        })
        .collect();
    offline::write_wav(dir.join("bombo.wav"), &kick, 1, RATE).unwrap();
    let pad: Vec<f32> = (0..16 * STEP)
        .map(|i| 0.4 * (std::f32::consts::TAU * 1000.0 * i as f32 / RATE as f32).sin())
        .collect();
    offline::write_wav(dir.join("pad.wav"), &pad, 1, RATE).unwrap();
}

const SONG_SIDE: &str = "bpm(120)\n\
    track(\"kick\", sample(\"bombo.wav\"):slice_grid(1))\n\
      :steps(\"1 . . .  . . . .  1 . . .  . . . .\")\n\
      :fx(\"compressor\", { threshold = -20, ratio = 4, attack = 1, release = 40 })\n\
    track(\"pad\", sample(\"pad.wav\"):slice_grid(1))\n\
      :steps(\"1 . . .  . . . .  . . . .  . . . .\")\n\
      :fx(\"dyneq\", { freq = 1000, threshold = -40, range = 12, sidechain = \"kick\" })\n";

fn open_side(test: &str) -> (PathBuf, App, Engine) {
    let path = fixture(test, Some(SONG_SIDE));
    write_sounds(&path);
    let (controller, engine) = new_engine(RATE);
    let app = App::open(&path, controller, RATE).unwrap();
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    (path, app, engine)
}

/// Mueve el motor `steps` pasos de 1/8 en 1/8, con un `tick` en cada uno.
fn play(app: &mut App, engine: &mut Engine, steps: usize) {
    for _ in 0..8 * steps {
        offline::render(engine, STEP / 8, 2);
        app.tick();
    }
}

#[test]
fn the_palette_opens_and_closes_it_and_it_follows_the_engine() {
    let (_, mut app, mut engine) = open_side("sidechain_view");
    assert!(app.view().sidechain.is_none());
    palette(&mut app, "vista del sidechain");
    let view = app.view().sidechain.unwrap();
    let rows: Vec<(&str, &str, Option<&str>)> = view
        .rows
        .iter()
        .map(|r| (r.track.as_str(), r.effect.as_str(), r.source.as_deref()))
        .collect();
    assert_eq!(
        rows,
        [
            ("kick", "compresor", None),
            ("pad", "eq dinámico 1.0 kHz", Some("kick")),
        ]
    );
    // Con el bombo sonando (paso 0), el pad baja; en los pasos sin bombo,
    // sube otra vez.
    play(&mut app, &mut engine, 2);
    let view = app.view().sidechain.unwrap();
    let pad = &view.rows[1];
    assert!(
        pad.reduction.iter().any(|&db| db > 6.0),
        "{:?}",
        pad.reduction
    );
    assert!(pad.key.iter().any(|&k| k > 0.3), "{:?}", pad.key);
    assert!(view.rows[0].reduction.iter().any(|&db| db > 1.0));
    play(&mut app, &mut engine, 5);
    let view = app.view().sidechain.unwrap();
    assert_eq!(view.rows[1].now_db, 0.0, "{:?}", view.rows[1].reduction);
    assert!(view.rows[1].reduction.len() <= crate::sidechain::HISTORY);

    // Se ve, con quién dispara a quién, en varios tamaños.
    for (width, height) in [(140, 45), (100, 30), (80, 24)] {
        let screen = text(&draw(&mut app, width, height));
        assert!(screen.contains("sidechain"), "{width}x{height}\n{screen}");
        assert!(
            screen.contains("pad · eq dinámico 1.0 kHz ← kick"),
            "{width}x{height}\n{screen}"
        );
        assert!(
            screen.contains("kick · compresor ← sí misma"),
            "{width}x{height}\n{screen}"
        );
    }
    let screen = text(&draw(&mut app, 140, 45));
    assert!(screen.contains("disparo"), "{screen}");
    assert!(screen.contains("reducción"), "{screen}");
    // El editor sigue a la vista.
    assert!(screen.contains("bpm(120)"), "{screen}");

    palette(&mut app, "vista del sidechain");
    assert!(app.view().sidechain.is_none());
    assert!(!text(&draw(&mut app, 140, 45)).contains("disparo"));
}

#[test]
fn without_dynamics_it_says_so() {
    let (_, mut app, _engine) = open("sidechain_empty", Some(SONG));
    palette(&mut app, "vista del sidechain");
    assert_eq!(app.view().sidechain.unwrap().rows, []);
    let screen = text(&draw(&mut app, 120, 40));
    assert!(
        screen.contains("ninguna pista tiene un EQ dinámico ni un compresor"),
        "{screen}"
    );
}

#[test]
fn in_the_main_window_it_goes_beside_the_waveform() {
    let path = fixture("sidechain_main", Some(SONG_SIDE));
    write_sounds(&path);
    let (mut main, mut engine) = main_window(&path, None);
    let (mut editor, _idle) = editor_window(&path);
    pump(&mut main, &mut editor);
    palette(&mut main, "vista del sidechain");
    play(&mut main, &mut engine, 2);
    for (width, height) in [(140, 45), (100, 30)] {
        let buffer = draw(&mut main, width, height);
        let screen = text(&buffer);
        assert!(!screen.contains("bpm(120)"), "{screen}");
        let (x, y) = find(&buffer, "┌ sidechain").unwrap_or_else(|| panic!("{screen}"));
        let (sx, sy) = find(&buffer, "┌ sample").unwrap();
        assert_eq!(y, sy, "{screen}");
        assert!(x > sx + width / 3, "{screen}");
        assert!(screen.contains("← kick"), "{screen}");
    }
}

/// Para ver la vista sin terminal:
/// `cargo test -p tui pantalla_sidechain -- --ignored --nocapture`.
#[test]
#[ignore]
fn pantalla_sidechain() {
    let (_, mut app, mut engine) = open_side("sidechain_pantalla");
    palette(&mut app, "vista del sidechain");
    play(&mut app, &mut engine, 10);
    println!("{}", text(&draw(&mut app, 140, 45)));
}
