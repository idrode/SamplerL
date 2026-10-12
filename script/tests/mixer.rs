//! Fase D en el script: mute, solo, metrónomo, tempo detectado y lo que se
//! oye en el render.

#[path = "../../engine/tests/support/synth_break.rs"]
mod synth;

use std::path::{Path, PathBuf};

use engine::offline;
use script::ScriptHost;

const RATE: u32 = 48_000;

fn fixture(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("mixer_{test}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut eight = vec![0.0f32; 24_000];
    for k in 0..8 {
        eight[k * 3000..k * 3000 + 300].fill(0.2 + 0.05 * k as f32);
    }
    offline::write_wav(dir.join("break8.wav"), &eight, 1, RATE).unwrap();
    dir
}

fn render(dir: &Path, body: &str) -> Vec<f32> {
    let source = format!("bpm(120)\nlocal amen = sample(\"break8.wav\"):slice(8)\n{body}");
    let song = ScriptHost::new(RATE)
        .eval(&source, "song.lua", dir)
        .unwrap_or_else(|e| panic!("{e}"));
    song.render(RATE, 2).unwrap()
}

const A: &str = "track(\"a\", amen):steps(\"1 . . .  1 . . .  1 . . .  1 . . .\")";
const B: &str = "track(\"b\", amen):steps(\". . 3 .  . . 3 .  . . 3 .  . . 3 .\")";

#[test]
fn script_mute_and_solo_sound_like_the_tracks_that_remain() {
    let dir = fixture("mute_solo");
    let only_a = render(&dir, A);
    let only_b = render(&dir, B);
    assert_eq!(render(&dir, &format!("{A}\n{B}:mute()")), only_a);
    assert_eq!(render(&dir, &format!("{A}\n{B}:solo()")), only_b);
    assert_eq!(
        render(&dir, &format!("{A}:solo():mute()\n{B}:solo()")),
        only_b
    );
    // mute(false) y solo(false) deshacen.
    assert_eq!(
        render(&dir, &format!("{A}\n{B}:mute():mute(false)")),
        render(&dir, &format!("{A}\n{B}"))
    );
}

#[test]
fn the_metronome_only_goes_into_the_render_when_asked() {
    let dir = fixture("metronome");
    let source =
        format!("bpm(120)\nmetronome(true)\nlocal amen = sample(\"break8.wav\"):slice(8)\n{A}");
    let song = ScriptHost::new(RATE)
        .eval(&source, "song.lua", &dir)
        .unwrap();
    assert_eq!(song.metronome, Some(true));
    // El metronome() del script no entra en el render.
    assert_eq!(song.render(RATE, 1).unwrap(), render(&dir, A)[..2 * 96_000]);
    // Pedido, sí: la diferencia es el clic de cada pulso (el primero, con
    // acento, más fuerte).
    let with = song.render_with_metronome(RATE, 1, true).unwrap();
    let without = song.render(RATE, 1).unwrap();
    let click: Vec<f32> = with
        .iter()
        .zip(&without)
        .map(|(a, b)| a - b)
        .step_by(2)
        .collect();
    let peak = |at: usize| {
        click[at..at + 1440]
            .iter()
            .fold(0.0f32, |m, s| m.max(s.abs()))
    };
    assert!(peak(0) > 0.4, "acento: {}", peak(0));
    for beat in 1..4 {
        assert!((0.25..0.35).contains(&peak(beat * 24_000)), "pulso {beat}");
    }
    assert!(click[2000..23_000].iter().all(|s| s.abs() < 1e-6));
    let off = ScriptHost::new(RATE)
        .eval("metronome(false)", "song.lua", &dir)
        .unwrap();
    assert_eq!(off.metronome, Some(false));
    assert_eq!(
        ScriptHost::new(RATE)
            .eval("bpm(120)", "song.lua", &dir)
            .unwrap()
            .metronome,
        None
    );
}

#[test]
fn detect_bpm_reads_the_synthetic_break() {
    let dir = fixture("detect_bpm");
    let (audio, _) = synth::synth_break();
    offline::write_wav(dir.join("amen.wav"), &audio, 1, synth::RATE).unwrap();
    let source = "local amen = sample(\"amen.wav\")\n\
                  local tempo, compases = amen:detect_bpm()\n\
                  bpm(tempo)\n\
                  error(\"=\" .. tempo .. \" \" .. compases, 0)";
    let e = ScriptHost::new(RATE)
        .eval(source, "song.lua", &dir)
        .err()
        .unwrap();
    assert!(e.message.starts_with("=140 1"), "{e}");
    // En silencio, nil: bpm(nil) da error con línea.
    offline::write_wav(dir.join("silencio.wav"), &vec![0.0; 4800], 1, RATE).unwrap();
    let e = ScriptHost::new(RATE)
        .eval(
            "local t = sample(\"silencio.wav\"):detect_bpm()\nassert(t == nil, \"no\")",
            "song.lua",
            &dir,
        )
        .map(|_| ())
        .map_err(|e| e.message);
    assert_eq!(e, Ok(()));
}
