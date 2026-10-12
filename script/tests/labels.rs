//! Etiquetas de corte: `:name()`, las del proyecto y su uso en `steps`.
//! Lo principal: con etiquetas suena idéntico, muestra a muestra, que con
//! los números (render offline).

use std::path::{Path, PathBuf};

use engine::{new_engine, offline};
use script::{Project, ScriptHost, SliceLabel};

const RATE: u32 = 48_000;

/// `break8.wav`: ocho cortes de 300 frames, cada 3000, de volumen distinto
/// (así cada corte suena distinto y un corte cambiado se nota).
fn fixture(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("labels_{test}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut eight = vec![0.0f32; 24_000];
    for k in 0..8 {
        eight[k * 3000..k * 3000 + 300].fill(0.2 + 0.05 * k as f32);
    }
    offline::write_wav(dir.join("break8.wav"), &eight, 1, RATE).unwrap();
    dir
}

fn host() -> ScriptHost {
    let mut host = ScriptHost::new(RATE);
    host.set_seed(Some(7));
    host
}

/// Evalúa y renderiza `bars` compases (en un motor aparte).
fn render(host: &mut ScriptHost, dir: &Path, source: &str, bars: u64) -> Vec<f32> {
    let song = host
        .eval(source, "song.lua", dir)
        .unwrap_or_else(|e| panic!("{e}\n{source}"));
    song.render(RATE, bars).unwrap()
}

fn error(host: &mut ScriptHost, dir: &Path, source: &str) -> String {
    match host.eval(source, "song.lua", dir) {
        Ok(_) => panic!("debería fallar:\n{source}"),
        Err(e) => e.message,
    }
}

const NAMED: &str = r#"bpm(120)
local amen = sample("break8.wav"):slice(8)
amen:name(1, "kick"):name(2, "snare"):name(5, "hat")
"#;

#[test]
fn labels_sound_identical_to_numbers() {
    let dir = fixture("identical");
    let mut host = host();
    let by_name = render(
        &mut host,
        &dir,
        &format!(
            "{NAMED}track(\"d\", amen):steps(\"kick . snare .  kick . snare .  kick . snare .  kick . snare .\")"
        ),
        2,
    );
    let by_number = render(
        &mut host,
        &dir,
        &format!("{NAMED}track(\"d\", amen):steps(\"1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .\")"),
        2,
    );
    assert!(by_name.iter().any(|s| s.abs() > 0.1), "suena");
    assert_eq!(by_name, by_number);

    // Con sufijos, en every y en chance (misma semilla), igual.
    let named = "hat^7@0.5 kick' snare 4  kickr4 . hat'^-5 .  x . 3 .  snare@1.5 . . kick";
    let numbered = "5^7@0.5 1' 2 4  1r4 . 5'^-5 .  1 . 3 .  2@1.5 . . 1";
    let song = |steps: &str, every: &str| {
        format!(
            "{NAMED}track(\"d\", amen):steps(\"{steps}\")\n  \
             :every(2, function(c) c:steps(\"{every}\") end)\n  \
             :chance(0.3, \"retrig\", 3)"
        )
    };
    let a = render(
        &mut host,
        &dir,
        &song(named, "kick . . .  snare . . .  hat . . .  . . . ."),
        4,
    );
    let b = render(
        &mut host,
        &dir,
        &song(numbered, "1 . . .  2 . . .  5 . . .  . . . ."),
        4,
    );
    assert_eq!(a, b);
}

#[test]
fn slice_at_keeps_its_label_and_later_names_apply_to_existing_tracks() {
    let dir = fixture("slice_at");
    let mut host = host();
    let a = render(
        &mut host,
        &dir,
        &format!(
            "{NAMED}track(\"s\", amen:slice_at(2)):steps(\"snare . . .  snare . . .  . . . .  . . . .\")"
        ),
        1,
    );
    let b = render(
        &mut host,
        &dir,
        &format!(
            "{NAMED}track(\"s\", amen:slice_at(2)):steps(\"x . . .  x . . .  . . . .  . . . .\")"
        ),
        1,
    );
    assert_eq!(a, b);
    // `name` después de `track`: vale para los `steps` siguientes.
    let source = "bpm(120)\nlocal amen = sample(\"break8.wav\"):slice(8)\n\
                  local t = track(\"d\", amen)\namen:name(3, \"caja\")\n\
                  t:steps(\"caja . . .  . . . .  . . . .  . . . .\")";
    let song = host.eval(source, "song.lua", &dir).unwrap();
    let track = &song.tracks[0];
    assert_eq!(track.labels[2].as_deref(), Some("caja"));
    assert_eq!(track.steps[0].unwrap().slice, 2);
    assert_eq!(track.origin.sample, "break8.wav");
    assert_eq!(track.origin.count, 8);
    assert_eq!(track.origin.indices.len(), 8);
    let song = host
        .eval(
            &format!("{NAMED}track(\"s\", amen:slice_at(5))"),
            "song.lua",
            &dir,
        )
        .unwrap();
    assert_eq!(song.tracks[0].labels, vec![Some("hat".to_string())]);
    assert_eq!(song.tracks[0].origin.indices, vec![4]);
}

#[test]
fn unknown_labels_and_bad_names_give_clear_errors() {
    let dir = fixture("errors");
    let mut host = host();
    let e = error(
        &mut host,
        &dir,
        &format!("{NAMED}track(\"d\", amen):steps(\"kick . snre .  . . . .  . . . .  . . . .\")"),
    );
    assert_eq!(
        e,
        "song.lua:4: steps: paso 3 (\"snre\"): la etiqueta \"snre\" no existe (hay: kick, snare, hat)"
    );
    let cases = [
        (
            "amen:name(1, \"12\")",
            "name(1, \"12\"): \"12\" es un número",
        ),
        ("amen:name(1, \"x\")", "ya es el corte 1"),
        ("amen:name(1, \"r4\")", "retrigger"),
        ("amen:name(1, \"bombo2r3\")", "retrigger"),
        ("amen:name(1, \"bom bo\")", "solo letras"),
        (
            "amen:name(9, \"kick\")",
            "name(9): el corte no existe (hay 8)",
        ),
        (
            "amen:name(3, \"kick\")",
            "name(3, \"kick\"): la etiqueta ya es del corte 1",
        ),
    ];
    for (call, expected) in cases {
        let e = error(&mut host, &dir, &format!("{NAMED}{call}"));
        assert!(e.starts_with("song.lua:4: "), "{e}");
        assert!(e.contains(expected), "{call}: {e}");
    }
    // Renombrar el mismo corte o quitar la etiqueta con nil sí vale.
    let song = host
        .eval(
            &format!("{NAMED}amen:name(1, \"bombo\"):name(5, nil)\ntrack(\"d\", amen)"),
            "song.lua",
            &dir,
        )
        .unwrap();
    assert_eq!(
        song.tracks[0].labels[..5],
        [Some("bombo".into()), Some("snare".into()), None, None, None]
    );
}

#[test]
fn project_labels_apply_to_the_same_sample_cut_the_same_way() {
    let dir = fixture("project");
    let mut host = host();
    let label = |slices, slice, name: &str| SliceLabel {
        sample: "break8.wav".into(),
        slices,
        slice,
        name: name.into(),
    };
    host.set_labels(vec![
        label(8, 1, "kick"),
        label(8, 2, "snare"),
        label(4, 1, "otro"),
    ]);
    let source = "bpm(120)\nlocal amen = sample(\"break8.wav\"):slice(8)\n\
                  track(\"d\", amen):steps(\"kick . snare .  kick . snare .  kick . snare .  kick . snare .\")";
    let by_name = render(&mut host, &dir, source, 1);
    let by_number = render(
        &mut host,
        &dir,
        "bpm(120)\nlocal amen = sample(\"break8.wav\"):slice(8)\n\
         track(\"d\", amen):steps(\"1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .\")",
        1,
    );
    assert_eq!(by_name, by_number);
    // Con otro número de cortes no se aplican (los cortes son otros).
    let e = error(
        &mut host,
        &dir,
        "local amen = sample(\"break8.wav\"):slice(6)\ntrack(\"d\", amen):steps(\"kick . . .  . . . .  . . . .  . . . .\")",
    );
    assert!(e.contains("la etiqueta \"kick\" no existe"), "{e}");
    // `:name()` manda sobre el proyecto.
    let song = host
        .eval(
            "local amen = sample(\"break8.wav\"):slice(8):name(1, \"bombo\")\ntrack(\"d\", amen)",
            "song.lua",
            &dir,
        )
        .unwrap();
    assert_eq!(song.tracks[0].labels[0].as_deref(), Some("bombo"));
    assert_eq!(song.tracks[0].labels[1].as_deref(), Some("snare"));

    // set_label: pone, cambia, quita y comprueba.
    host.set_label("break8.wav", 8, 3, Some("hat")).unwrap();
    host.set_label("break8.wav", 8, 1, Some("bombo")).unwrap();
    host.set_label("break8.wav", 8, 2, None).unwrap();
    let mine: Vec<(usize, &str)> = host
        .labels()
        .iter()
        .filter(|l| l.slices == 8)
        .map(|l| (l.slice, l.name.as_str()))
        .collect();
    assert_eq!(mine, [(3, "hat"), (1, "bombo")]);
    assert_eq!(
        host.set_label("break8.wav", 8, 4, Some("hat")).unwrap_err(),
        "la etiqueta \"hat\" ya es del corte 3"
    );
    assert!(
        host.set_label("break8.wav", 8, 4, Some("7"))
            .unwrap_err()
            .contains("es un número")
    );
    assert_eq!(
        host.set_label("break8.wav", 8, 9, Some("y")).unwrap_err(),
        "el corte 9 no existe (hay 8)"
    );
}

#[test]
fn projects_save_and_load_labels() {
    let dir = fixture("project_file");
    let path = dir.join("p.sampler");
    let project = Project {
        script: dir.join("song.lua"),
        samples: None,
        bpm: Some(170.0),
        labels: vec![
            SliceLabel {
                sample: "break8.wav".into(),
                slices: 8,
                slice: 1,
                name: "kick".into(),
            },
            SliceLabel {
                sample: "mis breaks/amen 2.wav".into(),
                slices: 16,
                slice: 12,
                name: "snare_2".into(),
            },
        ],
    };
    project.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("etiqueta = break8.wav 8 1 kick\n"), "{text}");
    assert!(
        text.contains("etiqueta = mis breaks/amen 2.wav 16 12 snare_2\n"),
        "{text}"
    );
    assert_eq!(Project::load(&path).unwrap().labels, project.labels);

    let bad = [
        (
            "etiqueta = amen.wav 8 1",
            "se esperaba «etiqueta = sample cortes corte nombre»",
        ),
        (
            "etiqueta = amen.wav 8 9 kick",
            "corte no válido: 9 (de 1 a 8)",
        ),
        ("etiqueta = amen.wav 8 1 12", "es un número"),
        ("etiqueta = amen.wav 8 1 x", "ya es el corte 1"),
        (
            "etiqueta = amen.wav 8 1 kick\netiqueta = amen.wav 8 2 kick",
            "la etiqueta «kick» ya es del corte 1",
        ),
        (
            "etiqueta = amen.wav 8 1 kick\netiqueta = amen.wav 8 1 bombo",
            "el corte 1 ya tiene la etiqueta «kick»",
        ),
    ];
    for (lines, expected) in bad {
        std::fs::write(&path, format!("script = song.lua\n{lines}\n")).unwrap();
        let e = Project::load(&path).unwrap_err().to_string();
        assert!(e.contains(expected), "{lines}: {e}");
        assert!(e.contains("p.sampler:"), "con línea: {e}");
    }
    // La misma etiqueta en otro sample o con otro número de cortes, sí.
    std::fs::write(
        &path,
        "script = song.lua\netiqueta = a.wav 8 1 kick\netiqueta = a.wav 4 1 kick\netiqueta = b.wav 8 1 kick\n",
    )
    .unwrap();
    assert_eq!(Project::load(&path).unwrap().labels.len(), 3);
}

#[test]
fn the_live_engine_plays_labels_like_numbers() {
    // Además del render aparte, cargando en el motor en vivo.
    let dir = fixture("live");
    let play = |steps: &str| {
        let (mut controller, mut engine) = new_engine(RATE);
        let mut host = host();
        host.load(
            &mut controller,
            &format!("{NAMED}track(\"d\", amen):steps(\"{steps}\")"),
            "song.lua",
            &dir,
        )
        .unwrap();
        controller.start().unwrap();
        offline::render(&mut engine, 2 * 96_000, 2)
    };
    let a = play("kick snare hat .  kick . . .  snare . . .  hat . kick .");
    let b = play("1 2 5 .  1 . . .  2 . . .  5 . 1 .");
    assert!(a.iter().any(|s| s.abs() > 0.1));
    assert_eq!(a, b);
}
