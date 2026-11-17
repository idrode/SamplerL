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

/// Lo que cuesta copiar en el hilo de audio (`cargo test -p engine
/// --release --test viz -- --ignored --nocapture`): 20 s con las 9 colas
/// encendidas (vaciándolas) frente a apagadas.
#[test]
#[ignore]
fn coste_en_el_hilo_de_audio() {
    let run = |on: bool| {
        let (mut controller, mut engine) = engine_with(Some([on; VIZ_SOURCES]));
        let mut out = vec![0.0f32; 2 * 512];
        let mut drained = Vec::with_capacity(VIZ_CAPACITY);
        let start = std::time::Instant::now();
        for _ in 0..(20 * RATE as usize / 512) {
            engine.process(&mut out, 2);
            if on {
                // Fuera del cálculo del coste del audio, la interfaz vacía.
                for s in 0..VIZ_SOURCES {
                    drained.clear();
                    controller.viz_samples(s, &mut drained);
                }
            }
        }
        start.elapsed().as_secs_f64()
    };
    let off = run(false);
    let on = run(true);
    println!(
        "20 s de audio: apagadas {:.1} ms, encendidas {:.1} ms (+{:.3} % de un núcleo, vaciando incluido)",
        off * 1000.0,
        on * 1000.0,
        (on - off) / 20.0 * 100.0
    );
}

/// La cola estéreo del master (la ventana del espectro estéreo): apagada,
/// nada; encendida, L y R exactos de la salida (tras el limitador), con
/// una pista panoteada a un lado; y la salida es la misma, encendida o no.
#[test]
fn the_stereo_master_copies_left_and_right_without_changing_the_mix() {
    let make = |pan: f32| {
        let tone: Vec<f32> = (0..24_000).map(|i| (i as f32 * 0.03).sin() * 0.4).collect();
        let sample = Arc::new(Sample::from_interleaved(&tone, 1, RATE, RATE).unwrap());
        let slices = Slices::grid(&sample, 1);
        let mut steps = vec![None; 16];
        steps[0] = Some(Step::new(0));
        let pattern = Pattern::from_steps(sample, &slices, steps)
            .unwrap()
            .with_pan(pan)
            .unwrap();
        let (mut controller, engine) = new_engine(RATE);
        let mut batch = Batch::new();
        batch.set_bpm(120.0).set_pattern(0, Some(pattern));
        controller.apply(batch).unwrap();
        controller.start().unwrap();
        (controller, engine)
    };
    let (mut l, mut r) = (Vec::new(), Vec::new());
    // Apagada: nada (y la salida de referencia).
    let (mut controller, mut engine) = make(-1.0);
    assert!(!controller.stereo_viz_on());
    let quiet = offline::render(&mut engine, 8_000, 2);
    controller.stereo_samples(&mut l, &mut r);
    assert!(l.is_empty() && r.is_empty());
    // Encendida: lo mismo que sale, canal a canal.
    let (mut controller, mut engine) = make(-1.0);
    controller.set_stereo_viz(true);
    let out = offline::render(&mut engine, 8_000, 2);
    assert_eq!(out, quiet, "la cola cambia la mezcla");
    controller.stereo_samples(&mut l, &mut r);
    let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
    let right: Vec<f32> = out.chunks(2).map(|f| f[1]).collect();
    assert_eq!(l, left);
    assert_eq!(r, right);
    // Panoteada a la izquierda: la derecha, en silencio.
    let energy = |x: &[f32]| x.iter().map(|s| s * s).sum::<f32>();
    assert!(energy(&l) > 1.0 && energy(&r) < 1e-6 * energy(&l));
    // Llena (sin vaciar), descarta y se queda en su capacidad; apagada otra
    // vez, deja de escribir.
    offline::render(&mut engine, 3 * engine::STEREO_FRAMES, 2);
    l.clear();
    r.clear();
    controller.stereo_samples(&mut l, &mut r);
    assert_eq!(l.len(), engine::STEREO_FRAMES);
    controller.set_stereo_viz(false);
    offline::render(&mut engine, 4_000, 2);
    l.clear();
    controller.stereo_samples(&mut l, &mut r);
    assert!(l.is_empty());
}
