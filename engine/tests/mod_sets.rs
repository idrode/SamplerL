//! Modulación por golpe (`engine::modulation`): cada golpe con su mod set
//! arranca sus cadenas desde cero. Render offline; se comprueban
//! propiedades (tiempos, niveles, frecuencias), no huellas.

use std::sync::Arc;

use engine::modulation::{
    Ahdsr, CUTOFF, Chain, Device, DeviceKind, ModSet, ModTable, ModValue, Op, PAN, PITCH, VOL,
};
use engine::notation::parse_steps;
use engine::{Batch, Pattern, Sample, Slices, Step, detect_mono, new_engine, offline};

const RATE: u32 = 48_000;
/// Un paso a 120 BPM.
const STEP: usize = 6000;

fn ms(ms: f32) -> usize {
    (ms / 1000.0 * RATE as f32) as usize
}

/// Un sample de `frames` frames de valor constante.
fn dc(level: f32, frames: usize) -> (Arc<Sample>, Slices) {
    let sample = Arc::new(Sample::from_interleaved(&vec![level; frames], 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    (sample, slices)
}

fn sine(freq: f32, frames: usize, amp: f32) -> (Arc<Sample>, Slices) {
    let mono: Vec<f32> = (0..frames)
        .map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / RATE as f32).sin())
        .collect();
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    (sample, slices)
}

fn ahdsr(a: f32, h: f32, d: f32, s: f32, r: f32) -> Device {
    Device::new(DeviceKind::Ahdsr(Ahdsr {
        attack: ModValue::fixed(a),
        hold: ModValue::fixed(h),
        decay: ModValue::fixed(d),
        sustain: ModValue::fixed(s),
        release: ModValue::fixed(r),
        curves: [0.0; 3],
    }))
}

fn set_with(domain: usize, devices: &[Device]) -> ModSet {
    let mut chain = Chain::new(domain);
    for d in devices {
        chain = chain.with_device(*d).unwrap();
    }
    let mut set = ModSet::default();
    set.chains[domain] = Some(chain);
    set
}

fn table(sets: &[ModSet]) -> ModTable {
    let mut t = ModTable::new();
    for (i, s) in sets.iter().enumerate() {
        t.set(i, Some(*s)).unwrap();
    }
    t
}

/// Renderiza `frames` con los patrones de cada pista y la tabla `t`.
fn render(patterns: Vec<(usize, Pattern)>, t: Option<ModTable>, frames: usize) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    let mut batch = Batch::new();
    for (track, p) in patterns {
        batch.set_pattern(track, Some(p));
    }
    if let Some(t) = t {
        batch.set_mod_sets(t);
    }
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
}

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

/// Un golpe al principio (el paso dura `STEP` frames).
fn one_hit(sample: Arc<Sample>, slices: &Slices, set: Option<u8>) -> Pattern {
    let steps = parse_steps("1 . . .  . . . .  . . . .  . . . .").unwrap();
    Pattern::from_steps(sample, slices, steps)
        .unwrap()
        .with_mod_sets(set, &[])
        .unwrap()
}

#[test]
fn ahdsr_reaches_its_levels_on_time_and_releases_at_the_end_of_the_step() {
    let (sample, slices) = dc(0.5, RATE as usize);
    let env = ahdsr(10.0, 5.0, 20.0, 0.5, 30.0);
    let out = left(&render(
        vec![(0, one_hit(sample, &slices, Some(0)))],
        Some(table(&[set_with(VOL, &[env])])),
        ms(400.0),
    ));
    let at = |t: f32| out[ms(t)];
    let close = |t: f32, want: f32| {
        assert!(
            (at(t) - want).abs() < 2e-3,
            "{t} ms: {} (quería {want})",
            at(t)
        );
    };
    // El fundido de entrada de la voz dura 5 ms; luego, la envolvente.
    close(7.5, 0.5 * 0.75);
    close(10.0, 0.5);
    close(13.0, 0.5);
    close(25.0, 0.5 * 0.75);
    // En los vértices, la interpolación a tasa de control (32 frames, 0,7
    // ms) los redondea: se mira justo después.
    close(36.0, 0.25);
    close(100.0, 0.25);
    // El note-off al acabar el paso (125 ms) y 30 ms de release.
    close(124.0, 0.25);
    close(140.0, 0.125);
    close(160.0, 0.0);
    close(300.0, 0.0);
}

#[test]
#[allow(irrefutable_let_patterns)]
fn curves_bend_the_segments() {
    let (sample, slices) = dc(0.5, RATE as usize);
    let mut a = ahdsr(0.0, 0.0, 100.0, 0.0, 10.0);
    if let DeviceKind::Ahdsr(e) = &mut a.kind {
        e.curves[1] = 1.0;
    }
    let out = left(&render(
        vec![(0, one_hit(sample, &slices, Some(0)))],
        Some(table(&[set_with(VOL, &[a])])),
        ms(120.0),
    ));
    // Con curva, a mitad del decay ya ha bajado más de la mitad.
    assert!(out[ms(55.0)] < 0.5 * 0.4, "{}", out[ms(55.0)]);
    assert!(out[ms(55.0)] > 0.0);
}

#[test]
fn operators_bypass_amount_and_polarity() {
    let (sample, slices) = dc(0.25, RATE as usize);
    // Sustain 1 desde el principio: el dispositivo da 1.
    let one = ahdsr(0.0, 0.0, 0.0, 1.0, 0.0);
    let half = Device {
        amount: ModValue::fixed(0.5),
        ..one
    };
    let level = |set: ModSet| {
        let out = left(&render(
            vec![(0, one_hit(sample.clone(), &slices, Some(0)))],
            Some(table(&[set])),
            ms(60.0),
        ));
        out[ms(50.0)] / 0.25
    };
    let close = |got: f32, want: f32| assert!((got - want).abs() < 1e-4, "{got} {want}");
    // Volumen: empieza en 1 y multiplica.
    close(level(set_with(VOL, &[half])), 0.5);
    close(level(set_with(VOL, &[half, half])), 0.25);
    let add = Device {
        op: Some(Op::Add),
        ..half
    };
    close(level(set_with(VOL, &[half, add])), 1.0);
    let sub = Device {
        op: Some(Op::Sub),
        ..half
    };
    close(level(set_with(VOL, &[sub])), 0.5);
    let div = Device {
        op: Some(Op::Div),
        ..half
    };
    close(level(set_with(VOL, &[div])), 2.0);
    // Apagado, como si no estuviera.
    let off = Device {
        bypass: true,
        ..half
    };
    close(level(set_with(VOL, &[half, off])), 0.5);
    // Bipolar: 1 sigue siendo 1; con sustain 0, -1 (y el volumen no baja
    // de 0).
    let low = Device {
        bipolar: true,
        op: Some(Op::Add),
        ..ahdsr(0.0, 0.0, 0.0, 0.0, 0.0)
    };
    close(level(set_with(VOL, &[low])), 0.0);
}

#[test]
fn without_a_set_the_hit_sounds_bit_for_bit_as_before() {
    let (sample, slices) = sine(330.0, RATE as usize, 0.3);
    let steps = parse_steps("1 . 1 .  . . . .  1 . . .  . . . .").unwrap();
    let plain = Pattern::from_steps(sample.clone(), &slices, steps).unwrap();
    let with = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_mod_sets(None, &[])
        .unwrap();
    // Con una tabla cargada que esta pista no usa, tampoco cambia nada.
    let set = set_with(VOL, &[ahdsr(10.0, 0.0, 10.0, 0.2, 10.0)]);
    let a = render(vec![(0, plain)], None, RATE as usize);
    let b = render(vec![(0, with)], Some(table(&[set])), RATE as usize);
    assert!(a.iter().zip(&b).all(|(x, y)| x.to_bits() == y.to_bits()));
}

#[test]
fn each_hit_has_its_own_state() {
    // Dos pistas con el mismo set, la segunda dos pasos más tarde: juntas
    // suenan como cada una por su lado (cada golpe empieza su envolvente).
    let (sample, slices) = dc(0.1, RATE as usize);
    let set = set_with(VOL, &[ahdsr(30.0, 0.0, 50.0, 0.3, 20.0)]);
    let a_steps = parse_steps("1 . . .  . . . .  . . . .  . . . .").unwrap();
    let b_steps = parse_steps(". . 1 .  . . . .  . . . .  . . . .").unwrap();
    let make = |steps| {
        Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_mod_sets(Some(0), &[])
            .unwrap()
    };
    let both = render(
        vec![(0, make(a_steps)), (1, make(b_steps))],
        Some(table(&[set])),
        4 * STEP,
    );
    let a = render(vec![(0, make(a_steps))], Some(table(&[set])), 4 * STEP);
    let b = render(vec![(1, make(b_steps))], Some(table(&[set])), 4 * STEP);
    for i in 0..both.len() {
        assert!((both[i] - (a[i] + b[i])).abs() < 1e-6, "frame {i}");
    }
    // Y la segunda es la primera dos pasos más tarde.
    let (la, lb) = (left(&a), left(&b));
    for i in 0..STEP {
        assert!((la[i] - lb[i + 2 * STEP]).abs() < 1e-6, "frame {i}");
    }
}

#[test]
fn pitch_moves_the_frequency() {
    let (sample, slices) = sine(440.0, RATE as usize, 0.3);
    // +1 en el pitch con 12 de rango: una octava arriba.
    let up = set_with(PITCH, &[ahdsr(0.0, 0.0, 0.0, 1.0, 0.0)]);
    let out = left(&render(
        vec![(0, one_hit(sample.clone(), &slices, Some(0)))],
        Some(table(&[up])),
        ms(100.0),
    ));
    let f = detect_mono(&out[ms(20.0)..ms(100.0)], RATE).unwrap().hz;
    assert!((f / 880.0 - 1.0).abs() < 0.005, "{f}");
    // Con 7 de rango y la mitad: 3,5 semitonos.
    let mut fifth = up;
    fifth.pitch_range = 7.0;
    fifth.chains[PITCH].as_mut().unwrap().devices[0]
        .as_mut()
        .unwrap()
        .amount = ModValue::fixed(0.5);
    let out = left(&render(
        vec![(0, one_hit(sample.clone(), &slices, Some(0)))],
        Some(table(&[fifth])),
        ms(100.0),
    ));
    let f = detect_mono(&out[ms(20.0)..ms(100.0)], RATE).unwrap().hz;
    let want = 440.0 * 2f32.powf(3.5 / 12.0);
    assert!((f / want - 1.0).abs() < 0.005, "{f} {want}");
    // Al subir, el corte se acaba antes: a ×2 hasta el note-off (125 ms,
    // 250 ms de sample) y luego, sin release, a ×1 lo que queda: 875 ms.
    let long = left(&render(
        vec![(0, one_hit(sample, &slices, Some(0)))],
        Some(table(&[up])),
        RATE as usize,
    ));
    let last = long.iter().rposition(|x| x.abs() > 1e-4).unwrap();
    assert!(
        (last as i64 - ms(875.0) as i64).abs() < ms(2.0) as i64,
        "{last}"
    );
}

#[test]
fn cutoff_filters_and_never_gives_nan() {
    let mut x = 0x1234_5678u32;
    let noise: Vec<f32> = (0..RATE as usize)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            0.5 * (x as f32 / u32::MAX as f32 * 2.0 - 1.0)
        })
        .collect();
    let sample = Arc::new(Sample::from_interleaved(&noise, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    let rms = |v: &[f32]| (v.iter().map(|s| s * s).sum::<f32>() / v.len() as f32).sqrt();
    let open = left(&render(
        vec![(0, one_hit(sample.clone(), &slices, None))],
        None,
        ms(100.0),
    ));
    // Corte a 0,3 (unos 160 Hz): el ruido baja mucho.
    let mut low = set_with(CUTOFF, &[ahdsr(0.0, 0.0, 0.0, 1.0, 0.0)]);
    low.chains[CUTOFF].as_mut().unwrap().input = 0.3;
    let shut = left(&render(
        vec![(0, one_hit(sample.clone(), &slices, Some(0)))],
        Some(table(&[low])),
        ms(100.0),
    ));
    assert!(rms(&shut[ms(20.0)..]) < 0.15 * rms(&open[ms(20.0)..]));
    // Extremos: el corte de 0 a todo en 1 ms, una y otra vez (cada golpe
    // un paso), y por encima de Nyquist: finito.
    let mut wild = set_with(CUTOFF, &[ahdsr(1.0, 0.0, 1.0, 0.0, 1.0)]);
    wild.chains[CUTOFF].as_mut().unwrap().devices[0]
        .as_mut()
        .unwrap()
        .amount = ModValue::fixed(64.0);
    let steps = parse_steps("1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1").unwrap();
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_mod_sets(Some(0), &[])
        .unwrap();
    let out = render(vec![(0, pattern)], Some(table(&[wild])), RATE as usize);
    assert!(out.iter().all(|x| x.is_finite()));
}

#[test]
fn pan_moves_the_hit() {
    let (sample, slices) = dc(0.5, RATE as usize);
    let right = set_with(PAN, &[ahdsr(0.0, 0.0, 0.0, 1.0, 0.0)]);
    let out = render(
        vec![(0, one_hit(sample, &slices, Some(0)))],
        Some(table(&[right])),
        ms(50.0),
    );
    let (l, r) = (out[2 * ms(40.0)], out[2 * ms(40.0) + 1]);
    assert!(l.abs() < 1e-6 && (r - 0.5).abs() < 1e-4, "{l} {r}");
}

#[test]
fn a_new_table_leaves_sounding_hits_alone() {
    // Un golpe largo con el volumen a la mitad; en el compás siguiente
    // entra otra tabla con el volumen entero: el golpe que suena sigue con
    // su mitad, sin saltos; el siguiente golpe ya suena entero.
    let (sample, slices) = dc(0.4, 4 * RATE as usize);
    let half = set_with(VOL, &[ahdsr(0.0, 0.0, 0.0, 0.5, 0.0)]);
    let full = set_with(VOL, &[ahdsr(0.0, 0.0, 0.0, 1.0, 0.0)]);
    let steps = parse_steps("1 . . .  . . . .  . . . .  . . . .").unwrap();
    let make = || {
        Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_mod_sets(Some(0), &[])
            .unwrap()
    };
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    let mut batch = Batch::new();
    batch
        .set_pattern(0, Some(make()))
        .set_mod_sets(table(&[half]));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    // Con el paso de 125 ms, el note-off llega pronto, pero sin release el
    // sustain cae a 0: un set sin note-off que corte.
    let first = left(&offline::render(&mut engine, 16 * STEP - 1000, 2));
    assert!((first[ms(50.0)] - 0.2).abs() < 1e-4);
    let mut batch = Batch::new();
    batch.set_mod_sets(table(&[full]));
    controller.apply(batch).unwrap();
    let rest = left(&offline::render(&mut engine, 4 * STEP, 2));
    // El compás nuevo empieza a los 1000 frames: el golpe nuevo, entero.
    assert!(
        (rest[1000 + ms(50.0)] - 0.4).abs() < 1e-4,
        "{}",
        rest[1000 + ms(50.0)]
    );
    assert!(rest.iter().chain(&first).all(|x| x.is_finite()));
}

#[test]
fn step_and_slice_sets_win_over_the_track_one() {
    let (sample, slices) = dc(0.5, RATE as usize);
    let quiet = set_with(VOL, &[ahdsr(0.0, 0.0, 0.0, 0.25, 0.0)]);
    let loud = set_with(VOL, &[ahdsr(0.0, 0.0, 0.0, 0.75, 0.0)]);
    let mut steps = [None; 16];
    steps[0] = Some(Step::new(0));
    steps[4] = Some(Step::new(0).with_mod(Some(1)));
    let p = Pattern::from_steps(sample.clone(), &slices, steps)
        .unwrap()
        .with_mod_sets(Some(0), &[])
        .unwrap();
    let out = left(&render(vec![(0, p)], Some(table(&[quiet, loud])), 6 * STEP));
    assert!((out[ms(50.0)] - 0.125).abs() < 1e-4);
    assert!((out[4 * STEP + ms(50.0)] - 0.375).abs() < 1e-4);
    // El del corte, sobre el de la pista.
    let p = one_hit(sample, &slices, Some(0))
        .with_mod_sets(Some(0), &[Some(1)])
        .unwrap();
    let out = left(&render(vec![(0, p)], Some(table(&[quiet, loud])), ms(60.0)));
    assert!((out[ms(50.0)] - 0.375).abs() < 1e-4);
}

fn synth_pattern(steps: &str, patch: engine::synth::Patch, set: Option<u8>) -> Pattern {
    let (silence, one) = dc(0.0, 1000);
    Pattern::from_steps(silence, &one, parse_steps(steps).unwrap())
        .unwrap()
        .with_synth(Some(patch), [None; engine::synth::PARAM_COUNT])
        .unwrap()
        .with_mod_sets(set, &[])
        .unwrap()
}

#[test]
fn a_synth_note_follows_its_set() {
    let mut patch = engine::synth::sub::patch(48.0);
    patch.amp.attack_ms = 0.5;
    patch.amp.sustain = 1.0;
    let steps = "1 . . .  . . . .  . . . .  . . . .";
    let plain = left(&render(
        vec![(0, synth_pattern(steps, patch, None))],
        None,
        ms(400.0),
    ));
    // Volumen con ataque de 100 ms: a los 50 ms, la mitad que sin él.
    let slow = set_with(VOL, &[ahdsr(100.0, 0.0, 0.0, 1.0, 10.0)]);
    let out = left(&render(
        vec![(0, synth_pattern(steps, patch, Some(0)))],
        Some(table(&[slow])),
        ms(400.0),
    ));
    let peak = |v: &[f32]| v.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    let ratio = peak(&out[ms(45.0)..ms(55.0)]) / peak(&plain[ms(45.0)..ms(55.0)]);
    assert!((ratio - 0.5).abs() < 0.05, "{ratio}");
    // Pitch: una octava arriba.
    let up = set_with(PITCH, &[ahdsr(0.0, 0.0, 0.0, 1.0, 0.0)]);
    let out = left(&render(
        vec![(0, synth_pattern(steps, patch, Some(0)))],
        Some(table(&[up])),
        ms(120.0),
    ));
    let f = detect_mono(&out[ms(20.0)..ms(120.0)], RATE).unwrap().hz;
    let want = 440.0 * 2f32.powf((60.0 - 69.0) / 12.0);
    assert!((f / want - 1.0).abs() < 0.01, "{f} {want}");
}

#[test]
fn legato_keeps_the_running_set() {
    // Un mono con slide: la nota ligada no vuelve a empezar el ataque.
    let mut patch = engine::synth::acid::patch(36.0);
    patch.amp.sustain = 1.0;
    let slow = set_with(VOL, &[ahdsr(200.0, 0.0, 0.0, 1.0, 10.0)]);
    let out = left(&render(
        vec![(
            0,
            synth_pattern("1 _ 1/ _  . . . .  . . . .  . . . .", patch, Some(0)),
        )],
        Some(table(&[slow])),
        4 * STEP,
    ));
    let peak = |v: &[f32]| v.iter().fold(0.0f32, |m, x| m.max(x.abs()));
    // Justo después de la segunda nota (a los 250 ms), sigue subiendo:
    // no vuelve a 0.
    let before = peak(&out[2 * STEP - ms(10.0)..2 * STEP]);
    let after = peak(&out[2 * STEP..2 * STEP + ms(10.0)]);
    assert!(after > 0.8 * before, "{before} {after}");
}

#[test]
fn ranges_are_validated() {
    let bad = [
        ahdsr(-1.0, 0.0, 0.0, 0.5, 0.0),
        ahdsr(0.0, 0.0, 0.0, 1.5, 0.0),
        ahdsr(0.0, f32::NAN, 0.0, 0.5, 0.0),
        ahdsr(0.0, 0.0, 70_000.0, 0.5, 0.0),
        Device {
            amount: ModValue::fixed(f32::INFINITY),
            ..ahdsr(0.0, 0.0, 0.0, 0.5, 0.0)
        },
    ];
    for d in bad {
        assert!(d.validate().is_err(), "{d:?}");
    }
    let mut set = set_with(VOL, &[ahdsr(1.0, 0.0, 1.0, 0.5, 1.0)]);
    set.pitch_range = 100.0;
    assert!(set.validate().is_err());
    assert!(ModTable::new().set(16, None).is_err());
}
