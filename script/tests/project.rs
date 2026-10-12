//! Proyectos (script, carpeta de samples y tempo) y render de la canción.

use std::path::{Path, PathBuf};

use engine::{bar_frames, new_engine, offline};
use script::{Project, ScriptHost};

const RATE: u32 = 48_000;

fn dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("project_{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_break(path: &Path) {
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    offline::write_wav(path, &mono, 1, RATE).unwrap();
}

#[test]
fn project_round_trip_with_relative_paths() {
    let dir = dir("round_trip");
    std::fs::create_dir_all(dir.join("proyectos")).unwrap();
    let project = Project {
        script: dir.join("song.lua"),
        samples: Some(dir.join("samples")),
        bpm: Some(140.0),
        labels: Vec::new(),
        positions: Vec::new(),
    };
    let path = dir.join("proyectos").join("p.sampler");
    project.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("script = ../song.lua\n"), "{text}");
    assert!(text.contains("samples = ../samples\n"), "{text}");
    assert!(text.contains("bpm = 140\n"), "{text}");

    let loaded = Project::load(&path).unwrap();
    assert_eq!(loaded.bpm, Some(140.0));
    assert_eq!(
        std::path::absolute(&loaded.script)
            .unwrap()
            .canonicalize()
            .ok(),
        dir.join("song.lua").canonicalize().ok()
    );
    assert!(loaded.script.ends_with("proyectos/../song.lua"));
    assert!(loaded.samples.unwrap().ends_with("proyectos/../samples"));

    // Sin carpeta de samples ni tempo.
    let minimal = Project {
        script: dir.join("song.lua"),
        samples: None,
        bpm: None,
        labels: Vec::new(),
        positions: Vec::new(),
    };
    let path = dir.join("min.sampler");
    minimal.save(&path).unwrap();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "# proyecto del sampler\nscript = song.lua\n"
    );
    assert_eq!(
        Project::load(&path).unwrap(),
        Project {
            script: dir.join("song.lua"),
            samples: None,
            bpm: None,
            labels: Vec::new(),
            positions: Vec::new(),
        }
    );
}

#[test]
fn project_errors_say_where() {
    let dir = dir("errors");
    let cases = [
        ("bpm = 120\n", "falta «script = ...»"),
        (
            "script = a.lua\ntempo = 3\n",
            ":2: clave desconocida «tempo»",
        ),
        ("script = a.lua\nbpm = rápido\n", ":2: bpm no válido"),
        ("script = a.lua\nbpm = 9999\n", ":2: bpm no válido"),
        (
            "# comentario\nscript a.lua\n",
            ":2: se esperaba «clave = valor»",
        ),
    ];
    for (text, fragment) in cases {
        let path = dir.join("p.sampler");
        std::fs::write(&path, text).unwrap();
        let err = Project::load(&path).unwrap_err().to_string();
        assert!(err.contains(fragment), "\"{err}\" sin \"{fragment}\"");
    }
    let err = Project::load(&dir.join("no-existe.sampler")).unwrap_err();
    assert!(err.to_string().contains("no-existe.sampler"));
}

#[test]
fn sample_dir_and_default_bpm_come_from_the_project() {
    let dir = dir("settings");
    std::fs::create_dir_all(dir.join("samples")).unwrap();
    write_break(&dir.join("samples").join("break.wav"));
    let without_bpm = r#"
local amen = sample("break.wav"):slice(2)
track("d", amen):steps("1 . . .  . . . .  . . . .  . . . .")
"#;
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    // Sin carpeta de samples, se busca junto al script: no está.
    let err = host
        .load(&mut controller, without_bpm, "song.lua", &dir)
        .err()
        .unwrap();
    assert!(err.message.contains("break.wav"), "{err}");

    host.set_sample_dir(Some(dir.join("samples")));
    host.set_default_bpm(Some(140.0));
    let song = host
        .load(&mut controller, without_bpm, "song.lua", &dir)
        .unwrap();
    assert_eq!(song.bpm, Some(140.0));
    // Si el script llama a bpm(), manda el script.
    let with_bpm = format!("bpm(90)\n{without_bpm}");
    let song = host
        .load(&mut controller, &with_bpm, "song.lua", &dir)
        .unwrap();
    assert_eq!(song.bpm, Some(90.0));
}

#[test]
fn song_render_matches_the_live_engine() {
    let dir = dir("render");
    write_break(&dir.join("break.wav"));
    let src = r#"bpm(150)
local amen = sample("break.wav"):slice(2)
track("d", amen)
  :steps("1 . 2 .  1 . 2'r2 .  1 . 2 .  1^5 . 2 .")
  :every(3, function(t) t:reverse() end)
  :fx("lowpass", { cutoff = 2000 })
track("k", amen:slice_at(1)):steps("x . . .  x . . .  x . . .  x . . ."):gain(0.6)
"#;
    let (mut controller, mut engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let song = host
        .load(&mut controller, src, "song.lua", &dir)
        .unwrap()
        .clone();
    // every(3): ciclo de 3 compases; al menos 4 → 6.
    assert_eq!(song.length_bars(4), 6);
    assert_eq!(song.length_bars(1), 3);

    let bars = song.length_bars(4);
    let rendered = song.render(RATE, bars).unwrap();
    assert_eq!(
        rendered.len() / 2,
        (bars * bar_frames(150.0, RATE)) as usize
    );
    assert!(rendered.iter().any(|&s| s.abs() > 0.2));

    // Igual que lo que suena en el motor en vivo.
    controller.start().unwrap();
    let live = offline::render(&mut engine, rendered.len() / 2, 2);
    assert_eq!(rendered, live);
}
