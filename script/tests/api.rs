//! API completa del ejemplo de CLAUDE.md: slice_at, every, chance y fx.

use std::path::{Path, PathBuf};
use std::time::Duration;

use engine::{Controller, Engine, Fx, Pattern, new_engine, offline};
use script::ScriptHost;

const RATE: u32 = 48_000;
/// A 120 BPM: 6000 frames por paso, 96000 por compás.
const STEP: usize = 6000;
const BAR: usize = 16 * STEP;
const SETTLE: usize = 200;
const EPS: f32 = 1e-3;

/// `break.wav`: dos cortes (bloque de 800 frames a 0.5 en 0 y a 0.25 en
/// 3000). `break8.wav`: ocho cortes de 300 frames, cada 3000 (62,5 ms).
fn fixture(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("api_{test}"));
    std::fs::create_dir_all(&dir).unwrap();
    let mut two = vec![0.0f32; 6000];
    two[..800].fill(0.5);
    two[3000..3800].fill(0.25);
    offline::write_wav(dir.join("break.wav"), &two, 1, RATE).unwrap();
    let mut eight = vec![0.0f32; 24_000];
    for k in 0..8 {
        eight[k * 3000..k * 3000 + 300].fill(0.2 + 0.05 * k as f32);
    }
    offline::write_wav(dir.join("break8.wav"), &eight, 1, RATE).unwrap();
    dir
}

fn setup(test: &str) -> (PathBuf, ScriptHost, Controller, Engine) {
    let (controller, engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.set_seed(Some(42));
    (fixture(test), host, controller, engine)
}

fn load(host: &mut ScriptHost, c: &mut Controller, dir: &Path, src: &str) -> Result<(), String> {
    host.load(c, src, "song.lua", dir)
        .map(|_| ())
        .map_err(|e| e.message)
}

fn render_left(engine: &mut Engine, frames: usize) -> Vec<f32> {
    offline::render(engine, frames, 2)
        .iter()
        .step_by(2)
        .copied()
        .collect()
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() < EPS
}

/// Renderiza `bars` compases de un script con la semilla 42.
fn render_script(dir: &Path, src: &str, seed: u64, bars: usize) -> Vec<f32> {
    let (mut controller, mut engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.set_seed(Some(seed));
    load(&mut host, &mut controller, dir, src).unwrap();
    controller.start().unwrap();
    render_left(&mut engine, bars * BAR)
}

#[test]
fn the_example_from_claude_md_runs() {
    let (dir, mut host, mut controller, mut engine) = setup("example");
    // El ejemplo tal cual, con otro nombre de archivo.
    let src = r#"
bpm(170)

local amen = sample("break8.wav"):slice(8)

track("drums", amen)
  :steps("1 . . 1  . . 3 .  1 . 5 .  . 3 . 6")

track("drums")
  :fx("filter", { type = "lowpass", cutoff = 3000 })
  :every(4, function(t) t:reverse() end)
  :chance(0.2, "retrig", 4)

track("kick", amen:slice_at(1))
  :steps("x . . . x . . . x . . . x . . .")
  :gain(0.9)
"#;
    load(&mut host, &mut controller, &dir, src).unwrap();
    let song = host.song().unwrap();
    assert_eq!(song.bpm, Some(170.0));
    let drums = &song.tracks[0];
    assert_eq!(drums.slices.len(), 8);
    assert_eq!(drums.fx, vec![Fx::lowpass(3000.0)]);
    // every(4) y chance (16 compases): ciclo de 16.
    assert_eq!(drums.bars.len(), 16);
    // every(4): los compases 0, 4, 8 y 12 van al revés.
    for (b, bar) in drums.bars.iter().enumerate() {
        let reversed = bar.iter().flatten().filter(|s| s.reverse).count();
        let hits = bar.iter().flatten().count();
        assert_eq!(reversed == hits, b % 4 == 0, "compás {b}");
    }
    // chance(0.2, "retrig", 4): alrededor del 20 % de los golpes.
    let hits: Vec<_> = drums.bars.iter().flat_map(|b| b.iter().flatten()).collect();
    let retrig = hits.iter().filter(|s| s.retrig == 4).count();
    assert!(
        (5..=35).contains(&(retrig * 100 / hits.len())),
        "{retrig}/{}",
        hits.len()
    );
    let kick = &song.tracks[1];
    assert_eq!(kick.slices.len(), 1);
    assert_eq!(kick.gain, 0.9);

    controller.start().unwrap();
    let audio = render_left(&mut engine, 4 * BAR);
    assert!(audio.iter().any(|&s| s.abs() > 0.1));
    assert!(audio.iter().all(|s| s.is_finite() && s.abs() <= 1.0));
}

#[test]
fn slice_at_takes_one_slice() {
    let (dir, mut host, mut controller, mut engine) = setup("slice_at");
    let src = r#"
local amen = sample("break.wav"):slice(2)
track("snare", amen:slice_at(2)):steps("x . . .  1 . . .  . . . .  . . . .")
"#;
    load(&mut host, &mut controller, &dir, src).unwrap();
    controller.start().unwrap();
    let l = render_left(&mut engine, BAR);
    assert!(near(l[SETTLE], 0.25));
    assert!(near(l[4 * STEP + SETTLE], 0.25));

    let bad = "local a = sample(\"break.wav\"):slice(2)\ntrack(\"k\", a:slice_at(3))\n";
    let err = load(&mut host, &mut controller, &dir, bad).unwrap_err();
    assert!(
        err.contains("song.lua:2:") && err.contains("slice_at(3)"),
        "{err}"
    );
}

#[test]
fn every_transforms_bars_0_n_2n() {
    let (dir, _, _, _) = setup("every");
    let src = r#"
local amen = sample("break.wav"):slice(2)
track("d", amen)
  :steps("1 . . .  2 . . .  . . . .  . . . .")
  :every(4, function(t) t:reverse() end)
  :every(3, function(t) t:steps("2 . . .  . . . .  . . . .  . . . ."):gain(0.5) end)
"#;
    let l = render_script(&dir, src, 42, 8);
    for bar in 0..8 {
        let at = |step: usize, offset: usize| l[bar * BAR + step * STEP + offset];
        match (bar % 4 == 0, bar % 3 == 0) {
            // every(3) (después) sustituye los pasos: corte 2 a media ganancia.
            (_, true) => {
                assert!(near(at(0, SETTLE), 0.125), "compás {bar}");
                assert_eq!(at(4, SETTLE), 0.0, "compás {bar}");
            }
            // Al revés: el bloque suena al final de los 3000 frames del corte.
            (true, false) => {
                assert_eq!(at(0, SETTLE), 0.0, "compás {bar}");
                assert!(near(at(0, 2600), 0.5), "compás {bar}");
            }
            (false, false) => {
                assert!(near(at(0, SETTLE), 0.5), "compás {bar}");
                assert!(near(at(4, SETTLE), 0.25), "compás {bar}");
            }
        }
    }
}

#[test]
fn chance_is_reproducible_with_a_seed() {
    let (dir, mut host, mut controller, _) = setup("chance");
    let src = r#"
local amen = sample("break.wav"):slice(2)
track("d", amen)
  :steps("1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1")
  :chance(0.25, "mute")
  :chance(0.5, "pitch", 12)
"#;
    // Misma semilla, mismo audio; otra semilla, otro.
    let a = render_script(&dir, src, 42, 4);
    assert_eq!(a, render_script(&dir, src, 42, 4));
    assert_ne!(a, render_script(&dir, src, 7, 4));

    load(&mut host, &mut controller, &dir, src).unwrap();
    let bars = &host.song().unwrap().tracks[0].bars;
    assert_eq!(bars.len(), 16);
    let total = 16 * 16;
    let muted = bars.iter().flatten().filter(|s| s.is_none()).count();
    let pitched = bars
        .iter()
        .flatten()
        .flatten()
        .filter(|s| s.semitones == 12.0)
        .count();
    assert!((40..=90).contains(&muted), "silenciados {muted}/{total}");
    let left = total - muted;
    assert!(
        (left * 35 / 100..=left * 65 / 100).contains(&pitched),
        "{pitched}/{left}"
    );
}

#[test]
fn fx_from_lua_matches_the_engine() {
    let (dir, mut host, mut controller, mut engine) = setup("fx");
    let src = r#"
local amen = sample("break.wav"):slice(2)
track("d", amen)
  :steps("1 . 2 .  1 . 2 .  1 . 2 .  1 . 2 .")
  :fx("filter", { type = "highpass", cutoff = 800, q = 2 })
  :fx("bitcrush", { bits = 4, downsample = 2 })
"#;
    load(&mut host, &mut controller, &dir, src).unwrap();
    controller.start().unwrap();
    let from_lua = render_left(&mut engine, 2 * BAR);

    let track = &host.song().unwrap().tracks[0];
    let chain = [
        Fx::Filter {
            kind: engine::FilterKind::HighPass,
            cutoff_hz: 800.0,
            q: 2.0,
        },
        Fx::bitcrush(4, 2),
    ];
    assert_eq!(track.fx, chain);
    let pattern = Pattern::from_bars(track.sample.clone(), &track.slices, &track.bars)
        .unwrap()
        .with_fx(&chain)
        .unwrap();
    let (mut c2, mut e2) = new_engine(RATE);
    c2.set_pattern(pattern).unwrap();
    c2.start().unwrap();
    assert_eq!(from_lua, render_left(&mut e2, 2 * BAR));
}

#[test]
fn api_errors_point_at_the_line_and_keep_playing() {
    let (dir, mut host, mut controller, mut engine) = setup("errors");
    host.set_time_limit(Duration::from_millis(100));
    let good = r#"
local amen = sample("break.wav"):slice(2)
track("d", amen):steps("1 . 2 .  . . . .  . . . .  . . . .")
"#;
    load(&mut host, &mut controller, &dir, good).unwrap();
    controller.start().unwrap();
    let mut audio = render_left(&mut engine, BAR + 777);

    let pre = "local a = sample(\"break.wav\"):slice(2)\nlocal t = track(\"d\", a):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n";
    let cases: &[(&str, &[&str])] = &[
        (
            "t:fx(\"reverb\")",
            &["song.lua:3:", "\"reverb\"", "no existe"],
        ),
        (
            "t:fx(\"filter\", { cutof = 100 })",
            &["song.lua:3:", "\"cutof\""],
        ),
        ("t:fx(\"lowpass\")", &["song.lua:3:", "falta cutoff"]),
        (
            "t:fx(\"lowpass\", { cutoff = 5 })",
            &["song.lua:3:", "frecuencia de corte"],
        ),
        (
            "t:fx(\"filter\", { type = \"bandpass\", cutoff = 100 })",
            &["song.lua:3:", "bandpass"],
        ),
        (
            "for i = 1, 5 do t:fx(\"bitcrush\") end",
            &["song.lua:3:", "máximo 4 efectos"],
        ),
        ("t:every(0, function(b) end)", &["song.lua:3:", "every(0)"]),
        ("t:every(4, \"x\")", &["song.lua:3:"]),
        (
            "t:every(2, function(b)\n  b:pitch(99)\nend)",
            &["song.lua:4:", "pitch(99)"],
        ),
        (
            "t:every(2, function(b)\n  track(\"e\", a)\nend)",
            &["song.lua:4:", "track() dentro de every()"],
        ),
        (
            "t:every(7, function() end):every(11, function() end)",
            &["77 compases"],
        ),
        ("t:chance(1.5, \"mute\")", &["song.lua:3:", "chance(1.5)"]),
        (
            "t:chance(0.5, \"explode\")",
            &["song.lua:3:", "\"explode\""],
        ),
        ("t:chance(0.5, \"retrig\")", &["song.lua:3:", "falta n"]),
        (
            "t:chance(0.5, \"retrig\", 20)",
            &["song.lua:3:", "retrig(20)"],
        ),
        (
            "t:steps(\"1 . . .  . . . .  . . . .  . . . 1r20\")",
            &["song.lua:3:", "retrigger"],
        ),
    ];
    for (line, fragments) in cases {
        let src = format!("{pre}{line}\n");
        let err = load(&mut host, &mut controller, &dir, &src).unwrap_err();
        for fragment in *fragments {
            assert!(
                err.contains(fragment),
                "error \"{err}\" sin \"{fragment}\"\n{src}"
            );
        }
    }
    audio.extend(render_left(&mut engine, BAR - 777));

    // Igual que si no se hubiera intentado ninguna recarga.
    let (mut c2, mut e2) = new_engine(RATE);
    let mut h2 = ScriptHost::new(RATE);
    load(&mut h2, &mut c2, &dir, good).unwrap();
    c2.start().unwrap();
    assert_eq!(audio, render_left(&mut e2, 2 * BAR));
}
