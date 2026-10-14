//! Instrumento: notas con polifonía, ADSR y bucle sostenido.

use std::f64::consts::TAU;
use std::sync::Arc;

use engine::{
    Adsr, Instrument, MAX_VOICES, Pattern, Sample, SendError, Slices, Step, new_engine, offline,
};

const RATE: u32 = 48_000;
/// 120 BPM: 6000 frames por paso.
const STEP: usize = 6000;

fn sample_from(mono: &[f32]) -> Arc<Sample> {
    Arc::new(Sample::from_interleaved(mono, 1, RATE, RATE).unwrap())
}

fn sine(freq: f64, seconds: f64, amplitude: f64) -> Arc<Sample> {
    let frames = (seconds * RATE as f64) as usize;
    let mono: Vec<f32> = (0..frames)
        .map(|i| (amplitude * (TAU * freq * i as f64 / RATE as f64).sin()) as f32)
        .collect();
    sample_from(&mono)
}

fn pattern(sample: &Arc<Sample>, steps: Vec<Option<Step>>, instrument: Instrument) -> Pattern {
    let slices = Slices::grid(sample, 1);
    Pattern::from_steps(sample.clone(), &slices, steps)
        .unwrap()
        .with_instrument(Some(instrument))
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

/// Un compás con golpes en los pasos indicados.
fn steps(hits: &[(usize, Step)]) -> Vec<Option<Step>> {
    let mut steps = vec![None; 16];
    for &(i, step) in hits {
        steps[i] = Some(step);
    }
    steps
}

/// Frecuencia por los cruces por cero (interpolados) de `x`.
fn frequency(x: &[f32]) -> f64 {
    let crossings: Vec<f64> = x
        .windows(2)
        .enumerate()
        .filter(|(_, w)| (w[0] < 0.0) != (w[1] < 0.0))
        .map(|(i, w)| i as f64 + w[0] as f64 / (w[0] - w[1]) as f64)
        .collect();
    let (first, last) = (crossings[0], crossings[crossings.len() - 1]);
    (crossings.len() - 1) as f64 / 2.0 / ((last - first) / RATE as f64)
}

/// Amplitud de la componente a `freq` en `x` (correlación con seno y
/// coseno, con ventana de Hann para que no se cuelen las vecinas).
fn level(x: &[f32], freq: f64) -> f64 {
    let w = TAU * freq / RATE as f64;
    let n = x.len() as f64;
    let (mut c, mut s, mut sum) = (0.0, 0.0, 0.0);
    for (k, &v) in x.iter().enumerate() {
        let hann = 0.5 - 0.5 * (TAU * k as f64 / n).cos();
        c += hann * v as f64 * (w * k as f64).cos();
        s += hann * v as f64 * (w * k as f64).sin();
        sum += hann;
    }
    2.0 * (c * c + s * s).sqrt() / sum
}

fn adsr(attack_ms: f32, decay_ms: f32, sustain: f32, release_ms: f32) -> Adsr {
    Adsr {
        attack_ms,
        decay_ms,
        sustain,
        release_ms,
    }
}

/// Cada nota suena a la frecuencia de la raíz por 2^(semitonos/12).
#[test]
fn each_note_has_its_frequency() {
    let root = 220.0;
    let sample = sine(root, 2.0, 0.5);
    for semitones in [0.0f32, 4.0, 7.0, 12.0, -12.0, -5.0, 19.0] {
        let p = pattern(
            &sample,
            steps(&[(0, Step::new(0).with_pitch(semitones).with_hold(7))]),
            Instrument::new(4),
        );
        let out = render(p, 8 * STEP);
        let f = frequency(&out[2000..7 * STEP]);
        let expected = root * 2f64.powf(semitones as f64 / 12.0);
        assert!(
            (f / expected - 1.0).abs() < 0.001,
            "{semitones:+}: {f:.2} Hz, se esperaba {expected:.2}"
        );
    }
}

/// ADSR exacta sobre un sample constante: rampa de ataque, decay hasta el
/// sustain y release lineal desde el final del paso (y sus `_`).
#[test]
fn adsr_shapes_each_note() {
    let sample = sample_from(&vec![0.5; RATE as usize * 2]);
    let mut instrument = Instrument::new(1);
    // 10 ms de ataque (480 frames), 20 de decay (960), sustain 0,5 y 50 de
    // release (2400).
    instrument.adsr = adsr(10.0, 20.0, 0.5, 50.0);
    let p = pattern(
        &sample,
        steps(&[(0, Step::new(0).with_hold(1))]),
        instrument,
    );
    let out = render(p, 3 * STEP);
    let near = |i: usize, expected: f32| {
        assert!(
            (out[i] - expected).abs() < 1e-5,
            "frame {i}: {} (se esperaba {expected})",
            out[i]
        );
    };
    near(0, 0.0);
    near(240, 0.25);
    near(480, 0.5);
    near(480 + 480, 0.5 * 0.75);
    near(480 + 960, 0.25);
    near(5000, 0.25);
    // El paso y su `_`: 12000 frames; luego, 2400 de release.
    let gate = 2 * STEP;
    near(gate - 1, 0.25);
    near(gate + 1200, 0.125);
    near(gate + 2400 - 1, 0.25 / 2400.0);
    assert!(out[gate + 2400..].iter().all(|&s| s == 0.0));
}

/// Las notas no se cortan entre sí: dos que se solapan suenan como la
/// suma de cada una por separado.
#[test]
fn notes_overlap_without_cutting() {
    let sample = sine(220.0, 2.0, 0.2);
    let mut instrument = Instrument::new(4);
    instrument.adsr = adsr(5.0, 0.0, 1.0, 300.0);
    let a = Step::new(0).with_hold(3);
    let b = Step::new(0).with_pitch(7.0).with_hold(1);
    let both = render(
        pattern(&sample, steps(&[(0, a), (2, b)]), instrument),
        8 * STEP,
    );
    let only_a = render(pattern(&sample, steps(&[(0, a)]), instrument), 8 * STEP);
    let only_b = render(pattern(&sample, steps(&[(2, b)]), instrument), 8 * STEP);
    for i in 0..8 * STEP {
        let sum = only_a[i] + only_b[i];
        assert!(
            (both[i] - sum).abs() < 1e-6,
            "frame {i}: {} != {sum}",
            both[i]
        );
    }
    // Y de verdad se solapan: en el paso 2 suenan las dos.
    let overlap = &both[2 * STEP + 1000..4 * STEP];
    assert!(level(overlap, 220.0) > 0.15);
    assert!(level(overlap, 220.0 * 2f64.powf(7.0 / 12.0)) > 0.15);
}

/// Un acorde abre una voz por nota.
#[test]
fn a_chord_opens_one_voice_per_note() {
    let sample = sine(220.0, 2.0, 0.15);
    let chord = Step::new(0).with_chord(&[4.0, 7.0]).with_hold(3);
    let out = render(
        pattern(&sample, steps(&[(0, chord)]), Instrument::new(4)),
        4 * STEP,
    );
    let mid = &out[2000..4 * STEP - 1000];
    for st in [0.0, 4.0, 7.0] {
        let f = 220.0 * 2f64.powf(st / 12.0);
        assert!(level(mid, f) > 0.13, "{st}: {}", level(mid, f));
    }
}

/// Voces que suenan a la vez, bloque a bloque (del estado del motor).
fn voices_over_time(pattern: Pattern, blocks: usize) -> Vec<usize> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    let mut out = vec![0.0f32; 256 * 2];
    (0..blocks)
        .map(|_| {
            engine.process(&mut out, 2);
            controller.status().tracks[0].voices
        })
        .collect()
}

/// Nunca suenan más notas que voces, aunque cada nota dure mucho más que
/// su paso; con voces de sobra, sí suenan todas.
#[test]
fn voices_never_exceed_the_limit() {
    let sample = sine(220.0, 2.0, 0.05);
    let every_step: Vec<_> = (0..16)
        .map(|i| (i, Step::new(0).with_pitch(i as f32).with_hold(3)))
        .collect();
    for limit in [1u8, 2, 3, 4, 6] {
        let mut instrument = Instrument::new(limit);
        instrument.adsr = adsr(0.0, 0.0, 1.0, 2000.0);
        let counts = voices_over_time(pattern(&sample, steps(&every_step), instrument), 1500);
        let most = *counts.iter().max().unwrap();
        assert_eq!(most, limit as usize, "límite {limit}: {counts:?}");
    }
    // Con todas las voces, cada nota (4 pasos + 2 s de release) se solapa
    // con muchas: se llega al máximo.
    let mut instrument = Instrument::new(MAX_VOICES as u8);
    instrument.adsr = adsr(0.0, 0.0, 1.0, 2000.0);
    let counts = voices_over_time(pattern(&sample, steps(&every_step), instrument), 1500);
    assert_eq!(*counts.iter().max().unwrap(), MAX_VOICES);
}

/// Sin voces libres se roba la más antigua: deja de sonar (tras el fundido
/// mínimo) y las demás siguen.
#[test]
fn the_oldest_note_is_stolen() {
    let sample = sine(220.0, 3.0, 0.2);
    let mut instrument = Instrument::new(2);
    instrument.adsr = adsr(5.0, 0.0, 1.0, 50.0);
    let notes = [0.0f32, 4.0, 7.0];
    let hits: Vec<_> = notes
        .iter()
        .enumerate()
        .map(|(i, &st)| (2 * i, Step::new(0).with_pitch(st).with_hold(9)))
        .collect();
    let out = render(pattern(&sample, steps(&hits), instrument), 10 * STEP);
    let freq = |st: f32| 220.0 * 2f64.powf(st as f64 / 12.0);
    // Antes de la tercera nota suenan las dos primeras...
    let before = &out[2 * STEP + 1000..4 * STEP];
    assert!(level(before, freq(0.0)) > 0.15);
    assert!(level(before, freq(4.0)) > 0.15);
    // ...y después, la primera ya no (robada) y las otras dos sí.
    let after = &out[4 * STEP + 1000..8 * STEP];
    assert!(
        level(after, freq(0.0)) < 0.002,
        "{}",
        level(after, freq(0.0))
    );
    assert!(level(after, freq(4.0)) > 0.15);
    assert!(level(after, freq(7.0)) > 0.15);
    // El robo no hace clic: hace el fundido mínimo (1,5 ms).
    let jump = out[4 * STEP - 10..4 * STEP + 200]
        .windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0f32, f32::max);
    let slope = 3.0 * 0.2 * TAU * freq(7.0) / RATE as f64;
    assert!((jump as f64) < slope, "salto {jump}");
}

/// Mayor salto entre dos muestras seguidas.
fn max_jump(x: &[f32]) -> f32 {
    x.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// Una nota sostenida más larga que el sample suena en bucle; con el
/// fundido cruzado el salto no hace clic aunque el bucle no dure un número
/// entero de periodos. Sin fundido, sí (control).
#[test]
fn a_sustained_loop_does_not_click() {
    let (freq, amplitude) = (440.0, 0.5);
    let sample = sine(freq, 1.0, amplitude);
    // Del frame 9000 al 30017: 192,6 periodos, el salto cae a media onda.
    let sustain_loop = Some((9000, 30_017));
    // Una nota de 16 pasos en un patrón de 32 (no vuelve a sonar).
    let mut held = vec![None; 32];
    held[0] = Some(Step::new(0).with_hold(15));
    let slope = (amplitude * TAU * freq / RATE as f64) as f32;
    let mut instrument = Instrument::new(1);
    instrument.adsr = adsr(5.0, 0.0, 1.0, 100.0);
    instrument.sustain_loop = sustain_loop;
    instrument.crossfade = 960;
    // 16 pasos a 120 BPM: 2 s, el doble del sample.
    let out = render(pattern(&sample, held.clone(), instrument), 16 * STEP + 9600);
    let sustain = &out[1000..16 * STEP];
    println!(
        "salto máximo con fundido: {:.5} (pendiente del seno {slope:.5})",
        max_jump(sustain)
    );
    // Un clic sería un salto muy mayor que la pendiente del seno; el fundido
    // lineal entre dos fases distintas añade como mucho 2·amplitud/fundido.
    let bound = slope + (2.0 * amplitude / 960.0) as f32;
    assert!(
        max_jump(sustain) <= bound,
        "salto {} (seno {slope})",
        max_jump(sustain)
    );
    // Suena todo el rato (el bucle sigue tras el final del sample), y con el
    // nivel del seno (el fundido no lo hunde mucho).
    for second in sustain.chunks(4800) {
        let peak = second.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        assert!(peak > 0.85 * amplitude as f32, "pico {peak}");
    }
    // El release también sigue en bucle y acaba en silencio.
    assert!(out[16 * STEP + 4800 + 10..].iter().all(|&s| s == 0.0));

    instrument.crossfade = 0;
    let hard = render(pattern(&sample, held, instrument), 16 * STEP);
    println!("salto máximo sin fundido: {:.5}", max_jump(&hard[1000..]));
    assert!(
        max_jump(&hard[1000..]) > 5.0 * slope,
        "sin fundido debería haber un salto: {}",
        max_jump(&hard[1000..])
    );
}

/// Sin bucle la nota acaba con el sample (con el fundido mínimo, sin clic).
#[test]
fn without_a_loop_the_note_ends_with_the_sample() {
    let sample = sine(220.0, 0.1, 0.5);
    let out = render(
        pattern(
            &sample,
            steps(&[(0, Step::new(0).with_hold(15))]),
            Instrument::new(1),
        ),
        2 * STEP,
    );
    let frames = sample.frames();
    assert!(out[frames..].iter().all(|&s| s == 0.0));
    assert!(
        out[frames - 200..frames - 100]
            .iter()
            .any(|s| s.abs() > 0.1)
    );
}

/// Parar el secuenciador corta las notas (fundido mínimo); un patrón sin
/// instrumento sigue sonando como siempre (un golpe corta al anterior).
#[test]
fn stop_cuts_the_notes() {
    let sample = sine(220.0, 2.0, 0.3);
    let mut instrument = Instrument::new(4);
    instrument.adsr = adsr(0.0, 0.0, 1.0, 1000.0);
    let (mut controller, mut engine) = new_engine(RATE);
    controller
        .set_pattern(pattern(
            &sample,
            steps(&[(0, Step::new(0).with_hold(15))]),
            instrument,
        ))
        .unwrap();
    controller.start().unwrap();
    let playing = offline::render(&mut engine, STEP, 2);
    assert!(playing.iter().any(|s| s.abs() > 0.2));
    controller.stop().unwrap();
    let stopped = offline::render(&mut engine, 4800, 2);
    // 1,5 ms de fundido (72 frames) y silencio.
    assert!(stopped[2 * 80..].iter().all(|&s| s == 0.0));
}

#[test]
fn rejects_out_of_range_instruments() {
    let sample = sine(220.0, 0.5, 0.5);
    let frames = sample.frames();
    let check = |instrument: Instrument| {
        let slices = Slices::grid(&sample, 1);
        Pattern::from_steps(sample.clone(), &slices, steps(&[(0, Step::new(0))]))
            .unwrap()
            .with_instrument(Some(instrument))
            .err()
    };
    let invalid = |e: Option<SendError>| matches!(e, Some(SendError::InvalidInstrument(_)));
    let base = Instrument::new(4);
    assert!(check(base).is_none());
    assert!(invalid(check(Instrument::new(0))));
    assert!(invalid(check(Instrument::new(17))));
    let with = |f: fn(&mut Instrument)| {
        let mut i = base;
        f(&mut i);
        i
    };
    assert!(invalid(check(with(|i| i.adsr.sustain = 1.5))));
    assert!(invalid(check(with(|i| i.adsr.attack_ms = -1.0))));
    assert!(invalid(check(with(|i| i.adsr.release_ms = 20_000.0))));
    assert!(invalid(check(with(|i| i.sustain_loop = Some((100, 50))))));
    let past_end = frames + 1;
    let mut i = base;
    i.sustain_loop = Some((100, past_end));
    assert!(invalid(check(i)));
    // El fundido necesita audio antes del inicio y dentro del bucle.
    i.sustain_loop = Some((100, 1000));
    i.crossfade = 101;
    assert!(invalid(check(i)));
    i.crossfade = 100;
    assert!(check(i).is_none());
}

/// Notas en vivo (pads, MIDI de entrada): `note_on` arranca la envolvente
/// y suena hasta `note_off`, que dispara el release desde ese frame. Con
/// el secuenciador parado, también.
#[test]
fn live_notes_hold_until_their_note_off() {
    let sample = sample_from(&vec![0.5; RATE as usize * 3]);
    let slices = Slices::grid(&sample, 1);
    let range = slices.get(0).unwrap();
    let mut instrument = Instrument::new(4);
    // 10 ms de ataque (480), sustain 0,5 sin decay largo, 50 ms de release.
    instrument.adsr = adsr(10.0, 0.0, 0.5, 50.0);
    let (mut controller, mut engine) = new_engine(RATE);
    controller
        .note_on(0, &sample, range.clone(), instrument, 0.0, 1.0, 60)
        .unwrap();
    let held = left(&offline::render(&mut engine, 30_000, 2));
    assert!((held[240] - 0.25).abs() < 1e-5, "{}", held[240]);
    // Mucho más que cualquier paso: sigue en el sustain.
    assert!((held[29_999] - 0.25).abs() < 1e-6, "{}", held[29_999]);
    controller.note_off(0, 60).unwrap();
    let released = left(&offline::render(&mut engine, 4000, 2));
    // El release (2400 frames) empieza en el primer frame tras el note off.
    assert!((released[0] - 0.25).abs() < 1e-5);
    assert!((released[1200] - 0.125).abs() < 1e-5, "{}", released[1200]);
    assert!(released[2400..].iter().all(|&s| s == 0.0));
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

/// Cada tecla, su nota: soltar una no suelta las demás; la misma tecla
/// otra vez suelta la anterior; las voces en vivo también se roban.
#[test]
fn live_keys_are_independent_and_share_the_voices() {
    let sample = sine(220.0, 3.0, 0.15);
    let range = Slices::grid(&sample, 1).get(0).unwrap();
    let mut instrument = Instrument::new(2);
    instrument.adsr = adsr(2.0, 0.0, 1.0, 20.0);
    let freq = |st: f32| 220.0 * 2f64.powf(st as f64 / 12.0);
    let (mut controller, mut engine) = new_engine(RATE);
    let on = |c: &mut engine::Controller, st: f32, key: u8| {
        c.note_on(0, &sample, range.clone(), instrument, st, 1.0, key)
            .unwrap()
    };
    on(&mut controller, 0.0, 1);
    on(&mut controller, 7.0, 2);
    offline::render(&mut engine, 4800, 2);
    controller.note_off(0, 1).unwrap();
    offline::render(&mut engine, 4800, 2);
    let only_fifth = left(&offline::render(&mut engine, 9600, 2));
    assert!(level(&only_fifth, freq(0.0)) < 0.002);
    assert!(level(&only_fifth, freq(7.0)) > 0.13);
    // La misma tecla otra vez (con otra nota): la anterior se suelta.
    on(&mut controller, 12.0, 2);
    offline::render(&mut engine, 4800, 2);
    let octave = left(&offline::render(&mut engine, 9600, 2));
    assert!(level(&octave, freq(7.0)) < 0.002);
    assert!(level(&octave, freq(12.0)) > 0.13);
    // Dos voces: una tercera tecla roba la más antigua.
    on(&mut controller, 4.0, 3);
    on(&mut controller, 5.0, 4);
    offline::render(&mut engine, 4800, 2);
    let stolen = left(&offline::render(&mut engine, 9600, 2));
    assert!(level(&stolen, freq(12.0)) < 0.002);
    assert!(level(&stolen, freq(4.0)) > 0.13 && level(&stolen, freq(5.0)) > 0.13);
    assert_eq!(controller.status().tracks[0].voices, 2);
    // Un note off de una tecla que no suena no hace nada.
    controller.note_off(0, 99).unwrap();
    assert!(
        controller
            .note_on(9, &sample, range.clone(), instrument, 0.0, 1.0, 1)
            .is_err()
    );
    assert!(
        controller
            .note_on(0, &sample, range, instrument, 40.0, 1.0, 1)
            .is_err()
    );
}
