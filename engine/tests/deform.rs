//! Deformaciones de un corte: `warp` y `smear` precalculados (la duración
//! es la del corte, el tono no cambia, el tiempo sigue la curva y el
//! mismo corte da siempre el mismo buffer), el cambio de buffer al compás
//! sin clics, y `formant` por corte en tiempo real (mueve el timbre y no la
//! fundamental).

use std::sync::Arc;

use engine::deform::render;
use engine::{
    HitShape, Pattern, RenderKey, Sample, SliceShape, Slices, Step, Warp, detect_mono, new_engine,
    offline,
};

const RATE: u32 = 48_000;

fn tone(parts: &[(f64, f64)]) -> Arc<Sample> {
    let mut mono = Vec::new();
    let mut phase = 0.0f64;
    for &(hz, seconds) in parts {
        for _ in 0..(seconds * RATE as f64) as usize {
            mono.push((0.5 * phase.sin()) as f32);
            phase += std::f64::consts::TAU * hz / RATE as f64;
        }
    }
    Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap())
}

fn left(sample: &Sample) -> Vec<f32> {
    (0..sample.frames()).map(|i| sample.frame(i).0).collect()
}

fn key(sample: &Sample, warp: Option<Warp>, smear: f32) -> RenderKey {
    RenderKey {
        start: 0,
        end: sample.frames(),
        warp,
        smear,
    }
}

#[test]
fn warp_keeps_the_duration_and_the_pitch_and_follows_the_curve() {
    // 300 Hz el primer medio segundo y 600 Hz el segundo.
    let sample = tone(&[(300.0, 0.5), (600.0, 0.5)]);
    let warp = Warp::midpoint(0.3).unwrap();
    let out = render(&sample, &key(&sample, Some(warp), 0.0));
    assert_eq!(out.frames(), sample.frames());
    // El cambio de nota (la mitad del original) suena donde la curva lee
    // 0.5: entre (0.5, 0.3) y (1, 1), en x = 0.5 + 0.2/0.7 · 0.5 = 0.643.
    assert!((warp.at(0.642_857) - 0.5).abs() < 1e-5);
    let l = left(&out);
    let at = |x: f64| (x * 48_000.0) as usize;
    let hz = |a: f64, b: f64| detect_mono(&l[at(a)..at(b)], RATE).unwrap().hz;
    // Antes del cambio, 300 Hz; después, 600 (el tono no cambia).
    for (a, b, want) in [
        (0.05, 0.45, 300.0),
        (0.47, 0.62, 300.0),
        (0.67, 0.95, 600.0),
    ] {
        let got = hz(a, b);
        assert!(
            (got / want - 1.0).abs() < 0.005,
            "de {a} a {b} s: {got} Hz, se esperaban {want}"
        );
    }
    // Sin warp no se notaría: en 0.55 s ya sonaría 600.
    let plain = left(&sample);
    let got = detect_mono(&plain[at(0.52)..at(0.62)], RATE).unwrap().hz;
    assert!((got / 600.0 - 1.0).abs() < 0.005);
}

#[test]
fn warp_points_and_their_checks() {
    let warp = Warp::new(&[(0.25, 0.1), (0.75, 0.5)]).unwrap();
    assert!((warp.at(0.25) - 0.1).abs() < 1e-6);
    assert!((warp.at(0.5) - 0.3).abs() < 1e-6);
    assert!((warp.at(1.0) - 1.0).abs() < 1e-6);
    assert!(Warp::new(&[]).is_err());
    assert!(Warp::new(&[(0.5, 0.0)]).is_err());
    assert!(Warp::new(&[(0.5, 0.3), (0.4, 0.6)]).is_err());
    assert!(Warp::new(&[(0.5, 0.3), (0.6, 0.2)]).is_err());
    assert!(Warp::new(&[(0.1, 0.1); 7]).is_err());
}

#[test]
fn smear_keeps_the_duration_and_is_the_same_every_time() {
    let sample = tone(&[(220.0, 0.4)]);
    let a = render(&sample, &key(&sample, None, 0.6));
    let b = render(&sample, &key(&sample, None, 0.6));
    assert_eq!(a.frames(), sample.frames());
    assert_eq!(left(&a), left(&b));
    // Cambia el sonido, no la nota: los granos leen a velocidad 1, pero
    // sumados con fases al azar mueven un poco el periodo que mide YIN
    // (medido: 222,9 Hz con smear = 0.6, un 1,3 %).
    assert_ne!(left(&a), left(&sample));
    let l = left(&a);
    let got = detect_mono(&l[4000..16_000], RATE).unwrap().hz;
    assert!((got / 220.0 - 1.0).abs() < 0.02, "{got} Hz");
    // Con el mismo nivel, más o menos (RMS igualado).
    let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
    let ratio = rms(&l[2000..17_000]) / rms(&left(&sample)[2000..17_000]);
    assert!((0.6..1.2).contains(&ratio), "nivel ×{ratio}");
}

/// Un golpe por compás de un corte con esa forma, durante `bars` compases;
/// con `then`, otra forma desde el segundo compás (entra al compás).
fn bars(
    sample: &Arc<Sample>,
    shape: SliceShape,
    then: Option<SliceShape>,
    bars: usize,
) -> Vec<f32> {
    let slices = Slices::grid(sample, 1);
    let pattern = |shape: SliceShape| {
        let mut steps = vec![None; 16];
        steps[0] = Some(Step::new(0));
        let p = Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_shapes(&[shape])
            .unwrap();
        let rendered = p
            .render_keys()
            .into_iter()
            .map(|k| (k, Arc::new(render(sample, &k))))
            .collect();
        p.with_rendered(rendered).unwrap()
    };
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern(shape)).unwrap();
    controller.start().unwrap();
    let bar = engine::bar_frames(120.0, RATE) as usize;
    let mut out = offline::render(&mut engine, bar / 2, 2);
    if let Some(then) = then {
        // A mitad del primer compás, con el golpe sonando.
        controller.set_pattern(pattern(then)).unwrap();
    }
    out.extend(offline::render(&mut engine, bar * bars - bar / 2, 2));
    out.iter().step_by(2).copied().collect()
}

#[test]
fn a_new_buffer_enters_at_the_bar_without_clicks() {
    // Un seno de 2 s, más largo que el compás: el golpe aún suena cuando
    // entra el siguiente.
    let sample = tone(&[(220.0, 2.0)]);
    let slow = SliceShape {
        warp: Some(Warp::midpoint(0.3).unwrap()),
        ..SliceShape::DEFAULT
    };
    let fast = SliceShape {
        warp: Some(Warp::midpoint(0.7).unwrap()),
        smear: 0.3,
        ..SliceShape::DEFAULT
    };
    let swapped = bars(&sample, slow, Some(fast), 2);
    let only_slow = bars(&sample, slow, None, 2);
    let only_fast = bars(&sample, fast, None, 2);
    let bar = engine::bar_frames(120.0, RATE) as usize;
    // El primer compás sigue siendo el de antes (el cambio espera al compás).
    assert_eq!(swapped[..bar], only_slow[..bar]);
    // El segundo es el nuevo (pasado el fundido del golpe que se corta).
    assert_eq!(swapped[bar + 200..], only_fast[bar + 200..]);
    // Y en la unión no salta más que el seno (más el ruido de los granos).
    let jump = swapped[bar - 500..bar + 500]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    let sine = 0.5 * std::f32::consts::TAU * 220.0 / RATE as f32;
    assert!(jump < sine * 2.0, "salto {jump} (el seno, {sine})");
}

#[test]
fn per_step_warp_and_smear_and_the_same_key_is_one_buffer() {
    let sample = tone(&[(220.0, 0.3)]);
    let slices = Slices::grid(&sample, 1);
    let mut steps = vec![None; 16];
    let smeared = Step::new(0).with_hit(HitShape {
        smear: Some(0.5),
        ..HitShape::default()
    });
    steps[0] = Some(smeared);
    steps[4] = Some(smeared);
    steps[8] = Some(Step::new(0));
    let p = Pattern::from_steps(sample.clone(), &slices, steps)
        .unwrap()
        .with_shapes(&[SliceShape::DEFAULT])
        .unwrap();
    let keys = p.render_keys();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].smear, 0.5);
    // Sin el buffer, error (el patrón anterior seguiría sonando).
    let error = Pattern::from_steps(sample.clone(), &slices, vec![Some(smeared); 16])
        .unwrap()
        .with_rendered(Vec::new())
        .err()
        .expect("falta el buffer");
    assert!(error.to_string().contains("falta un corte precalculado"));
}

/// Amplitud del armónico `k` de 150 Hz en `x`.
fn harmonic(x: &[f32], k: usize) -> f64 {
    let w = std::f64::consts::TAU * 150.0 * k as f64 / RATE as f64;
    let (mut re, mut im) = (0.0, 0.0);
    for (n, v) in x.iter().enumerate() {
        re += *v as f64 * (w * n as f64).cos();
        im += *v as f64 * (w * n as f64).sin();
    }
    (re * re + im * im).sqrt() / x.len() as f64
}

/// El armónico más fuerte (dónde está el formante).
fn peak(x: &[f32]) -> usize {
    (1..=30)
        .max_by(|&a, &b| harmonic(x, a).total_cmp(&harmonic(x, b)))
        .unwrap()
}

#[test]
fn slice_formant_moves_the_timbre_and_not_the_fundamental() {
    // 150 Hz con un formante en 900 Hz (el armónico 6).
    let mono: Vec<f32> = (0..48_000)
        .map(|n| {
            let t = n as f64 / RATE as f64;
            (1..=30)
                .map(|k| {
                    let f = 150.0 * k as f64;
                    let amp = (-((f - 900.0) / 250.0).powi(2)).exp();
                    0.1 * amp * (std::f64::consts::TAU * f * t).sin()
                })
                .sum::<f64>() as f32
        })
        .collect();
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    let play = |shape: SliceShape| -> Vec<f32> {
        let mut steps = vec![None; 64];
        steps[0] = Some(Step::new(0));
        let p = Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_shapes(&[shape])
            .unwrap();
        let (mut controller, mut engine) = new_engine(RATE);
        controller.set_bpm(120.0).unwrap();
        controller.set_pattern(p).unwrap();
        controller.start().unwrap();
        offline::render(&mut engine, 48_000, 2)
            .iter()
            .step_by(2)
            .copied()
            .collect()
    };
    let plain = play(SliceShape::DEFAULT);
    let moved = play(SliceShape {
        formant: Some(7.0),
        ..SliceShape::DEFAULT
    });
    let window = 9600..38_400;
    assert_eq!(peak(&plain[window.clone()]), 6);
    // Una quinta arriba: de 900 Hz a ~1350 (el armónico 9).
    let p = peak(&moved[window.clone()]);
    assert!((8..=10).contains(&p), "el formante está en el armónico {p}");
    let f0 = detect_mono(&moved[window.clone()], RATE).unwrap().hz;
    assert!((f0 / 150.0 - 1.0).abs() < 0.005, "fundamental {f0} Hz");
    // Con formant 0 y sin pitch, nada cambia.
    let zero = play(SliceShape {
        formant: Some(0.0),
        ..SliceShape::DEFAULT
    });
    assert_eq!(zero, plain);
    // Con ^, error (los formantes se mueven con el tono).
    let mut steps = vec![None; 16];
    steps[0] = Some(Step::new(0).with_pitch(3.0));
    assert!(
        Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_shapes(&[SliceShape {
                formant: Some(2.0),
                ..SliceShape::DEFAULT
            }])
            .is_err()
    );
}

/// Lo que tarda precalcular un corte de 1 s (`cargo test -p engine
/// --release --test deform -- --ignored --nocapture coste`): la carga lo
/// hace en su hilo, así que es lo que tarda de más al guardar.
#[test]
#[ignore]
fn coste_de_precalcular() {
    let sample = tone(&[(220.0, 0.5), (330.0, 0.5)]);
    for (name, warp, smear) in [
        ("warp", Some(Warp::midpoint(0.3).unwrap()), 0.0),
        ("smear", None, 0.5),
        ("warp y smear", Some(Warp::midpoint(0.3).unwrap()), 0.5),
    ] {
        let start = std::time::Instant::now();
        let out = render(&sample, &key(&sample, warp, smear));
        println!(
            "{name}: {:.1} ms por segundo de corte",
            start.elapsed().as_secs_f64() * 1000.0
        );
        assert_eq!(out.frames(), sample.frames());
    }
}
