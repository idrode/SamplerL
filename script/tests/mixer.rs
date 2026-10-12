//! Fase D en el script: mute, solo y lo que se oye en el render.

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
