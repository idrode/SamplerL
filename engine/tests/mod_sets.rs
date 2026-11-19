//! Modulación por golpe (`engine::modulation`): cada golpe con su mod set
//! arranca sus cadenas desde cero. Render offline; se comprueban
//! propiedades (tiempos, niveles, frecuencias), no huellas.

use std::sync::Arc;

use engine::modulation::{
    Ahdsr, CUTOFF, Chain, Device, DeviceKind, EnvLength, EnvLoop, Fader, FaderScale, KeyTrack,
    ModLfo, ModSet, ModTable, ModValue, ModWave, Op, PAN, PITCH, PointEnv, Stepper, VOL,
    VelocityTrack,
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

fn table(sets: &[ModSet]) -> Box<ModTable> {
    let mut t = ModTable::new();
    for (i, s) in sets.iter().enumerate() {
        t.set(i, Some(*s)).unwrap();
    }
    t
}

/// Renderiza `frames` con los patrones de cada pista y la tabla `t`.
fn render(patterns: Vec<(usize, Pattern)>, t: Option<Box<ModTable>>, frames: usize) -> Vec<f32> {
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
fn curves_bend_the_segments() {
    let (sample, slices) = dc(0.5, RATE as usize);
    let mut a = ahdsr(0.0, 0.0, 100.0, 0.0, 10.0);
    let DeviceKind::Ahdsr(e) = &mut a.kind else {
        unreachable!()
    };
    e.curves[1] = 1.0;
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

fn lfo(wave: ModWave, hz: f32, f: impl FnOnce(&mut ModLfo)) -> Device {
    let mut l = ModLfo {
        wave,
        rate: ModValue::fixed(hz),
        ..ModLfo::DEFAULT
    };
    f(&mut l);
    Device {
        bipolar: true,
        ..Device::new(DeviceKind::Lfo(l))
    }
}

/// El volumen (de 0 a 1) que da un set de un solo golpe sobre un sample
/// constante, en cada frame.
fn vol_curve(set: ModSet, frames: usize) -> Vec<f32> {
    let (sample, slices) = dc(0.5, 2 * RATE as usize);
    let out = left(&render(
        vec![(0, one_hit(sample, &slices, Some(0)))],
        Some(table(&[set])),
        frames,
    ));
    out.iter().map(|x| x / 0.5).collect()
}

/// Frames entre los cruces hacia arriba por `level` de `v` desde `from`.
fn crossings(v: &[f32], level: f32, from: usize) -> Vec<usize> {
    (from + 1..v.len())
        .filter(|&i| v[i - 1] < level && v[i] >= level)
        .collect()
}

#[test]
fn the_lfo_has_its_rate_phase_and_delay() {
    // Unipolar sumado a 0: el volumen es 0,5 + 0,5·sen.
    let unipolar = |l: Device| Device {
        bipolar: false,
        op: Some(Op::Add),
        ..l
    };
    let make = |l: Device| {
        let mut set = set_with(VOL, &[unipolar(l)]);
        set.chains[VOL].as_mut().unwrap().input = 0.0;
        set
    };
    let v = vol_curve(make(lfo(ModWave::Sine, 10.0, |_| {})), ms(1000.0));
    let up = crossings(&v, 0.5, ms(10.0));
    let period = (up[up.len() - 1] - up[0]) as f32 / (up.len() - 1) as f32;
    assert!((period - 4800.0).abs() < 2.0, "{period}");
    // Empieza en su fase: el seno sube desde 0,5; con 90°, arriba.
    assert!((v[ms(6.0)] - (0.5 + 0.5 * (std::f32::consts::TAU * 0.06).sin())).abs() < 0.01);
    let top = vol_curve(make(lfo(ModWave::Sine, 10.0, |l| l.phase = 90.0)), ms(10.0));
    assert!((top[ms(6.0)] - (0.5 + 0.5 * (std::f32::consts::TAU * 0.31).sin())).abs() < 0.01);
    // Con retardo de 200 ms: quieto en 0,5 hasta entonces.
    let late = vol_curve(
        make(lfo(ModWave::Sine, 10.0, |l| l.delay = 200.0)),
        ms(400.0),
    );
    assert!(
        late[ms(10.0)..ms(199.0)]
            .iter()
            .all(|x| (x - 0.5).abs() < 1e-4)
    );
    assert!(late[ms(225.0)] > 0.95);
    // Atado al compás: un ciclo cada negra (4 pasos, 500 ms a 120 BPM).
    let sync = vol_curve(
        make(lfo(ModWave::Sine, 1.0, |l| {
            l.sync = Some(engine::Sync { num: 1, den: 4 })
        })),
        ms(1600.0),
    );
    let up = crossings(&sync, 0.5, ms(10.0));
    let period = (up[up.len() - 1] - up[0]) as f32 / (up.len() - 1) as f32;
    assert!((period - 24_000.0).abs() < 2.0, "{period}");
}

#[test]
fn the_lfo_restarts_with_each_hit_unless_free() {
    let (sample, slices) = dc(0.5, RATE as usize);
    let steps = parse_steps("1 . . 1  . . . .  . . . .  . . . .").unwrap();
    let render_with = |l: Device| {
        let mut set = set_with(PAN, &[l]);
        set.chains[PAN].as_mut().unwrap().input = 0.0;
        let p = Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_mod_sets(Some(0), &[])
            .unwrap();
        let out = render(vec![(0, p)], Some(table(&[set])), 5 * STEP);
        // El pan: lo que pesa la derecha.
        (0..5 * STEP)
            .map(|i| out[2 * i + 1] - out[2 * i])
            .collect::<Vec<f32>>()
    };
    // 3 Hz: el segundo golpe (a los 375 ms) vuelve a empezar igual.
    let reset = render_with(lfo(ModWave::Sine, 3.0, |_| {}));
    for i in ms(6.0)..ms(100.0) {
        assert!((reset[i] - reset[i + 3 * STEP]).abs() < 1e-4, "frame {i}");
    }
    // Libre: sigue la canción (no se repite).
    let free = render_with(lfo(ModWave::Sine, 3.0, |l| l.free = true));
    let differs = (ms(6.0)..ms(100.0)).any(|i| (free[i] - free[i + 3 * STEP]).abs() > 0.05);
    assert!(differs);
}

#[test]
fn every_lfo_wave_has_its_shape_and_random_repeats() {
    let shape_of = |wave| {
        let mut set = set_with(
            VOL,
            &[Device {
                op: Some(Op::Add),
                bipolar: false,
                ..lfo(wave, 2.0, |_| {})
            }],
        );
        set.chains[VOL].as_mut().unwrap().input = 0.0;
        vol_curve(set, ms(1000.0))
    };
    // Sierra: sube de 0 a 1 en cada ciclo (500 ms).
    let saw = shape_of(ModWave::Saw);
    assert!((saw[ms(250.0)] - 0.5).abs() < 0.01);
    assert!(saw[ms(490.0)] > 0.95 && saw[ms(510.0)] < 0.05);
    // Cuadrada: arriba la primera mitad, abajo la segunda.
    let square = shape_of(ModWave::Square);
    assert!((square[ms(100.0)] - 1.0).abs() < 1e-4 && square[ms(350.0)].abs() < 1e-4);
    // Triángulo: arriba al cuarto.
    let tri = shape_of(ModWave::Triangle);
    assert!((tri[ms(125.0)] - 1.0).abs() < 0.01);
    // Aleatoria: fija en cada ciclo y la misma en cada render.
    let a = shape_of(ModWave::Random);
    let b = shape_of(ModWave::Random);
    assert_eq!(a, b);
    assert!((a[ms(100.0)] - a[ms(400.0)]).abs() < 1e-5);
    assert!(a.iter().all(|x| (0.0..=1.0).contains(x)));
}

#[test]
fn fader_and_operand() {
    let fade = Device::new(DeviceKind::Fader(Fader {
        from: 1.0,
        to: 0.0,
        time: 100.0,
        delay: 50.0,
        scale: FaderScale::Linear,
    }));
    let v = vol_curve(set_with(VOL, &[fade]), ms(300.0));
    assert!((v[ms(40.0)] - 1.0).abs() < 1e-3);
    assert!((v[ms(100.0)] - 0.5).abs() < 2e-3);
    assert!(v[ms(160.0)].abs() < 1e-3);
    // Exponencial: a mitad, menos camino que la recta.
    let exp = Device::new(DeviceKind::Fader(Fader {
        scale: FaderScale::Exp,
        ..match fade.kind {
            DeviceKind::Fader(f) => f,
            _ => unreachable!(),
        }
    }));
    let v = vol_curve(set_with(VOL, &[exp]), ms(300.0));
    assert!(v[ms(100.0)] > 0.5);
    // El operando, un valor fijo.
    let op = Device::new(DeviceKind::Operand(ModValue::fixed(0.3)));
    let v = vol_curve(set_with(VOL, &[op]), ms(60.0));
    assert!((v[ms(50.0)] - 0.3).abs() < 1e-4);
}

/// Un set de volumen con una envolvente de puntos `(posición, valor,
/// curva)`.
fn points_set(
    points: &[(f64, f32, engine::LaneCurve)],
    length: EnvLength,
    looping: EnvLoop,
    sustain: Option<u8>,
) -> ModSet {
    let points: Vec<engine::LanePoint> = points
        .iter()
        .map(|&(at, value, curve)| engine::LanePoint { at, value, curve })
        .collect();
    let mut set = set_with(
        VOL,
        &[Device::new(DeviceKind::Points(PointEnv {
            lane: 0,
            length,
            looping,
            sustain,
        }))],
    );
    set.envs[0] = Some(engine::Lane::new(&points, 1.0, false).unwrap());
    set
}

#[test]
fn the_point_envelope_follows_its_points_and_loops() {
    use engine::LaneCurve::{Linear, Step as Hold};
    let pts = [(0.0, 0.0, Linear), (0.25, 1.0, Linear), (1.0, 0.0, Linear)];
    let v = vol_curve(
        points_set(&pts, EnvLength::Ms(400.0), EnvLoop::Off, None),
        ms(600.0),
    );
    let close = |v: &[f32], t: f32, want: f32| {
        assert!(
            (v[ms(t)] - want).abs() < 2e-3,
            "{t} ms: {} (quería {want})",
            v[ms(t)]
        );
    };
    close(&v, 50.0, 0.5);
    close(&v, 101.0, 1.0 - 1.0 / 300.0);
    close(&v, 250.0, 0.5);
    close(&v, 500.0, 0.0);
    // En bucle, adelante: a los 450 ms, como a los 50.
    let fwd = vol_curve(
        points_set(&pts, EnvLength::Ms(400.0), EnvLoop::Forward, None),
        ms(600.0),
    );
    close(&fwd, 450.0, 0.5);
    // Atrás: a los 50 ms, como en la posición 0,875.
    let back = vol_curve(
        points_set(&pts, EnvLength::Ms(400.0), EnvLoop::Backward, None),
        ms(600.0),
    );
    close(&back, 50.0, 1.0 / 6.0);
    // Ida y vuelta: a los 450 ms (posición 1,125), la 0,875.
    let ping = vol_curve(
        points_set(&pts, EnvLength::Ms(400.0), EnvLoop::PingPong, None),
        ms(600.0),
    );
    close(&ping, 450.0, 1.0 / 6.0);
    // Escalón: se queda en el valor de su punto.
    let step = vol_curve(
        points_set(
            &[(0.0, 0.3, Hold), (0.5, 0.9, Linear), (1.0, 0.9, Linear)],
            EnvLength::Ms(200.0),
            EnvLoop::Off,
            None,
        ),
        ms(300.0),
    );
    close(&step, 90.0, 0.3);
    close(&step, 110.0, 0.9);
    // Con la duración en figuras: una negra a 120 BPM son 500 ms.
    let beat = vol_curve(
        points_set(&pts, EnvLength::Steps(4.0), EnvLoop::Off, None),
        ms(600.0),
    );
    close(&beat, 125.0, 1.0);
}

#[test]
fn the_point_envelope_holds_its_sustain_until_the_note_off() {
    use engine::LaneCurve::Linear;
    // Sube en 50 ms, se queda en el sustain (punto 1) hasta el note-off
    // (el final del paso, 125 ms) y luego baja lo que le queda (150 ms).
    let pts = [(0.0, 0.0, Linear), (0.25, 1.0, Linear), (1.0, 0.0, Linear)];
    let v = vol_curve(
        points_set(&pts, EnvLength::Ms(200.0), EnvLoop::Off, Some(1)),
        ms(400.0),
    );
    assert!((v[ms(110.0)] - 1.0).abs() < 2e-3, "{}", v[ms(110.0)]);
    assert!((v[ms(200.0)] - 0.5).abs() < 2e-3, "{}", v[ms(200.0)]);
    assert!(v[ms(280.0)].abs() < 2e-3);
    // Si el note-off llega antes del sustain, sigue sin pararse.
    let slow = vol_curve(
        points_set(&pts, EnvLength::Ms(1000.0), EnvLoop::Off, Some(1)),
        ms(600.0),
    );
    assert!(
        (slow[ms(500.0)] - (1.0 - 0.25 / 0.75)).abs() < 3e-3,
        "{}",
        slow[ms(500.0)]
    );
}

#[test]
fn point_envelopes_are_validated() {
    use engine::LaneCurve::Linear;
    let pts = [(0.0, 0.0, Linear), (1.0, 1.0, Linear)];
    let bad = [
        points_set(&pts, EnvLength::Ms(0.0), EnvLoop::Off, None),
        points_set(&pts, EnvLength::Ms(400.0), EnvLoop::Forward, Some(1)),
        points_set(&pts, EnvLength::Ms(400.0), EnvLoop::Off, Some(5)),
    ];
    for set in bad {
        assert!(set.validate().is_err());
    }
    let mut missing = points_set(&pts, EnvLength::Ms(400.0), EnvLoop::Off, None);
    missing.envs[0] = None;
    assert!(missing.validate().is_err());
}

fn stepper_set(values: &[f32], f: impl FnOnce(&mut Stepper)) -> ModSet {
    let mut st = Stepper {
        id: 0,
        first: 0,
        len: values.len() as u16,
        advance: 1,
        random: false,
        seed: 1,
        reset: 0,
    };
    f(&mut st);
    let mut set = set_with(VOL, &[Device::new(DeviceKind::Stepper(st))]);
    set.steps[..values.len()].copy_from_slice(values);
    set
}

/// El volumen de cada golpe de una pista con un golpe en cada paso.
fn levels_per_step(set: ModSet, steps: &str, bars: usize) -> Vec<f32> {
    let (sample, slices) = dc(0.5, RATE as usize);
    let p = Pattern::from_steps(sample, &slices, parse_steps(steps).unwrap())
        .unwrap()
        .with_mod_sets(Some(0), &[])
        .unwrap();
    let out = left(&render(vec![(0, p)], Some(table(&[set])), bars * 16 * STEP));
    (0..bars * 16)
        .map(|k| out[k * STEP + ms(50.0)] / 0.5)
        .collect()
}

const EVERY: &str = "1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1";

#[test]
fn the_stepper_advances_with_each_hit_and_wraps() {
    let close = |got: &[f32], want: &[f32]| {
        for (k, (g, w)) in got.iter().zip(want).enumerate() {
            assert!((g - w).abs() < 1e-4, "golpe {k}: {g} (quería {w})");
        }
    };
    let v = [1.0, 0.5, 0.25];
    close(
        &levels_per_step(stepper_set(&v, |_| {}), EVERY, 1),
        &[1.0, 0.5, 0.25, 1.0, 0.5, 0.25, 1.0],
    );
    // De dos en dos sobre tres (no divide): 0, 2, 1, 0...
    close(
        &levels_per_step(stepper_set(&v, |s| s.advance = 2), EVERY, 1),
        &[1.0, 0.25, 0.5, 1.0, 0.25, 0.5],
    );
    // Con 0, se queda.
    close(
        &levels_per_step(stepper_set(&v, |s| s.advance = 0), EVERY, 1),
        &[1.0, 1.0, 1.0],
    );
    // Vuelve al principio cada compás (16 golpes no es múltiplo de 3).
    let reset = levels_per_step(stepper_set(&v, |s| s.reset = 1), EVERY, 2);
    close(&reset[16..19], &[1.0, 0.5, 0.25]);
    let no_reset = levels_per_step(stepper_set(&v, |_| {}), EVERY, 2);
    close(&no_reset[16..17], &[0.5]);
    // El cursor es del set: dos pistas que lo comparten se lo reparten
    // (con un sample más corto que un paso, para oír cada golpe solo).
    let (sample, slices) = dc(0.5, 4000);
    let make = |steps: &str| {
        Pattern::from_steps(sample.clone(), &slices, parse_steps(steps).unwrap())
            .unwrap()
            .with_mod_sets(Some(0), &[])
            .unwrap()
    };
    let out = render(
        vec![
            (0, make("1 . 1 .  . . . .  . . . .  . . . .")),
            (1, make(". 1 . 1  . . . .  . . . .  . . . .")),
        ],
        Some(table(&[stepper_set(&v, |_| {})])),
        4 * STEP,
    );
    let l = left(&out);
    let got: Vec<f32> = (0..4).map(|k| l[k * STEP + ms(50.0)] / 0.5).collect();
    close(&got, &[1.0, 0.5, 0.25, 1.0]);
}

#[test]
fn the_random_stepper_repeats_with_its_seed() {
    let v: Vec<f32> = (0..8).map(|i| i as f32 / 8.0).collect();
    let a = levels_per_step(stepper_set(&v, |s| s.random = true), EVERY, 1);
    let b = levels_per_step(stepper_set(&v, |s| s.random = true), EVERY, 1);
    assert_eq!(a, b);
    assert!(a.iter().all(|x| v.iter().any(|y| (x - y).abs() < 1e-4)));
    assert!(a.windows(2).any(|w| w[0] != w[1]));
    let other = levels_per_step(
        stepper_set(&v, |s| {
            s.random = true;
            s.seed = 99
        }),
        EVERY,
        1,
    );
    assert_ne!(a, other);
}

#[test]
fn velocity_and_key_tracking() {
    let vel = |clamp| {
        set_with(
            VOL,
            &[Device::new(DeviceKind::Velocity(VelocityTrack {
                min: 0.2,
                max: 1.0,
                clamp,
            }))],
        )
    };
    // `@0.5`: la velocidad es el volumen del golpe (y la voz suena a 0,5).
    let steps = "1@0.5 1 . .  . . . .  . . . .  . . . .";
    let scaled = levels_per_step(vel(false), steps, 1);
    assert!((scaled[0] - 0.5 * 0.6).abs() < 1e-4, "{}", scaled[0]);
    assert!((scaled[1] - 1.0).abs() < 1e-4);
    let clamped = levels_per_step(vel(true), "1@0.1 . . .  . . . .  . . . .  . . . .", 1);
    assert!((clamped[0] - 0.1 * 0.2).abs() < 1e-4, "{}", clamped[0]);
    // La nota: de 0,2 en do 4 (60) a 1 en do 5 (72).
    let key = set_with(
        VOL,
        &[Device::new(DeviceKind::Key(KeyTrack {
            min: 0.2,
            max: 1.0,
            low: 60.0,
            high: 72.0,
        }))],
    );
    let (sample, slices) = dc(0.5, RATE as usize);
    let p = Pattern::from_steps(
        sample,
        &slices,
        parse_steps("1 . 1^6 .  1^12 . 1^-3 .  . . . .  . . . .").unwrap(),
    )
    .unwrap()
    .with_mod_sets(Some(0), &[])
    .unwrap();
    let out = left(&render(vec![(0, p)], Some(table(&[key])), 8 * STEP));
    let at = |k: usize| out[k * STEP + ms(50.0)] / 0.5;
    assert!((at(0) - 0.2).abs() < 1e-3);
    assert!((at(2) - 0.6).abs() < 1e-3);
    assert!((at(4) - 1.0).abs() < 1e-3);
    assert!((at(6) - 0.2).abs() < 1e-3);
}
