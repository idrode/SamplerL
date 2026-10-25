//! El sintetizador en una pista del motor: notas a su altura, accent,
//! slide (legato, sin volver a disparar las envolventes), notas en vivo,
//! paro y choke, curvas de sus parámetros y 8 voces con unison dentro de
//! ±1.

use std::sync::Arc;

use engine::synth::{
    self, CUTOFF, ENV_AMOUNT, FilterMode, PARAM_COUNT, Patch, RESONANCE, SynthCurves, Wave, acid,
};
use engine::{Curve, MAX_SEMITONES, Pattern, STEPS, Sample, Slices, Step, new_engine, offline};

const RATE: u32 = 48_000;
/// 120 BPM: 6000 frames por paso.
const STEP: usize = 6000;
const NO_CURVES: SynthCurves = [None; PARAM_COUNT];

fn pattern(patch: Patch, curves: SynthCurves, steps: Vec<Option<Step>>) -> Pattern {
    let sample = Arc::new(Sample::silence(RATE));
    let slices = Slices::grid(&sample, 1);
    Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_synth(Some(patch), curves)
        .unwrap()
}

/// Canal izquierdo de `frames` frames con el patrón en la pista 0.
fn render(pattern: Pattern, frames: usize) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

/// Un seno sin filtro que oiga (paso alto a 20 Hz), con la amplitud fija
/// mientras dura la nota.
fn sine() -> Patch {
    let mut p = Patch::new()
        .with(CUTOFF, 20.0)
        .with(RESONANCE, 0.0)
        .with(ENV_AMOUNT, 0.0);
    p.wave = Wave::Sine;
    p.filter = FilterMode::Highpass;
    p.root = 69.0;
    p
}

/// Frecuencia por los cruces por cero que suben en `y`.
fn frequency(y: &[f32]) -> f64 {
    let ups: Vec<f64> = y
        .windows(2)
        .enumerate()
        .filter(|(_, w)| w[0] <= 0.0 && w[1] > 0.0)
        .map(|(i, w)| i as f64 + (-w[0] / (w[1] - w[0])) as f64)
        .collect();
    let periods = (ups.len() - 1) as f64;
    RATE as f64 * periods / (ups[ups.len() - 1] - ups[0])
}

fn rms(y: &[f32]) -> f32 {
    (y.iter().map(|v| v * v).sum::<f32>() / y.len() as f32).sqrt()
}

fn steps(list: &[(usize, Step)]) -> Vec<Option<Step>> {
    let mut steps = vec![None; STEPS];
    for &(i, step) in list {
        steps[i] = Some(step);
    }
    steps
}

/// Cada paso suena a la raíz más sus semitonos: 440 Hz, una quinta arriba
/// y una octava abajo.
#[test]
fn notes_sound_at_their_pitch() {
    let y = render(
        pattern(
            sine(),
            NO_CURVES,
            steps(&[
                (0, Step::new(0)),
                (4, Step::new(0).with_pitch(7.0)),
                (8, Step::new(0).with_pitch(-12.0)),
            ]),
        ),
        12 * STEP,
    );
    for (step, hz) in [(0, 440.0), (4, 440.0 * 1.4983), (8, 220.0)] {
        let at = step * STEP + STEP / 2;
        let f = frequency(&y[at..at + 2 * STEP]);
        assert!(
            (f / hz - 1.0).abs() < 0.002,
            "paso {step}: {f:.2} Hz, no {hz}"
        );
    }
}

/// Un acid sin accent y con él: el accent dobla la amplitud con `accent`
/// = 1 (y abre el filtro).
#[test]
fn an_accent_is_louder_and_brighter() {
    let mut patch = acid::patch(45.0);
    acid::param("accent").unwrap().set(&mut patch, 1.0);
    let plain = render(pattern(patch, NO_CURVES, steps(&[(0, Step::new(0))])), STEP);
    let accented = render(
        pattern(patch, NO_CURVES, steps(&[(0, Step::new(0).accented())])),
        STEP,
    );
    let (a, b) = (rms(&plain[..STEP / 2]), rms(&accented[..STEP / 2]));
    assert!(b > 1.8 * a, "con accent {b}, sin él {a}");
}

/// Una nota que baja hasta casi callarse (decay corto sin sustain): la
/// siguiente sin slide vuelve a empezar la envolvente (suena fuerte); con
/// slide, sigue donde estaba (no se dispara) y su altura se desliza desde
/// la anterior en vez de saltar.
#[test]
fn a_slide_ties_the_note_without_retriggering() {
    let mut patch = sine();
    patch.mono = true;
    patch.voices = 1;
    patch.amp.decay_ms = 50.0;
    patch.amp.sustain = 0.05;
    let second = |slide: bool| {
        let next = Step::new(0).with_pitch(12.0);
        let next = if slide { next.slid() } else { next };
        render(
            pattern(patch, NO_CURVES, steps(&[(0, Step::new(0)), (1, next)])),
            3 * STEP,
        )
    };
    let retriggered = second(false);
    let tied = second(true);
    let after = STEP + 200..STEP + 1200;
    let (r, t) = (rms(&retriggered[after.clone()]), rms(&tied[after]));
    assert!(r > 5.0 * t, "sin slide {r}, con slide {t}");
    // Antes del segundo paso, la primera nota no ha soltado (su gate dura
    // hasta que llega la ligada): el mismo nivel en los dos.
    let before = STEP - 600..STEP;
    let (a, b) = (rms(&retriggered[before.clone()]), rms(&tied[before]));
    assert!(b >= 0.9 * a, "la ligada soltó antes: {b} frente a {a}");
    // Altura: sin slide salta a 880 Hz; con slide pasa por en medio.
    let early = STEP + 100..STEP + 700;
    let jump = frequency(&retriggered[early.clone()]);
    let glide = frequency(&tied[early]);
    assert!((jump / 880.0 - 1.0).abs() < 0.01, "sin slide: {jump:.1} Hz");
    assert!(glide > 450.0 && glide < 860.0, "con slide: {glide:.1} Hz");
    let late = STEP + 4000..STEP + 6000;
    assert!((frequency(&tied[late]) / 880.0 - 1.0).abs() < 0.01);
}

/// En vivo: suena hasta su note off y después hace su release; la misma
/// tecla otra vez suelta la anterior.
#[test]
fn a_live_note_holds_until_its_note_off() {
    let (mut controller, mut engine) = new_engine(RATE);
    let patch = sine();
    controller.synth_note_on(0, patch, 0.0, 1.0, 60).unwrap();
    let held = offline::render(&mut engine, 2 * STEP, 2);
    controller.note_off(0, 60).unwrap();
    let released = offline::render(&mut engine, 2 * STEP, 2);
    assert!(rms(&held[2 * STEP..]) > 0.5);
    assert!(rms(&released[2 * STEP..]) < 1e-3, "el release no acabó");
    assert!(
        controller
            .synth_note_on(0, patch, MAX_SEMITONES + 1.0, 1.0, 1)
            .is_err()
    );
}

/// Parar corta las voces (con el fundido mínimo).
#[test]
fn stop_cuts_the_synth() {
    let (mut controller, mut engine) = new_engine(RATE);
    let mut patch = sine();
    patch.amp.release_ms = 2000.0;
    controller
        .set_pattern(pattern(patch, NO_CURVES, steps(&[(0, Step::new(0))])))
        .unwrap();
    controller.start().unwrap();
    let _ = offline::render(&mut engine, STEP / 2, 2);
    controller.stop().unwrap();
    let after = offline::render(&mut engine, STEP, 2);
    assert!(after[2 * 200..].iter().all(|v| v.abs() < 1e-6));
}

/// Una rampa del corte se oye: el acid suena más brillante (más
/// diferencia entre muestras) al final de la rampa que al principio.
#[test]
fn a_cutoff_ramp_opens_the_filter() {
    let mut patch = acid::patch(45.0);
    acid::param("envmod").unwrap().set(&mut patch, 0.0);
    let mut curves = NO_CURVES;
    curves[CUTOFF] = Some(Curve::Ramp {
        from: 100.0,
        to: 5000.0,
        steps: 16.0,
    });
    let held = Step::new(0).with_hold(15);
    let y = render(pattern(patch, curves, steps(&[(0, held)])), STEPS * STEP);
    let edge = |r: std::ops::Range<usize>| {
        let s = &y[r];
        rms(&s.windows(2).map(|w| w[1] - w[0]).collect::<Vec<_>>()) / rms(s)
    };
    let (start, end) = (edge(STEP..2 * STEP), edge(14 * STEP..15 * STEP));
    assert!(end > 3.0 * start, "al principio {start}, al final {end}");
}

/// Ocho voces con unison de 7 en dos pistas con acordes de 4 notas, todo
/// fuerte: la salida del motor (con su limitador) sigue dentro de ±1.
#[test]
fn eight_unison_voices_stay_within_one() {
    let mut patch = Patch::new()
        .with(synth::DETUNE, 30.0)
        .with(synth::SPREAD, 1.0);
    patch.unison = 7;
    patch.voices = 4;
    let chord = Step::new(0)
        .with_gain(4.0)
        .with_hold(15)
        .with_chord(&[3.0, 7.0, 10.0]);
    let (mut controller, mut engine) = new_engine(RATE);
    for t in 0..2 {
        controller
            .set_track_pattern(
                t,
                Some(pattern(
                    patch,
                    NO_CURVES,
                    steps(&[(0, chord.with_pitch(-12.0 * t as f32))]),
                )),
            )
            .unwrap();
    }
    controller.start().unwrap();
    let out = offline::render(&mut engine, 4 * STEP, 2);
    assert!(out.iter().all(|v| v.is_finite() && v.abs() <= 1.0));
    assert!(out.iter().any(|v| v.abs() > 0.9), "no suena fuerte");
    assert_eq!(controller.status().tracks[0].voices, 4);
}

/// Lo que el motor no deja: acordes en un mono, sintetizador con
/// instrumento, parámetros y curvas fuera de rango.
#[test]
fn invalid_synths_are_rejected() {
    let sample = Arc::new(Sample::silence(RATE));
    let slices = Slices::grid(&sample, 1);
    let chord = steps(&[(3, Step::new(0).with_chord(&[4.0]))]);
    let base = || Pattern::from_steps(sample.clone(), &slices, chord.clone()).unwrap();
    assert!(
        base()
            .with_synth(Some(acid::patch(36.0)), NO_CURVES)
            .is_err()
    );
    assert!(base().with_synth(Some(Patch::new()), NO_CURVES).is_ok());
    assert!(
        base()
            .with_synth(Some(Patch::new().with(CUTOFF, 5.0)), NO_CURVES)
            .is_err()
    );
    let mut curves = NO_CURVES;
    curves[RESONANCE] = Some(Curve::Ramp {
        from: 0.0,
        to: 2.0,
        steps: 4.0,
    });
    assert!(base().with_synth(Some(Patch::new()), curves).is_err());
    let synth = base().with_synth(Some(Patch::new()), NO_CURVES).unwrap();
    assert!(
        synth
            .with_instrument(Some(engine::Instrument::new(2)))
            .is_err()
    );
}

/// Un subharm que se oye tal cual (senos, paso alto a 20 Hz, sostenido),
/// con las capas `on` (capa, rdiv) sonando y una envolvente de capa corta
/// (sube en 0,5 ms y se apaga en 20): cada redisparo es un golpe aislado.
fn rhythmic(on: &[(usize, u8)]) -> Patch {
    let mut p = synth::subharm::patch(69.0)
        .with(CUTOFF, 20.0)
        .with(RESONANCE, 0.0)
        .with(ENV_AMOUNT, 0.0)
        .with(synth::LAYER_ATTACK, 0.5)
        .with(synth::LAYER_DECAY, 20.0);
    p.wave = Wave::Sine;
    p.filter = FilterMode::Highpass;
    p.amp.sustain = 1.0;
    let mut layers = synth::subharm::DEFAULT_LAYERS.map(|l| synth::Layer { level: 0.0, ..l });
    for &(i, rdiv) in on {
        layers[i].level = 1.0;
        layers[i].rdiv = rdiv;
        layers[i].div = 1;
    }
    p.layers = Some(layers);
    p
}

/// Frames donde empieza cada golpe: la primera muestra que pasa de una
/// milésima del pico tras 800 frames de silencio (un golpe tarda unos
/// 1400 en bajar de ahí: los más juntos, en 5 contra 7, están a 2742).
fn onsets(y: &[f32]) -> Vec<usize> {
    let peak = y.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    let mut out = Vec::new();
    let mut quiet = 0usize;
    for (i, v) in y.iter().enumerate() {
        if v.abs() > 1e-3 * peak {
            if quiet > 800 || out.is_empty() {
                out.push(i);
            }
            quiet = 0;
        } else {
            quiet += 1;
        }
    }
    out
}

/// Lo que tarda un golpe en asomar: el diezmado (unos 16 frames) y el
/// ataque de 0,5 ms.
const LATENCY: usize = 48;

/// Comprueba que los golpes de `y` caen en `expected` (cada uno, como
/// mucho `LATENCY` frames después) y que no hay más.
fn check_onsets(y: &[f32], expected: &[usize], what: &str) {
    let got = onsets(y);
    assert_eq!(
        got.len(),
        expected.len(),
        "{what}: golpes {got:?}, se esperaban {expected:?}"
    );
    for (g, e) in got.iter().zip(expected) {
        assert!(
            *g >= *e && *g <= e + LATENCY,
            "{what}: golpe en {g}, se esperaba en {e}"
        );
    }
}

/// Una nota que dura dos compases (una por compás, la segunda pegada).
fn two_bars() -> Vec<Option<Step>> {
    steps(&[(0, Step::new(0).with_hold(15))])
}

/// Con `rdiv = r`, la capa vuelve a sonar en cada `j · compás / r` (con
/// enteros), en los dos compases: 3, 4, 5 y 7.
#[test]
fn layers_retrigger_at_bar_over_rdiv() {
    let bar = 16 * STEP;
    for r in [3u8, 4, 5, 7] {
        let y = render(pattern(rhythmic(&[(0, r)]), NO_CURVES, two_bars()), 2 * bar);
        let expected: Vec<usize> = (0..2)
            .flat_map(|b| (0..r as usize).map(move |j| b * bar + j * bar / r as usize))
            .collect();
        check_onsets(&y, &expected, &format!("rdiv {r}"));
    }
}

/// Dos capas en poliritmo (3 contra 4, 5 contra 7) siguen el mismo reloj:
/// sus golpes son la unión de los de cada una y vuelven a coincidir al
/// empezar cada compás, sin deriva, durante cuatro compases.
#[test]
fn polyrhythms_meet_at_every_bar() {
    let bar = 16 * STEP;
    for (a, b) in [(3u8, 4u8), (5, 7)] {
        let y = render(
            pattern(rhythmic(&[(0, a), (1, b)]), NO_CURVES, two_bars()),
            4 * bar,
        );
        let mut expected: Vec<usize> = (0..4)
            .flat_map(|k| {
                let pulses = |r: u8| (0..r as usize).map(move |j| k * bar + j * bar / r as usize);
                pulses(a).chain(pulses(b)).collect::<Vec<_>>()
            })
            .collect();
        expected.sort();
        expected.dedup();
        check_onsets(&y, &expected, &format!("{a} contra {b}"));
    }
}

/// Con el transporte parado (una nota en vivo), el compás es virtual y
/// empieza con la nota: los golpes caen en `nota + j · compás / r`, con el
/// compás del tempo de ahora.
#[test]
fn a_live_note_counts_a_virtual_bar_from_its_note_on() {
    let (mut controller, mut engine) = new_engine(RATE);
    let silence = offline::render(&mut engine, 1234, 2);
    assert!(silence.iter().all(|v| *v == 0.0));
    controller
        .synth_note_on(0, rhythmic(&[(0, 4)]), 0.0, 1.0, 60)
        .unwrap();
    let bar = 16 * STEP;
    let y: Vec<f32> = offline::render(&mut engine, 2 * bar, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect();
    let expected: Vec<usize> = (0..8).map(|j| j * bar / 4).collect();
    check_onsets(&y, &expected, "nota en vivo, rdiv 4");
}

/// Una nota que empieza a medio compás suena ya y sigue la rejilla del
/// compás (no la suya): con `rdiv = 4`, en el paso 6 suena, y luego en el
/// 8 y el 12.
#[test]
fn a_note_in_the_middle_of_a_bar_joins_the_grid() {
    let bar = 16 * STEP;
    let y = render(
        pattern(
            rhythmic(&[(0, 4)]),
            NO_CURVES,
            steps(&[(6, Step::new(0).with_hold(9))]),
        ),
        bar,
    );
    check_onsets(&y, &[6 * STEP, 8 * STEP, 12 * STEP], "a medio compás");
}

/// Cambiar el `rdiv` entra con el patrón, al empezar el compás siguiente:
/// el primer compás sigue en 3 y el segundo pasa a 5.
#[test]
fn an_rdiv_change_waits_for_the_next_bar() {
    let bar = 16 * STEP;
    let (mut controller, mut engine) = new_engine(RATE);
    controller
        .set_pattern(pattern(rhythmic(&[(0, 3)]), NO_CURVES, two_bars()))
        .unwrap();
    controller.start().unwrap();
    let mut y: Vec<f32> = offline::render(&mut engine, bar / 3 + 100, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect();
    controller
        .set_pattern(pattern(rhythmic(&[(0, 5)]), NO_CURVES, two_bars()))
        .unwrap();
    y.extend(
        offline::render(&mut engine, 2 * bar - y.len(), 2)
            .iter()
            .step_by(2),
    );
    let mut expected: Vec<usize> = (0..3).map(|j| j * bar / 3).collect();
    expected.extend((0..5).map(|j| bar + j * bar / 5));
    check_onsets(&y, &expected, "de 3 a 5");
}
