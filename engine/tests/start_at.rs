//! Arrancar en un compás cualquiera (`Controller::start_at`): suena lo que
//! sonaría si la canción hubiera llegado hasta él (las vueltas de los
//! patrones, la rampa de tempo, la entrada de la línea de tiempo) y, con
//! el compás 0, exactamente lo mismo que `start`.

use std::sync::Arc;

use engine::{
    Arrangement, Batch, Controller, Engine, MAX_START_BAR, Pattern, Sample, SendError, Slices,
    Step, TempoRamp, new_engine, offline,
};

const RATE: u32 = 48_000;
/// A 120 BPM, un compás.
const BAR: usize = 96_000;

fn hit(level: f32) -> (Arc<Sample>, Slices) {
    let sample = Arc::new(Sample::from_interleaved(&vec![level; 1000], 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    (sample, slices)
}

/// Un ciclo de `gains.len()` vueltas con un golpe al empezar cada una, a
/// ese volumen (0: la vuelta calla).
fn laps(gains: &[f32]) -> Pattern {
    let (sample, slices) = hit(0.5);
    let bars: Vec<Vec<Option<Step>>> = gains
        .iter()
        .map(|&gain| {
            let mut steps = vec![None; 16];
            if gain > 0.0 {
                steps[0] = Some(Step::new(0).with_gain(gain));
            }
            steps
        })
        .collect();
    Pattern::from_bars(sample, &slices, &bars).unwrap()
}

fn downbeat(level: f32) -> Pattern {
    let (sample, slices) = hit(level);
    let mut steps = vec![None; 16];
    steps[0] = Some(Step::new(0));
    Pattern::from_steps(sample, &slices, steps).unwrap()
}

/// Nivel a mitad del golpe de cada compás.
fn levels(audio: &[f32], bars: usize) -> Vec<f32> {
    (0..bars).map(|b| audio[2 * (b * BAR + 500)]).collect()
}

fn engine_with(batch: Batch) -> (Controller, Engine) {
    let (mut controller, engine) = new_engine(RATE);
    controller.apply(batch).unwrap();
    (controller, engine)
}

#[test]
fn starting_at_a_bar_plays_the_lap_that_falls_there() {
    let mut batch = Batch::new();
    batch
        .set_bpm(120.0)
        .set_pattern(0, Some(laps(&[1.0, 0.5, 0.25, 0.125])));
    let (mut controller, mut engine) = engine_with(batch);
    controller.start_at(2).unwrap();
    let audio = offline::render(&mut engine, 4 * BAR, 2);
    // Las vueltas 2, 3, 0, 1: como si hubieran pasado dos compases.
    assert_eq!(levels(&audio, 4), [0.125, 0.0625, 0.5, 0.25]);
    let status = controller.status();
    assert_eq!(status.bar, 5, "el compás cuenta desde el de arranque");
}

#[test]
fn the_bar_after_a_start_at_matches_the_song_that_got_there() {
    // Solo la última vuelta tiene golpe: lo de antes no deja cola, así que
    // arrancar en ella suena bit a bit como llegar a ella.
    let pattern = || laps(&[0.0, 0.0, 0.0, 1.0]);
    let mut batch = Batch::new();
    batch.set_bpm(120.0).set_pattern(0, Some(pattern()));
    let (mut controller, mut engine) = engine_with(batch);
    controller.start().unwrap();
    let whole = offline::render(&mut engine, 4 * BAR, 2);

    let mut batch = Batch::new();
    batch.set_bpm(120.0).set_pattern(0, Some(pattern()));
    let (mut controller, mut engine) = engine_with(batch);
    controller.start_at(3).unwrap();
    let cued = offline::render(&mut engine, BAR, 2);
    assert!(cued.iter().any(|&s| s != 0.0));
    assert_eq!(&whole[2 * 3 * BAR..], &cued[..]);
}

#[test]
fn start_at_zero_is_start() {
    let pattern = || laps(&[1.0, 0.5]);
    let render = |at: Option<u64>| {
        let mut batch = Batch::new();
        batch.set_bpm(133.0).set_pattern(0, Some(pattern()));
        let (mut controller, mut engine) = engine_with(batch);
        match at {
            Some(bar) => controller.start_at(bar).unwrap(),
            None => controller.start().unwrap(),
        }
        offline::render(&mut engine, 3 * BAR, 2)
    };
    assert_eq!(render(None), render(Some(0)));
}

#[test]
fn the_tempo_ramp_counts_the_bars_before_the_start() {
    let ramp = TempoRamp {
        from: 100.0,
        to: 180.0,
        bars: 8,
    };
    let mut batch = Batch::new();
    batch
        .set_tempo_ramp(ramp)
        .set_pattern(0, Some(downbeat(0.5)));
    let (mut controller, mut engine) = engine_with(batch);
    controller.start_at(4).unwrap();
    offline::render(&mut engine, 512, 2);
    let status = controller.status();
    assert_eq!(status.bpm, ramp.bpm_at(4), "a mitad de la rampa");
    // La posición: los pasos de los cuatro compases de antes.
    let position = controller.position().unwrap();
    assert!((64.0..65.0).contains(&position), "posición {position}");

    // Pasada la rampa, su tempo final.
    controller.stop().unwrap();
    controller.start_at(20).unwrap();
    offline::render(&mut engine, 512, 2);
    assert_eq!(controller.status().bpm, 180.0);
}

#[test]
fn with_a_timeline_it_starts_in_the_entry_where_the_bar_falls() {
    let arrangement = || {
        let mut a = Arrangement::new();
        a.add_section(1, vec![(0, downbeat(0.5))]).unwrap();
        a.add_section(2, vec![(0, laps(&[1.0, 0.5, 0.25]))])
            .unwrap();
        a.push(0, 2).unwrap();
        a.push(1, 3).unwrap();
        a
    };
    let cue = |bar: u64| {
        let mut batch = Batch::new();
        batch.set_bpm(120.0).set_arrangement(Some(arrangement()));
        let (mut controller, mut engine) = engine_with(batch);
        controller.start_at(bar).unwrap();
        let audio = offline::render(&mut engine, 4 * BAR, 2);
        (levels(&audio, 4), controller)
    };
    // En la entrada B, su primer compás: sus vueltas desde el principio.
    let (heard, _) = cue(2);
    assert_eq!(heard, [0.5, 0.25, 0.125, 0.5]);
    // A media entrada B: sus vueltas siguen por donde irían.
    let (heard, _) = cue(3);
    assert_eq!(heard, [0.25, 0.125, 0.5, 0.5]);
    // Pasado el final, da la vuelta: el compás 6 es el segundo de A.
    let (heard, controller) = cue(6);
    assert_eq!(heard, [0.5, 0.5, 0.25, 0.125]);
    let status = controller.status();
    assert_eq!(status.section.map(|s| (s.entry, s.bar)), Some((1, 2)));

    // Un arreglo que ya sonaba: al volver a arrancar, también.
    let mut batch = Batch::new();
    batch.set_bpm(120.0).set_arrangement(Some(arrangement()));
    let (mut controller, mut engine) = engine_with(batch);
    controller.start().unwrap();
    offline::render(&mut engine, 3 * BAR, 2);
    controller.stop().unwrap();
    offline::render(&mut engine, 512, 2);
    controller.start_at(3).unwrap();
    let audio = offline::render(&mut engine, 2 * BAR, 2);
    assert_eq!(levels(&audio, 2), [0.25, 0.125]);
}

#[test]
fn a_bar_too_far_is_an_error() {
    let (mut controller, _engine) = new_engine(RATE);
    assert_eq!(
        controller.start_at(MAX_START_BAR + 1),
        Err(SendError::InvalidStartBar(MAX_START_BAR + 1))
    );
    assert!(controller.start_at(MAX_START_BAR).is_ok());
}
