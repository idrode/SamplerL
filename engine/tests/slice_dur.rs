//! Variantes de duración de un corte (`dur` y `fit`): la duración que
//! sale es la pedida, `trim` y `loop` no hacen clic y `stretch` conserva el
//! tono. Con `dur` sin poner, todo es lo de siempre (bit a bit).

use std::sync::Arc;

use engine::{
    Dur, Fit, HitShape, Pattern, PlayMode, STEPS, Sample, SliceShape, Slices, Step, detect_mono,
    new_engine, offline,
};

const RATE: u32 = 48_000;

/// Un seno de `hz` y `seconds` segundos, a media escala.
fn sine(hz: f32, seconds: f32) -> Arc<Sample> {
    let n = (seconds * RATE as f32) as usize;
    let mono: Vec<f32> = (0..n)
        .map(|i| 0.5 * (std::f32::consts::TAU * hz * i as f32 / RATE as f32).sin())
        .collect();
    Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap())
}

/// Un golpe en el paso 0 de `sample` (un corte, el sample entero) con esa
/// forma; `step` cambia el paso. Devuelve el canal izquierdo.
fn render_step(
    sample: Arc<Sample>,
    shape: SliceShape,
    step: Step,
    bpm: f64,
    frames: usize,
) -> Vec<f32> {
    let slices = Slices::grid(&sample, 1);
    // Un patrón de 64 pasos: el golpe no se repite dentro del render.
    let mut steps = vec![None; 64];
    steps[0] = Some(step);
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_shapes(&[shape])
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(bpm).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

fn render(sample: Arc<Sample>, shape: SliceShape, bpm: f64, frames: usize) -> Vec<f32> {
    render_step(sample, shape, Step::new(0), bpm, frames)
}

fn with(f: impl FnOnce(&mut SliceShape)) -> SliceShape {
    let mut s = SliceShape::DEFAULT;
    f(&mut s);
    s
}

/// Frames que suena (hasta el último que no es cero).
fn sounding(left: &[f32]) -> usize {
    left.iter().rposition(|x| *x != 0.0).map_or(0, |i| i + 1)
}

/// La mayor diferencia entre dos muestras seguidas en `range`.
fn max_jump(left: &[f32], range: std::ops::Range<usize>) -> f32 {
    left[range]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// La mayor diferencia entre dos muestras seguidas de un seno de `hz` a
/// media escala (lo que salta él solo).
fn sine_jump(hz: f32) -> f32 {
    0.5 * std::f32::consts::TAU * hz / RATE as f32
}

#[test]
fn trim_lasts_what_is_asked_in_ms_figures_and_bars() {
    let sample = sine(220.0, 4.0);
    // A 120 BPM un paso son 6000 frames.
    let cases = [
        (Dur::Ms(250.0), 120.0, 12_000),
        (Dur::Note { num: 1, den: 8 }, 120.0, 12_000),
        (Dur::Note { num: 3, den: 16 }, 120.0, 18_000),
        (Dur::Bars(1), 120.0, 96_000),
        // A 170 BPM un paso no es un número entero de frames (4235,29...):
        // la duración se redondea una vez, sin acumular.
        (Dur::Note { num: 1, den: 4 }, 170.0, 16_941),
        (Dur::Bars(1), 170.0, 67_765),
    ];
    for (dur, bpm, frames) in cases {
        let left = render(
            sample.clone(),
            with(|s| s.dur = Some(dur)),
            bpm,
            frames + 4000,
        );
        let got = sounding(&left);
        // El último frame de la voz es el final del fundido (cero).
        assert!(
            got.abs_diff(frames) <= 1,
            "{dur:?} a {bpm}: {got} frames, se pedían {frames}"
        );
        // La cola, sin clic: nunca salta más que el seno.
        assert!(max_jump(&left, frames - 200..frames + 10) <= sine_jump(220.0) * 1.05);
    }
}

#[test]
fn trim_longer_than_the_slice_ends_with_the_slice() {
    let sample = sine(220.0, 0.1);
    let left = render(
        sample,
        with(|s| s.dur = Some(Dur::Ms(500.0))),
        120.0,
        30_000,
    );
    assert!(sounding(&left).abs_diff(4800) <= 1);
}

#[test]
fn release_shapes_the_end_of_a_trim() {
    let sample = sine(220.0, 2.0);
    let left = render(
        sample,
        with(|s| {
            s.dur = Some(Dur::Ms(500.0));
            s.release_ms = 100.0;
        }),
        120.0,
        40_000,
    );
    // La cola dura 100 ms dentro de los 500: a mitad de ella, la mitad.
    assert!(sounding(&left).abs_diff(24_000) <= 1);
    let peak =
        |range: std::ops::Range<usize>| left[range].iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!((peak(21_400..21_800) / 0.5 - 0.5).abs() < 0.05);
}

#[test]
fn loop_sustains_without_clicks_and_lasts_dur() {
    // 220 Hz en un corte de 200 ms: el tramo central (50 a 150 ms) no
    // encaja con el periodo, así que un salto seco haría clic.
    let sample = sine(220.0, 0.2);
    let shape = |crossfade_ms: f32| {
        with(|s| {
            s.dur = Some(Dur::Ms(1000.0));
            s.fit = Fit::Loop;
            s.loop_start_ms = Some(50.0);
            s.loop_end_ms = Some(137.3);
            s.crossfade_ms = crossfade_ms;
        })
    };
    let left = render(sample.clone(), shape(10.0), 120.0, 60_000);
    assert!(sounding(&left).abs_diff(48_000) <= 1);
    // Suena todo el rato (la nota se sostiene) y nunca salta más que el seno.
    for (i, w) in left[100..47_900].chunks(480).enumerate() {
        assert!(w.iter().any(|x| x.abs() > 0.3), "bloque {i} en silencio");
    }
    let jump = max_jump(&left, 0..48_100);
    assert!(jump <= sine_jump(220.0) * 1.05, "salto {jump}");
    // Sin fundido, la unión sí salta: la prueba lo vería.
    let dry = render(sample, shape(0.0), 120.0, 60_000);
    assert!(max_jump(&dry, 0..48_000) > sine_jump(220.0) * 3.0);
}

#[test]
fn loop_defaults_to_the_middle_half() {
    // Una rampa: lo que suena dice por dónde se lee.
    let mono: Vec<f32> = (0..1000).map(|i| 0.1 + 0.3 * i as f32 / 1000.0).collect();
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let left = render(
        sample,
        with(|s| {
            s.dur = Some(Dur::Ms(100.0));
            s.fit = Fit::Loop;
            s.crossfade_ms = 0.0;
        }),
        120.0,
        6000,
    );
    // Primero el corte hasta 750; luego, de 250 a 750 una y otra vez.
    let value = |frame: usize| 0.1 + 0.3 * frame as f32 / 1000.0;
    assert_eq!(left[600], value(600));
    assert_eq!(left[750], value(250));
    assert_eq!(left[1250], value(250));
    assert_eq!(left[1300], value(300));
}

#[test]
fn stretch_lasts_dur_and_keeps_the_pitch() {
    let sample = sine(440.0, 0.5);
    for (ms, frames) in [(1000.0, 48_000), (250.0, 12_000), (1500.0, 72_000)] {
        let left = render(
            sample.clone(),
            with(|s| {
                s.dur = Some(Dur::Ms(ms));
                s.fit = Fit::Stretch;
            }),
            120.0,
            frames + 6000,
        );
        let got = sounding(&left);
        assert!(got.abs_diff(frames) <= 1, "{ms} ms: {got} frames");
        // YIN sobre el resultado: la misma nota.
        let middle = &left[frames / 4..frames * 3 / 4];
        let guess = detect_mono(middle, RATE).expect("tiene tono");
        let error = (guess.hz / 440.0 - 1.0).abs();
        assert!(error < 0.005, "{ms} ms: {} Hz", guess.hz);
    }
}

#[test]
fn stretch_with_pitch_moves_the_pitch_but_not_the_duration() {
    let sample = sine(440.0, 0.5);
    let left = render_step(
        sample,
        with(|s| {
            s.dur = Some(Dur::Ms(1000.0));
            s.fit = Fit::Stretch;
        }),
        Step::new(0).with_pitch(7.0),
        120.0,
        54_000,
    );
    assert!(sounding(&left).abs_diff(48_000) <= 1);
    let guess = detect_mono(&left[12_000..36_000], RATE).unwrap();
    let want = 440.0 * 2f32.powf(7.0 / 12.0);
    assert!((guess.hz / want - 1.0).abs() < 0.005, "{} Hz", guess.hz);
}

#[test]
fn the_step_overrides_the_slice() {
    let sample = sine(220.0, 2.0);
    let step = Step::new(0).with_hit(HitShape {
        dur: Some(Dur::Note { num: 1, den: 16 }),
        ..HitShape::default()
    });
    let left = render_step(
        sample.clone(),
        with(|s| s.dur = Some(Dur::Ms(800.0))),
        step,
        120.0,
        50_000,
    );
    assert!(sounding(&left).abs_diff(6000) <= 1);
    // Y lo que el paso no dice lo pone el corte.
    let step = Step::new(0).with_hit(HitShape {
        fit: Some(Fit::Stretch),
        ..HitShape::default()
    });
    let left = render_step(
        sample,
        with(|s| s.dur = Some(Dur::Ms(3000.0))),
        step,
        120.0,
        150_000,
    );
    assert!(sounding(&left).abs_diff(144_000) <= 1);
}

#[test]
fn without_dur_nothing_changes() {
    let sample = sine(220.0, 0.3);
    let plain = render(sample.clone(), SliceShape::DEFAULT, 120.0, 20_000);
    // `fit` y los puntos del bucle sin `dur` no hacen nada.
    let idle = render(
        sample,
        with(|s| {
            s.fit = Fit::Loop;
            s.loop_start_ms = Some(10.0);
        }),
        120.0,
        20_000,
    );
    assert_eq!(plain, idle);
}

#[test]
fn ranges_are_checked() {
    let bad = |f: fn(&mut SliceShape)| with(f).validate().is_err();
    assert!(bad(|s| s.dur = Some(Dur::Ms(0.0))));
    assert!(bad(|s| s.dur = Some(Dur::Ms(70_000.0))));
    assert!(bad(|s| s.dur = Some(Dur::Bars(17))));
    assert!(bad(|s| s.dur = Some(Dur::Note { num: 0, den: 8 })));
    assert!(bad(|s| s.dur = Some(Dur::Note { num: 17, den: 1 })));
    assert!(bad(|s| {
        s.dur = Some(Dur::Ms(100.0));
        s.play = PlayMode::Loop;
    }));
    assert!(bad(|s| {
        s.loop_start_ms = Some(50.0);
        s.loop_end_ms = Some(20.0);
    }));
    assert!(bad(|s| s.loop_start_ms = Some(-1.0)));
    // Por paso, el patrón lo rechaza con el número de paso.
    let sample = sine(220.0, 0.1);
    let slices = Slices::grid(&sample, 1);
    let mut steps = vec![None; STEPS];
    steps[3] = Some(Step::new(0).with_hit(HitShape {
        dur: Some(Dur::Bars(40)),
        ..HitShape::default()
    }));
    assert!(Pattern::from_steps(sample, &slices, steps).is_err());
}
