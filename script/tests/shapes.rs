//! `cortes:set()`, `pista:mode()` y `pista:choke()`: el script suena igual
//! que el motor con lo mismo, y los errores dicen la línea.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{Pattern, Sample, SliceShape, Slices, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;
const STEPS: &str = "1 . 2 .  1 1 2 .  . 1 . 2  1 . 2 2";

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("shapes");
        std::fs::create_dir_all(&dir).unwrap();
        let mut mono = vec![0.0f32; 24_000];
        mono[..10_000].fill(0.4);
        mono[12_000..22_000].fill(0.2);
        offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!("bpm(120)\nlocal b = sample(\"break.wav\"):slice(2)\n{source}");
    host.load(&mut controller, &source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

fn engine_render(patterns: Vec<(usize, Pattern)>) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    for (t, p) in patterns {
        controller.set_track_pattern(t, Some(p)).unwrap();
    }
    controller.start().unwrap();
    offline::render(&mut engine, 2 * engine::bar_frames(120.0, RATE) as usize, 2)
}

fn base() -> (Arc<Sample>, Slices) {
    let sample = Arc::new(Sample::load_wav(dir().join("break.wav"), RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    (sample, slices)
}

#[test]
fn shapes_gate_and_choke_sound_like_the_engine() {
    let song = song(&format!(
        "b:set({{ release = 10 }}):set(2, {{ attack = 4 }})\n\
         track(\"d\", b):steps(\"{STEPS}\"):mode(\"gate\"):choke(2)\n\
         track(\"k\", b:slice_at(1)):steps(\"x . . .  x . . .  x . . .  x . . .\"):choke(2)"
    ))
    .unwrap();
    let shapes = [
        SliceShape {
            attack_ms: 0.0,
            release_ms: 10.0,
            ..SliceShape::DEFAULT
        },
        SliceShape {
            attack_ms: 4.0,
            release_ms: 10.0,
            ..SliceShape::DEFAULT
        },
    ];
    assert_eq!(song.tracks[0].shapes, shapes);
    assert_eq!(song.tracks[1].shapes, shapes[..1]);
    assert!(song.tracks[0].gate && !song.tracks[1].gate);
    assert_eq!(song.tracks[1].choke, Some(2));

    let (sample, slices) = base();
    let steps = engine::notation::parse_steps(STEPS).unwrap();
    let d = Pattern::from_steps(sample.clone(), &slices, steps)
        .unwrap()
        .with_shapes(&shapes)
        .unwrap()
        .with_gate(true)
        .with_choke(Some(2))
        .unwrap();
    let kick = engine::notation::parse_steps("x . . .  x . . .  x . . .  x . . .").unwrap();
    let k = Pattern::from_steps(sample, &slices.only(0).unwrap(), kick)
        .unwrap()
        .with_shapes(&shapes[..1])
        .unwrap()
        .with_choke(Some(2))
        .unwrap();
    assert_eq!(
        song.render(RATE, 2).unwrap(),
        engine_render(vec![(0, d), (1, k)])
    );
}

#[test]
fn set_after_track_still_applies() {
    let song = song(&format!(
        "track(\"d\", b):steps(\"{STEPS}\")\nb:set(1, {{ attack = 30 }})"
    ))
    .unwrap();
    assert_eq!(song.tracks[0].shapes[0].attack_ms, 30.0);
}

#[test]
fn shape_errors_point_at_the_line() {
    let error = |source: &str| song(source).err().expect("debería fallar");
    assert_eq!(
        error("b:set(3, { attack = 5 })"),
        "song.lua:3: set(3): el corte no existe (hay 2)"
    );
    assert_eq!(
        error("b:set({ ataque = 5 })"),
        "song.lua:3: set: parámetro desconocido \"ataque\" (admite: attack, release, pan, length, play)"
    );
    assert!(error("b:set({ release = 9000 })").contains("cola fuera de rango"));
    assert!(error("b:set(1)").contains("set(i, { ... })"));
    assert!(error("track(\"d\", b):mode(\"legato\")").contains("es \"gate\" u \"oneshot\""));
    assert!(error("track(\"d\", b):choke(20)").contains("el grupo va de 1 a 16"));
    assert!(
        song("track(\"d\", b):choke(3):choke(0)").unwrap().tracks[0]
            .choke
            .is_none()
    );
}

#[test]
fn pan_length_and_play_sound_like_the_engine() {
    let song = song(&format!(
        "b:set(1, {{ pan = -1, length = 0.25, play = \"loop\" }})\n\
         b:set(2, {{ pan = 0.5, play = \"pingpong\" }})\n\
         track(\"d\", b):steps(\"{STEPS}\")"
    ))
    .unwrap();
    let shapes = [
        SliceShape {
            pan: -1.0,
            length: 0.25,
            play: engine::PlayMode::Loop,
            ..SliceShape::DEFAULT
        },
        SliceShape {
            pan: 0.5,
            play: engine::PlayMode::PingPong,
            ..SliceShape::DEFAULT
        },
    ];
    assert_eq!(song.tracks[0].shapes, shapes);
    let (sample, slices) = base();
    let d = Pattern::from_steps(
        sample,
        &slices,
        engine::notation::parse_steps(STEPS).unwrap(),
    )
    .unwrap()
    .with_shapes(&shapes)
    .unwrap();
    assert_eq!(song.render(RATE, 2).unwrap(), engine_render(vec![(0, d)]));
    let error = song_error("b:set({ play = \"atras\" })");
    assert!(error.contains("play = \"atras\" no existe"), "{error}");
    assert!(song_error("b:set({ pan = 2 })").contains("pan fuera de rango"));
    assert!(song_error("b:set({ length = 0 })").contains("length fuera de rango"));
}

fn song_error(source: &str) -> String {
    song(source).err().expect("debería fallar")
}
