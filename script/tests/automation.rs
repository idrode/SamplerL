//! `ramp()`, `lfo()` y `~` en Lua: el script suena igual que el motor con
//! la misma automatización, recargar no la desplaza y los errores dicen la
//! línea.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{Automation, Curve, Fx, LfoRate, Pattern, Sample, Slices, Step, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;
const STEPS: &str = "1 . 2 .  1 . 2 2  . 1 . 2  1 . . 2";

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("automation");
        std::fs::create_dir_all(&dir).unwrap();
        // Dos cortes de ruido (determinista): el filtro se nota.
        let mut x = 1u32;
        let mut noise = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (x as f32 / u32::MAX as f32 - 0.5) * 0.6
        };
        let mut mono = vec![0.0f32; 12_000];
        for s in &mut mono[..5000] {
            *s = noise();
        }
        for s in &mut mono[6000..11_000] {
            *s = 0.5 * noise();
        }
        offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

const HEAD: &str = "bpm(120)\nlocal b = sample(\"break.wav\"):slice(2)\n";

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(
        &mut controller,
        &format!("{HEAD}{source}"),
        "song.lua",
        &dir(),
    )
    .cloned()
    .map_err(|e| e.to_string())
}

fn engine_render(steps: &str, fx: &[Fx], automation: &[Automation], bars: usize) -> Vec<f32> {
    let sample = Arc::new(Sample::load_wav(dir().join("break.wav"), RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    let steps = engine::notation::parse_steps(steps).unwrap();
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_fx(fx)
        .unwrap()
        .with_automation(automation)
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    offline::render(
        &mut engine,
        bars * engine::bar_frames(120.0, RATE) as usize,
        2,
    )
}

#[test]
fn ramps_and_lfos_sound_like_the_engine() {
    let song = song(&format!(
        "track(\"d\", b):steps(\"{STEPS}\")\n  \
         :fx(\"lowpass\", {{ cutoff = ramp(200, 8000, \"2 compases\"), q = lfo(\"1 compás\", 0.7, 4) }})\n  \
         :fx(\"distortion\", {{ drive = 3, mix = lfo(1.5, 0, 0.8) }})"
    ))
    .unwrap();
    let automation = [
        Automation {
            fx: 0,
            param: 0,
            curve: Curve::Ramp {
                from: 200.0,
                to: 8000.0,
                steps: 32.0,
            },
        },
        Automation {
            fx: 0,
            param: 1,
            curve: Curve::Lfo {
                min: 0.7,
                max: 4.0,
                rate: LfoRate::Steps(16.0),
            },
        },
        Automation {
            fx: 1,
            param: 1,
            curve: Curve::Lfo {
                min: 0.0,
                max: 0.8,
                rate: LfoRate::Hz(1.5),
            },
        },
    ];
    assert_eq!(song.tracks[0].automation, automation);
    // El valor fijo de un parámetro con curva es su primer extremo.
    let fx = [
        Fx::Filter {
            kind: engine::FilterKind::LowPass,
            cutoff_hz: 200.0,
            q: 0.7,
        },
        Fx::distortion(3.0, 0.0),
    ];
    assert_eq!(song.tracks[0].fx, fx);
    assert_eq!(
        song.render(RATE, 4).unwrap(),
        engine_render(STEPS, &fx, &automation, 4)
    );
}

#[test]
fn per_step_cutoffs_sound_like_the_engine() {
    let steps = "1~300 . 2 .  1 . 2~6000 2  . 1~800 . 2  1 . . 2";
    let song = song(&format!(
        "track(\"d\", b):steps(\"{steps}\"):fx(\"highpass\", {{ cutoff = 40 }})\n  \
         :fx(\"lowpass\", {{ cutoff = 3000 }})"
    ))
    .unwrap();
    assert_eq!(song.tracks[0].steps[0], Some(Step::new(0).with_lock(300.0)));
    let fx = [Fx::highpass(40.0), Fx::lowpass(3000.0)];
    assert_eq!(
        song.render(RATE, 2).unwrap(),
        engine_render(steps, &fx, &[], 2)
    );
}

#[test]
fn reloading_does_not_shift_the_automation() {
    let source = format!(
        "{HEAD}track(\"d\", b):steps(\"{STEPS}\")\n  \
         :fx(\"lowpass\", {{ cutoff = ramp(300, 6000, \"3 compases\") }})"
    );
    let bar = engine::bar_frames(120.0, RATE) as usize;
    let render = |reload: bool| {
        let (mut controller, mut engine) = new_engine(RATE);
        let mut host = ScriptHost::new(RATE);
        host.load(&mut controller, &source, "song.lua", &dir())
            .unwrap();
        controller.start().unwrap();
        let mut audio = offline::render(&mut engine, 2 * bar + 1000, 2);
        if reload {
            // El mismo script otra vez a mitad: entra en el compás 3.
            host.load(&mut controller, &source, "song.lua", &dir())
                .unwrap();
        }
        audio.extend(offline::render(&mut engine, 3 * bar, 2));
        audio
    };
    assert_eq!(render(true), render(false));
}

#[test]
fn automation_errors_point_at_the_line() {
    let error = |source: &str| song(source).err().expect("debería fallar");
    assert_eq!(
        error("track(\"d\", b):fx(\"bitcrush\", { bits = ramp(4, 8, \"1 compás\") })"),
        "song.lua:3: fx(\"bitcrush\"): bits no se puede automatizar (este efecto no tiene parámetros automatizables)"
    );
    assert!(
        error("track(\"d\", b):fx(\"lowpass\", { cutoff = ramp(100, 30000, \"1 compás\") })")
            .contains("cutoff: la automatización se sale del rango del parámetro")
    );
    assert!(
        error("track(\"d\", b):fx(\"lowpass\", { cutoff = ramp(100, 800, \"un rato\") })")
            .contains("la duración es \"N compases\" o \"N pasos\"")
    );
    assert!(error("local x = lfo(100, 0, 1)").contains("de 0.01 a 50 Hz"));
    assert!(error("local x = lfo(\"mucho\", 0, 1)").contains("la frecuencia es un número"));
    assert_eq!(
        error("track(\"d\", b):fx(\"lowpass\", { cutoff = \"alto\" })"),
        "song.lua:3: fx(\"lowpass\"): cutoff tiene que ser un número, ramp() o lfo()"
    );
    assert_eq!(
        error("track(\"d\", b):steps(\"1~5 . . .  . . . .  . . . .  . . . .\")"),
        "song.lua:3: steps: corte (~) fuera de rango (20 a 20000 Hz)"
    );
    assert!(
        error("track(\"d\", b):steps(\"1~500 . . .  . . . .  . . . .  . . . .\")")
            .contains("~ (corte por paso) necesita un filtro en la pista")
    );
    assert!(
        error("track(\"d\", b):steps(\"1~ . . .  . . . .  . . . .  . . . .\")")
            .contains("falta el corte en Hz después de ~")
    );
}

#[test]
fn only_float_parameters_can_move() {
    let error = song("track(\"d\", b):fx(\"comp\", { ratio = lfo(1, 2, 8) })")
        .err()
        .unwrap();
    assert_eq!(
        error,
        "song.lua:3: fx(\"comp\"): ratio no se puede automatizar (sí: threshold, makeup)"
    );
}
