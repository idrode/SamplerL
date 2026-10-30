//! Fase D en el script: mute, solo, metrónomo, tempo detectado y lo que se
//! oye en el render.

#[path = "../../engine/tests/support/synth_break.rs"]
mod synth;

use std::path::{Path, PathBuf};

use engine::offline;
use script::{Cut, ScriptHost};

const RATE: u32 = 48_000;

fn fixture(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("mixer_{test}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut eight = vec![0.0f32; 24_000];
    for k in 0..8 {
        eight[k * 3000..k * 3000 + 300].fill(0.2 + 0.05 * k as f32);
    }
    offline::write_wav(dir.join("break8.wav"), &eight, 1, RATE).unwrap();
    dir
}

fn render(dir: &Path, body: &str) -> Vec<f32> {
    let source = format!("bpm(120)\nlocal amen = sample(\"break8.wav\"):slice(8)\n{body}");
    let song = ScriptHost::new(RATE)
        .eval(&source, "song.lua", dir)
        .unwrap_or_else(|e| panic!("{e}"));
    song.render(RATE, 2).unwrap()
}

const A: &str = "track(\"a\", amen):steps(\"1 . . .  1 . . .  1 . . .  1 . . .\")";
const B: &str = "track(\"b\", amen):steps(\". . 3 .  . . 3 .  . . 3 .  . . 3 .\")";

#[test]
fn script_mute_and_solo_sound_like_the_tracks_that_remain() {
    let dir = fixture("mute_solo");
    let only_a = render(&dir, A);
    let only_b = render(&dir, B);
    assert_eq!(render(&dir, &format!("{A}\n{B}:mute()")), only_a);
    assert_eq!(render(&dir, &format!("{A}\n{B}:solo()")), only_b);
    assert_eq!(
        render(&dir, &format!("{A}:solo():mute()\n{B}:solo()")),
        only_b
    );
    // mute(false) y solo(false) deshacen.
    assert_eq!(
        render(&dir, &format!("{A}\n{B}:mute():mute(false)")),
        render(&dir, &format!("{A}\n{B}"))
    );
}

#[test]
fn the_metronome_only_goes_into_the_render_when_asked() {
    let dir = fixture("metronome");
    let source =
        format!("bpm(120)\nmetronome(true)\nlocal amen = sample(\"break8.wav\"):slice(8)\n{A}");
    let song = ScriptHost::new(RATE)
        .eval(&source, "song.lua", &dir)
        .unwrap();
    assert_eq!(song.metronome, Some(true));
    // El metronome() del script no entra en el render.
    assert_eq!(song.render(RATE, 1).unwrap(), render(&dir, A)[..2 * 96_000]);
    // Pedido, sí: la diferencia es el clic de cada pulso (el primero, con
    // acento, más fuerte).
    let with = song.render_with_metronome(RATE, 1, true).unwrap();
    let without = song.render(RATE, 1).unwrap();
    let click: Vec<f32> = with
        .iter()
        .zip(&without)
        .map(|(a, b)| a - b)
        .step_by(2)
        .collect();
    let peak = |at: usize| {
        click[at..at + 1440]
            .iter()
            .fold(0.0f32, |m, s| m.max(s.abs()))
    };
    assert!(peak(0) > 0.4, "acento: {}", peak(0));
    for beat in 1..4 {
        assert!((0.25..0.35).contains(&peak(beat * 24_000)), "pulso {beat}");
    }
    assert!(click[2000..23_000].iter().all(|s| s.abs() < 1e-6));
    let off = ScriptHost::new(RATE)
        .eval("metronome(false)", "song.lua", &dir)
        .unwrap();
    assert_eq!(off.metronome, Some(false));
    assert_eq!(
        ScriptHost::new(RATE)
            .eval("bpm(120)", "song.lua", &dir)
            .unwrap()
            .metronome,
        None
    );
}

#[test]
fn detect_bpm_reads_the_synthetic_break() {
    let dir = fixture("detect_bpm");
    let (audio, _) = synth::synth_break();
    offline::write_wav(dir.join("amen.wav"), &audio, 1, synth::RATE).unwrap();
    let source = "local amen = sample(\"amen.wav\")\n\
                  local tempo, compases = amen:detect_bpm()\n\
                  bpm(tempo)\n\
                  error(\"=\" .. tempo .. \" \" .. compases, 0)";
    let e = ScriptHost::new(RATE)
        .eval(source, "song.lua", &dir)
        .err()
        .unwrap();
    assert!(e.message.starts_with("=140 1"), "{e}");
    // En silencio, nil: bpm(nil) da error con línea.
    offline::write_wav(dir.join("silencio.wav"), &vec![0.0; 4800], 1, RATE).unwrap();
    let e = ScriptHost::new(RATE)
        .eval(
            "local t = sample(\"silencio.wav\"):detect_bpm()\nassert(t == nil, \"no\")",
            "song.lua",
            &dir,
        )
        .map(|_| ())
        .map_err(|e| e.message);
    assert_eq!(e, Ok(()));
}

#[test]
fn slices_moved_by_hand_win_over_the_detector() {
    use script::{Project, SlicePosition};
    let dir = fixture("move");
    let steps = "track(\"d\", amen):steps(\"1 2 3 4  . . . .  . . . .  . . . .\")";
    // break8: cortes cada 3000 frames (62,5 ms). El corte 2 pasa a 70 ms.
    let eval = |host: &mut ScriptHost, body: &str| {
        host.eval(
            &format!("bpm(120)\nlocal amen = sample(\"break8.wav\"):slice(8)\n{body}"),
            "song.lua",
            &dir,
        )
        .unwrap_or_else(|e| panic!("{e}"))
    };
    let moved = eval(
        &mut ScriptHost::new(RATE),
        &format!("amen:move(2, 70)\n{steps}"),
    );
    let track = &moved.tracks[0];
    assert_eq!(track.slices.get(0), Some(0..3360));
    assert_eq!(track.slices.get(1), Some(3360..6000));
    assert_eq!(track.adjusted[..3], [false, true, false]);

    // El mismo ajuste desde el proyecto suena idéntico.
    let mut host = ScriptHost::new(RATE);
    host.set_positions(vec![SlicePosition {
        sample: "break8.wav".into(),
        slices: Cut::detect(8),
        slice: 2,
        ms: 70.0,
    }]);
    let from_project = eval(&mut host, steps);
    assert_eq!(from_project.tracks[0].slices, track.slices);
    assert_eq!(
        from_project.render(RATE, 1).unwrap(),
        moved.render(RATE, 1).unwrap()
    );
    // Y no suena como sin ajustar: el corte 1 dura más y el 2 empieza tarde.
    assert_ne!(
        moved.render(RATE, 1).unwrap(),
        render(&dir, steps)[..2 * 96_000]
    );
    // slice_at conserva la marca.
    let at = eval(&mut host, "track(\"s\", amen:slice_at(2))");
    assert_eq!(at.tracks[0].adjusted, vec![true]);
    assert_eq!(at.tracks[0].slices.get(0), Some(3360..6000));

    // Un ajuste del proyecto que se sale de sus vecinos avisa y no se aplica.
    host.set_position("break8.wav", Cut::detect(8), 2, Some(200.0));
    let song = eval(&mut host, steps);
    assert_eq!(song.tracks[0].slices.get(1), Some(3000..6000));
    assert_eq!(
        song.warnings,
        [
            "el ajuste del corte 2 de break8.wav (200 ms) se sale de sus vecinos \
          (1.0 a 124.0 ms): no se aplica"
        ]
    );
    host.set_position("break8.wav", Cut::detect(8), 2, None);
    assert!(host.positions().is_empty());

    // En Lua, error con línea.
    let e = ScriptHost::new(RATE)
        .eval(
            "local amen = sample(\"break8.wav\"):slice(8)\namen:move(2, 200)",
            "song.lua",
            &dir,
        )
        .err()
        .unwrap();
    assert_eq!(
        e.message,
        "song.lua:2: move(2, 200): se sale de sus vecinos (1.0 a 124.0 ms)"
    );

    // El proyecto los guarda y los lee.
    let path = dir.join("p.sampler");
    let project = Project {
        script: dir.join("song.lua"),
        samples: None,
        bpm: None,
        labels: Vec::new(),
        positions: vec![SlicePosition {
            sample: "break8.wav".into(),
            slices: Cut::detect(8),
            slice: 2,
            ms: 70.5,
        }],
        edits: Vec::new(),
        learned: Vec::new(),
    };
    project.save(&path).unwrap();
    assert!(
        std::fs::read_to_string(&path)
            .unwrap()
            .contains("corte = break8.wav 8 2 70.5\n")
    );
    assert_eq!(Project::load(&path).unwrap().positions, project.positions);
    for (line, expected) in [
        ("corte = a.wav 8 9 10", "corte no válido"),
        ("corte = a.wav 8 2 -3", "posición no válida"),
        (
            "corte = a.wav 8 2 10\ncorte = a.wav 8 2 20",
            "ya tiene un ajuste",
        ),
    ] {
        std::fs::write(&path, format!("script = song.lua\n{line}\n")).unwrap();
        let e = Project::load(&path).unwrap_err().to_string();
        assert!(e.contains(expected), "{line}: {e}");
    }
}

#[test]
fn slice_adjustments_do_not_depend_on_their_order() {
    // break8: cortes cada 62,5 ms. El 3 pasa de 125 a 135 ms y el 2 a
    // 130: el 2 solo cabe con el 3 ya movido. Guardados en cualquier orden (el
    // último tocado va al final), todos se aplican igual.
    let dir = fixture("move_order");
    let steps = "track(\"d\", amen):steps(\"1 2 3 4  . . . .  . . . .  . . . .\")";
    let cut = Cut::detect(8);
    let mut first = None;
    for order in [[3, 1, 2], [2, 3, 1], [1, 2, 3]] {
        let mut host = ScriptHost::new(RATE);
        for slice in order {
            let ms = match slice {
                1 => 5.0,
                2 => 130.0,
                _ => 135.0,
            };
            host.set_position("break8.wav", cut, slice, Some(ms));
        }
        let song = host
            .eval(
                &format!("bpm(120)\nlocal amen = sample(\"break8.wav\"):slice(8)\n{steps}"),
                "song.lua",
                &dir,
            )
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(song.warnings.is_empty(), "{order:?}: {:?}", song.warnings);
        let track = &song.tracks[0];
        assert_eq!(track.slices.get(0), Some(240..6240), "{order:?}");
        assert_eq!(track.slices.get(1), Some(6240..6480), "{order:?}");
        assert_eq!(track.slices.get(2), Some(6480..9000), "{order:?}");
        assert_eq!(track.adjusted[..4], [true, true, true, false]);
        match &first {
            None => first = Some(track.slices.clone()),
            Some(slices) => assert_eq!(&track.slices, slices),
        }
    }

    // Dos ajustes que chocan entre sí (un proyecto editado a mano): el de
    // la derecha no se aplica; el otro, sí.
    let mut host = ScriptHost::new(RATE);
    host.set_position("break8.wav", cut, 3, Some(90.0));
    host.set_position("break8.wav", cut, 2, Some(100.0));
    let song = host
        .eval(
            &format!("bpm(120)\nlocal amen = sample(\"break8.wav\"):slice(8)\n{steps}"),
            "song.lua",
            &dir,
        )
        .unwrap();
    assert_eq!(song.tracks[0].slices.get(1), Some(4800..6000));
    assert_eq!(song.tracks[0].adjusted[..3], [false, true, false]);
    assert_eq!(song.warnings.len(), 1, "{:?}", song.warnings);
    assert_eq!(
        song.warnings[0],
        "el ajuste del corte 3 de break8.wav (90 ms) se sale de sus vecinos \
         (101.0 a 186.5 ms): no se aplica"
    );
}
