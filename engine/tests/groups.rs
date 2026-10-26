//! Grupos: las pistas de un grupo se suman antes de sus efectos, su volumen
//! y su pan; los envíos de cada pista no pasan por el grupo y los del
//! grupo mandan la suma. Los grupos entran al compás, y sin efectos ni
//! volumen un grupo no cambia nada.

use std::sync::Arc;

use engine::bus::{Buses, DelayParams, Division, delay_frames};
use engine::{
    Batch, Fx, Group, Groups, Pattern, STEPS, Sample, SendError, Slices, Step, bar_frames,
    new_engine, offline,
};

const RATE: u32 = 48_000;
const BPM: f64 = 120.0;

/// Un sample con un golpe de 400 frames a `level` (un seno grave, para que
/// los efectos tengan algo que hacer) y silencio hasta 3000.
fn hit(level: f32) -> (Arc<Sample>, Slices) {
    let mut mono = vec![0.0f32; 3000];
    for (i, v) in mono[..400].iter_mut().enumerate() {
        *v = level * (i as f32 * 0.05).sin();
    }
    let sample = Arc::new(Sample::from_interleaved(&mono, 1, RATE, RATE).unwrap());
    let slices = Slices::grid(&sample, 1);
    (sample, slices)
}

fn on(steps_on: &[usize]) -> Vec<Option<Step>> {
    let mut steps = vec![None; STEPS];
    for &k in steps_on {
        steps[k] = Some(Step::new(0));
    }
    steps
}

fn pattern(level: f32, steps_on: &[usize]) -> Pattern {
    let (sample, slices) = hit(level);
    Pattern::from_steps(sample, &slices, on(steps_on)).unwrap()
}

/// Renderiza las pistas (`patterns[i]` en la pista `i`) con esos grupos y
/// buses.
fn render(patterns: Vec<Pattern>, groups: Option<Groups>, buses: Buses, frames: usize) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch.set_bpm(BPM).set_buses(buses);
    for (t, p) in patterns.into_iter().enumerate() {
        batch.set_pattern(t, Some(p));
    }
    if let Some(groups) = groups {
        batch.set_groups(groups);
    }
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    offline::render(&mut engine, frames, 2)
}

/// Las pistas `members` en el grupo 0.
fn grouped(group: Group, members: &[usize]) -> Groups {
    let mut groups = Groups::new();
    groups.groups[0] = Some(group);
    for &t in members {
        groups.member[t] = Some(0);
    }
    groups
}

fn bar() -> usize {
    bar_frames(BPM, RATE) as usize
}

#[test]
fn a_plain_group_changes_nothing() {
    let tracks = || vec![pattern(0.3, &[0, 4]), pattern(0.2, &[0, 6])];
    let frames = 2 * bar();
    let direct = render(tracks(), None, Buses::default(), frames);
    let through = render(
        tracks(),
        Some(grouped(Group::new(), &[0, 1])),
        Buses::default(),
        frames,
    );
    assert!(direct.iter().any(|&v| v != 0.0));
    assert_eq!(direct, through);
}

#[test]
fn the_group_gain_and_pan_apply_to_its_tracks() {
    let tracks = || vec![pattern(0.3, &[0, 4]), pattern(0.2, &[0, 6])];
    let frames = bar();
    let direct = render(tracks(), None, Buses::default(), frames);
    let half = render(
        tracks(),
        Some(grouped(Group::new().with_gain(0.5).unwrap(), &[0, 1])),
        Buses::default(),
        frames,
    );
    for (d, h) in direct.iter().zip(&half) {
        assert_eq!(*h, 0.5 * d);
    }
    // Pan a la izquierda del todo: el derecho, en silencio.
    let left = render(
        tracks(),
        Some(grouped(Group::new().with_pan(-1.0).unwrap(), &[0, 1])),
        Buses::default(),
        frames,
    );
    for (d, l) in direct.chunks(2).zip(left.chunks(2)) {
        assert_eq!(l[0], d[0]);
        assert_eq!(l[1], 0.0);
    }
}

#[test]
fn only_the_members_go_through_the_group() {
    let frames = bar();
    let solo = |t: usize| {
        let mut tracks = vec![pattern(0.3, &[0]), pattern(0.2, &[4])];
        let mut silent = tracks.remove(t);
        silent = silent.with_gain(0.0).unwrap();
        tracks.insert(t, silent);
        tracks
    };
    // La pista 1 callada: sigue sonando la 0, sin grupo.
    let mute = Group::new().with_gain(0.0).unwrap();
    let out = render(solo(1), Some(grouped(mute, &[1])), Buses::default(), frames);
    let alone = render(solo(1), None, Buses::default(), frames);
    assert_eq!(out, alone);
    // Y la que está en el grupo callado, no.
    let out = render(solo(0), Some(grouped(mute, &[1])), Buses::default(), frames);
    assert!(out.iter().all(|&v| v == 0.0));
}

#[test]
fn the_group_effects_see_the_sum_of_its_tracks() {
    // Una distorsión sobre el grupo suena como una sola pista que tocara
    // los dos golpes sumados, no como cada uno distorsionado aparte.
    let drive = Fx::distortion(8.0, 1.0);
    let frames = bar();
    let both = || vec![pattern(0.3, &[0]), pattern(0.2, &[0])];
    let glued = render(
        both(),
        Some(grouped(Group::new().with_fx(&[drive]).unwrap(), &[0, 1])),
        Buses::default(),
        frames,
    );
    let summed = render(
        vec![pattern(0.5, &[0]).with_fx(&[drive]).unwrap()],
        None,
        Buses::default(),
        frames,
    );
    let apart = render(
        vec![
            pattern(0.3, &[0]).with_fx(&[drive]).unwrap(),
            pattern(0.2, &[0]).with_fx(&[drive]).unwrap(),
        ],
        None,
        Buses::default(),
        frames,
    );
    let diff = |a: &[f32], b: &[f32]| {
        a.iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max)
    };
    assert!(diff(&glued, &summed) < 1e-4, "{}", diff(&glued, &summed));
    assert!(diff(&glued, &apart) > 0.05, "{}", diff(&glued, &apart));
}

#[test]
fn track_sends_skip_the_group_and_group_sends_take_the_sum() {
    let division = Division::Eighth;
    let d = delay_frames(division, BPM, RATE);
    let buses = Buses {
        delay: DelayParams {
            division,
            feedback: 0.0,
            level: 1.0,
        },
        ..Buses::default()
    };
    let frames = 2 * d + 3000;
    // Grupo callado y la pista con envío: solo se oye el eco.
    let sending = pattern(0.3, &[0]).with_sends([0.5, 0.0]).unwrap();
    let dry = render(vec![pattern(0.3, &[0])], None, buses, frames);
    let echo = render(
        vec![sending],
        Some(grouped(Group::new().with_gain(0.0).unwrap(), &[0])),
        buses,
        frames,
    );
    assert!(echo[..2 * d].iter().all(|&v| v == 0.0));
    for t in 0..3000 {
        assert_eq!(echo[2 * (d + t)], 0.5 * dry[2 * t], "t={t}");
    }
    // El envío del grupo manda la suma de sus pistas (tras su volumen).
    let group = Group::new()
        .with_gain(0.5)
        .unwrap()
        .with_sends([1.0, 0.0])
        .unwrap();
    let both = render(
        vec![pattern(0.3, &[0]), pattern(0.2, &[0])],
        Some(grouped(group, &[0, 1])),
        buses,
        frames,
    );
    let plain = render(
        vec![pattern(0.3, &[0]), pattern(0.2, &[0])],
        None,
        buses,
        frames,
    );
    for t in 0..3000 {
        let at = 2 * (d + t);
        assert!(
            (both[at] - 0.5 * plain[2 * t]).abs() < 1e-6,
            "t={t}: {} {}",
            both[at],
            plain[2 * t]
        );
    }
}

#[test]
fn groups_enter_at_the_bar() {
    let (mut controller, mut engine) = new_engine(RATE);
    let mut batch = Batch::new();
    batch
        .set_bpm(BPM)
        .set_pattern(0, Some(pattern(0.3, &[0, 4, 8, 12])));
    controller.apply(batch).unwrap();
    controller.start().unwrap();
    let first = offline::render(&mut engine, bar() / 2, 2);
    let mut batch = Batch::new();
    batch.set_groups(grouped(Group::new().with_gain(0.0).unwrap(), &[0]));
    controller.apply(batch).unwrap();
    // El resto del compás suena igual; desde el siguiente, callado.
    let rest = offline::render(&mut engine, bar() - bar() / 2, 2);
    let next = offline::render(&mut engine, bar(), 2);
    assert!(first.iter().any(|&v| v != 0.0));
    assert!(rest.iter().any(|&v| v != 0.0));
    assert!(next.iter().all(|&v| v == 0.0));
}

#[test]
fn a_track_cannot_go_to_a_missing_group() {
    let (mut controller, _engine) = new_engine(RATE);
    let mut groups = Groups::new();
    groups.member[2] = Some(1);
    let mut batch = Batch::new();
    batch.set_groups(groups);
    assert!(matches!(
        controller.apply(batch),
        Err(SendError::InvalidGroup(_))
    ));
    assert!(matches!(
        Group::new().with_gain(5.0),
        Err(SendError::InvalidGroup(_))
    ));
    assert!(matches!(
        Group::new().with_fx(&[Fx::gain(1.0); 5]),
        Err(SendError::InvalidGroup(_))
    ));
}
