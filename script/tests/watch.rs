//! Recarga automática al guardar: cuantizada al compás y sin romper lo que
//! suena si el archivo nuevo es inválido.

use std::path::{Path, PathBuf};

use engine::{Controller, Engine, new_engine, offline};
use script::{ScriptHost, ScriptWatcher, WatchEvent};

const RATE: u32 = 48_000;
/// A 120 BPM: 6000 frames por paso, 96000 por compás.
const STEP: usize = 6000;
const BAR: usize = 16 * STEP;
const SETTLE: usize = 200;
const EPS: f32 = 1e-3;

const SONG_A: &str = r#"
bpm(120)
local amen = sample("break.wav"):slice(2)
track("d", amen):steps("1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1")
"#;
const SONG_B: &str = r#"
bpm(120)
local amen = sample("break.wav"):slice(2)
track("d", amen):steps("2 . . .  2 . . .  2 . . .  2 . . .")
"#;
/// Guardado a medias (el editor aún no terminó de escribir).
const SONG_B_HALF: &str = r#"
bpm(120)
local amen = sample("break.wav"):slice(2)
track("d", amen):steps("2 . . .  2 . . "#;

fn setup(test: &str) -> (PathBuf, ScriptWatcher, ScriptHost, Controller, Engine) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("watch_{test}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
    let path = dir.join("song.lua");
    std::fs::write(&path, SONG_A).unwrap();
    let (controller, engine) = new_engine(RATE);
    (
        path.clone(),
        ScriptWatcher::new(path),
        ScriptHost::new(RATE),
        controller,
        engine,
    )
}

fn render_left(engine: &mut Engine, frames: usize) -> Vec<f32> {
    offline::render(engine, frames, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

fn level(l: &[f32], bar: usize, step: usize) -> f32 {
    l[bar * BAR + step * STEP + SETTLE]
}

#[test]
fn saving_the_script_reloads_it_at_the_next_bar() {
    let (path, mut watcher, mut host, mut controller, mut engine) = setup("reload");
    assert_eq!(
        watcher.poll(&mut host, &mut controller),
        WatchEvent::Reloaded
    );
    assert_eq!(
        watcher.poll(&mut host, &mut controller),
        WatchEvent::Unchanged
    );
    controller.start().unwrap();
    let mut l = render_left(&mut engine, BAR / 2 + 99);

    // Se guarda a mitad de compás.
    std::fs::write(&path, SONG_B).unwrap();
    assert_eq!(
        watcher.poll(&mut host, &mut controller),
        WatchEvent::Reloaded
    );
    assert_eq!(
        watcher.poll(&mut host, &mut controller),
        WatchEvent::Unchanged
    );
    l.extend(render_left(&mut engine, BAR / 2 - 99 + BAR));

    // El compás 0 termina entero con A (corte 1 en todos los pasos).
    for step in 0..16 {
        assert!((level(&l, 0, step) - 0.5).abs() < EPS, "paso {step}");
    }
    // El compás 1 ya es B.
    assert!((level(&l, 1, 0) - 0.25).abs() < EPS);
    assert_eq!(level(&l, 1, 1), 0.0);
}

#[test]
fn invalid_or_half_saved_scripts_keep_the_previous_one_playing() {
    let (path, mut watcher, mut host, mut controller, mut engine) = setup("invalid");
    assert_eq!(
        watcher.poll(&mut host, &mut controller),
        WatchEvent::Reloaded
    );
    controller.start().unwrap();
    let mut l = render_left(&mut engine, BAR / 3);

    // Guardado a medias: error con línea; se informa una vez.
    std::fs::write(&path, SONG_B_HALF).unwrap();
    match watcher.poll(&mut host, &mut controller) {
        WatchEvent::Failed(e) => assert!(e.message.contains("song.lua:"), "{e}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        watcher.poll(&mut host, &mut controller),
        WatchEvent::Unchanged
    );
    l.extend(render_left(&mut engine, BAR / 3));

    // Error en tiempo de ejecución con línea.
    std::fs::write(&path, format!("{SONG_A}\nbpm(9999)\n")).unwrap();
    match watcher.poll(&mut host, &mut controller) {
        WatchEvent::Failed(e) => assert!(e.message.contains("song.lua:6:"), "{e}"),
        other => panic!("{other:?}"),
    }
    // El archivo desaparece un momento (algunos editores guardan así).
    std::fs::remove_file(&path).unwrap();
    assert!(matches!(
        watcher.poll(&mut host, &mut controller),
        WatchEvent::Failed(_)
    ));
    assert_eq!(
        watcher.poll(&mut host, &mut controller),
        WatchEvent::Unchanged
    );
    l.extend(render_left(&mut engine, BAR - 2 * (BAR / 3) + BAR));

    // Dos compases de A, como si no hubiera pasado nada.
    let (mut c2, mut e2) = new_engine(RATE);
    let mut h2 = ScriptHost::new(RATE);
    let dir = path.parent().unwrap();
    h2.load(&mut c2, SONG_A, "song.lua", dir).unwrap();
    c2.start().unwrap();
    assert_eq!(l, render_left(&mut e2, 2 * BAR));

    // Se guarda la versión completa: entra en el compás siguiente.
    std::fs::write(&path, SONG_B).unwrap();
    assert_eq!(
        watcher.poll(&mut host, &mut controller),
        WatchEvent::Reloaded
    );
    let next = render_left(&mut engine, BAR);
    assert!((next[SETTLE] - 0.25).abs() < EPS);
    assert_eq!(next[STEP + SETTLE], 0.0);
}

#[test]
fn a_failed_save_is_not_retried_until_the_file_changes_again() {
    let (path, mut watcher, mut host, mut controller, _engine) = setup("retry");
    std::fs::write(&path, "bpm(\n").unwrap();
    assert!(matches!(
        watcher.poll(&mut host, &mut controller),
        WatchEvent::Failed(_)
    ));
    assert_eq!(
        watcher.poll(&mut host, &mut controller),
        WatchEvent::Unchanged
    );
    assert!(host.song().is_none());
    std::fs::write(&path, SONG_A).unwrap();
    assert_eq!(
        watcher.poll(&mut host, &mut controller),
        WatchEvent::Reloaded
    );
    assert!(host.song().is_some());
}
