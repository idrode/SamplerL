//! Remuestreo: `Song::render_track` suena como la canción con solo esa
//! pista, y dura lo que tarda en repetirse.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;

fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("resample");
        std::fs::create_dir_all(&dir).unwrap();
        let mut mono = vec![0.0f32; 6000];
        mono[..800].fill(0.5);
        mono[3000..3800].fill(0.25);
        offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

fn song(tracks: &str) -> Song {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let source = format!("bpm(120)\nlocal b = sample(\"break.wav\"):slice(2)\n{tracks}");
    host.load(&mut controller, &source, "song.lua", &dir())
        .unwrap()
        .clone()
}

const DRUMS: &str = "track(\"d\", b):steps(\"1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .\")\n  \
                     :fx(\"lowpass\", { cutoff = 2000 }):send(\"delay\", 0.4)\n";
const KICK: &str = "track(\"k\", b:slice_at(1)):steps(\"x . . .  x . . .  x . . .  x . . .\"):send(\"reverb\", 0.5)\n";

#[test]
fn a_track_alone_sounds_like_the_song_with_only_it() {
    let both = song(&format!("{DRUMS}{KICK}"));
    let only = song(DRUMS);
    assert_eq!(
        both.render_track(RATE, 2, "d").unwrap().unwrap(),
        only.render(RATE, 2).unwrap()
    );
    // Aunque esté en mute en el script.
    let muted = song(&format!("{DRUMS}{KICK}track(\"d\"):mute()\n"));
    assert_eq!(
        muted.render_track(RATE, 2, "d").unwrap().unwrap(),
        only.render(RATE, 2).unwrap()
    );
    assert!(both.render_track(RATE, 1, "nada").is_none());
}

#[test]
fn a_track_lasts_until_it_repeats() {
    let s = song(&format!(
        "{DRUMS}track(\"t\", b):length(12):steps(\"1 . . .  . . . .  . . . .\")\n\
         track(\"e\", b):steps(\"1 . . .  . . . .  . . . .  . . . .\"):every(4, function(c) c:reverse() end)\n"
    ));
    assert_eq!(s.track_length_bars("d"), Some(1));
    assert_eq!(s.track_length_bars("t"), Some(3));
    assert_eq!(s.track_length_bars("e"), Some(4));
    assert_eq!(s.track_length_bars("x"), None);
}
