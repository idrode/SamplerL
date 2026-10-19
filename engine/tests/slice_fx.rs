//! Efectos por corte (`Pattern::with_slice_fx`): solo cambia el corte que
//! los tiene, cada golpe empieza sus efectos sin memoria, las curvas los
//! mueven, sirven con todo lo de los pasos y los pads, y el resto suena bit
//! a bit como sin ellos. Todo con render offline.

use std::sync::Arc;

use engine::notation::parse_steps;
use engine::{
    Automation, Batch, Curve, DynEqParams, Fx, Granular, Instrument, Pattern, STEPS, Sample,
    SendError, SliceFx, SliceShape, Slices, Step, new_engine, offline,
};

const RATE: u32 = 48_000;
/// Un paso a 120 BPM.
const STEP: usize = 6000;

/// Cuatro cortes de un paso cada uno: ruido (determinista) en sus 4000
/// primeros frames y silencio después.
fn noise_sample() -> (Arc<Sample>, Slices) {
    let mut state = 0x1234_5678u32;
    let mut mono = vec![0.0f32; 4 * STEP];
    for (i, x) in mono.iter_mut().enumerate() {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        if i % STEP < 4000 {
            *x = 0.4 * ((state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0);
        }
    }
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 4);
    (sample, slices)
}

/// Cuatro cortes de un paso, constantes a 0.5.
fn dc_sample() -> (Arc<Sample>, Slices) {
    let sample = Arc::new(Sample::from_interleaved(&vec![0.5; 4 * STEP], 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 4);
    (sample, slices)
}

fn chain(fx: &[Fx]) -> Option<SliceFx> {
    Some(SliceFx::new(fx, &[]).unwrap())
}

fn render(patterns: Vec<Pattern>, frames: usize) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    let mut batch = Batch::new();
    for (t, p) in patterns.into_iter().enumerate() {
        batch.set_pattern(t, Some(p));
    }
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
}

fn pattern(sample: &Arc<Sample>, slices: &Slices, text: &str) -> Pattern {
    Pattern::from_steps(sample.clone(), slices, parse_steps(text).unwrap()).unwrap()
}

/// Los frames (estéreo) del paso `k`.
fn window(audio: &[f32], k: usize) -> &[f32] {
    &audio[2 * k * STEP..2 * (k + 1) * STEP]
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

/// Cuánto cambia de una muestra a la siguiente frente a su nivel: alto
/// con agudos, bajo con graves (canal izquierdo).
fn brightness(x: &[f32]) -> f32 {
    let left: Vec<f32> = x.iter().step_by(2).copied().collect();
    let diff: Vec<f32> = left.windows(2).map(|w| w[1] - w[0]).collect();
    rms(&diff) / rms(&left).max(1e-9)
}

/// Lo que queda tras una media móvil de 32 frames (los graves) frente al
/// nivel (canal izquierdo).
fn lows(x: &[f32]) -> f32 {
    let left: Vec<f32> = x.iter().step_by(2).copied().collect();
    let avg: Vec<f32> = left
        .windows(32)
        .map(|w| w.iter().sum::<f32>() / 32.0)
        .collect();
    rms(&avg) / rms(&left).max(1e-9)
}

const TEXT: &str = "1 . 2 .  3 . . .  4 . 1 .  . . 2 .";
/// Paso de cada golpe de `TEXT` y su corte (desde 1).
const HITS: [(usize, usize); 6] = [(0, 1), (2, 2), (4, 3), (8, 4), (10, 1), (14, 2)];

#[test]
fn only_the_chosen_slice_changes() {
    let (sample, slices) = noise_sample();
    let plain = render(vec![pattern(&sample, &slices, TEXT)], 16 * STEP);
    let mut resonant = Fx::lowpass(500.0);
    if let Fx::Filter { q, .. } = &mut resonant {
        *q = 4.0;
    }
    for fx in [
        vec![Fx::bitcrush(4, 3)],
        vec![resonant, Fx::distortion(10.0, 1.0)],
    ] {
        let with = pattern(&sample, &slices, TEXT)
            .with_slice_fx(&[None, None, chain(&fx)])
            .unwrap();
        let audio = render(vec![with], 16 * STEP);
        for (k, slice) in HITS {
            let (a, b) = (window(&plain, k), window(&audio, k));
            if slice == 3 {
                assert_ne!(a, b, "{fx:?}: el corte 3 tiene que cambiar");
            } else {
                assert_eq!(a, b, "{fx:?}: el corte {slice} (paso {k}) no cambia");
            }
        }
    }
}

#[test]
fn no_effects_sound_bit_identical() {
    let (sample, slices) = noise_sample();
    let plain = render(vec![pattern(&sample, &slices, TEXT)], 16 * STEP);
    // Sin cadenas, con cadenas vacías o con efectos en un corte que no
    // suena: todo igual.
    for chains in [
        vec![],
        vec![None, None],
        vec![Some(SliceFx::new(&[], &[]).unwrap()); 4],
        vec![None; 4]
            .into_iter()
            .chain([chain(&[Fx::bitcrush(2, 8)])])
            .collect(),
    ] {
        let with = pattern(&sample, &slices, TEXT)
            .with_slice_fx(&chains)
            .unwrap();
        assert_eq!(render(vec![with], 16 * STEP), plain, "{chains:?}");
    }
}

#[test]
fn two_slices_with_different_effects_at_once() {
    let (sample, slices) = noise_sample();
    let text = "1 2 1 2  1 2 1 2  1 2 1 2  1 2 1 2";
    let plain = render(vec![pattern(&sample, &slices, text)], 16 * STEP);
    let with = pattern(&sample, &slices, text)
        .with_slice_fx(&[
            chain(&[Fx::lowpass(400.0)]),
            chain(&[Fx::highpass(6000.0), Fx::gain(2.0)]),
        ])
        .unwrap();
    let audio = render(vec![with], 16 * STEP);
    for k in 0..16 {
        let (a, b) = (window(&plain, k), window(&audio, k));
        if k % 2 == 0 {
            assert!(brightness(b) < 0.3 * brightness(a), "paso {k}: paso bajo");
        } else {
            assert!(
                lows(b) < 0.3 * lows(a),
                "paso {k}: paso alto ({} frente a {})",
                lows(b),
                lows(a)
            );
        }
    }
    // La misma variable en dos pistas, cada una con su cadena: suenan a la
    // vez, cada golpe con la suya.
    let both = render(
        vec![
            pattern(&sample, &slices, "1 . . .  1 . . .  1 . . .  1 . . .")
                .with_slice_fx(&[chain(&[Fx::lowpass(400.0)])])
                .unwrap(),
            pattern(&sample, &slices, "2 . . .  2 . . .  2 . . .  2 . . .")
                .with_slice_fx(&[None, chain(&[Fx::highpass(6000.0)])])
                .unwrap(),
        ],
        4 * STEP,
    );
    let alone = |text: &str, chains: &[Option<SliceFx>]| {
        render(
            vec![
                pattern(&sample, &slices, text)
                    .with_slice_fx(chains)
                    .unwrap(),
            ],
            4 * STEP,
        )
    };
    let low = alone(
        "1 . . .  1 . . .  1 . . .  1 . . .",
        &[chain(&[Fx::lowpass(400.0)])],
    );
    let high = alone(
        "2 . . .  2 . . .  2 . . .  2 . . .",
        &[None, chain(&[Fx::highpass(6000.0)])],
    );
    for i in 0..both.len() {
        assert!(
            (both[i] - (low[i] + high[i])).abs() < 1e-6,
            "frame {}",
            i / 2
        );
    }
}

#[test]
fn every_hit_starts_its_effects_fresh() {
    let (sample, slices) = noise_sample();
    let text = "1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1";
    // Con downsample 7 (6000 no es múltiplo), un bitcrusher de pista que
    // siguiera de golpe en golpe cogería cada uno en otra fase; el del
    // corte empieza de cero en cada golpe.
    let per_slice = render(
        vec![
            pattern(&sample, &slices, text)
                .with_slice_fx(&[chain(&[Fx::bitcrush(16, 7)])])
                .unwrap(),
        ],
        16 * STEP,
    );
    for k in 1..16 {
        assert_eq!(window(&per_slice, k), window(&per_slice, 0), "golpe {k}");
    }
    let per_track = render(
        vec![
            pattern(&sample, &slices, text)
                .with_fx(&[Fx::bitcrush(16, 7)])
                .unwrap(),
        ],
        2 * STEP,
    );
    assert_ne!(window(&per_track, 1), window(&per_track, 0));
}

#[test]
fn ramps_and_lfos_move_slice_parameters() {
    let (sample, slices) = dc_sample();
    let ramp = Automation {
        fx: 0,
        param: 0,
        curve: Curve::Ramp {
            from: 0.0,
            to: 1.0,
            steps: 16.0,
        },
    };
    let chains = [Some(SliceFx::new(&[Fx::gain(1.0)], &[ramp]).unwrap())];
    let text = "1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1";
    let make = || {
        pattern(&sample, &slices, text)
            .with_slice_fx(&chains)
            .unwrap()
    };
    let audio = render(vec![make()], 16 * STEP);
    assert_eq!(audio, render(vec![make()], 16 * STEP), "determinista");
    // A mitad de cada paso, el volumen de la rampa en ese punto.
    for k in 0..16 {
        let mid = audio[2 * (k * STEP + STEP / 2)];
        let expected = 0.5 * (k as f32 + 0.5) / 16.0;
        assert!(
            (mid - expected).abs() < 0.01,
            "paso {k}: {mid} frente a {expected}"
        );
    }
    // Una curva fuera del rango del parámetro, o de un efecto que no está.
    let over = Automation {
        curve: Curve::Ramp {
            from: 0.0,
            to: 5.0,
            steps: 16.0,
        },
        ..ramp
    };
    assert!(SliceFx::new(&[Fx::gain(1.0)], &[over]).is_err());
    assert!(SliceFx::new(&[Fx::gain(1.0)], &[Automation { fx: 1, ..ramp }]).is_err());
}

#[test]
fn works_with_reverse_pitch_retrigger_gate_choke_and_pads() {
    let (sample, slices) = noise_sample();
    // El corte 1 en silencio con su gain a 0: así suenen al revés, con
    // pitch (^ y %), con retrigger o en gate, sus golpes no se oyen; el 2,
    // sí.
    let text = "1' . 2 .  1^7 . 2 .  1%-5 . 2 .  1r3 . 2^-12 .";
    let silent = [chain(&[Fx::gain(0.0)])];
    for gate in [false, true] {
        let with = pattern(&sample, &slices, text)
            .with_slice_fx(&silent)
            .unwrap()
            .with_gate(gate)
            .with_choke(Some(1))
            .unwrap();
        let audio = render(vec![with], 16 * STEP);
        for k in (0..16).step_by(2) {
            let w = window(&audio, k);
            if k % 4 == 0 {
                // El fundido del golpe anterior, cortado, ocupa el principio.
                assert!(w[400..].iter().all(|&x| x == 0.0), "gate {gate}, paso {k}");
            } else {
                assert!(rms(w) > 0.05, "gate {gate}, paso {k}");
            }
        }
    }

    // Pads: tocan el corte con sus efectos si es el sample de la pista.
    let pad = |index: usize, other: bool| {
        let (mut controller, mut engine) = new_engine(RATE);
        let with = pattern(&sample, &slices, ". . . .  . . . .  . . . .  . . . .")
            .with_slice_fx(&silent)
            .unwrap();
        controller.set_track_pattern(0, Some(with)).unwrap();
        controller.start().unwrap();
        offline::render(&mut engine, 256, 2);
        let played = if other {
            noise_sample().0
        } else {
            sample.clone()
        };
        controller
            .play_slice_with_gain(0, &played, &slices, index, SliceShape::DEFAULT, 1.0)
            .unwrap();
        rms(&offline::render(&mut engine, STEP, 2))
    };
    assert_eq!(pad(0, false), 0.0, "el pad del corte 1 pasa por su gain");
    assert!(pad(1, false) > 0.05, "el del corte 2 no tiene efectos");
    assert!(
        pad(0, true) > 0.05,
        "otro sample no usa los efectos de la pista"
    );
}

#[test]
fn granular_ignores_them_and_instruments_refuse_them() {
    let (sample, slices) = noise_sample();
    let clouds = |chains: &[Option<SliceFx>]| {
        let p = pattern(&sample, &slices, "1 _ _ _  2 _ _ _  . . . .  . . . .")
            .with_slice_fx(chains)
            .unwrap()
            .with_granular(Some(Granular::new(3)))
            .unwrap();
        render(vec![p], 16 * STEP)
    };
    assert_eq!(clouds(&[chain(&[Fx::bitcrush(2, 4)])]), clouds(&[]));

    let with = pattern(&sample, &slices, "1 . . .  . . . .  . . . .  . . . .")
        .with_slice_fx(&[chain(&[Fx::lowpass(500.0)])])
        .unwrap();
    assert!(matches!(
        with.with_instrument(Some(Instrument::new(4))),
        Err(SendError::InvalidFx(_))
    ));
    let instrument = pattern(&sample, &slices, "1 . . .  . . . .  . . . .  . . . .")
        .with_instrument(Some(Instrument::new(4)))
        .unwrap();
    assert!(matches!(
        instrument.with_slice_fx(&[chain(&[Fx::lowpass(500.0)])]),
        Err(SendError::InvalidFx(_))
    ));
}

#[test]
fn eight_tracks_of_hot_slices_never_pass_one() {
    let (sample, slices) = noise_sample();
    let hot = [Fx::distortion(50.0, 1.0), Fx::gain(4.0)];
    let patterns: Vec<Pattern> = (0..8)
        .map(|t| {
            let steps: Vec<Option<Step>> = (0..STEPS)
                .map(|k| Some(Step::new((k + t) % 4).with_gain(4.0)))
                .collect();
            Pattern::from_steps(sample.clone(), &slices, steps)
                .unwrap()
                .with_gain(4.0)
                .unwrap()
                .with_slice_fx(&[chain(&hot), chain(&hot), chain(&hot), chain(&hot)])
                .unwrap()
        })
        .collect();
    let audio = render(patterns, 16 * STEP);
    let peak = audio.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    assert!(peak <= 1.0, "pico {peak}");
    assert!(peak > 0.9, "tiene que estar saturado: {peak}");
}

#[test]
fn a_resonant_tail_rings_after_the_voice_and_then_stops() {
    let mut state = 7u32;
    let mono: Vec<f32> = (0..STEP)
        .map(|_| {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            0.3 * ((state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0)
        })
        .collect();
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    let mut resonant = Fx::lowpass(300.0);
    if let Fx::Filter { q, .. } = &mut resonant {
        *q = 20.0;
    }
    let p = pattern(&sample, &slices, "1 . . .  . . . .  . . . .  . . . .")
        .with_slice_fx(&[chain(&[resonant])])
        .unwrap();
    let audio = render(vec![p], 4 * STEP);
    // La voz acaba en el frame 6000 (con su fundido); el filtro sigue.
    let after = &audio[2 * (STEP + 100)..2 * (STEP + 1000)];
    assert!(rms(after) > 1e-3, "la cola suena: {}", rms(after));
    // Y se calla tras los 100 ms de cola (más un trozo de mezcla).
    let tail_end = STEP + 4800 + 256;
    assert!(audio[2 * tail_end..].iter().all(|&x| x == 0.0));
}

#[test]
fn chains_are_checked() {
    let too_many = [Fx::gain(1.0); 5];
    assert!(SliceFx::new(&too_many, &[]).is_err());
    assert!(SliceFx::new(&[Fx::gain(4.5)], &[]).is_err());
    assert!(SliceFx::new(&[Fx::lowpass(5.0)], &[]).is_err());
    let keyed = Fx::DynEq(DynEqParams::new(100.0).with_sidechain(1));
    assert!(SliceFx::new(&[keyed], &[]).is_err());
    assert!(SliceFx::new(&[Fx::DynEq(DynEqParams::new(100.0))], &[]).is_ok());
}
