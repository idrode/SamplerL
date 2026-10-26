//! Expresión por nota (fase Q, MPE): el bend, la presión y el slide de
//! cada nota en vivo mueven lo que el patch les asigna, solo en su voz,
//! suavizados y sin clics; sin asignar nada, todo suena como siempre.

use engine::synth::{
    AXES, BEND, CUTOFF, ENV_AMOUNT, ExprMap, ExprTarget, FilterMode, GLIDE, PRESSURE, Patch,
    RESONANCE, SLIDE, Wave, sh101,
};
use engine::{Controller, Engine, new_engine, offline};

const RATE: u32 = 48_000;
/// Una nota sola: medio segundo.
const HALF: usize = 24_000;

/// Un seno casi limpio (paso alto a 20 Hz, sin envolvente de filtro), con
/// la fase fija (los renders se pueden sumar) y raíz en el la de 440 Hz.
fn sine() -> Patch {
    let mut p = Patch::new()
        .with(CUTOFF, 20.0)
        .with(RESONANCE, 0.0)
        .with(ENV_AMOUNT, 0.0);
    p.wave = Wave::Sine;
    p.filter = FilterMode::Highpass;
    p.root = 69.0;
    p.zero_phase = true;
    p.amp.sustain = 1.0;
    p
}

fn map(target: ExprTarget, range: f32, curve: f32) -> Option<ExprMap> {
    Some(ExprMap {
        target,
        range,
        curve,
    })
}

/// Bend al pitch (12 semitonos por octava de bend: lo que manda el
/// controlador, tal cual).
fn bending(mut p: Patch) -> Patch {
    p.expr[BEND] = map(ExprTarget::Pitch, 12.0, 1.0);
    p
}

fn left(stereo: &[f32]) -> Vec<f32> {
    stereo.iter().step_by(2).copied().collect()
}

/// Frecuencia por los cruces por cero que suben.
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

/// Lo más que salta de una muestra a la siguiente.
fn max_step(y: &[f32]) -> f32 {
    y.windows(2)
        .map(|w| (w[1] - w[0]).abs())
        .fold(0.0, f32::max)
}

/// Lo agudo de un sonido: el RMS de su derivada entre su RMS.
fn brightness(y: &[f32]) -> f32 {
    let d: Vec<f32> = y.windows(2).map(|w| w[1] - w[0]).collect();
    rms(&d) / rms(y)
}

fn engine() -> (Controller, Engine) {
    new_engine(RATE)
}

/// Notas `(tecla, semitonos, expresión inicial)` en vivo, `frames` de
/// audio (izquierda).
fn play(patch: Patch, notes: &[(u8, f32, [f32; AXES])], frames: usize) -> Vec<f32> {
    let (mut controller, mut engine) = engine();
    for &(key, semitones, expr) in notes {
        controller
            .synth_note_on_expr(0, patch, semitones, 0.5, key, expr)
            .unwrap();
    }
    left(&offline::render(&mut engine, frames, 2))
}

#[test]
fn the_initial_bend_sounds_from_the_start_of_the_note() {
    // El bend que el canal ya traía al pulsar: +7 semitonos, sin subir.
    let audio = play(bending(sine()), &[(60, 0.0, [7.0, 0.0, 0.0])], HALF);
    let fifth = 440.0 * (7.0f64 / 12.0).exp2();
    let early = frequency(&audio[480..4800]);
    assert!((early / fifth - 1.0).abs() < 0.005, "{early:.1} Hz");
}

#[test]
fn a_bend_moves_only_the_voice_of_its_key() {
    // Flojo: la suma no llega al limitador del master (que no es lineal).
    let patch = bending(sine());
    let none = [0.0; AXES];
    let both = {
        let (mut controller, mut engine) = engine();
        controller
            .synth_note_on_expr(0, patch, 0.0, 0.2, 60, none)
            .unwrap();
        controller
            .synth_note_on_expr(0, patch, 3.0, 0.2, 63, none)
            .unwrap();
        let mut audio = left(&offline::render(&mut engine, 4800, 2));
        // Un tono arriba en la tecla 60; la 63 sigue igual.
        controller.note_mod(0, 60, BEND, 2.0).unwrap();
        audio.extend(left(&offline::render(&mut engine, HALF, 2)));
        audio
    };
    let alone = |key: u8, semitones: f32, bend: f32| {
        let (mut controller, mut engine) = engine();
        controller
            .synth_note_on_expr(0, patch, semitones, 0.2, key, none)
            .unwrap();
        let mut audio = left(&offline::render(&mut engine, 4800, 2));
        controller.note_mod(0, key, BEND, bend).unwrap();
        audio.extend(left(&offline::render(&mut engine, HALF, 2)));
        audio
    };
    let a = alone(60, 0.0, 2.0);
    let b = alone(63, 3.0, 0.0);
    // Las dos voces juntas suenan como cada una por su lado.
    for (i, (y, (x, z))) in both.iter().zip(a.iter().zip(&b)).enumerate() {
        assert!((y - (x + z)).abs() < 1e-5, "muestra {i}");
    }
    let tone = 440.0 * (2.0f64 / 12.0).exp2();
    let late = frequency(&a[4800 + 4800..]);
    assert!((late / tone - 1.0).abs() < 0.005, "{late:.1} Hz");
    let c = 440.0 * (3.0f64 / 12.0).exp2();
    assert!((frequency(&b[4800..]) / c - 1.0).abs() < 0.005);
}

#[test]
fn a_bend_glides_without_clicks() {
    let (mut controller, mut engine) = engine();
    let patch = bending(sine());
    controller
        .synth_note_on_expr(0, patch, 0.0, 0.5, 60, [0.0; AXES])
        .unwrap();
    let mut audio = left(&offline::render(&mut engine, 4800, 2));
    let calm = max_step(&audio[2400..]);
    // Un salto de una octava de golpe (lo peor que puede llegar).
    controller.note_mod(0, 60, BEND, 12.0).unwrap();
    audio.extend(left(&offline::render(&mut engine, HALF, 2)));
    // Al doble de frecuencia, el paso entre muestras es el doble: nada
    // más que eso.
    assert!(max_step(&audio[4800..]) < 2.2 * calm);
    assert!((frequency(&audio[4800 + 4800..]) / 880.0 - 1.0).abs() < 0.005);
}

#[test]
fn pressure_opens_the_filter_of_its_note() {
    let mut patch = Patch::new()
        .with(CUTOFF, 300.0)
        .with(ENV_AMOUNT, 0.0)
        .with(RESONANCE, 0.0);
    patch.amp.sustain = 1.0;
    patch.zero_phase = true;
    patch.expr[PRESSURE] = map(ExprTarget::Param(CUTOFF), 4.0, 1.0);
    let at = |pressure: f32| {
        let audio = play(patch, &[(48, 0.0, [0.0, pressure, 0.0])], HALF);
        brightness(&audio[4800..])
    };
    let (soft, hard) = (at(0.0), at(1.0));
    assert!(hard > 2.0 * soft, "presión 0: {soft}, 1: {hard}");
    // La curva: con 2, la mitad de presión abre menos que con 1.
    let mut curved = patch;
    curved.expr[PRESSURE] = map(ExprTarget::Param(CUTOFF), 4.0, 2.0);
    let half = |p: Patch| {
        let audio = play(p, &[(48, 0.0, [0.0, 0.5, 0.0])], HALF);
        brightness(&audio[4800..])
    };
    assert!(half(curved) < half(patch));
}

#[test]
fn pressure_to_gain_follows_the_finger_without_clicks() {
    let mut patch = sine();
    patch.expr[PRESSURE] = map(ExprTarget::Gain, 1.0, 1.0);
    let (mut controller, mut engine) = engine();
    controller
        .synth_note_on_expr(0, patch, 0.0, 0.5, 60, [0.0, 0.0, 0.0])
        .unwrap();
    let silent = left(&offline::render(&mut engine, 4800, 2));
    assert!(rms(&silent) < 1e-6, "sin presión, calla");
    // Presión a tope de golpe: sube suavizada.
    controller.note_mod(0, 60, PRESSURE, 1.0).unwrap();
    let rise = left(&offline::render(&mut engine, HALF, 2));
    let full = rms(&rise[HALF / 2..]);
    assert!(full > 0.3, "{full}");
    // El paso más grande, poco más que el del seno a ese nivel.
    let sine_step = 2.0 * std::f32::consts::PI * 440.0 / RATE as f32 * 0.5;
    assert!(max_step(&rise) < 1.3 * sine_step, "{}", max_step(&rise));
    // Con `range` 0,5, sin presión suena a la mitad.
    patch.expr[PRESSURE] = map(ExprTarget::Gain, 0.5, 1.0);
    let half = play(patch, &[(60, 0.0, [0.0, 0.0, 0.0])], HALF);
    let ratio = rms(&half[HALF / 2..]) / full;
    assert!((ratio - 0.5).abs() < 0.02, "{ratio}");
}

#[test]
fn slide_moves_its_parameter_and_unmapped_axes_do_nothing() {
    let mut patch = Patch::new().with(CUTOFF, 400.0).with(ENV_AMOUNT, 0.0);
    patch.amp.sustain = 1.0;
    patch.zero_phase = true;
    let plain = play(patch, &[(48, 0.0, [0.0; AXES])], HALF);
    // Sin asignar, los ejes no cambian nada (ni un bit).
    let (mut controller, mut engine) = engine();
    controller
        .synth_note_on_expr(0, patch, 0.0, 0.5, 48, [5.0, 1.0, 1.0])
        .unwrap();
    controller.note_mod(0, 48, SLIDE, 0.3).unwrap();
    let ignored = left(&offline::render(&mut engine, HALF, 2));
    assert_eq!(plain, ignored);
    // Asignado al corte, el slide lo abre.
    patch.expr[SLIDE] = map(ExprTarget::Param(CUTOFF), 3.0, 1.0);
    let open = play(patch, &[(48, 0.0, [0.0, 0.0, 1.0])], HALF);
    assert!(brightness(&open[4800..]) > 1.5 * brightness(&plain[4800..]));
}

#[test]
fn the_mono_sh101_bends_too() {
    let mut patch = sh101::patch(57.0);
    patch.expr[BEND] = map(ExprTarget::Pitch, 12.0, 1.0);
    let (mut controller, mut engine) = engine();
    controller
        .synth_note_on_expr(0, patch, 0.0, 0.8, 57, [0.0; AXES])
        .unwrap();
    offline::render(&mut engine, 4800, 2);
    controller.note_mod(0, 57, BEND, 12.0).unwrap();
    let audio = left(&offline::render(&mut engine, HALF, 2));
    // La fundamental de la sierra filtrada: cuenta los cruces del grave.
    let f = frequency(&audio[HALF / 2..]);
    assert!((f / 440.0 - 1.0).abs() < 0.03, "{f:.1} Hz");
}

#[test]
fn notes_without_expression_ignore_the_mapping() {
    // Presión al volumen: una nota MIDI sin presión calla, pero un pad del
    // teclado (sin expresión) suena como siempre, y no le llega nada.
    let mut patch = sine();
    let plain = {
        let (mut controller, mut engine) = engine();
        controller.synth_note_on(0, patch, 0.0, 0.5, 60).unwrap();
        left(&offline::render(&mut engine, HALF, 2))
    };
    patch.expr[PRESSURE] = map(ExprTarget::Gain, 1.0, 1.0);
    patch.expr[BEND] = map(ExprTarget::Pitch, 12.0, 1.0);
    let (mut controller, mut engine) = engine();
    controller.synth_note_on(0, patch, 0.0, 0.5, 60).unwrap();
    controller.note_mod(0, 60, BEND, 12.0).unwrap();
    let pad = left(&offline::render(&mut engine, HALF, 2));
    assert_eq!(plain, pad);
}

#[test]
fn bad_expression_is_an_error() {
    let (mut controller, _engine) = engine();
    assert!(controller.note_mod(0, 60, BEND, 97.0).is_err());
    assert!(controller.note_mod(0, 60, PRESSURE, 1.5).is_err());
    assert!(controller.note_mod(0, 60, AXES, 0.0).is_err());
    let mut patch = sine();
    patch.expr[PRESSURE] = map(ExprTarget::Param(GLIDE), 1.0, 1.0);
    assert!(controller.synth_note_on(0, patch, 0.0, 1.0, 60).is_err());
    patch.expr[PRESSURE] = map(ExprTarget::Gain, 2.0, 1.0);
    assert!(controller.synth_note_on(0, patch, 0.0, 1.0, 60).is_err());
    patch.expr[PRESSURE] = map(ExprTarget::Pitch, 2.0, 0.0);
    assert!(controller.synth_note_on(0, patch, 0.0, 1.0, 60).is_err());
}
