//! Las muestras para las visualizaciones: apagadas, el motor no escribe
//! nada; encendidas, la cola del master es la salida en mono y la de una
//! pista lo que sale de ella; llena, descarta sin bloquear; y el
//! encendido entra al compás, con el lote.

use std::sync::Arc;

use engine::{
    Batch, Pattern, Sample, Slices, Step, VIZ_CAPACITY, VIZ_MASTER, VIZ_SOURCES, new_engine,
    offline,
};

const RATE: u32 = 48_000;

fn engine_with(on: Option<[bool; VIZ_SOURCES]>) -> (engine::Controller, engine::Engine) {
    let tone: Vec<f32> = (0..24_000).map(|i| (i as f32 * 0.03).sin() * 0.4).collect();
    let sample = Arc::new(Sample::from_interleaved(&tone, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    let mut steps = vec![None; 16];
    steps[0] = Some(Step::new(0));
    steps[8] = Some(Step::new(0));
    let pattern = Pattern::from_steps(sample, &slices, steps).unwrap();
    let (mut controller, engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch.set_bpm(120.0).set_pattern(0, Some(pattern));
    if let Some(on) = on {
        batch.set_viz(on);
    }
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    (controller, engine)
}

fn drained(controller: &mut engine::Controller, source: usize) -> Vec<f32> {
    let mut out = Vec::new();
    controller.viz_samples(source, &mut out);
    out
}

#[test]
fn off_writes_nothing() {
    let (mut controller, mut engine) = engine_with(None);
    offline::render(&mut engine, 20_000, 2);
    for source in 0..VIZ_SOURCES {
        assert!(drained(&mut controller, source).is_empty());
    }
}

#[test]
fn on_the_master_is_the_output_in_mono_and_a_track_what_it_plays() {
    let mut on = [false; VIZ_SOURCES];
    on[0] = true;
    on[VIZ_MASTER] = true;
    let (mut controller, mut engine) = engine_with(Some(on));
    let out = offline::render(&mut engine, 10_000, 2);
    let master = drained(&mut controller, VIZ_MASTER);
    let mono: Vec<f32> = out.chunks(2).map(|f| 0.5 * (f[0] + f[1])).collect();
    assert_eq!(master, mono);
    // Una sola pista y sin efectos de master: lo mismo.
    let track = drained(&mut controller, 0);
    assert_eq!(track.len(), 10_000);
    assert!(track.iter().any(|x| *x != 0.0));
    // Las demás, apagadas.
    assert!(drained(&mut controller, 1).is_empty());
}

#[test]
fn full_it_drops_and_keeps_its_capacity() {
    let mut on = [false; VIZ_SOURCES];
    on[VIZ_MASTER] = true;
    let (mut controller, mut engine) = engine_with(Some(on));
    offline::render(&mut engine, 3 * VIZ_CAPACITY, 2);
    let got = drained(&mut controller, VIZ_MASTER);
    assert_eq!(got.len(), VIZ_CAPACITY);
    // Vaciada, vuelve a llenarse.
    offline::render(&mut engine, 1000, 2);
    assert_eq!(drained(&mut controller, VIZ_MASTER).len(), 1000);
}

#[test]
fn switching_on_enters_at_the_bar() {
    let (mut controller, mut engine) = engine_with(None);
    offline::render(&mut engine, 10_000, 2);
    let mut on = [false; VIZ_SOURCES];
    on[VIZ_MASTER] = true;
    let mut batch = Batch::new();
    batch.set_viz(on);
    controller.apply(batch).unwrap();
    // Hasta el compás (96 000 frames), nada.
    offline::render(&mut engine, 80_000, 2);
    assert!(drained(&mut controller, VIZ_MASTER).is_empty());
    offline::render(&mut engine, 10_000, 2);
    assert_eq!(drained(&mut controller, VIZ_MASTER).len(), 4000);
}
