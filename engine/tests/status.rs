//! Estado del motor para la interfaz y salida nula (sin dispositivo).

use std::sync::Arc;
use std::time::{Duration, Instant};

use engine::notation::parse_steps;
use engine::{Pattern, Sample, Slices, new_engine, offline, open_null_output};

const RATE: u32 = 48_000;
const STEP: usize = 6000;

fn pattern() -> Pattern {
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::detect(&sample, 2);
    let steps = parse_steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .").unwrap();
    Pattern::from_steps(sample, &slices, steps).unwrap()
}

#[test]
fn status_follows_the_sequencer() {
    let (mut controller, mut engine) = new_engine(RATE);
    let before = controller.status();
    assert!(!before.running);
    assert_eq!(before.step, None);
    assert_eq!(before.tracks[0].hit_start, None);

    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern()).unwrap();
    controller.start().unwrap();
    // Justo antes del paso 2 se lee (y vacía) el pico del paso 0.
    offline::render(&mut engine, 2 * STEP - 100, 2);
    assert!((controller.status().tracks[0].peak - 0.5).abs() < 1e-6);
    offline::render(&mut engine, 200, 2);
    let status = controller.status();
    assert!(status.running);
    assert_eq!(status.bpm, 120.0);
    assert_eq!((status.bar, status.step), (0, Some(2)));
    // Paso 2: corte 2, que empieza en el frame 3000.
    assert_eq!(status.tracks[0].hit_start, Some(3000));
    assert!(
        (status.tracks[0].peak - 0.25).abs() < 1e-6,
        "{}",
        status.tracks[0].peak
    );
    // El pico se pone a cero al leerlo; las demás pistas, en silencio.
    assert_eq!(controller.status().tracks[0].peak, 0.0);
    assert_eq!(status.tracks[1].hit_start, None);

    // Compás siguiente.
    offline::render(&mut engine, 16 * STEP, 2);
    let status = controller.status();
    assert_eq!((status.bar, status.step), (1, Some(2)));

    controller.stop().unwrap();
    offline::render(&mut engine, 512, 2);
    assert!(!controller.status().running);
}

#[test]
fn null_output_moves_the_engine_in_real_time() {
    let (mut controller, engine) = new_engine(RATE);
    controller.set_bpm(400.0).unwrap();
    controller.set_pattern(pattern()).unwrap();
    controller.start().unwrap();
    let output = open_null_output(engine, RATE);
    // A 400 BPM, un paso dura 37,5 ms: en ~300 ms deben pasar varios.
    let start = Instant::now();
    let mut last = None;
    while start.elapsed() < Duration::from_millis(300) {
        std::thread::sleep(Duration::from_millis(10));
        last = controller.status().step;
    }
    drop(output);
    let status = controller.status();
    assert!(status.running);
    assert!(last.is_some_and(|s| s >= 3), "{last:?}");
    // Tras soltar la salida, el motor ya no avanza.
    std::thread::sleep(Duration::from_millis(80));
    assert_eq!(controller.status().step, status.step);
}

#[test]
fn the_master_meter_sees_the_sum_before_the_limiter() {
    // Una pista: pico 0,5, el limitador no actúa.
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    controller.set_pattern(pattern()).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, STEP, 2);
    let master = controller.status().master;
    assert!((master.peak - 0.5).abs() < 1e-6, "{master:?}");
    assert_eq!(master.limited_frames, 0);

    // Tres pistas a la vez a 0,5: la suma (1,5) satura y el limitador actúa
    // en los frames en que pasa de 0,9 (los 800 del bloque, salvo el fundido
    // de entrada). La salida nunca pasa de 1.
    let (mut controller, mut engine) = new_engine(RATE);
    controller.set_bpm(120.0).unwrap();
    for t in 0..3 {
        controller.set_track_pattern(t, Some(pattern())).unwrap();
    }
    controller.start().unwrap();
    let audio = offline::render(&mut engine, STEP, 2);
    let master = controller.status().master;
    assert!((master.peak - 1.5).abs() < 1e-5, "{master:?}");
    assert!((700..=800).contains(&master.limited_frames), "{master:?}");
    assert!(audio.iter().all(|s| s.abs() <= 1.0));
    // Se vacía al leerlo.
    assert_eq!(controller.status().master, engine::MasterStatus::default());
}

#[test]
fn the_position_follows_the_song_in_steps() {
    let (mut controller, mut engine) = engine::new_engine(48_000);
    assert_eq!(controller.position(), None);
    controller.set_bpm(120.0).unwrap();
    controller.start().unwrap();
    // 120 BPM: 6000 frames por paso. Tras 15000 frames, el paso 2,5.
    engine::offline::render(&mut engine, 15_000, 2);
    assert_eq!(controller.position(), Some(2.5));
    // Leerla no vacía los picos ni cambia nada.
    assert_eq!(controller.position(), Some(2.5));
    controller.stop().unwrap();
    engine::offline::render(&mut engine, 100, 2);
    assert_eq!(controller.position(), None);
}
