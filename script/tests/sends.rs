//! `bus()` y `pista:send()`: el script suena igual que el motor con los
//! mismos envíos y buses, y los errores dicen la línea.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{
    Batch, Buses, DelayParams, Division, Pattern, ReverbParams, Sample, Slices, new_engine, offline,
};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("sends");
        std::fs::create_dir_all(&dir).unwrap();
        let mut mono = vec![0.0f32; 6000];
        mono[..800].fill(0.3);
        mono[3000..3800].fill(0.2);
        offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!("bpm(130)\nlocal b = sample(\"break.wav\"):slice(2)\n{source}");
    host.load(&mut controller, &source, "song.lua", &dir())
        .cloned()
        .map_err(|e| e.to_string())
}

const STEPS: &str = "1 . . .  2 . . .  . . 1 .  . . . .";

#[test]
fn the_script_sounds_like_the_engine_with_the_same_sends() {
    let script = song(&format!(
        "bus(\"delay\", {{ time = \"3/16\", feedback = 0.6, level = 0.9 }})\n\
         bus(\"reverb\", {{ size = 0.9 }})\n\
         track(\"d\", b):steps(\"{STEPS}\"):send(\"delay\", 0.5):send(\"reverb\", 0.25)"
    ))
    .unwrap();
    let buses = Buses {
        delay: DelayParams {
            division: Division::DottedEighth,
            feedback: 0.6,
            level: 0.9,
        },
        reverb: ReverbParams {
            size: 0.9,
            ..ReverbParams::default()
        },
        ..Buses::default()
    };
    assert_eq!(script.buses, buses);
    assert_eq!(script.tracks[0].sends, [0.5, 0.25]);

    let sample = Arc::new(Sample::load_wav(dir().join("break.wav"), RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    let steps = engine::notation::parse_steps(STEPS).unwrap();
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_sends([0.5, 0.25])
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_bpm(130.0)
        .set_buses(buses)
        .set_pattern(0, Some(pattern));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let frames = 4 * engine::bar_frames(130.0, RATE) as usize;
    assert_eq!(
        script.render(RATE, 4).unwrap(),
        offline::render(&mut engine, frames, 2)
    );
}

#[test]
fn without_bus_the_defaults_and_sends_change_the_sound() {
    let plain = song(&format!("track(\"d\", b):steps(\"{STEPS}\")")).unwrap();
    assert_eq!(plain.buses, Buses::default());
    let sent = song(&format!(
        "track(\"d\", b):steps(\"{STEPS}\"):send(\"delay\", 0.4)"
    ))
    .unwrap();
    let (a, b) = (
        plain.render(RATE, 2).unwrap(),
        sent.render(RATE, 2).unwrap(),
    );
    assert_ne!(a, b);
    // Antes del primer eco (una corchea), igual.
    let eighth = engine::bus::delay_frames(Division::Eighth, 130.0, RATE);
    assert_eq!(a[..2 * eighth], b[..2 * eighth]);
    // Envío 0: como si no hubiera.
    let zero = song(&format!(
        "track(\"d\", b):steps(\"{STEPS}\"):send(\"delay\", 0)"
    ))
    .unwrap();
    assert_eq!(zero.render(RATE, 2).unwrap(), a);
}

#[test]
fn bus_and_send_errors_point_at_the_line() {
    let error = |source: &str| song(source).err().expect("debería fallar");
    assert_eq!(
        error("bus(\"chorus\", {})"),
        "song.lua:3: bus(\"chorus\"): no existe (delay, reverb)"
    );
    assert_eq!(
        error("bus(\"delay\", { tiempo = \"1/8\" })"),
        "song.lua:3: bus(\"delay\"): parámetro desconocido \"tiempo\" (admite: time, feedback, level)"
    );
    assert!(error("bus(\"delay\", { time = \"1/3\" })").contains("time = \"1/3\" no existe"));
    assert!(error("bus(\"delay\", { feedback = 1 })").contains("feedback = 1 fuera de rango"));
    assert!(error("bus(\"reverb\", { size = 2 })").contains("size = 2 fuera de rango (0 a 1)"));
    assert_eq!(
        error("track(\"d\", b):send(\"delay\", 1.5)"),
        "song.lua:3: send(\"delay\", 1.5): el nivel va de 0 a 1"
    );
    assert!(error("track(\"d\", b):send(\"eco\", 0.5)").contains("el bus no existe"));
    assert!(
        error(&format!(
            "track(\"d\", b):steps(\"{STEPS}\"):every(2, function(c) bus(\"delay\", {{}}) end)"
        ))
        .contains("no se puede llamar a bus() dentro de every()")
    );
}
