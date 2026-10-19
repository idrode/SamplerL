//! `:fx("eq", {...})` suena igual que `Fx::Eq` en el motor, con sus valores
//! por defecto, `ramp`/`lfo` en sus parámetros y errores que señalan la
//! línea.

use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use engine::{EqBand, EqKind, Fx, Pattern, Sample, Slices, new_engine, offline};
use script::ScriptHost;

const RATE: u32 = 48_000;
const STEPS: &str = "1 . 2 .  1 . 2 2  . 1 . 2  1 . . 2";

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("eq");
        std::fs::create_dir_all(&dir).unwrap();
        // Dos golpes de ruido rosa a ojo (suma de senos): tienen de todo.
        let mut mono = vec![0.0f32; 6000];
        for (i, x) in mono.iter_mut().enumerate() {
            let t = i as f32 / RATE as f32;
            let env = if i < 1500 || (3000..4500).contains(&i) {
                0.2
            } else {
                0.0
            };
            *x = env
                * [80.0, 310.0, 1200.0, 4700.0, 9000.0]
                    .iter()
                    .map(|f| (std::f32::consts::TAU * f * t).sin())
                    .sum::<f32>();
        }
        offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn script(fx: &str) -> Result<Vec<f32>, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!(
        "bpm(120)\nlocal b = sample(\"break.wav\"):slice(2)\ntrack(\"d\", b):steps(\"{STEPS}\"){fx}"
    );
    host.load(&mut controller, &source, "song.lua", &dir())
        .map(|s| s.render(RATE, 2).unwrap())
        .map_err(|e| e.to_string())
}

fn engine_with(chain: &[Fx]) -> Vec<f32> {
    let sample = Arc::new(Sample::load_wav(dir().join("break.wav"), RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    let steps = engine::notation::parse_steps(STEPS).unwrap();
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_fx(chain)
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, 2 * engine::bar_frames(120.0, RATE) as usize, 2)
}

#[test]
fn the_eq_sounds_like_the_engine() {
    let a = script(
        ":fx(\"eq\", { freq1 = 310, gain1 = -4, q1 = 3, \
         type2 = \"highshelf\", freq2 = 6000, gain2 = 3, q2 = 0.707 })",
    )
    .unwrap();
    let b = engine_with(&[Fx::eq(&[
        EqBand::bell(310.0, -4.0, 3.0),
        EqBand {
            kind: EqKind::HighShelf,
            freq_hz: 6000.0,
            gain_db: 3.0,
            q: 0.707,
        },
    ])]);
    assert_eq!(a, b);
    assert_ne!(a, engine_with(&[]));
    // Sin parámetros: plano, idéntico a no tenerlo.
    assert_eq!(script(":fx(\"eq\")").unwrap(), engine_with(&[]));
    // Valores por defecto de cada banda: su frecuencia, q 1, campana.
    assert_eq!(
        script(":fx(\"eq\", { gain3 = 2 })").unwrap(),
        engine_with(&[Fx::eq(&[
            EqBand::flat(0),
            EqBand::flat(1),
            EqBand::bell(2000.0, 2.0, 1.0),
        ])])
    );
}

#[test]
fn its_bands_take_ramp_and_lfo() {
    let moving =
        script(":fx(\"eq\", { freq1 = ramp(200, 2000, \"2 compases\"), gain1 = lfo(1, -9, 0) })")
            .unwrap();
    let still = script(":fx(\"eq\", { freq1 = 200, gain1 = -9 })").unwrap();
    assert_ne!(moving, still);
    // Fuera de rango, el error de siempre.
    let e = script(":fx(\"eq\", { gain2 = lfo(1, -30, 0) })").unwrap_err();
    assert!(e.contains("gain") && e.contains("rango"), "{e}");
}

#[test]
fn its_errors_point_at_the_line() {
    let error = |fx: &str| script(fx).unwrap_err();
    assert_eq!(
        error(":fx(\"eq\", { gain1 = 40 })"),
        "song.lua:3: fx(\"eq\"): gain fuera de rango (-24 a 24 dB)"
    );
    assert!(error(":fx(\"eq\", { freq4 = 5 })").contains("freq fuera de rango"));
    assert!(error(":fx(\"eq\", { q2 = 0 })").contains("q fuera de rango"));
    let e = error(":fx(\"eq\", { type1 = \"notch\" })");
    assert!(e.contains("type1 \"notch\" no existe"), "{e}");
    let e = error(":fx(\"eq\", { gain5 = 2 })");
    assert!(e.contains("parámetro desconocido \"gain5\""), "{e}");
}
