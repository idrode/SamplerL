//! Secuenciador de 16 pasos: cada paso dispara en el frame esperado a un
//! BPM dado, el patrón se repite, y los cambios entran al final del compás.

use std::sync::Arc;

use engine::{Engine, Pattern, STEPS, Sample, SendError, Slices, new_engine, offline};

const RATE: u32 = 48_000;
/// Nivel de cada corte del sample de prueba.
const LEVEL_A: f32 = 0.5;
const LEVEL_B: f32 = 0.25;
/// Más que cualquier fundido (2 ms): ahí el corte ya suena a nivel completo.
const SETTLE: usize = (RATE / 250) as usize;

/// Dos cortes: un bloque constante de 800 frames a LEVEL_A en 0 y otro a
/// LEVEL_B en 3000, cada uno seguido de silencio. Cada corte dura 3000
/// frames, menos que un paso a los BPM de los tests, así que entre disparos
/// hay silencio y cada disparo se ve como un arranque desde cero.
fn two_slice_sample() -> (Arc<Sample>, Slices) {
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(LEVEL_A);
    mono[3000..3800].fill(LEVEL_B);
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    assert_eq!(slices.iter().collect::<Vec<_>>(), vec![0..3000, 3000..6000]);
    (sample, slices)
}

/// Frame del paso `k` dentro del compás, con aritmética entera exacta:
/// round(k · RATE · 60 / (bpm · 4)).
fn step_offset(k: u64, bpm: u64) -> u64 {
    let num = k * RATE as u64 * 15;
    (2 * num + bpm) / (2 * bpm)
}

fn bar_len(bpm: u64) -> u64 {
    step_offset(STEPS as u64, bpm)
}

/// Disparos encontrados en el canal izquierdo: (frame del disparo, nivel).
/// El fundido de entrada empieza en ganancia cero, así que el primer frame
/// no nulo es el siguiente al del disparo.
fn triggers(audio: &[f32]) -> Vec<(u64, f32)> {
    let left: Vec<f32> = audio.iter().step_by(2).copied().collect();
    (1..left.len() - SETTLE)
        .filter(|&i| left[i - 1] == 0.0 && left[i] != 0.0)
        .map(|i| ((i - 1) as u64, left[i + SETTLE]))
        .collect()
}

/// Renderiza con bloques de tamaño irregular, para que los pasos caigan en
/// cualquier punto del bloque.
fn render_odd_blocks(engine: &mut Engine, frames: usize) -> Vec<f32> {
    let mut out = vec![0.0f32; frames * 2];
    for block in out.chunks_mut(333 * 2) {
        engine.process(block, 2);
    }
    out
}

fn pattern(sample: &Arc<Sample>, slices: &Slices, steps: &str) -> Pattern {
    let steps: Vec<Option<usize>> = steps.split_whitespace().map(|s| s.parse().ok()).collect();
    Pattern::new(sample.clone(), slices, steps.try_into().unwrap()).unwrap()
}

const ALTERNATING: &str = "0 1 0 1  0 1 0 1  0 1 0 1  0 1 0 1";

#[test]
fn every_step_falls_on_the_expected_frame() {
    // 120 BPM: 6000 frames por paso exactos. 170 BPM: 4235,29..., no entero.
    for bpm in [120, 170] {
        let (sample, slices) = two_slice_sample();
        let (mut controller, mut engine) = new_engine(RATE);
        controller
            .set_pattern(pattern(&sample, &slices, ALTERNATING))
            .unwrap();
        controller.set_bpm(bpm as f64).unwrap();
        controller.start().unwrap();

        let bars = 4;
        let audio = render_odd_blocks(&mut engine, (bars * bar_len(bpm)) as usize + 1000);
        let found = triggers(&audio);

        // Todos los pasos de los 4 compases (el patrón se repite) más el
        // primero del quinto, que cae dentro del margen final. Nada más.
        assert_eq!(found.len(), bars as usize * STEPS + 1, "BPM {bpm}");
        for (n, &(frame, level)) in found.iter().enumerate() {
            let (bar, k) = ((n / STEPS) as u64, (n % STEPS) as u64);
            let expected = bar * bar_len(bpm) + step_offset(k, bpm);
            assert_eq!(frame, expected, "BPM {bpm}, compás {bar}, paso {k}");
            let expected_level = if k % 2 == 0 { LEVEL_A } else { LEVEL_B };
            assert_eq!(level, expected_level, "BPM {bpm}, compás {bar}, paso {k}");
        }
    }
}

#[test]
fn rests_do_not_trigger_and_stop_silences() {
    let (sample, slices) = two_slice_sample();
    let (mut controller, mut engine) = new_engine(RATE);
    controller
        .set_pattern(pattern(
            &sample,
            &slices,
            "0 . . 1  . . 0 .  1 . 0 .  . 1 . 1",
        ))
        .unwrap();
    controller.start().unwrap();

    let bpm = 120;
    let audio = render_odd_blocks(&mut engine, (2 * bar_len(bpm)) as usize);
    let hits = [0u64, 3, 6, 8, 10, 13, 15];
    let expected: Vec<u64> = (0..2)
        .flat_map(|bar| {
            hits.iter()
                .map(move |&k| bar * bar_len(bpm) + step_offset(k, bpm))
        })
        .collect();
    let found: Vec<u64> = triggers(&audio).into_iter().map(|(f, _)| f).collect();
    assert_eq!(found, expected);

    controller.stop().unwrap();
    let after = offline::render(&mut engine, bar_len(bpm) as usize, 2);
    assert!(after[(SETTLE * 2)..].iter().all(|&s| s == 0.0));
    assert!(!controller.is_playing());
}

#[test]
fn changes_are_quantized_to_the_end_of_the_bar() {
    let (sample, slices) = two_slice_sample();
    let (mut controller, mut engine) = new_engine(RATE);
    controller
        .set_pattern(pattern(
            &sample,
            &slices,
            "0 0 0 0  0 0 0 0  0 0 0 0  0 0 0 0",
        ))
        .unwrap();
    controller.set_bpm(120.0).unwrap();
    controller.start().unwrap();

    // Cambio a mitad del primer compás, en un frame cualquiera.
    let mut audio = render_odd_blocks(&mut engine, 50_123);
    controller
        .set_pattern(pattern(
            &sample,
            &slices,
            "1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1",
        ))
        .unwrap();
    controller.set_bpm(170.0).unwrap();
    let first_bar = bar_len(120);
    let total = first_bar + 2 * bar_len(170) + 1000;
    audio.extend(render_odd_blocks(&mut engine, total as usize - 50_123));

    // Tres compases más el primer paso del cuarto, dentro del margen final.
    let found = triggers(&audio);
    assert_eq!(found.len(), 3 * STEPS + 1);
    for (n, &(frame, level)) in found.iter().enumerate() {
        let (bar, k) = ((n / STEPS) as u64, (n % STEPS) as u64);
        // El primer compás termina entero con el patrón y BPM viejos.
        let (expected, expected_level) = if bar == 0 {
            (step_offset(k, 120), LEVEL_A)
        } else {
            (
                first_bar + (bar - 1) * bar_len(170) + step_offset(k, 170),
                LEVEL_B,
            )
        };
        assert_eq!(frame, expected, "compás {bar}, paso {k}");
        assert_eq!(level, expected_level, "compás {bar}, paso {k}");
    }
}

#[test]
fn rejects_invalid_bpm_and_slice_numbers() {
    let (sample, slices) = two_slice_sample();
    let (mut controller, _engine) = new_engine(RATE);
    assert_eq!(controller.set_bpm(0.0), Err(SendError::InvalidBpm(0.0)));
    assert!(controller.set_bpm(f64::NAN).is_err());

    let mut steps = [None; STEPS];
    steps[5] = Some(2);
    assert!(matches!(
        Pattern::new(sample, &slices, steps),
        Err(SendError::NoSuchSlice { index: 2, count: 2 })
    ));
}
