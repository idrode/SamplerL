//! `slice_grid(n)`, `cortes:add(ms)`, `cortes:remove(i)` y las ediciones
//! del proyecto (`partir`, `unir`): los cortes que salen, que suenan igual
//! que el motor, y la numeración de etiquetas y ajustes.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{Pattern, Sample, Slices, new_engine, offline};
use script::{Cut, EditKind, Project, ScriptHost, SliceEdit, SliceLabel, Song};

const RATE: u32 = 48_000;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("grid");
        std::fs::create_dir_all(&dir).unwrap();
        // 1 s: una rampa (cada muestra distinta) para ver qué se toca.
        let mono: Vec<f32> = (0..48_000).map(|i| 0.5 * i as f32 / 48_000.0).collect();
        offline::write_wav(dir.join("rampa.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn load(host: &mut ScriptHost, source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    host.load(&mut controller, source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

fn song(source: &str) -> Result<Song, String> {
    load(&mut ScriptHost::new(RATE), source)
}

fn starts(song: &Song) -> Vec<usize> {
    song.tracks[0].slices.iter().map(|r| r.start).collect()
}

#[test]
fn a_grid_cuts_in_equal_parts_and_sounds_like_the_engine() {
    let steps = "1 2 3 4  5 6 7 8  1 3 5 7  2 4 6 8";
    let song = song(&format!(
        "bpm(120)\nlocal r = sample(\"rampa.wav\"):slice_grid(8)\ntrack(\"d\", r):steps(\"{steps}\")"
    ))
    .unwrap();
    assert_eq!(starts(&song), (0..8).map(|k| k * 6000).collect::<Vec<_>>());
    assert_eq!(song.tracks[0].origin.cut(), Cut::grid(8));

    let sample = Arc::new(Sample::load_wav(dir().join("rampa.wav"), RATE).unwrap());
    let slices = Slices::grid(&sample, 8);
    let pattern = Pattern::from_steps(
        sample,
        &slices,
        engine::notation::parse_steps(steps).unwrap(),
    )
    .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    let frames = 2 * engine::bar_frames(120.0, RATE) as usize;
    assert_eq!(
        song.render(RATE, 2).unwrap(),
        offline::render(&mut engine, frames, 2)
    );
}

#[test]
fn add_and_remove_change_the_numbering() {
    let song = song(
        "local r = sample(\"rampa.wav\"):slice_grid(4):name(3, \"tres\")\n\
         r:add(100)\n\
         r:remove(4)\n\
         track(\"d\", r)",
    )
    .unwrap();
    // 0, 100 ms (nuevo), 250, (500 quitado), 750.
    assert_eq!(starts(&song), [0, 4800, 12_000, 36_000]);
    // La etiqueta iba con el corte de 500 ms, que se ha quitado; los cortes
    // saben de qué corte de la rejilla salen (el añadido, de ninguno).
    let t = &song.tracks[0];
    assert_eq!(t.labels, [None, None, None, None]);
    assert_eq!(t.origin.indices, [Some(0), None, Some(1), Some(3)]);
    // Errores con línea.
    let e = |s: &str| {
        song_err(&format!(
            "local r = sample(\"rampa.wav\"):slice_grid(4)\n{s}"
        ))
    };
    assert!(e("r:add(250.5)").contains("song.lua:2: add(250.5): ahí no cabe un corte nuevo"));
    assert!(e("r:remove(1)").contains("el corte 1 no tiene anterior"));
    assert!(e("r:remove(9)").contains("el corte no existe (hay 4)"));
}

fn song_err(source: &str) -> String {
    song(source).err().expect("debería fallar")
}

#[test]
fn project_edits_apply_before_positions_and_labels() {
    let mut host = ScriptHost::new(RATE);
    let cut = Cut::grid(4);
    host.set_label("rampa.wav", cut, 4, Some("cuatro")).unwrap();
    // Partir el 2 (en 375 ms): el nuevo es el 3 y la etiqueta del 4 pasa al 5.
    host.split_slice("rampa.wav", cut, 3, 375.0);
    assert_eq!(host.labels()[0].slice, 5);
    let source = "local r = sample(\"rampa.wav\"):slice_grid(4)\ntrack(\"d\", r)";
    let song = load(&mut host, source).unwrap();
    assert_eq!(starts(&song), [0, 12_000, 18_000, 24_000, 36_000]);
    assert_eq!(song.tracks[0].labels[4].as_deref(), Some("cuatro"));
    // Unir el 3 (el nuevo): vuelve a como estaba.
    host.join_slice("rampa.wav", cut, 3);
    assert_eq!(host.labels()[0].slice, 4);
    let song = load(&mut host, source).unwrap();
    assert_eq!(starts(&song), [0, 12_000, 24_000, 36_000]);
    // Las de slice(4) (detector) no se mezclan con las de la rejilla.
    host.set_label("rampa.wav", Cut::detect(4), 1, Some("otra"))
        .unwrap();
    let song = load(&mut host, source).unwrap();
    assert_eq!(song.tracks[0].labels[0], None);

    // Un partir que no cabe se avisa y no se aplica.
    let mut host = ScriptHost::new(RATE);
    host.split_slice("rampa.wav", cut, 2, 250.2);
    let song = load(&mut host, source).unwrap();
    assert_eq!(starts(&song).len(), 4);
    assert!(
        song.warnings[0].starts_with("partir rampa.wav en 250.2 ms"),
        "{:?}",
        song.warnings
    );
}

#[test]
fn the_project_saves_and_reads_edits_and_grid_keys() {
    let path = dir().join("p.sampler");
    let project = Project {
        script: dir().join("song.lua"),
        samples: None,
        bpm: None,
        labels: vec![SliceLabel {
            sample: "rampa.wav".into(),
            slices: Cut::grid(16),
            slice: 17,
            name: "extra".into(),
        }],
        positions: Vec::new(),
        edits: vec![
            SliceEdit {
                sample: "rampa.wav".into(),
                cut: Cut::grid(16),
                kind: EditKind::Split(12.5),
            },
            SliceEdit {
                sample: "rampa.wav".into(),
                cut: Cut::detect(8),
                kind: EditKind::Join(3),
            },
        ],
        learned: Vec::new(),
    };
    project.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("partir = rampa.wav rejilla16 12.5\n"),
        "{text}"
    );
    assert!(text.contains("unir = rampa.wav 8 3\n"), "{text}");
    assert!(
        text.contains("etiqueta = rampa.wav rejilla16 17 extra\n"),
        "{text}"
    );
    assert_eq!(Project::load(&path).unwrap().edits, project.edits);
    assert_eq!(Project::load(&path).unwrap().labels, project.labels);
    // Sin el partir, el corte 17 de una rejilla de 16 no existe.
    std::fs::write(
        &path,
        "script = song.lua\netiqueta = rampa.wav rejilla16 17 extra\n",
    )
    .unwrap();
    let e = Project::load(&path).unwrap_err().to_string();
    assert!(e.contains("corte no válido: 17 (de 1 a 16)"), "{e}");
    std::fs::write(&path, "script = song.lua\nunir = rampa.wav 8 1\n").unwrap();
    assert!(
        Project::load(&path)
            .unwrap_err()
            .to_string()
            .contains("de 2 a 64")
    );
}

/// `slice_grid(1)` es el sample entero; `slice(1)` no: su único corte
/// empieza en el golpe más fuerte y lo de antes queda fuera (la guía lo
/// dice en «Errores comunes de Lua»).
#[test]
fn the_whole_sample_is_slice_grid_1_not_slice_1() {
    // 1 s: un golpe flojo al principio y uno fuerte a la mitad.
    let hit = |i: usize, at: usize, gain: f32| {
        let t = i.wrapping_sub(at);
        if t < 2000 {
            gain * (t as f32 * 0.3).sin() * (1.0 - t as f32 / 2000.0)
        } else {
            0.0
        }
    };
    let mono: Vec<f32> = (0..48_000)
        .map(|i| hit(i, 0, 0.2) + hit(i, 24_000, 0.9))
        .collect();
    offline::write_wav(dir().join("dos_golpes.wav"), &mono, 1, RATE).unwrap();

    let whole =
        song("track(\"d\", sample(\"dos_golpes.wav\"):slice_grid(1)):steps(\"1 . . .  . . . .  . . . .  . . . .\")")
            .unwrap();
    assert_eq!(whole.tracks[0].slices.len(), 1);
    assert_eq!(whole.tracks[0].slices.iter().next().unwrap(), 0..48_000);

    let loudest =
        song("track(\"d\", sample(\"dos_golpes.wav\"):slice(1)):steps(\"1 . . .  . . . .  . . . .  . . . .\")")
            .unwrap();
    let only = loudest.tracks[0].slices.iter().next().unwrap();
    assert_eq!(loudest.tracks[0].slices.len(), 1);
    assert!((23_900..24_100).contains(&only.start), "{only:?}");
    assert_eq!(only.end, 48_000);
}
