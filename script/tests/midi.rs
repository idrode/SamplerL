//! `pista:midi(canal, nota_base)`: el patrón manda sus golpes como notas.

use std::path::Path;

use engine::{new_engine, offline};
use script::ScriptHost;

const RATE: u32 = 48_000;

#[test]
fn tracks_send_their_hits_as_midi_notes() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("midi_script");
    std::fs::create_dir_all(&dir).unwrap();
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
    let source = "bpm(120)\nlocal b = sample(\"break.wav\"):slice(2)\n\
                  track(\"d\", b):steps(\"1 . . .  2@0.5 . . .  . . . .  . . . .\"):midi(3)\n";
    let (mut controller, mut engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let song = host
        .load(&mut controller, source, "song.lua", &dir)
        .unwrap();
    assert_eq!(song.tracks[0].midi, Some((3, 36)));
    controller.start().unwrap();
    offline::render(&mut engine, 16 * 6000, 2);
    let notes: Vec<(u64, u8, u8)> = controller
        .midi_events()
        .map(|n| (n.frame, n.note, n.velocity))
        .collect();
    assert_eq!(
        notes,
        [
            (0, 36, 100),
            (6000, 36, 0),
            (24_000, 37, 50),
            (30_000, 37, 0)
        ]
    );
    let mut host = ScriptHost::new(RATE);
    let err = host
        .load(
            &mut controller,
            "local b = sample(\"break.wav\"):slice(2)\ntrack(\"d\", b):midi(17)",
            "song.lua",
            &dir,
        )
        .err()
        .unwrap();
    assert_eq!(
        err.to_string(),
        "song.lua:2: midi(17, 36): el canal va de 1 a 16 y la nota de 0 a 127"
    );
}
