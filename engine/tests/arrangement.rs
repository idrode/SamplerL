//! Secciones y línea de tiempo en el motor: cada sección suena en su
//! compás exacto, sus vueltas cuentan desde su inicio, los saltos y la
//! repetición entran al compás siguiente y recargar conserva la posición.

use std::sync::Arc;

use engine::{
    Arrangement, Batch, Controller, Engine, Pattern, Sample, SendError, Slices, Step, new_engine,
    offline,
};

const RATE: u32 = 48_000;
/// A 120 BPM, un compás.
const BAR: usize = 96_000;

/// Un golpe de continua a `level` (1000 frames).
fn hit(level: f32) -> (Arc<Sample>, Slices) {
    let sample = Arc::new(Sample::from_interleaved(&vec![level; 1000], 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    (sample, slices)
}

/// Un golpe al empezar cada compás, de continua a `level`.
fn downbeat(level: f32) -> Pattern {
    let (sample, slices) = hit(level);
    let mut steps = vec![None; 16];
    steps[0] = Some(Step::new(0));
    Pattern::from_steps(sample, &slices, steps).unwrap()
}

/// Ciclo de dos vueltas: la primera con golpe a `a`, la segunda a `b`
/// (dos samples no caben en un patrón: el volumen del paso los distingue).
fn two_laps() -> Pattern {
    let (sample, slices) = hit(0.5);
    let lap = |gain: f32| {
        let mut steps = vec![None; 16];
        steps[0] = Some(Step::new(0).with_gain(gain));
        steps
    };
    Pattern::from_bars(sample, &slices, &[lap(1.0), lap(0.5)]).unwrap()
}

/// Nivel que suena a mitad del golpe de cada compás (tras el fundido).
fn levels(audio: &[f32], bars: usize) -> Vec<f32> {
    (0..bars).map(|b| audio[2 * (b * BAR + 500)]).collect()
}

fn arrangement(sections: Vec<(u64, Pattern)>, timeline: &[(usize, u32)]) -> Arrangement {
    let mut a = Arrangement::new();
    for (id, pattern) in sections {
        a.add_section(id, vec![(0, pattern)]).unwrap();
    }
    for &(s, bars) in timeline {
        a.push(s, bars).unwrap();
    }
    a
}

fn start(arrangement: Arrangement) -> (Controller, Engine) {
    let (mut controller, engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch.set_bpm(120.0).set_arrangement(Some(arrangement));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    (controller, engine)
}

#[test]
fn each_section_sounds_in_its_exact_bar() {
    let a = arrangement(
        vec![(1, downbeat(0.5)), (2, downbeat(0.25))],
        &[(0, 2), (1, 1)],
    );
    let (controller, mut engine) = start(a);
    let audio = offline::render(&mut engine, 4 * BAR, 2);
    // A A B y vuelta a empezar.
    assert_eq!(levels(&audio, 4), [0.5, 0.5, 0.25, 0.5]);
    // El cambio, justo en el frame del compás: antes, nada de B.
    let first_b = (2 * BAR..3 * BAR).find(|&f| audio[2 * f] != 0.0).unwrap();
    assert_eq!(first_b, 2 * BAR + 1, "el golpe de B empieza en su compás");
    let status = controller.status();
    let section = status.section.unwrap();
    assert_eq!((section.entry, section.bar), (0, 0));
}

#[test]
fn laps_count_from_the_start_of_the_section() {
    // Una sección de un compás y luego otra con un ciclo de dos vueltas:
    // la segunda empieza por su primera vuelta aunque la canción vaya por
    // el compás 1.
    let a = arrangement(
        vec![(1, downbeat(0.25)), (2, two_laps())],
        &[(0, 1), (1, 4)],
    );
    let (_controller, mut engine) = start(a);
    let audio = offline::render(&mut engine, 5 * BAR, 2);
    assert_eq!(levels(&audio, 5), [0.25, 0.5, 0.25, 0.5, 0.25]);
}

#[test]
fn without_an_arrangement_nothing_changes() {
    let render = |clear: bool| {
        let (mut controller, mut engine) = new_engine(RATE);
        let mut batch = Batch::new();
        batch.set_bpm(120.0).set_pattern(0, Some(two_laps()));
        if clear {
            batch.set_arrangement(None);
        }
        controller.apply(batch).unwrap();
        controller.start().unwrap();
        offline::render(&mut engine, 3 * BAR, 2)
    };
    assert!(render(false) == render(true));
}

/// Manda un arreglo nuevo con la canción en marcha.
fn reload(controller: &mut Controller, arrangement: Arrangement) {
    let mut batch = Batch::new();
    batch.set_arrangement(Some(arrangement));
    controller.apply(batch).unwrap();
}

#[test]
fn reloading_keeps_the_place() {
    let make = || {
        arrangement(
            vec![(1, downbeat(0.5)), (2, two_laps())],
            &[(0, 1), (1, 4), (0, 1)],
        )
    };
    let (mut controller, mut engine) = start(make());
    let mut audio = offline::render(&mut engine, 2 * BAR + BAR / 2, 2);
    // A mitad del compás 2 (el segundo de la sección de dos vueltas) se
    // recarga lo mismo: sigue por su compás 3 (su segunda vuelta otra vez,
    // contada desde su inicio).
    reload(&mut controller, make());
    audio.extend(offline::render(&mut engine, 4 * BAR - BAR / 2, 2));
    let (_c, mut reference) = start(make());
    let plain = offline::render(&mut reference, 6 * BAR, 2);
    assert_eq!(levels(&audio, 6), levels(&plain, 6));
    assert_eq!(levels(&audio, 6), [0.5, 0.5, 0.25, 0.5, 0.25, 0.5]);
    assert!(!controller.status().section_lost);
}

#[test]
fn a_missing_section_goes_back_to_the_start() {
    let (mut controller, mut engine) = start(arrangement(
        vec![(1, downbeat(0.5)), (2, downbeat(0.25))],
        &[(0, 1), (1, 4)],
    ));
    offline::render(&mut engine, BAR + BAR / 2, 2);
    // La sección 2 desaparece: el compás siguiente es el principio.
    reload(
        &mut controller,
        arrangement(
            vec![(1, downbeat(0.5)), (3, downbeat(0.125))],
            &[(1, 1), (0, 1)],
        ),
    );
    let audio = offline::render(&mut engine, BAR / 2 + 2 * BAR, 2);
    let status = controller.status();
    assert!(status.section_lost);
    let next = |b: usize| audio[2 * (BAR / 2 + b * BAR + 500)];
    assert_eq!([next(0), next(1)], [0.125, 0.5]);
    assert!(!controller.status().section_lost, "se lee una vez");
}

#[test]
fn jumps_and_loops_enter_on_the_next_bar() {
    let (mut controller, mut engine) = start(arrangement(
        vec![
            (1, downbeat(0.5)),
            (2, downbeat(0.25)),
            (3, downbeat(0.125)),
        ],
        &[(0, 4), (1, 1), (2, 1)],
    ));
    offline::render(&mut engine, BAR / 2, 2);
    // A mitad del primer compás: saltar a la entrada 2.
    controller.jump_to(2).unwrap();
    let audio = offline::render(&mut engine, BAR / 2 + 3 * BAR, 2);
    let at = |b: usize| audio[2 * (BAR / 2 + b * BAR + 500)];
    // Sigue el compás en curso, luego la 2, y la línea de tiempo vuelve a
    // empezar.
    assert_eq!([at(0), at(1)], [0.125, 0.5]);
    // Repetir: la entrada 0 (de 4 compases) ya no pasa a la siguiente.
    controller.loop_section(true).unwrap();
    let audio = offline::render(&mut engine, 6 * BAR, 2);
    assert!(controller.status().section.unwrap().looping);
    // (Esta parte empieza justo al empezar un compás.)
    assert_eq!(levels(&audio, 6), [0.5; 6]);
    // Volver a la línea de tiempo: al acabar la entrada, la siguiente.
    controller.loop_section(false).unwrap();
    let audio = offline::render(&mut engine, 4 * BAR, 2);
    let after = levels(&audio, 4);
    assert!(after.contains(&0.25), "{after:?}");
}

#[test]
fn starting_again_goes_back_to_the_start() {
    let (mut controller, mut engine) = start(arrangement(
        vec![(1, downbeat(0.5)), (2, downbeat(0.25))],
        &[(0, 1), (1, 1)],
    ));
    offline::render(&mut engine, BAR + BAR / 2, 2);
    controller.stop().unwrap();
    controller.start().unwrap();
    let audio = offline::render(&mut engine, 2 * BAR, 2);
    assert_eq!(levels(&audio, 2), [0.5, 0.25]);
}

#[test]
fn arrangements_are_checked() {
    let mut a = Arrangement::new();
    assert!(a.push(0, 1).is_err(), "sección que no existe");
    let s = a.add_section(1, vec![(0, downbeat(0.5))]).unwrap();
    assert!(matches!(
        a.push(s, 0),
        Err(SendError::InvalidArrangement(_))
    ));
    assert!(a.add_section(2, vec![(9, downbeat(0.5))]).is_err());
    let (mut controller, _engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch.set_arrangement(Some(Arrangement::new()));
    assert!(matches!(
        controller.apply(batch),
        Err(SendError::InvalidArrangement(_))
    ));
}
