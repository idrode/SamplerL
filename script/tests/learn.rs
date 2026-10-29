//! MIDI learn en el script: qué se puede asignar, cómo se traduce una
//! asignación por nombre a la tabla del motor (también si cambia el orden de
//! las pistas o de los efectos), que el mando mueve el sonido al cargar y
//! sin mandos todo suena igual, y las líneas `mando` del proyecto.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{KnobCurve, KnobTarget, new_engine, offline};
use script::learn::{self, Learned};
use script::{Project, ScriptHost, Song};

const RATE: u32 = 48_000;

#[path = "../../engine/tests/support/synth_break.rs"]
mod synth;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("learn");
        std::fs::create_dir_all(&dir).unwrap();
        let (audio, _) = synth::synth_break();
        offline::write_wav(dir.join("amen.wav"), &audio, 1, RATE).unwrap();
        dir
    })
    .clone()
}

const SONG: &str = r#"bpm(170)
bus("reverb", { size = ramp(0.2, 0.8, "4 compases") })
local amen = sample("amen.wav"):slice(8)
track("drums", amen)
  :steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
  :fx("lowpass", { cutoff = 6000 })
  :fx("lowpass", { cutoff = lfo("2 compases", 300, 3000) })
  :fx("bitcrush", { bits = 8 })
track("bass", synth("acid", {}))
  :melody("c2 . c3 .  . . . .  . . . .  . . . .")
"#;

fn song(source: &str) -> Song {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(&mut controller, source, "song.lua", &dir())
        .cloned()
        .unwrap()
}

fn line(text: &str) -> Learned {
    Learned::parse(text).unwrap()
}

#[test]
fn what_can_be_learned_comes_from_the_registry() {
    let all = learn::learnable(&song(SONG));
    let names: Vec<String> = all
        .iter()
        .map(|l| format!("{} {}", l.track, l.param))
        .collect();
    for wanted in [
        "drums lowpass.cutoff",
        "drums lowpass.q",
        "drums lowpass#2.cutoff",
        "drums bitcrush.bits",
        "drums gain",
        "drums pan",
        "drums send.delay",
        "drums send.reverb",
        "bass synth.cutoff",
        "bass gain",
        "bus reverb.size",
        "bus delay.feedback",
    ] {
        assert!(names.contains(&wanted.to_string()), "{wanted}: {names:?}");
    }
    // Lo que no es continuo en el registro (el tipo de filtro, el tiempo del
    // delay) no está.
    assert!(
        !names
            .iter()
            .any(|n| n.ends_with(".type") || n.ends_with(".time")),
        "{names:?}"
    );
    // Una asignación nueva, con su rango entero y la curva que le va.
    let cutoff = all.iter().find(|l| l.param == "lowpass.cutoff").unwrap();
    let assigned = cutoff.assign(Some(1), 21);
    assert_eq!(assigned.curve, KnobCurve::Log);
    assert_eq!((assigned.min, assigned.max), (cutoff.min, cutoff.max));
}

#[test]
fn an_assignment_by_name_finds_its_target_wherever_it_is() {
    let s = song(SONG);
    let mut slots: [Option<String>; engine::MAX_TRACKS] = Default::default();
    slots[2] = Some("drums".into());
    slots[5] = Some("bass".into());
    let learned = vec![
        line("1 21 drums lowpass.cutoff 200 8000 log"),
        line("1 22 drums lowpass#2.cutoff 200 8000 log"),
        line("* 23 drums send.reverb 0 1 lin"),
        line("1 24 bass synth.cutoff 100 4000 log"),
        line("1 25 bus reverb.size 0 1 lin"),
        line("1 26 nadie gain 0 1 lin"),
        line("1 27 drums distortion.drive 0 1 lin"),
    ];
    let (knobs, warnings) = learn::resolve(&learned, &s, &slots);
    let target = |i: usize| knobs[i].map(|k| (k.target, k.latch));
    assert_eq!(
        target(0),
        Some((
            KnobTarget::Fx {
                track: 2,
                slot: 0,
                param: 0
            },
            true
        ))
    );
    // El segundo filtro tiene un LFO: el mando lo devuelve al soltarse.
    assert_eq!(
        target(1),
        Some((
            KnobTarget::Fx {
                track: 2,
                slot: 1,
                param: 0
            },
            false
        ))
    );
    assert_eq!(
        target(2),
        Some((
            KnobTarget::Mix {
                track: 2,
                index: (engine::MIX_SEND + 1) as u8
            },
            true
        ))
    );
    assert_eq!(
        target(3).map(|t| t.0),
        Some(KnobTarget::Synth {
            track: 5,
            param: engine::synth::CUTOFF as u8
        })
    );
    // El tamaño de la reverb tiene una rampa.
    assert_eq!(
        target(4),
        Some((
            KnobTarget::Bus {
                param: engine::bus::REVERB_SIZE as u8
            },
            false
        ))
    );
    assert_eq!((target(5), target(6)), (None, None));
    assert_eq!(warnings.len(), 2, "{warnings:?}");
    assert!(
        warnings[0].contains("nadie · gain (CC 26 · canal 1): ese parámetro no está en el script")
    );
}

/// Cargado con mandos, el mando mueve el sonido; sin moverlo, y sin
/// mandos, suena bit a bit igual.
#[test]
fn a_learned_knob_moves_the_sound_after_loading() {
    // Un solo filtro, abierto (el LFO del segundo de SONG empieza en
    // 300 Hz y ya se come los agudos).
    let source = "bpm(170)\nlocal amen = sample(\"amen.wav\"):slice(8)\n\
        track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n\
        :fx(\"lowpass\", { cutoff = 6000 })\n";
    let render = |learned: Vec<Learned>, moves: &[(u8, u8, u8)]| {
        let (mut controller, mut engine) = new_engine(RATE);
        let mut host = ScriptHost::new(RATE);
        host.set_learned(learned);
        host.load(&mut controller, source, "song.lua", &dir())
            .unwrap();
        controller.start().unwrap();
        for &(channel, cc, value) in moves {
            host.move_learned(&mut controller, channel, cc, value)
                .unwrap();
        }
        offline::render(&mut engine, 48_000, 2)
    };
    let plain = render(Vec::new(), &[]);
    let knob = vec![line("1 21 drums lowpass.cutoff 200 8000 log")];
    assert_eq!(render(knob.clone(), &[]), plain);
    // Otro canal: no lo oye.
    assert_eq!(render(knob.clone(), &[(2, 21, 0)]), plain);
    let closed = render(knob, &[(1, 21, 0)]);
    // Los agudos (lo que cambia de una muestra a la siguiente), en el
    // segundo medio segundo.
    let highs = |x: &[f32]| {
        x[48_000..]
            .windows(4)
            .step_by(2)
            .map(|w| (w[2] - w[0]).powi(2))
            .sum::<f32>()
    };
    assert!(
        highs(&closed) < 0.3 * highs(&plain),
        "{} {}",
        highs(&closed),
        highs(&plain)
    );
}

#[test]
fn the_project_keeps_its_knobs() {
    let path = dir().join("mandos.sampler");
    let project = Project {
        script: dir().join("song.lua"),
        samples: None,
        bpm: None,
        labels: Vec::new(),
        positions: Vec::new(),
        edits: Vec::new(),
        learned: vec![
            line("1 21 drums lowpass.cutoff 200 8000 log"),
            line("* 74 bus reverb.size 0 1 lin"),
        ],
    };
    project.save(&path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(
        text.contains("mando = 1 21 drums lowpass.cutoff 200 8000 log\n"),
        "{text}"
    );
    assert!(
        text.contains("mando = * 74 bus reverb.size 0 1 lin\n"),
        "{text}"
    );
    assert_eq!(Project::load(&path).unwrap().learned, project.learned);
    std::fs::write(&path, "script = song.lua\nmando = 1 21 drums\n").unwrap();
    let e = Project::load(&path).unwrap_err();
    assert!(e.0.contains(":2: «1 21 drums»: canal cc pista"), "{e}");
}
