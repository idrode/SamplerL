//! `cortes:fx(cuál, nombre, parámetros)` desde Lua: solo cambia el corte
//! elegido (por número, etiqueta o lista), se comparte con las pistas de
//! la variable, admite `ramp()`/`lfo()`, avisa con granos y da los errores
//! con la línea sin dejar de sonar lo anterior.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::{Controller, Engine, Fx, new_engine, offline};
use script::{ScriptHost, Song};

const RATE: u32 = 48_000;
/// Un paso a 120 BPM.
const STEP: usize = 6000;
const BAR: usize = 16 * STEP;

/// `ruido.wav`: cuatro trozos de un paso, cada uno con ruido en sus 4000
/// primeros frames y silencio después (se corta con `slice_grid(4)`).
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("slice_fx");
        std::fs::create_dir_all(&dir).unwrap();
        let mut state = 0x1234_5678u32;
        let mut mono = vec![0.0f32; 4 * STEP];
        for (i, x) in mono.iter_mut().enumerate() {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            if i % STEP < 4000 {
                *x = 0.4 * ((state >> 8) as f32 / (1u32 << 24) as f32 * 2.0 - 1.0);
            }
        }
        offline::write_wav(dir.join("ruido.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

const PRE: &str = "bpm(120)\nlocal a = sample(\"ruido.wav\"):slice_grid(4)\n";
const STEPS: &str = "1 . 2 .  3 . . .  4 . 1 .  . . 2 .";
/// Paso de cada golpe de `STEPS` y su corte (desde 1).
const HITS: [(usize, usize); 6] = [(0, 1), (2, 2), (4, 3), (8, 4), (10, 1), (14, 2)];

fn song(source: &str) -> Result<Song, String> {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.load(
        &mut controller,
        &format!("{PRE}{source}"),
        "song.lua",
        &dir(),
    )
    .cloned()
    .map_err(|e| e.to_string())
}

fn audio(source: &str) -> Vec<f32> {
    song(source).unwrap().render(RATE, 1).unwrap()
}

fn window(audio: &[f32], k: usize) -> &[f32] {
    &audio[2 * k * STEP..2 * (k + 1) * STEP]
}

/// Los cortes (desde 1) cuyos golpes suenan distinto que en `plain`.
fn changed(plain: &[f32], with: &[f32]) -> Vec<usize> {
    let mut slices: Vec<usize> = HITS
        .iter()
        .filter(|(k, _)| window(plain, *k) != window(with, *k))
        .map(|&(_, s)| s)
        .collect();
    slices.dedup();
    slices.sort();
    slices.dedup();
    slices
}

#[test]
fn only_the_chosen_slices_change() {
    let track = format!("track(\"d\", a):steps(\"{STEPS}\")\n");
    let plain = audio(&track);
    let cases = [
        ("a:fx(3, \"bitcrush\", { bits = 4 })", vec![3]),
        (
            "a:name(3, \"snare\")\na:fx(\"snare\", \"lowpass\", { cutoff = 400 })",
            vec![3],
        ),
        (
            "a:name(2, \"hat\")\na:fx({ \"hat\", 4 }, \"highpass\", { cutoff = 5000 })",
            vec![2, 4],
        ),
        ("a:fx(1, \"gain\", { level = 0.5 })", vec![1]),
    ];
    for (code, expected) in cases {
        // Antes o después de la pista: la variable es la misma.
        for source in [format!("{code}\n{track}"), format!("{track}{code}\n")] {
            assert_eq!(changed(&plain, &audio(&source)), expected, "{source}");
        }
    }
    // Sin efectos de corte, bit a bit lo de siempre (también con etiquetas).
    assert_eq!(audio(&format!("a:name(3, \"snare\")\n{track}")), plain);
}

#[test]
fn the_chain_reaches_the_track_with_its_curves() {
    let song = song(&format!(
        "a:name(3, \"snare\")\n\
         a:fx(\"snare\", \"bitcrush\", {{ bits = 6 }})\n\
         a:fx(\"snare\", \"lowpass\", {{ cutoff = ramp(800, 8000, \"2 compases\"), q = 2 }})\n\
         a:fx({{ 1, \"snare\" }}, \"gain\", {{ level = lfo(\"1 compás\", 0.2, 1) }})\n\
         track(\"d\", a):steps(\"{STEPS}\")\n\
         track(\"solo3\", a:slice_at(3)):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n"
    ))
    .unwrap();
    assert!(song.warnings.is_empty(), "{:?}", song.warnings);
    let d = &song.tracks[0];
    assert_eq!(d.slice_fx.len(), 4);
    assert!(d.slice_fx[1].fx.is_empty() && d.slice_fx[3].fx.is_empty());
    assert_eq!(d.slice_fx[0].fx, [Fx::gain(0.2)]);
    let snare = &d.slice_fx[2];
    assert_eq!(snare.fx.len(), 3);
    assert_eq!(snare.fx[0], Fx::bitcrush(6, 1));
    // La curva del filtro (ranura 1) y la del gain (ranura 2).
    let slots: Vec<(usize, usize)> = snare.automation.iter().map(|a| (a.fx, a.param)).collect();
    assert_eq!(slots, [(1, 0), (2, 0)]);
    // `slice_at` se lleva los efectos de su corte.
    assert_eq!(song.tracks[1].slice_fx.len(), 1);
    assert_eq!(song.tracks[1].slice_fx[0], *snare);
    // Determinista.
    assert_eq!(song.render(RATE, 2).unwrap(), song.render(RATE, 2).unwrap());
}

#[test]
fn add_and_remove_keep_each_chain_on_its_slice() {
    let base = "track(\"d\", a):steps(\"1 . 2 .  3 . . .  . . . .  . . . .\")\n";
    // Quitar el corte 2 y añadir uno después: el efecto del corte 3 sigue
    // en el mismo trozo del sample.
    let with = |code: &str| song(&format!("{code}\n{base}")).unwrap();
    let a = with("a:fx(3, \"bitcrush\", { bits = 4 })\na:remove(2)");
    assert_eq!(a.tracks[0].slice_fx.len(), 3);
    assert_eq!(a.tracks[0].slice_fx[1].fx, [Fx::bitcrush(4, 1)]);
    let b = with("a:fx(3, \"bitcrush\", { bits = 4 })\na:add(10)");
    assert_eq!(b.tracks[0].slice_fx.len(), 5);
    assert_eq!(b.tracks[0].slice_fx[3].fx, [Fx::bitcrush(4, 1)]);
}

#[test]
fn grains_warn_and_instruments_refuse() {
    let song = song(
        "a:fx(1, \"bitcrush\", { bits = 4 })\n\
         track(\"g\", a):steps(\"1 _ _ _  . . . .  . . . .  . . . .\"):grains({ size = 40 })\n",
    )
    .unwrap();
    assert!(
        song.warnings
            .iter()
            .any(|w| w.contains("\"g\"") && w.contains("no suenan en los granos")),
        "{:?}",
        song.warnings
    );
    let err = self::song("local i = sample(\"ruido.wav\"):instrument({})\ni:fx(1, \"gain\", {})\n")
        .err()
        .expect("debería fallar");
    assert!(
        err.contains("song.lua:4:") && err.contains("un instrumento no tiene efectos por corte"),
        "{err}"
    );
}

#[test]
fn eight_hot_tracks_never_pass_one() {
    let mut source = String::from(
        "a:fx({ 1, 2, 3, 4 }, \"distortion\", { drive = 50 })\n\
         a:fx({ 1, 2, 3, 4 }, \"gain\", { level = 4 })\n",
    );
    for t in 0..8 {
        source.push_str(&format!(
            "track(\"t{t}\", a):steps(\"1@4 2@4 3@4 4@4  1@4 2@4 3@4 4@4  1@4 2@4 3@4 4@4  1@4 2@4 3@4 4@4\"):gain(4)\n"
        ));
    }
    let out = audio(&source);
    let peak = out.iter().fold(0.0f32, |m, v| m.max(v.abs()));
    assert!(peak <= 1.0, "pico {peak}");
    assert!(peak > 0.9, "tenía que actuar el limitador: {peak}");
}

fn load(host: &mut ScriptHost, c: &mut Controller, src: &str) -> Result<(), String> {
    host.load(c, src, "song.lua", &dir())
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

#[test]
fn errors_say_the_line_and_the_old_song_keeps_playing() {
    let good =
        format!("{PRE}a:fx(3, \"bitcrush\", {{ bits = 6 }})\ntrack(\"d\", a):steps(\"{STEPS}\")\n");
    let (mut controller, mut engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    load(&mut host, &mut controller, &good).unwrap();
    controller.start().unwrap();
    let mut heard = render_left(&mut engine, BAR + 777);

    let pre = format!("{PRE}a:name(3, \"snare\")\n");
    let cases: &[(&str, &[&str])] = &[
        (
            "a:fx(\"caja\", \"bitcrush\", {})",
            &[
                "song.lua:4:",
                "ningún corte se llama \"caja\"",
                "etiquetas: snare",
            ],
        ),
        (
            "a:fx(9, \"bitcrush\", {})",
            &["song.lua:4:", "el corte 9 no existe (hay 4)"],
        ),
        (
            "a:fx(0, \"gain\", {})",
            &["song.lua:4:", "el corte 0 no existe"],
        ),
        (
            "a:fx({}, \"gain\", {})",
            &["song.lua:4:", "lista de cortes está vacía"],
        ),
        (
            "a:fx(true, \"gain\", {})",
            &["song.lua:4:", "un número, una etiqueta"],
        ),
        (
            "a:fx(3, \"chorus\", {})",
            &["song.lua:4:", "\"chorus\"", "no existe"],
        ),
        (
            "a:fx(3, \"bitcrush\", { bits = 20 })",
            &["song.lua:4:", "bits fuera de rango"],
        ),
        (
            "a:fx(\"snare\", \"gain\", { level = 5 })",
            &["song.lua:4:", "level fuera de rango"],
        ),
        (
            "a:fx(3, \"lowpass\", { cutof = 300 })",
            &["song.lua:4:", "\"cutof\""],
        ),
        (
            "a:fx(3, \"gain\", { level = ramp(0, 8, \"1 compás\") })",
            &["song.lua:4:", "se sale del rango"],
        ),
        (
            "a:fx(3, \"filter\", { cutoff = 300, type = lfo(1, 0, 1) })",
            &["song.lua:4:", "type no admite ramp() ni lfo()"],
        ),
        (
            "a:fx(3, \"dyneq\", { freq = 200, sidechain = \"d\" })",
            &["song.lua:4:", "no puede escuchar otra pista"],
        ),
        (
            "for i = 1, 5 do a:fx({ 1, \"snare\" }, \"gain\", {}) end",
            &["song.lua:4:", "ya tiene 4 efectos"],
        ),
    ];
    for (line, fragments) in cases {
        let src = format!("{pre}{line}\ntrack(\"d\", a):steps(\"{STEPS}\")\n");
        let err = load(&mut host, &mut controller, &src).unwrap_err();
        for fragment in *fragments {
            assert!(
                err.contains(fragment),
                "error \"{err}\" sin \"{fragment}\"\n{src}"
            );
        }
    }
    heard.extend(render_left(&mut engine, BAR - 777));

    // Igual que si no se hubiera intentado ninguna recarga.
    let (mut c2, mut e2) = new_engine(RATE);
    let mut h2 = ScriptHost::new(RATE);
    load(&mut h2, &mut c2, &good).unwrap();
    c2.start().unwrap();
    assert_eq!(heard, render_left(&mut e2, 2 * BAR));
}

#[test]
fn a_change_enters_at_the_bar() {
    let (mut controller, mut engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let plain = format!("{PRE}track(\"d\", a):steps(\"{STEPS}\")\n");
    let crushed =
        format!("{PRE}a:fx(1, \"bitcrush\", {{ bits = 3 }})\ntrack(\"d\", a):steps(\"{STEPS}\")\n");
    load(&mut host, &mut controller, &plain).unwrap();
    controller.start().unwrap();
    let first = offline::render(&mut engine, STEP, 2);
    load(&mut host, &mut controller, &crushed).unwrap();
    let rest = offline::render(&mut engine, 2 * BAR - STEP, 2);
    let heard: Vec<f32> = first.into_iter().chain(rest).collect();
    // El primer compás, entero sin efectos; el segundo, con ellos.
    let (mut c2, mut e2) = new_engine(RATE);
    let mut h2 = ScriptHost::new(RATE);
    load(&mut h2, &mut c2, &plain).unwrap();
    c2.start().unwrap();
    let before = offline::render(&mut e2, BAR, 2);
    assert_eq!(&heard[..2 * BAR], &before[..]);
    let second = &heard[2 * BAR..];
    assert_ne!(
        window(second, 0),
        window(&before, 0),
        "el corte 1, con bitcrush"
    );
    assert_eq!(window(second, 2), window(&before, 2), "el corte 2, igual");
}
