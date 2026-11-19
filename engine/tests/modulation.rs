//! Efectos de modulación de pista: flanger, phaser y chorus. Render
//! offline: con `mix = 0` suenan bit a bit como sin el efecto; con el LFO
//! quieto, el flanger es un peine y el phaser un hueco donde dice la
//! teoría; con los valores extremos no hay NaN ni se dispara; la cola sigue
//! tras acabar la voz y se apaga sin corte; el LFO atado al compás es el
//! mismo en cada render.

use std::sync::Arc;

use engine::notation::parse_steps;
use engine::{
    ChorusParams, FlangerParams, Fx, Lfo, LfoWave, Pattern, PhaserParams, Sample, Slices, Sync,
    new_engine, offline,
};

const RATE: u32 = 48_000;

fn sine(freq: f32, frames: usize, amp: f32) -> Vec<f32> {
    (0..frames)
        .map(|i| amp * (std::f32::consts::TAU * freq * i as f32 / RATE as f32).sin())
        .collect()
}

/// Ruido blanco determinista.
fn noise(frames: usize, amp: f32) -> Vec<f32> {
    let mut x = 0x2545_f491u32;
    (0..frames)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            amp * (x as f32 / u32::MAX as f32 * 2.0 - 1.0)
        })
        .collect()
}

/// Un golpe del sample `mono` al empezar, con la cadena `chain`; estéreo.
fn render(mono: &[f32], chain: &[Fx], frames: usize) -> Vec<f32> {
    let sample = Arc::new(Sample::from_interleaved(mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    let steps = parse_steps("1 . . .  . . . .  . . . .  . . . .").unwrap();
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_fx(chain)
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
}

fn channel(audio: &[f32], ch: usize) -> Vec<f32> {
    audio.iter().skip(ch).step_by(2).copied().collect()
}

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|s| s * s).sum::<f32>() / x.len() as f32).sqrt()
}

fn still(rate_hz: f32) -> Lfo {
    Lfo {
        rate_hz,
        sync: None,
        wave: LfoWave::Sine,
        phase_deg: 0.0,
    }
}

fn flanger(f: impl FnOnce(&mut FlangerParams)) -> Fx {
    let mut p = FlangerParams::DEFAULT;
    f(&mut p);
    Fx::Flanger(p)
}

fn phaser(f: impl FnOnce(&mut PhaserParams)) -> Fx {
    let mut p = PhaserParams::DEFAULT;
    f(&mut p);
    Fx::Phaser(p)
}

fn chorus(f: impl FnOnce(&mut ChorusParams)) -> Fx {
    let mut p = ChorusParams::DEFAULT;
    f(&mut p);
    Fx::Chorus(p)
}

#[test]
fn with_mix_zero_the_signal_passes_bit_for_bit() {
    let input = noise(RATE as usize / 2, 0.5);
    let dry = render(&input, &[], RATE as usize);
    for fx in [
        flanger(|p| p.mix = 0.0),
        phaser(|p| p.mix = 0.0),
        chorus(|p| p.mix = 0.0),
        // También con la realimentación al máximo y el LFO atado.
        flanger(|p| {
            p.mix = 0.0;
            p.feedback = -0.95;
            p.lfo.sync = Some(Sync::per_bar(4));
        }),
    ] {
        let wet = render(&input, &[fx], RATE as usize);
        assert!(
            wet.iter()
                .zip(&dry)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "{fx:?}"
        );
    }
}

#[test]
fn a_still_flanger_is_a_comb() {
    // Retardo de 1 ms, sin barrido ni realimentación, mitad y mitad:
    // (1 + e^{-jωd}) / 2 tiene un hueco en 500 Hz y un pico en 1000 Hz.
    let fx = flanger(|p| {
        p.lfo = still(0.1);
        p.depth_ms = 0.0;
        p.delay_ms = 1.0;
        p.feedback = 0.0;
        p.mix = 0.5;
    });
    let at = |freq: f32| {
        let input = sine(freq, RATE as usize, 0.5);
        let out = channel(&render(&input, &[fx], RATE as usize), 0);
        rms(&out[10_000..40_000]) / rms(&input[10_000..40_000])
    };
    assert!(at(500.0) < 0.01, "hueco: {}", at(500.0));
    assert!(at(1000.0) > 0.99, "pico: {}", at(1000.0));
    // Invertida, al revés: el hueco en 1000 Hz.
    let inverted = flanger(|p| {
        p.lfo = still(0.1);
        p.depth_ms = 0.0;
        p.delay_ms = 1.0;
        p.feedback = 0.0;
        p.mix = 0.5;
        p.invert = true;
    });
    let input = sine(1000.0, RATE as usize, 0.5);
    let out = channel(&render(&input, &[inverted], RATE as usize), 0);
    assert!(rms(&out[10_000..40_000]) / rms(&input[10_000..40_000]) < 0.01);
}

#[test]
fn a_fractional_delay_reads_between_samples() {
    // 1,0104 ms son 48,5 frames: el hueco cae en 1/(2·d) = 494,8 Hz. Con la
    // interpolación (y no redondeando el retardo) sigue siendo hondo.
    let delay_ms = 48.5 / 48.0;
    let fx = flanger(|p| {
        p.lfo = still(0.1);
        p.depth_ms = 0.0;
        p.delay_ms = delay_ms;
        p.feedback = 0.0;
        p.mix = 0.5;
    });
    let notch = 1000.0 / (2.0 * delay_ms);
    let input = sine(notch, RATE as usize, 0.5);
    let out = channel(&render(&input, &[fx], RATE as usize), 0);
    let ratio = rms(&out[10_000..40_000]) / rms(&input[10_000..40_000]);
    assert!(ratio < 0.01, "{ratio}");
}

#[test]
fn a_still_phaser_has_its_notch_at_the_center() {
    // Dos etapas: cada una gira π/2 en `center`, las dos π; mitad y mitad,
    // un hueco ahí. Lejos (a 4 octavas) pasa casi todo.
    let fx = phaser(|p| {
        p.lfo = still(0.1);
        p.depth_oct = 0.0;
        p.center_hz = 1000.0;
        p.feedback = 0.0;
        p.stages = 2;
        p.mix = 0.5;
    });
    let at = |freq: f32| {
        let input = sine(freq, RATE as usize, 0.5);
        let out = channel(&render(&input, &[fx], RATE as usize), 0);
        rms(&out[10_000..40_000]) / rms(&input[10_000..40_000])
    };
    assert!(at(1000.0) < 0.01, "hueco: {}", at(1000.0));
    assert!(at(62.5) > 0.95, "grave: {}", at(62.5));
    // Solo la mojada, sin realimentación: un paso-todo no cambia el nivel.
    let wet = phaser(|p| {
        p.lfo = still(0.1);
        p.stages = 12;
        p.feedback = 0.0;
        p.mix = 1.0;
    });
    let input = sine(700.0, RATE as usize, 0.5);
    let out = channel(&render(&input, &[wet], RATE as usize), 0);
    let ratio = rms(&out[10_000..40_000]) / rms(&input[10_000..40_000]);
    assert!((ratio - 1.0).abs() < 0.01, "{ratio}");
}

#[test]
fn extreme_values_stay_finite_and_bounded() {
    // Un sample al máximo (y un seno grave que entra en la realimentación).
    let mut input = noise(RATE as usize / 2, 1.0);
    input.extend(sine(40.0, RATE as usize / 2, 1.0));
    let chains: Vec<Fx> = vec![
        flanger(|p| {
            p.feedback = 0.95;
            p.depth_ms = 10.0;
            p.delay_ms = 15.0;
            p.mix = 1.0;
            p.lfo.rate_hz = 20.0;
        }),
        flanger(|p| {
            p.feedback = -0.95;
            p.depth_ms = 0.0;
            p.delay_ms = 0.1;
            p.mix = 1.0;
        }),
        phaser(|p| {
            p.feedback = 0.95;
            p.depth_oct = 4.0;
            p.center_hz = 10_000.0;
            p.stages = 12;
            p.mix = 1.0;
            p.lfo.rate_hz = 20.0;
        }),
        phaser(|p| {
            p.feedback = -0.95;
            p.center_hz = 50.0;
            p.depth_oct = 4.0;
            p.stages = 12;
        }),
        chorus(|p| {
            p.depth_ms = 10.0;
            p.delay_ms = 30.0;
            p.voices = 4;
            p.mix = 1.0;
            p.lfo.rate_hz = 20.0;
        }),
    ];
    for fx in chains {
        let out = render(&input, &[fx], 2 * RATE as usize);
        assert!(out.iter().all(|x| x.is_finite()), "{fx:?}");
        let peak = out.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        // El limitador del master deja todo por debajo de 1; dentro, la
        // realimentación no se dispara.
        assert!(peak <= 1.0, "{fx:?}: {peak}");
    }
    // Y en un silencio que nunca sonó: ceros exactos.
    let quiet = render(&vec![0.0; 1000], &[chains_all()[0]], RATE as usize);
    assert!(quiet.iter().all(|&x| x == 0.0));
}

fn chains_all() -> Vec<Fx> {
    vec![flanger(|_| {}), phaser(|_| {}), chorus(|_| {})]
}

#[test]
fn the_tail_rings_after_the_voice_and_fades_out_without_a_cut() {
    // Un golpe corto de ruido con mucha realimentación: la voz acaba a los
    // 2000 frames y la cola del flanger sigue y se apaga sola.
    let input = noise(2000, 0.5);
    let fx = flanger(|p| {
        p.feedback = 0.9;
        p.delay_ms = 10.0;
        p.mix = 0.5;
    });
    // A 0,9 cada 11 ms, baja de −120 dB en ~1,4 s (el compás, a 120 BPM,
    // dura 2 s: el siguiente golpe no entra).
    let out = channel(&render(&input, &[fx], 2 * RATE as usize - 1000), 0);
    let after = &out[2100..];
    let window = |k: usize| {
        after[k * 4800..(k + 1) * 4800]
            .iter()
            .fold(0.0f32, |m, x| m.max(x.abs()))
    };
    assert!(window(0) > 0.05, "la cola suena tras la voz");
    // Baja ventana a ventana (cada 100 ms), sin saltos, y acaba en nada.
    for k in 1..18 {
        assert!(window(k) < window(k - 1), "ventana {k}");
    }
    assert!(window(18) < 1e-7);
    // El phaser con la realimentación al máximo resuena (un lazo de
    // paso-todos con ganancia 0,95) y también se apaga solo; el chorus no
    // tiene realimentación: acaba con su retardo más largo.
    let out = channel(
        &render(
            &input,
            &[phaser(|p| p.feedback = 0.95)],
            2 * RATE as usize - 1000,
        ),
        0,
    );
    let after = &out[2100..];
    let window = |k: usize| {
        after[k * 4800..(k + 1) * 4800]
            .iter()
            .fold(0.0f32, |m, x| m.max(x.abs()))
    };
    assert!(
        window(18) < 1e-6 * window(0),
        "{} {}",
        window(0),
        window(18)
    );
    let out = channel(&render(&input, &[chorus(|_| {})], RATE as usize), 0);
    assert!(out[2100 + 2400..].iter().all(|x| x.abs() < 1e-6));
}

#[test]
fn panic_fades_the_tail_and_the_next_hit_sounds_wet_again() {
    let input = noise(2000, 0.5);
    let fx = flanger(|p| {
        p.feedback = 0.9;
        p.delay_ms = 10.0;
        p.mix = 1.0;
    });
    let sample = Arc::new(Sample::from_interleaved(&input, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    let steps = parse_steps("1 . . .  . . . .  1 . . .  . . . .").unwrap();
    let pattern = Pattern::from_steps(sample, &slices, steps)
        .unwrap()
        .with_fx(&[fx])
        .unwrap();
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern).unwrap();
    controller.start().unwrap();
    let before = channel(&offline::render(&mut engine, 4800, 2), 0);
    assert!(
        before[4000..].iter().any(|x| x.abs() > 0.01),
        "suena la cola"
    );
    controller.panic(false).unwrap();
    // En 5 ms baja a cero sin saltos grandes.
    let fade = channel(&offline::render(&mut engine, 4800, 2), 0);
    assert!(fade[480..].iter().all(|&x| x == 0.0));
    let jump = before[4799] - fade[0];
    assert!(jump.abs() < 0.1, "{jump}");
    // El segundo golpe (a mitad del compás, 1 s) suena mojado otra vez.
    let rest = channel(&offline::render(&mut engine, RATE as usize, 2), 0);
    let second = &rest[RATE as usize - 9600..];
    assert!(rms(second) > 0.01, "{}", rms(second));
}

#[test]
fn the_synced_lfo_is_the_same_on_every_render_and_moves_the_comb() {
    let input = noise(RATE as usize, 0.5);
    let fx = flanger(|p| {
        p.lfo.sync = Some(Sync { num: 1, den: 4 });
        p.depth_ms = 5.0;
    });
    let a = render(&input, &[fx], RATE as usize);
    let b = render(&input, &[fx], RATE as usize);
    assert_eq!(a, b);
    // El desfase estéreo: con 90°, los canales son distintos; con 0, no.
    assert!(channel(&a, 0) != channel(&a, 1));
    let mono = flanger(|p| {
        p.lfo.sync = Some(Sync { num: 1, den: 4 });
        p.lfo.phase_deg = 0.0;
    });
    let m = render(&input, &[mono], RATE as usize);
    assert_eq!(channel(&m, 0), channel(&m, 1));
}

#[test]
fn ranges_are_validated() {
    let bad = [
        flanger(|p| p.feedback = 0.96),
        flanger(|p| p.delay_ms = 0.0),
        flanger(|p| p.depth_ms = 11.0),
        flanger(|p| p.mix = f32::NAN),
        flanger(|p| p.lfo.rate_hz = f32::INFINITY),
        flanger(|p| p.lfo.phase_deg = 200.0),
        flanger(|p| p.lfo.sync = Some(Sync { num: 0, den: 4 })),
        phaser(|p| p.stages = 3),
        phaser(|p| p.center_hz = 20_000.0),
        phaser(|p| p.feedback = -1.0),
        chorus(|p| p.voices = 5),
        chorus(|p| p.delay_ms = 5.0),
    ];
    for fx in bad {
        assert!(fx.validate().is_err(), "{fx:?}");
    }
    for fx in chains_all() {
        assert_eq!(fx.validate(), Ok(()));
    }
}

#[test]
fn flanger_and_chorus_do_not_go_in_a_slice() {
    for fx in [flanger(|_| {}), chorus(|_| {})] {
        assert!(engine::SliceFx::new(&[fx], &[]).is_err());
    }
    assert!(engine::SliceFx::new(&[phaser(|_| {})], &[]).is_ok());
}

#[test]
fn the_result_does_not_depend_on_the_block_size() {
    let input = noise(RATE as usize / 2, 0.5);
    let sample = Arc::new(Sample::from_interleaved(&input, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    let steps = parse_steps("1 . . .  . . . .  1 . . .  . . . .").unwrap();
    let chain = [
        flanger(|p| p.lfo.sync = Some(Sync { num: 3, den: 16 })),
        phaser(|p| p.lfo.rate_hz = 3.0),
        chorus(|p| p.voices = 3),
    ];
    let run = |block: usize| {
        let pattern = Pattern::from_steps(sample.clone(), &slices, steps)
            .unwrap()
            .with_fx(&chain)
            .unwrap();
        let (mut controller, mut engine) = new_engine(RATE);
        controller.set_bpm(137.0).unwrap();
        controller.set_pattern(pattern).unwrap();
        controller.start().unwrap();
        let mut out = vec![0.0f32; RATE as usize * 2];
        for piece in out.chunks_mut(block * 2) {
            engine.process(piece, 2);
        }
        out
    };
    let whole = run(512);
    assert!(whole.iter().any(|x| x.abs() > 0.01));
    for block in [1, 37, 300] {
        assert!(
            run(block)
                .iter()
                .zip(&whole)
                .all(|(a, b)| a.to_bits() == b.to_bits()),
            "bloques de {block}"
        );
    }
}

/// Coste de CPU, en release: `cargo test -p engine --release --test
/// modulation -- --ignored --nocapture coste`. Ocho pistas con un golpe en
/// cada paso (la voz siempre suena), 20 s de render; el porcentaje es de un
/// núcleo a 48 kHz. «Por instancia» resta las ocho pistas sin efectos.
#[test]
#[ignore]
fn coste() {
    let input = noise(RATE as usize / 4, 0.5);
    let sample = Arc::new(Sample::from_interleaved(&input, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 1);
    let steps = parse_steps("1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1").unwrap();
    let seconds = 20.0;
    let run = |track_fx: &[Fx], slice_fx: &[Fx], tracks: usize| {
        let (mut controller, mut engine) = new_engine(RATE);
        controller.set_bpm(170.0).unwrap();
        for t in 0..tracks {
            let mut pattern = Pattern::from_steps(sample.clone(), &slices, steps)
                .unwrap()
                .with_fx(track_fx)
                .unwrap();
            if !slice_fx.is_empty() {
                pattern = pattern
                    .with_slice_fx(&[Some(engine::SliceFx::new(slice_fx, &[]).unwrap())])
                    .unwrap();
            }
            controller.set_track_pattern(t, Some(pattern)).unwrap();
        }
        controller.start().unwrap();
        let mut out = vec![0.0f32; 512 * 2];
        let blocks = (seconds * RATE as f64 / 512.0) as usize;
        let start = std::time::Instant::now();
        for _ in 0..blocks {
            engine.process(&mut out, 2);
        }
        100.0 * start.elapsed().as_secs_f64() / seconds
    };
    let base = run(&[], &[], 8);
    println!("8 pistas sin efectos: {base:.2} % de un núcleo");
    let cases: Vec<(&str, Vec<Fx>, Vec<Fx>)> = vec![
        ("flanger", vec![flanger(|_| {})], vec![]),
        (
            "flanger con sync",
            vec![flanger(|p| p.lfo.sync = Some(Sync::per_bar(1)))],
            vec![],
        ),
        ("phaser 4 etapas", vec![phaser(|_| {})], vec![]),
        ("phaser 12 etapas", vec![phaser(|p| p.stages = 12)], vec![]),
        ("chorus 2 voces", vec![chorus(|_| {})], vec![]),
        ("chorus 4 voces", vec![chorus(|p| p.voices = 4)], vec![]),
        ("lowpass (para comparar)", vec![Fx::lowpass(2000.0)], vec![]),
        (
            "los cuatro en cada pista",
            vec![
                flanger(|_| {}),
                phaser(|p| p.stages = 12),
                chorus(|p| p.voices = 4),
                Fx::lowpass(2000.0),
            ],
            vec![],
        ),
        (
            "phaser 4 etapas por corte (en la voz)",
            vec![],
            vec![phaser(|_| {})],
        ),
        (
            "phaser 12 etapas por corte (en la voz)",
            vec![],
            vec![phaser(|p| p.stages = 12)],
        ),
        (
            "lowpass por corte (para comparar)",
            vec![],
            vec![Fx::lowpass(2000.0)],
        ),
    ];
    for (name, track, slice) in cases {
        let total = run(&track, &slice, 8);
        let each = (total - base) / (8.0 * track.len().max(1) as f64);
        println!("{name}: {total:.2} % en 8 pistas; {each:.3} % por instancia");
    }
}
