//! Un valor no finito (NaN o infinito) que llega a la mezcla no puede
//! dejarla muda para siempre (N3 de `docs/AUDITORIA-2.md`).
//!
//! El mecanismo: el filtro de la pista (un biquad, con memoria) recoge el
//! NaN y lo devuelve en cada muestra siguiente; mientras la pista tenga una
//! voz, su cadena no se vacía; su salida NaN, sumada al master, lo hace NaN
//! entero, y el limitador lo convierte en silencio. La reverb guarda el NaN
//! en sus combs hasta que se duerme (un segundo sin entrada). Los samples se
//! sanean al cargar; esto prueba la red del master, por si llega por otro
//! camino (un sample hecho a mano, un efecto que diverge).

use super::*;
use crate::FilterKind;
use crate::bus::REVERB;
use crate::offline;

const RATE: u32 = 48_000;

/// Un golpe de 1 s con un NaN dentro, sin sanear (como si llegara por un
/// camino que no pasa por la carga).
fn poisoned() -> Arc<Sample> {
    let mut data = Vec::new();
    for i in 0..RATE as usize {
        let x = if i == 2000 {
            f32::NAN
        } else {
            0.5 * (i as f32 * 0.05).sin()
        };
        data.extend([x, x]);
    }
    Arc::new(Sample::from_raw(data, RATE))
}

fn clean() -> Arc<Sample> {
    Arc::new(Sample::from_interleaved(&[0.5; 4000], 1, RATE, RATE).unwrap())
}

fn hit_every_bar(sample: Arc<Sample>) -> Pattern {
    let slices = Slices::detect(&sample, 1);
    let steps = crate::notation::parse_steps("1 . . .  . . . .  . . . .  . . . .").unwrap();
    Pattern::from_steps(sample, &slices, steps).unwrap()
}

fn engine_with(poison_fx: &[Fx], poison_send: f32) -> (Controller, Engine) {
    let (mut controller, engine) = new_engine(RATE);
    let mut sends = [0.0; crate::bus::BUSES];
    sends[REVERB] = poison_send;
    let bad = hit_every_bar(poisoned())
        .with_fx(poison_fx)
        .unwrap()
        .with_sends(sends)
        .unwrap();
    // La pista buena toca en cada paso.
    let good = {
        let sample = clean();
        let slices = Slices::detect(&sample, 1);
        let steps = crate::notation::parse_steps("1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1").unwrap();
        Pattern::from_steps(sample, &slices, steps).unwrap()
    };
    let mut batch = Batch::new();
    batch.set_pattern(0, Some(bad));
    batch.set_pattern(1, Some(good));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    (controller, engine)
}

/// Frames en silencio absoluto de cada compás.
fn silent_per_bar(audio: &[f32], bar: usize) -> Vec<usize> {
    audio
        .chunks(2 * bar)
        .map(|b| b.chunks(2).filter(|f| f[0] == 0.0 && f[1] == 0.0).count())
        .collect()
}

#[test]
fn a_nan_in_a_filtered_track_does_not_silence_the_mix_forever() {
    let bar = crate::bar_frames(120.0, RATE) as usize;
    let filter = Fx::Filter {
        kind: FilterKind::LowPass,
        cutoff_hz: 3000.0,
        q: 0.707,
    };
    let (_controller, mut engine) = engine_with(&[filter], 0.0);
    let audio = offline::render(&mut engine, 4 * bar, 2);
    let silent = silent_per_bar(&audio, bar);
    // La pista buena suena en cada paso: ningún compás entero en silencio.
    assert!(silent.iter().all(|&s| s < bar / 2), "{silent:?}");
    assert!(audio.iter().all(|x| x.is_finite()));
}

#[test]
fn a_nan_in_the_reverb_does_not_silence_the_mix() {
    let bar = crate::bar_frames(120.0, RATE) as usize;
    let (_controller, mut engine) = engine_with(&[], 0.5);
    let audio = offline::render(&mut engine, 4 * bar, 2);
    let silent = silent_per_bar(&audio, bar);
    assert!(silent.iter().all(|&s| s < bar / 2), "{silent:?}");
}

#[test]
fn the_meters_see_a_non_finite_master() {
    let bar = crate::bar_frames(120.0, RATE) as usize;
    let filter = Fx::Filter {
        kind: FilterKind::LowPass,
        cutoff_hz: 3000.0,
        q: 0.707,
    };
    let (controller, mut engine) = engine_with(&[filter], 0.0);
    offline::render(&mut engine, bar, 2);
    assert!(controller.status().master.non_finite > 0);
}
