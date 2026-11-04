//! `warp`, `smear` y `formant` en `cortes:set()` y en las llaves: los
//! cortes precalculados llegan a la pista, suenan como el motor con lo
//! mismo, se reaprovechan entre cargas y entre pistas, y los errores dicen
//! la línea (sin dejar de sonar lo anterior).

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{Pattern, Sample, Slices, Warp, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;
const STEPS: &str = "1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .";

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("deform");
        std::fs::create_dir_all(&dir).unwrap();
        let mono: Vec<f32> = (0..24_000)
            .map(|i| {
                let hz = if i < 12_000 { 220.0 } else { 330.0 };
                0.4 * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin()
            })
            .collect();
        offline::write_wav(dir.join("dos.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

const HEAD: &str = "bpm(120)\nlocal d = sample(\"dos.wav\"):slice_grid(2)\n";

fn load(host: &mut ScriptHost, source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    host.load(
        &mut controller,
        &format!("{HEAD}{source}"),
        "song.lua",
        &dir(),
    )
    .cloned()
    .map_err(|e| e.to_string())
}

fn song(source: &str) -> Result<Song, String> {
    load(&mut ScriptHost::new(RATE), source)
}

#[test]
fn warp_and_smear_are_rendered_and_sound_like_the_engine() {
    let song = song(&format!(
        "d:set(1, {{ warp = 0.3 }}):set(2, {{ smear = 0.4, warp = {{ {{0.25, 0.1}}, {{0.75, 0.5}} }} }})\n\
         track(\"a\", d):steps(\"{STEPS}\")"
    ))
    .unwrap();
    let track = &song.tracks[0];
    assert_eq!(track.shapes[0].warp, Some(Warp::midpoint(0.3).unwrap()));
    assert_eq!(
        track.shapes[1].warp,
        Some(Warp::new(&[(0.25, 0.1), (0.75, 0.5)]).unwrap())
    );
    assert_eq!(track.rendered.len(), 2);
    for (key, buffer) in &track.rendered {
        assert_eq!(buffer.frames(), key.end - key.start);
    }
    // Lo mismo hecho a mano con el motor.
    let sample = Arc::new(Sample::load_wav(dir().join("dos.wav"), RATE).unwrap());
    let slices = Slices::grid(&sample, 2);
    let pattern = Pattern::from_steps(
        sample.clone(),
        &slices,
        engine::notation::parse_steps(STEPS).unwrap(),
    )
    .unwrap()
    .with_shapes(&track.shapes)
    .unwrap();
    let rendered = pattern
        .render_keys()
        .into_iter()
        .map(|k| (k, Arc::new(engine::deform::render(&sample, &k))))
        .collect();
    let pattern = pattern.with_rendered(rendered).unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_track_pattern(0, Some(pattern)).unwrap();
    controller.start().unwrap();
    let want = offline::render(&mut engine, 2 * engine::bar_frames(120.0, RATE) as usize, 2);
    assert_eq!(song.render(RATE, 2).unwrap(), want);
}

#[test]
fn renders_are_reused_between_loads_and_tracks() {
    let mut host = ScriptHost::new(RATE);
    let source = format!(
        "d:set(1, {{ smear = 0.5 }})\n\
         track(\"a\", d):steps(\"{STEPS}\")\n\
         track(\"b\", d:slice_at(1)):steps(\"x . . .  x . . .  x . . .  x . . .\")"
    );
    let first = load(&mut host, &source).unwrap();
    let a = first.tracks[0].rendered[0].1.clone();
    // La otra pista, con el mismo corte y lo mismo, el mismo buffer.
    assert!(Arc::ptr_eq(&a, &first.tracks[1].rendered[0].1));
    // Recargar sin cambiar nada no lo vuelve a calcular.
    let again = load(&mut host, &source).unwrap();
    assert!(Arc::ptr_eq(&a, &again.tracks[0].rendered[0].1));
    // Con otro valor, otro buffer.
    let other = load(&mut host, &source.replace("0.5", "0.6")).unwrap();
    assert!(!Arc::ptr_eq(&a, &other.tracks[0].rendered[0].1));
}

#[test]
fn braces_and_formant() {
    let song = song(
        "d:set(2, { formant = 4 })\n\
         track(\"a\", d):steps(\"1{smear=0.5} . 1{warp=0.4}{formant=-3} .  2 . 2%0 .  . . . .  . . . .\")",
    )
    .unwrap();
    let bar = &song.tracks[0].bars[0];
    assert_eq!(bar[0].unwrap().hit.smear, Some(0.5));
    assert_eq!(bar[2].unwrap().hit.warp, Some(Warp::midpoint(0.4).unwrap()));
    assert_eq!(bar[2].unwrap().hit.formant, Some(-3.0));
    assert_eq!(song.tracks[0].shapes[1].formant, Some(4.0));
    assert_eq!(song.tracks[0].rendered.len(), 2);
    assert!(song.render(RATE, 1).is_ok());
    // Con curva por golpe.
    let song2 = song_ok("d:set(1, { formant = ramp(-5, 5, \"1 compás\") })");
    assert!(song2.tracks.is_empty());
}

fn song_ok(source: &str) -> Song {
    song(source).unwrap()
}

#[test]
fn errors_point_at_the_line() {
    let error = |source: &str| song(source).err().expect("debería fallar");
    assert!(error("d:set(1, { warp = 1.2 })").contains("song.lua:3: set(1): los puntos de warp"));
    assert!(
        error("d:set(1, { warp = { {0.5, 0.6}, {0.4, 0.7} } })")
            .contains("los puntos de warp van en orden")
    );
    assert!(error("d:set(1, { warp = \"rápido\" })").contains("warp es un número"));
    assert!(error("d:set(1, { smear = 2 })").contains("smear fuera de rango"));
    assert!(error("d:set(1, { formant = 30 })").contains("formant"));
    let e = error("d:set(1, { formant = 2 })\ntrack(\"a\", d):length(1):steps(\"1^3\")");
    assert!(e.contains("formant del corte va con % o sin pitch"), "{e}");
    let e = error("track(\"a\", d):length(1):steps(\"1{warp=0}\")");
    assert!(e.contains("los puntos de warp"), "{e}");
    // Si falla, lo que sonaba sigue: el modelo del host es el de antes,
    // con su buffer.
    let mut host = ScriptHost::new(RATE);
    let good = load(
        &mut host,
        "d:set(1, { smear = 0.3 })\ntrack(\"a\", d):length(1):steps(\"1\")",
    )
    .unwrap();
    assert!(
        load(
            &mut host,
            "d:set(1, { smear = 3 })\ntrack(\"a\", d):length(1):steps(\"1\")"
        )
        .is_err()
    );
    let kept = host.song().expect("el modelo de antes");
    assert_eq!(kept.tracks[0].rendered.len(), 1);
    assert!(Arc::ptr_eq(
        &kept.tracks[0].rendered[0].1,
        &good.tracks[0].rendered[0].1
    ));
}
