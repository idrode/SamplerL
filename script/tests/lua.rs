//! API de Lua con render offline: patrones, varias pistas, recarga en
//! caliente cuantizada al compás y errores que no rompen lo que suena.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use engine::{Controller, Engine, new_engine, offline};
use script::ScriptHost;

const RATE: u32 = 48_000;
/// A 120 BPM: 6000 frames por paso, 96000 por compás.
const STEP: usize = 6000;
const BAR: usize = 16 * STEP;
/// Pasado el fundido de entrada (≤ 2 ms).
const SETTLE: usize = 200;
/// El WAV de prueba se guarda en 16 bits: 0.5 queda en 16383/32767.
const EPS: f32 = 1e-3;

/// Carpeta propia para cada test, con `break.wav`: dos cortes, un bloque
/// constante de 800 frames a 0.5 en 0 y otro a 0.25 en 3000.
fn fixture(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(test);
    std::fs::create_dir_all(&dir).unwrap();
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
    dir
}

fn setup(test: &str) -> (PathBuf, ScriptHost, Controller, Engine) {
    let (controller, engine) = new_engine(RATE);
    (fixture(test), ScriptHost::new(RATE), controller, engine)
}

fn load(host: &mut ScriptHost, c: &mut Controller, dir: &Path, src: &str) -> Result<(), String> {
    host.load(c, src, "song.lua", dir)
        .map(|_| ())
        .map_err(|e| e.message)
}

fn render_left(engine: &mut Engine, frames: usize) -> Vec<f32> {
    offline::render(engine, frames, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

fn level(left: &[f32], bar: usize, step: usize) -> f32 {
    left[bar * BAR + step * STEP + SETTLE]
}

fn assert_level(left: &[f32], bar: usize, step: usize, expected: f32) {
    let actual = level(left, bar, step);
    assert!(
        (actual - expected).abs() < EPS,
        "compás {bar}, paso {step}: {actual} (esperado {expected})"
    );
}

const SONG_A: &str = r#"
bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 . 2 .  . . . .  . . . .  . . . .")
track("kick", amen):steps("x . . .  x . . .  . . . .  . . . .")
"#;

#[test]
fn api_from_claude_md_renders_two_tracks() {
    let (dir, mut host, mut controller, mut engine) = setup("api");
    load(&mut host, &mut controller, &dir, SONG_A).unwrap();
    controller.start().unwrap();
    let l = render_left(&mut engine, BAR);

    // Paso 0: corte 1 en las dos pistas (0.5 + 0.5, por el limitador). Paso 2: corte 2 en
    // "drums". Paso 4: "x" (= corte 1) en "kick". Paso 1: silencio.
    assert_level(&l, 0, 0, engine::soft_limit(1.0));
    assert_level(&l, 0, 1, 0.0);
    assert_level(&l, 0, 2, 0.25);
    assert_level(&l, 0, 4, 0.5);

    let song = host.song().unwrap();
    assert_eq!(song.bpm, Some(120.0));
    let names: Vec<&str> = song.tracks.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["drums", "kick"]);
    assert_eq!(song.tracks[0].slices.len(), 2);
}

#[test]
fn track_by_name_and_effects_from_the_notation() {
    let (dir, mut host, mut controller, mut engine) = setup("by_name");
    let src = r#"
local amen = sample("break.wav"):slice(2)
track("drums", amen)
track("drums"):steps("1@0.5 . 2' .  . . . .  . . . .  . . . .")
"#;
    load(&mut host, &mut controller, &dir, src).unwrap();
    controller.start().unwrap();
    let l = render_left(&mut engine, BAR);
    assert_level(&l, 0, 0, 0.25);
    // Corte 2 al revés: el bloque está al final de sus 3000 frames.
    assert!(l[2 * STEP..2 * STEP + 2200].iter().all(|&s| s == 0.0));
    assert!((l[2 * STEP + 2600] - 0.25).abs() < EPS);
}

#[test]
fn reload_enters_at_the_next_bar() {
    let (dir, mut host, mut controller, mut engine) = setup("reload");
    let every_step = r#"
bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1")
"#;
    load(&mut host, &mut controller, &dir, every_step).unwrap();
    controller.start().unwrap();
    let mut l = render_left(&mut engine, BAR / 2 + 77);

    // Recarga a mitad de compás: otro patrón y 60 BPM (pasos de 12000).
    let changed = r#"
bpm(60)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("2 . . .  2 . . .  . . . .  . . . .")
"#;
    load(&mut host, &mut controller, &dir, changed).unwrap();
    l.extend(render_left(&mut engine, BAR / 2 - 77 + BAR));

    for step in 0..16 {
        assert_level(&l, 0, step, 0.5);
    }
    // Compás 1: corte 2 en el paso 0 (frame 96000) y en el paso 4, que a
    // 60 BPM cae en 96000 + 4 · 12000; en el paso 1 a 120 BPM, nada.
    assert!((l[BAR + SETTLE] - 0.25).abs() < EPS);
    assert_eq!(l[BAR + STEP + SETTLE], 0.0);
    assert!((l[BAR + 4 * 2 * STEP + SETTLE] - 0.25).abs() < EPS);
    assert_eq!(host.song().unwrap().bpm, Some(60.0));
}

#[test]
fn errors_keep_the_previous_pattern_playing() {
    let (dir, mut host, mut controller, mut engine) = setup("errors");
    host.set_time_limit(Duration::from_millis(100));
    load(&mut host, &mut controller, &dir, SONG_A).unwrap();
    controller.start().unwrap();
    let mut audio = render_left(&mut engine, BAR + 1234);

    let nine_tracks: String =
        std::iter::once("local a = sample(\"break.wav\"):slice(2)\n".to_string())
            .chain((0..9).map(|i| format!("track(\"t{i}\", a)\n")))
            .collect();
    let bad: Vec<(String, &[&str])> = vec![
        ("bpm(120\n".into(), &["song.lua:"]),
        ("local x = nil\nx.y = 1\n".into(), &["song.lua:2:"]),
        (
            "local a = sample(\"break.wav\"):slice(2)\n\ntrack(\"d\", a):steps(\"1 . 2^ .  . . . .  . . . .  . . . .\")\n".into(),
            &["song.lua:3:", "paso 3 (\"2^\")", "falta el número después de ^"],
        ),
        (
            "local a = sample(\"break.wav\"):slice(2)\ntrack(\"d\", a):steps(\"5 . . .  . . . .  . . . .  . . . .\")\n".into(),
            &["song.lua:2:", "el corte 5 no existe", "tiene 2"],
        ),
        (
            "local a = sample(\"break.wav\"):slice(2)\ntrack(\"d\", a):steps(\"1@9 . . .  . . . .  . . . .  . . . .\")\n".into(),
            &["song.lua:2:", "ganancia"],
        ),
        ("sample(\"nope.wav\")\n".into(), &["song.lua:1:", "nope.wav"]),
        ("track(\"hats\"):steps(\"1\")\n".into(), &["song.lua:1:", "\"hats\" no existe"]),
        ("bpm(1000)\n".into(), &["song.lua:1:", "BPM 1000"]),
        ("while true do end\n".into(), &["tarda"]),
        (nine_tracks, &["el máximo es 8"]),
        ("io.open(\"x\")\n".into(), &["song.lua:1:", "io"]),
    ];
    for (src, fragments) in &bad {
        let err = load(&mut host, &mut controller, &dir, src).unwrap_err();
        for fragment in *fragments {
            assert!(
                err.contains(fragment),
                "error \"{err}\" sin \"{fragment}\"\n{src}"
            );
        }
    }
    audio.extend(render_left(&mut engine, 2 * BAR - 1234));

    // Lo que suena es idéntico a no haber intentado ninguna recarga.
    let (mut controller2, mut engine2) = new_engine(RATE);
    let mut host2 = ScriptHost::new(RATE);
    load(&mut host2, &mut controller2, &dir, SONG_A).unwrap();
    controller2.start().unwrap();
    let reference = render_left(&mut engine2, 3 * BAR);
    assert_eq!(audio, reference);

    // El modelo sigue siendo el de la última carga válida.
    assert_eq!(host.song().unwrap().tracks.len(), 2);
}

#[test]
fn reload_reuses_loaded_samples_and_keeps_track_slots() {
    let (dir, mut host, mut controller, mut engine) = setup("slots");
    load(&mut host, &mut controller, &dir, SONG_A).unwrap();
    let first = host.song().unwrap().tracks[1].sample.clone();
    let kick_slot = host.track_slot("kick").unwrap();
    controller.start().unwrap();
    let mut l = render_left(&mut engine, BAR / 2);

    // Se quita "drums": "kick" conserva su pista y "drums" calla en el
    // compás siguiente.
    let only_kick = r#"
local amen = sample("break.wav"):slice(2)
track("kick", amen):steps("x . . .  x . . .  . . . .  . . . .")
"#;
    load(&mut host, &mut controller, &dir, only_kick).unwrap();
    l.extend(render_left(&mut engine, BAR / 2 + BAR));

    assert!(Arc::ptr_eq(&first, &host.song().unwrap().tracks[0].sample));
    assert_eq!(host.track_slot("kick"), Some(kick_slot));
    assert_eq!(host.track_slot("drums"), None);
    assert_level(&l, 0, 0, engine::soft_limit(1.0));
    assert_level(&l, 0, 2, 0.25);
    assert_level(&l, 1, 0, 0.5);
    assert_level(&l, 1, 2, 0.0);
    assert_level(&l, 1, 4, 0.5);
}

#[test]
fn load_file_resolves_samples_next_to_the_script() {
    let (dir, mut host, mut controller, mut engine) = setup("file");
    let path = dir.join("song.lua");
    std::fs::write(&path, SONG_A).unwrap();
    host.load_file(&mut controller, &path).unwrap();
    controller.start().unwrap();
    let l = render_left(&mut engine, BAR);
    assert_level(&l, 0, 0, engine::soft_limit(1.0));

    std::fs::write(&path, "bpm(120)\n\nbpm(\"rápido\")\n").unwrap();
    let err = host.load_file(&mut controller, &path).err().unwrap();
    assert!(err.message.contains("song.lua:3:"), "{err}");
}

#[test]
fn track_gain_from_lua() {
    let (dir, mut host, mut controller, mut engine) = setup("gain");
    let src = r#"
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 . . .  . . . .  . . . .  . . . ."):gain(0.5)
"#;
    load(&mut host, &mut controller, &dir, src).unwrap();
    controller.start().unwrap();
    let l = render_left(&mut engine, BAR);
    assert_level(&l, 0, 0, 0.25);
    assert_eq!(host.song().unwrap().tracks[0].gain, 0.5);

    let bad = "local a = sample(\"break.wav\"):slice(2)\ntrack(\"d\", a):gain(5)\n";
    let err = load(&mut host, &mut controller, &dir, bad).unwrap_err();
    assert!(
        err.contains("song.lua:2:") && err.contains("volumen de pista 5"),
        "{err}"
    );
}
