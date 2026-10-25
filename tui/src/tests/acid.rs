//! El acid (`synth`) en la interfaz: notas con accent y slide en el
//! tracker, su tipo y sus parámetros en lugar del sample, los pads y el
//! MIDI tocan notas en vivo, y el resaltado en vivo.

use super::*;
use crate::midi::{MidiEvent, TimedEvent};
use std::time::{Duration, Instant};

const ACID: &str = r#"bpm(120)
local bass = synth("acid", { cutoff = 320, res = 0.8, envmod = lfo("4 pasos", 0.2, 0.9) })
track("bass", bass):melody("c c! eb/ .  . . . .  . . . .  . . . .")
"#;

fn rms(x: &[f32]) -> f32 {
    (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
}

#[test]
fn the_tracker_writes_accents_and_slides_and_the_panels_name_the_synth() {
    let (_, mut app, mut engine) = open("acid_view", Some(ACID));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let view = app.view();
    let track = &view.tracks[0];
    assert_eq!(track.cell_text(&track.steps[0]), "C2");
    assert_eq!(track.cell_text(&track.steps[1]), "C2!");
    assert_eq!(track.cell_text(&track.steps[2]), "D#2/");
    let synth = track.synth.clone().unwrap();
    assert_eq!(synth.kind, "acid");
    assert!(synth.params.contains("cutoff 320") && synth.params.contains("envmod ∿"));
    offline::render(&mut engine, STEP / 2, 2);
    app.tick();
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("bass · ♪ acid C2 1/1"), "{screen}");
    assert!(screen.contains("sintetizador · bass · acid"), "{screen}");
    assert!(screen.contains("C2!"), "{screen}");
}

#[test]
fn pads_and_midi_play_live_synth_notes() {
    let quiet = "bpm(120)\ntrack(\"bass\", synth(\"acid\")):steps(\". . . .  . . . .  . . . .  . . . .\")\n";
    let (_, mut app, mut engine) = open("acid_live", Some(quiet));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    app.handle(esc());
    let t0 = Instant::now();
    // El pad 4: cuatro semitonos sobre la raíz (C2).
    app.press_pad_at(4, t0);
    assert_eq!(app.view().message, "♪ bass: E2");
    let held = offline::render(&mut engine, STEP, 2);
    assert!(rms(&held) > 0.01, "el pad no suena");
    app.release_expired_pads(t0 + Duration::from_secs(1));
    offline::render(&mut engine, STEP, 2);
    let after = offline::render(&mut engine, STEP, 2);
    assert!(rms(&after) < 1e-4, "no se suelta: {}", rms(&after));

    // Una nota MIDI: a su altura, hasta su note off.
    app.midi_event(TimedEvent {
        micros: 0,
        event: MidiEvent::NoteOn {
            channel: 1,
            note: 43,
            velocity: 100,
        },
    });
    assert_eq!(app.view().message, "♪ bass: G2");
    assert!(rms(&offline::render(&mut engine, STEP, 2)) > 0.01);
    app.midi_event(TimedEvent {
        micros: 0,
        event: MidiEvent::NoteOff {
            channel: 1,
            note: 43,
        },
    });
    offline::render(&mut engine, STEP, 2);
    assert!(rms(&offline::render(&mut engine, STEP, 2)) < 1e-4);
}

#[test]
fn the_sounding_note_of_a_synth_lights_up() {
    let (_, mut app, mut engine) = open("acid_highlight", Some(ACID));
    offline::render(&mut engine, 100, 2);
    app.tick();
    let view = app.view();
    assert_eq!(view.highlights.len(), 1, "{:?}", view.highlights);
    let h = &view.highlights[0];
    let line = &ACID.lines().nth(h.row).unwrap();
    let token: String = line.chars().skip(h.cols.start).take(h.cols.len()).collect();
    assert_eq!(token, "c");
    // El paso 2, el ligado: su token entero, con la marca.
    offline::render(&mut engine, 2 * STEP, 2);
    app.tick();
    let h = &app.view().highlights[0];
    let token: String = line.chars().skip(h.cols.start).take(h.cols.len()).collect();
    assert_eq!(token, "eb/");
}

/// La interfaz con un acid, para verla sin terminal:
/// `cargo test -p tui pantalla_acid -- --ignored --nocapture`.
#[test]
#[ignore]
fn pantalla_acid() {
    let (_, mut app, mut engine) = open("acid_screen", Some(ACID));
    offline::render(&mut engine, STEP + 100, 2);
    app.tick();
    println!("{}", text(&draw(&mut app, 140, 40)));
}

/// El sub en los paneles: su tipo, su raíz (C1) y sus parámetros,
/// también los de la envolvente; el tracker, por su nombre.
#[test]
fn the_sub_shows_its_params() {
    let song = "bpm(120)\n\
                track(\"sub\", synth(\"subbass\", { tone = 180, release = 120 }))\n\
                  :melody(\"c _ _ _  . . g0/ _  . . . .  . . . .\")\n";
    let (_, mut app, mut engine) = open("sub_view", Some(song));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let view = app.view();
    let track = &view.tracks[0];
    assert_eq!(track.cell_text(&track.steps[0]), "C1");
    assert_eq!(track.cell_text(&track.steps[6]), "G0/");
    let synth = track.synth.clone().unwrap();
    assert_eq!(synth.kind, "sub");
    assert_eq!(
        synth.params,
        "wave sine · root C1 · tone 180 · drive 0 · glide 40 · attack 5 · release 120"
    );
    // Con el sub sonando, también el bloque sub.
    let song = "bpm(120)\n\
                track(\"b\", synth(\"acid\", { sub = 0.5, sub_div = 3, sub_wave = \"sine\" }))\n\
                  :melody(\"c . . .  . . . .  . . . .  . . . .\")\n";
    let (_, with_sub, _with_sub_engine) = open("sub_block_view", Some(song));
    let params = with_sub.view().tracks[0].synth.clone().unwrap().params;
    assert!(
        params.ends_with("sub 0.5 · sub_div 3 · sub_wave sine · sub_pan 0 · sub_drive 0"),
        "{params}"
    );
    offline::render(&mut engine, STEP / 2, 2);
    app.tick();
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("sub · ♪ sub C1 1/1"), "{screen}");
}

/// El subharm en el panel: sus parámetros y las capas que suenan.
#[test]
fn the_subharm_shows_its_layers() {
    let song = "bpm(120)\n\
                track(\"sh\", subharm{ s1 = { div = 3, pan = 0.5, level = 0.7 } })\n  \
                  :melody(\"c _ _ _  . . . .  . . . .  . . . .\")\n";
    let (_, mut app, mut engine) = open("subharm_view", Some(song));
    assert!(app.view().error.is_none(), "{:?}", app.view().error);
    let params = app.view().tracks[0].synth.clone().unwrap().params;
    assert!(
        params.ends_with("o1 ÷1 · s1 ÷3 pan 0.5 level 0.7"),
        "{params}"
    );
    offline::render(&mut engine, STEP / 2, 2);
    app.tick();
    let screen = text(&draw(&mut app, 140, 40));
    assert!(screen.contains("sintetizador · sh · subharm"), "{screen}");
    assert!(screen.contains("sh · ♪ subharm C2 1/1"), "{screen}");
}
