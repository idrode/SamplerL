//! Sidechain entre pistas (`Fx::Duck`): por patrón, cada golpe del origen
//! baja el destino `depth` dB en `attack` y lo devuelve exacto en
//! `release`, con su curva; por audio, un seno de nivel conocido dispara el
//! detector en su umbral, con su ratio, su rodilla y su filtro; la banda
//! solo baja los graves; el mute del origen se comporta según `tap`; sin
//! NaN ni denormales; y los cambios en vivo no hacen clic.

use std::sync::Arc;

use engine::{
    DuckCurve, DuckMode, DuckParams, DuckTap, Fx, Pattern, Sample, Slices, Step, new_engine,
    offline,
};

const RATE: u32 = 48_000;
const BPM: f64 = 120.0;
/// A 120 BPM, un paso son 6000 frames y un compás 96000.
const STEP: usize = 6000;
const BAR: usize = 16 * STEP;

/// Un sample hecho de `f(i)` durante `frames`, con fundidos de 5 ms.
fn sample_of(frames: usize, f: impl Fn(usize) -> f32) -> (Arc<Sample>, Slices) {
    let fade = 240.0;
    let mono: Vec<f32> = (0..frames)
        .map(|i| (i as f32 / fade).min(1.0).min((frames - i) as f32 / fade) * f(i))
        .collect();
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    (sample, slices)
}

fn sine(freq: f32, amp: f32) -> impl Fn(usize) -> f32 {
    move |i| amp * (std::f32::consts::TAU * freq * i as f32 / RATE as f32).sin()
}

fn steps(hits: &[usize]) -> Vec<Option<Step>> {
    (0..16)
        .map(|k| hits.contains(&k).then(|| Step::new(0)))
        .collect()
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

/// El destino: un seno de 1 kHz el compás entero (más largo: sin fundido
/// al final del compás), con `chain`.
fn dest(chain: &[Fx]) -> Pattern {
    let (sample, slices) = sample_of(BAR + STEP, sine(1000.0, 0.5));
    Pattern::from_steps(sample, &slices, steps(&[0]))
        .unwrap()
        .with_fx(chain)
        .unwrap()
}

/// Un origen mudo con golpes en `hits` (solo cuentan sus golpes).
fn silent_kick(hits: &[usize]) -> Pattern {
    let (sample, slices) = sample_of(STEP, |_| 0.0);
    Pattern::from_steps(sample, &slices, steps(hits)).unwrap()
}

/// Renderiza un compás (el segundo, si `second`) con los patrones en sus
/// ranuras y las pistas `muted` en mute.
fn render(patterns: Vec<(usize, Pattern)>, muted: &[usize], bars: usize) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(BPM).unwrap();
    for (slot, pattern) in patterns {
        controller.set_track_pattern(slot, Some(pattern)).unwrap();
    }
    for &t in muted {
        controller.set_mute(t, true).unwrap();
    }
    controller.start().unwrap();
    left(&offline::render(&mut engine, bars * BAR, 2))
}

/// La ganancia en cada frame: lo de `out` contra lo de `plain` (en los
/// frames donde `plain` no está cerca del cero).
fn gain_at(out: &[f32], plain: &[f32], i: usize) -> f32 {
    // El pico de la ventana de un periodo (48 frames a 1 kHz) alrededor.
    let peak = |x: &[f32]| x[i - 24..i + 24].iter().fold(0.0f32, |a, b| a.max(b.abs()));
    peak(out) / peak(plain)
}

fn db(g: f32) -> f32 {
    20.0 * g.log10()
}

fn pattern_duck(curve: DuckCurve) -> Fx {
    Fx::Duck(DuckParams {
        depth_db: -12.0,
        attack_ms: 0.0,
        release_ms: 100.0,
        curve,
        ..DuckParams::new(1)
    })
}

#[test]
fn each_hit_ducks_by_depth_and_comes_back_exactly_after_the_release() {
    let plain = render(vec![(0, dest(&[])), (1, silent_kick(&[4, 12]))], &[], 1);
    let out = render(
        vec![
            (0, dest(&[pattern_duck(DuckCurve::Linear)])),
            (1, silent_kick(&[4, 12])),
        ],
        &[],
        1,
    );
    let hit = 4 * STEP;
    let release = 4800; // 100 ms
    // Antes del golpe, idéntico bit a bit.
    assert_eq!(out[STEP..hit], plain[STEP..hit]);
    // Justo tras el golpe, −12 dB (sin ataque); a media relajación, −6.
    let g = db(gain_at(&out, &plain, hit + 30));
    assert!((g + 12.0).abs() < 0.3, "{g}");
    let g = db(gain_at(&out, &plain, hit + release / 2));
    assert!((g + 6.0).abs() < 0.3, "{g}");
    // Acabada la relajación, la pista vuelve exacta (bit a bit).
    assert_eq!(
        out[hit + release + 1..12 * STEP],
        plain[hit + release + 1..12 * STEP]
    );
    // Y baja otra vez con el siguiente golpe.
    let g = db(gain_at(&out, &plain, 12 * STEP + 30));
    assert!((g + 12.0).abs() < 0.3, "{g}");
}

#[test]
fn the_attack_and_the_curves_shape_the_envelope() {
    let plain = render(vec![(0, dest(&[])), (1, silent_kick(&[4]))], &[], 1);
    let hit = 4 * STEP;
    // Ataque de 10 ms: a la mitad, la mitad de la profundidad (en dB).
    let attack = Fx::Duck(DuckParams {
        depth_db: -12.0,
        attack_ms: 10.0,
        release_ms: 100.0,
        ..DuckParams::new(1)
    });
    let out = render(vec![(0, dest(&[attack])), (1, silent_kick(&[4]))], &[], 1);
    let g = db(gain_at(&out, &plain, hit + 240));
    assert!((g + 6.0).abs() < 0.6, "{g}");
    let g = db(gain_at(&out, &plain, hit + 480 + 24));
    assert!((g + 12.0).abs() < 0.4, "{g}");
    // A media relajación: lineal y suave, −6 dB; exponencial, casi arriba.
    let half = |curve| {
        let out = render(
            vec![(0, dest(&[pattern_duck(curve)])), (1, silent_kick(&[4]))],
            &[],
            1,
        );
        db(gain_at(&out, &plain, hit + 2400))
    };
    assert!((half(DuckCurve::Smooth) + 6.0).abs() < 0.4);
    let exp = half(DuckCurve::Exp);
    // (e^−2,5 − e^−5) / (1 − e^−5) de 12 dB: 0,91 dB.
    assert!((exp + 0.91).abs() < 0.3, "{exp}");
    // Al primer cuarto, la suave aún casi abajo y la exponencial ya a
    // menos de la mitad.
    let quarter = |curve| {
        let out = render(
            vec![(0, dest(&[pattern_duck(curve)])), (1, silent_kick(&[4]))],
            &[],
            1,
        );
        db(gain_at(&out, &plain, hit + 1200))
    };
    assert!(quarter(DuckCurve::Smooth) < -10.0);
    assert!(quarter(DuckCurve::Exp) > -4.0);
}

#[test]
fn a_muted_source_still_ducks_before_its_fader_but_not_after() {
    let kick = || {
        let (sample, slices) = sample_of(STEP, sine(60.0, 0.8));
        Pattern::from_steps(sample, &slices, steps(&[4])).unwrap()
    };
    let hit = 4 * STEP;
    let plain = render(vec![(0, dest(&[])), (1, kick())], &[1], 1);
    // Por patrón, antes del volumen (`pre`, por defecto): en mute baja.
    let pre = render(
        vec![(0, dest(&[pattern_duck(DuckCurve::Linear)])), (1, kick())],
        &[1],
        1,
    );
    assert!(db(gain_at(&pre, &plain, hit + 30)) < -11.0);
    // Después (`post`): en mute, nada; bit a bit lo de sin sidechain.
    let post = |mode: DuckMode| {
        Fx::Duck(DuckParams {
            mode,
            tap: DuckTap::Post,
            threshold_db: -40.0,
            attack_ms: 0.0,
            ..DuckParams::new(1)
        })
    };
    for mode in [DuckMode::Pattern, DuckMode::Audio] {
        let out = render(vec![(0, dest(&[post(mode)])), (1, kick())], &[1], 1);
        assert_eq!(out, plain, "{mode:?}");
    }
    // Sin mute, `post` baja (por audio, oye lo que suena).
    let heard = render(vec![(0, dest(&[])), (1, kick())], &[], 1);
    let out = render(
        vec![(0, dest(&[post(DuckMode::Audio)])), (1, kick())],
        &[],
        1,
    );
    assert_ne!(out[hit..hit + STEP], heard[hit..hit + STEP]);
}

/// El destino con un sidechain por audio de un origen (mudo) que es un
/// seno de `freq` y amplitud `amp` el compás entero; la reducción medida
/// en el centro del compás, en dB.
fn audio_reduction(freq: f32, amp: f32, duck: DuckParams) -> f32 {
    let source = || {
        let (sample, slices) = sample_of(BAR + STEP, sine(freq, amp));
        Pattern::from_steps(sample, &slices, steps(&[0])).unwrap()
    };
    let plain = render(vec![(0, dest(&[])), (1, source())], &[1], 1);
    let out = render(
        vec![(0, dest(&[Fx::Duck(duck.audio())])), (1, source())],
        &[1],
        1,
    );
    // La media de la ganancia en medio compás (la del seno bajo ondula un
    // poco con la ventana de 10 ms).
    let from = BAR / 4;
    let gains: Vec<f32> = (from..from + BAR / 2)
        .step_by(48)
        .map(|i| db(gain_at(&out, &plain, i)))
        .collect();
    gains.iter().sum::<f32>() / gains.len() as f32
}

#[test]
fn a_low_sine_of_known_level_triggers_the_detector_at_its_threshold() {
    let base = DuckParams {
        depth_db: -24.0,
        knee_db: 0.0,
        ratio: 4.0,
        ..DuckParams::new(1)
    };
    // 0,25 de amplitud son −12 dB: 12 dB sobre −24, ×(1 − 1/4) = 9 dB.
    let r = audio_reduction(
        60.0,
        0.25,
        DuckParams {
            threshold_db: -24.0,
            ..base
        },
    );
    assert!((r + 9.0).abs() < 0.6, "{r}");
    // En el umbral, sin rodilla: nada (menos de medio dB).
    let r = audio_reduction(
        60.0,
        0.25,
        DuckParams {
            threshold_db: -12.0,
            ..base
        },
    );
    assert!(r > -0.5, "{r}");
    // En el umbral con rodilla de 6 dB: 0,75 · 3² / 12 = 0,56 dB.
    let r = audio_reduction(
        60.0,
        0.25,
        DuckParams {
            threshold_db: -12.0,
            knee_db: 6.0,
            ..base
        },
    );
    assert!((r + 0.56).abs() < 0.3, "{r}");
    // Como mucho la profundidad.
    let r = audio_reduction(
        60.0,
        0.25,
        DuckParams {
            threshold_db: -60.0,
            depth_db: -6.0,
            ..base
        },
    );
    assert!((r + 6.0).abs() < 0.3, "{r}");
    // Con el paso bajo del detector a 150 Hz, un seno de 2 kHz casi no
    // cuenta; sin él, igual que el grave.
    let r = audio_reduction(
        2000.0,
        0.25,
        DuckParams {
            threshold_db: -24.0,
            ..base
        },
    );
    assert!((r + 9.0).abs() < 0.6, "{r}");
    let r = audio_reduction(
        2000.0,
        0.25,
        DuckParams {
            threshold_db: -24.0,
            filter_hz: 150.0,
            ..base
        },
    );
    assert!(r > -0.5, "{r}");
}

/// La amplitud del componente de `freq` en `x` (correlación con seno y
/// coseno, en periodos enteros).
fn amplitude(x: &[f32], freq: f32) -> f32 {
    let (mut s, mut c) = (0.0f64, 0.0f64);
    for (i, v) in x.iter().enumerate() {
        let w = std::f64::consts::TAU * freq as f64 * i as f64 / RATE as f64;
        s += *v as f64 * w.sin();
        c += *v as f64 * w.cos();
    }
    (2.0 * (s * s + c * c).sqrt() / x.len() as f64) as f32
}

#[test]
fn with_a_band_only_the_lows_go_down() {
    let mixed = |chain: &[Fx]| {
        let (sample, slices) = sample_of(BAR + STEP, |i| sine(60.0, 0.4)(i) + sine(2000.0, 0.2)(i));
        Pattern::from_steps(sample, &slices, steps(&[0]))
            .unwrap()
            .with_fx(chain)
            .unwrap()
    };
    let duck = Fx::Duck(DuckParams {
        depth_db: -12.0,
        attack_ms: 0.0,
        release_ms: 5000.0,
        curve: DuckCurve::Smooth,
        band_hz: 200.0,
        ..DuckParams::new(1)
    });
    let plain = render(vec![(0, mixed(&[])), (1, silent_kick(&[4]))], &[], 1);
    let out = render(vec![(0, mixed(&[duck])), (1, silent_kick(&[4]))], &[], 1);
    // 100 ms tras el golpe (la curva suave aún está abajo del todo), 0,1 s
    // de ventana.
    let w = 4 * STEP + 4800..4 * STEP + 9600;
    let low = db(amplitude(&out[w.clone()], 60.0) / amplitude(&plain[w.clone()], 60.0));
    let high = db(amplitude(&out[w.clone()], 2000.0) / amplitude(&plain[w.clone()], 2000.0));
    assert!((low + 12.0).abs() < 1.0, "graves {low}");
    assert!(high.abs() < 0.3, "agudos {high}");
    // Antes del golpe, el cruce solo gira la fase: el nivel de los dos, igual.
    let w = STEP..3 * STEP;
    for f in [60.0, 2000.0] {
        let g = db(amplitude(&out[w.clone()], f) / amplitude(&plain[w.clone()], f));
        assert!(g.abs() < 0.2, "{f} Hz {g}");
    }
}

#[test]
fn silence_and_extreme_keys_leave_no_nan_and_settle_to_zero() {
    // Un origen enorme (1e6) por audio, un paso al principio de cada
    // compás, y luego silencio: la salida finita, y tras la relajación el
    // destino vuelve exacto.
    let huge = || {
        let (sample, slices) = sample_of(STEP, |_| 1.0e6);
        Pattern::from_steps(sample, &slices, steps(&[0])).unwrap()
    };
    let duck = Fx::Duck(DuckParams {
        threshold_db: -60.0,
        release_ms: 50.0,
        ..DuckParams::new(1).audio()
    });
    let plain = render(vec![(0, dest(&[]))], &[], 2);
    let out = render(vec![(0, dest(&[duck])), (1, huge())], &[1], 2);
    assert!(out.iter().all(|x| x.is_finite()));
    // Del paso 8 al final de cada compás, bit a bit: el origen calla desde
    // el paso 1; el detector (RMS de 10 ms) tarda unos 400 ms en bajar
    // 180 dB y la reducción, unas 7 constantes de 50 ms en llegar a medio
    // centésimo de dB (y de ahí, a cero).
    for bar in 0..2 {
        let w = bar * BAR + 8 * STEP..(bar + 1) * BAR;
        assert_eq!(out[w.clone()], plain[w], "compás {bar}");
    }
    // La reducción publicada: cero exacto en el silencio.
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(BPM).unwrap();
    controller
        .set_track_pattern(0, Some(dest(&[duck])))
        .unwrap();
    controller.set_track_pattern(1, Some(huge())).unwrap();
    controller.set_mute(1, true).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, 2 * STEP, 2);
    // Como mucho la profundidad (−9 dB por defecto).
    let r = controller.status().tracks[0].reduction[0];
    assert!(r > 8.0 && r <= 9.0, "{r}");
    offline::render(&mut engine, 8 * STEP, 2);
    let _ = controller.status();
    offline::render(&mut engine, STEP, 2);
    assert_eq!(controller.status().tracks[0].reduction[0], 0.0);
}

#[test]
fn a_new_depth_at_the_bar_comes_in_without_a_click() {
    // El destino suena dos compases; en el segundo, otra profundidad (al
    // compás), justo con un golpe en el paso 0 de los dos.
    let duck = |depth: f32| {
        Fx::Duck(DuckParams {
            depth_db: depth,
            attack_ms: 5.0,
            release_ms: 2000.0,
            ..DuckParams::new(1)
        })
    };
    let (sample, slices) = sample_of(3 * BAR, sine(100.0, 0.5));
    let long = |chain: &[Fx]| {
        // El mismo sample en los dos: la voz sigue al cambiar de patrón.
        let (sample, slices) = (sample.clone(), slices.clone());
        // Un solo golpe en 32 pasos: el seno no vuelve a empezar.
        let once: Vec<Option<Step>> = (0..32).map(|k| (k == 0).then(|| Step::new(0))).collect();
        Pattern::from_steps(sample, &slices, once)
            .unwrap()
            .with_fx(chain)
            .unwrap()
    };
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(BPM).unwrap();
    controller
        .set_track_pattern(0, Some(long(&[duck(-6.0)])))
        .unwrap();
    controller
        .set_track_pattern(1, Some(silent_kick(&[6])))
        .unwrap();
    controller.start().unwrap();
    let mut out = left(&offline::render(&mut engine, BAR - 3000, 2));
    // A media relajación del golpe del paso 6, otra profundidad.
    controller
        .set_track_pattern(0, Some(long(&[duck(-18.0)])))
        .unwrap();
    out.extend(left(&offline::render(&mut engine, BAR + 3000, 2)));
    // El mayor salto entre muestras: el del propio seno (a 100 Hz y 0,5,
    // como mucho 0,0065) con algo de margen; un escalón de ganancia en
    // un pico sería mucho más.
    let (at, worst) = out
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .enumerate()
        .fold((0, 0.0f32), |a, (i, d)| if d > a.1 { (i, d) } else { a });
    assert!(worst < 0.01, "{worst} en {at}");
}

/// Coste del sidechain: el tiempo de `Engine::process` de 60 s de audio
/// con un destino que suena todo el rato y un origen con un golpe por
/// negra, sin sidechain, por patrón, por audio y por audio con banda.
/// `cargo test --release -p engine --test duck coste -- --ignored --nocapture`
#[test]
#[ignore = "banco: cargo test --release -p engine --test duck coste -- --ignored --nocapture"]
fn coste() {
    let kick = || {
        let (sample, slices) = sample_of(STEP, sine(60.0, 0.8));
        Pattern::from_steps(sample, &slices, steps(&[0, 4, 8, 12])).unwrap()
    };
    let cases: [(&str, Vec<Fx>); 5] = [
        ("sin sidechain", vec![]),
        ("por patrón", vec![pattern_duck(DuckCurve::Linear)]),
        (
            "por patrón, banda",
            vec![Fx::Duck(DuckParams {
                band_hz: 150.0,
                ..DuckParams::new(1)
            })],
        ),
        (
            "por audio",
            vec![Fx::Duck(DuckParams {
                filter_hz: 150.0,
                ..DuckParams::new(1).audio()
            })],
        ),
        (
            "por audio, banda",
            vec![Fx::Duck(DuckParams {
                filter_hz: 150.0,
                band_hz: 150.0,
                ..DuckParams::new(1).audio()
            })],
        ),
    ];
    let seconds = 60;
    let mut base = 0.0;
    for (name, chain) in cases {
        let (mut controller, mut engine) = new_engine(RATE);
        controller.set_bpm(BPM).unwrap();
        controller.set_track_pattern(0, Some(dest(&chain))).unwrap();
        controller.set_track_pattern(1, Some(kick())).unwrap();
        controller.start().unwrap();
        let mut out = vec![0.0f32; 2 * 256];
        let blocks = seconds * RATE as usize / 256;
        let start = std::time::Instant::now();
        for _ in 0..blocks {
            engine.process(&mut out, 2);
        }
        let took = start.elapsed().as_secs_f64();
        let ms_per_s = 1000.0 * took / seconds as f64;
        if base == 0.0 {
            base = ms_per_s;
        }
        println!(
            "{name:<20} {ms_per_s:6.3} ms por segundo de audio ({:+.3} ms, {:.3} % de un núcleo)",
            ms_per_s - base,
            (ms_per_s - base) / 10.0
        );
    }
}
