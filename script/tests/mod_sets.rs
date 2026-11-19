//! `mod(nombre, cadenas)` desde Lua: monta el set del motor, se asigna a la
//! pista, a un corte y a un paso (con su prioridad), se quita, cada
//! dispositivo del registro se lee con su plantilla, y cada error dice su
//! línea mientras sigue sonando lo anterior.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::modulation::{DeviceKind, PITCH, VOL};
use engine::{Controller, Engine, new_engine, offline};
use script::{ScriptHost, Song, api};

const RATE: u32 = 48_000;
const STEP: usize = 6000;
const BAR: usize = 16 * STEP;

/// `dc.wav`: cuatro trozos de un paso a 0,5 (cada uno con 4000 frames y
/// silencio después).
fn dir() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("mod_sets");
        std::fs::create_dir_all(&dir).unwrap();
        let mono: Vec<f32> = (0..4 * STEP)
            .map(|i| if i % STEP < 4000 { 0.5 } else { 0.0 })
            .collect();
        offline::write_wav(dir.join("dc.wav"), &mono, 1, RATE).unwrap();
        dir
    })
    .clone()
}

const PRE: &str = "bpm(120)\nlocal a = sample(\"dc.wav\"):slice_grid(4)\n";
const STEPS: &str = "1 . 2 .  3 . . .  4 . 1 .  . . 2 .";

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

fn left(audio: &[f32]) -> Vec<f32> {
    audio.iter().step_by(2).copied().collect()
}

/// El nivel de cada paso de la canción (a 50 ms de su principio).
fn levels(source: &str) -> Vec<f32> {
    let audio = left(&song(source).unwrap().render(RATE, 1).unwrap());
    (0..16).map(|k| audio[k * STEP + 2400] / 0.5).collect()
}

#[test]
fn the_table_makes_the_engine_set() {
    let song = song(
        "mod(\"pluck\", {\n\
           vol = { { \"ahdsr\", a = 2, d = 180, s = 0.2, r = 60 }, { \"velocity\", min = 0.5, max = 1 } },\n\
           pitch = { \"lfo\", rate = 5, amount = 0.25 },\n\
           range = 7,\n\
         })\n\
         track(\"d\", a):steps(\"1 . . .  . . . .  . . . .  . . . .\"):mod(\"pluck\")\n",
    )
    .unwrap();
    assert_eq!(song.mod_sets.len(), 1);
    let set = song.mod_sets[0].set;
    assert_eq!(set.pitch_range, 7.0);
    let vol = set.chains[VOL].unwrap();
    let DeviceKind::Ahdsr(env) = vol.devices[0].unwrap().kind else {
        panic!()
    };
    assert_eq!(
        (env.attack.value, env.decay.value, env.sustain.value),
        (2.0, 180.0, 0.2)
    );
    assert!(matches!(
        vol.devices[1].unwrap().kind,
        DeviceKind::Velocity(_)
    ));
    // Una cadena de un solo dispositivo, sin lista; el LFO, bipolar.
    let lfo = set.chains[PITCH].unwrap().devices[0].unwrap();
    assert!(matches!(lfo.kind, DeviceKind::Lfo(_)) && lfo.bipolar);
    assert_eq!(song.tracks[0].modset, Some(0));
}

#[test]
fn track_slice_and_step_sets_and_their_priority() {
    let sets = "mod(\"half\", { vol = { \"value\", value = 0.5 } })\n\
                mod(\"quarter\", { vol = { \"value\", value = 0.25 } })\n";
    // La pista entera a la mitad.
    let l = levels(&format!(
        "{sets}track(\"d\", a):steps(\"{STEPS}\"):mod(\"half\")\n"
    ));
    assert!((l[0] - 0.5).abs() < 1e-4 && (l[2] - 0.5).abs() < 1e-4);
    // El corte 2 a un cuarto (manda sobre la pista), con el handle.
    let l = levels(&format!(
        "{sets}a:mod(2, \"quarter\")\ntrack(\"d\", a):steps(\"{STEPS}\"):mod(\"half\")\n"
    ));
    assert!((l[0] - 0.5).abs() < 1e-4 && (l[2] - 0.25).abs() < 1e-4);
    // El del paso manda sobre los dos.
    let l = levels(&format!(
        "{sets}a:mod(2, \"quarter\")\n\
         track(\"d\", a):steps(\"1{{mod=quarter}} . 2{{mod=half}} .  3 . . .  4 . 1 .  . . 2 .\"):mod(\"half\")\n"
    ));
    assert!((l[0] - 0.25).abs() < 1e-4 && (l[2] - 0.5).abs() < 1e-4);
    // Con un efecto en las mismas llaves.
    let l = levels(&format!(
        "{sets}track(\"d\", a):steps(\"1{{mod=quarter}}{{gain=2}} . . .  . . . .  . . . .  . . . .\")\n"
    ));
    assert!((l[0] - 0.5).abs() < 1e-4, "{}", l[0]);
    // Quitarlas: suena entero.
    let l = levels(&format!(
        "{sets}a:mod(2, \"quarter\"):mod_clear()\ntrack(\"d\", a):steps(\"{STEPS}\"):mod(\"half\"):mod_clear()\n"
    ));
    assert!((l[0] - 1.0).abs() < 1e-4 && (l[2] - 1.0).abs() < 1e-4);
}

#[test]
fn without_mod_the_song_is_the_same_bit_for_bit() {
    let plain = song(&format!("track(\"d\", a):steps(\"{STEPS}\")\n"))
        .unwrap()
        .render(RATE, 1)
        .unwrap();
    let defined = song(&format!(
        "mod(\"x\", {{ vol = {{ \"ahdsr\", d = 50, s = 0 }} }})\ntrack(\"d\", a):steps(\"{STEPS}\")\n"
    ))
    .unwrap()
    .render(RATE, 1)
    .unwrap();
    assert!(
        plain
            .iter()
            .zip(&defined)
            .all(|(a, b)| a.to_bits() == b.to_bits())
    );
}

#[test]
fn every_registry_device_template_loads() {
    for device in api::MOD_DEVICES {
        let (table, _) = api::expand_template(device.template);
        let src = format!(
            "mod(\"x\", {{ vol = {{ {table} }} }})\ntrack(\"d\", a):steps(\"{STEPS}\"):mod(\"x\")\n"
        );
        let loaded = song(&src).unwrap_or_else(|e| panic!("{}: {e}\n{src}", device.name));
        assert!(
            loaded
                .render(RATE, 1)
                .unwrap()
                .iter()
                .all(|x| x.is_finite())
        );
        // Y cada parámetro suyo, con su plantilla.
        for p in device.params {
            let (item, _) = api::expand_template(p.template);
            let src = format!(
                "mod(\"x\", {{ vol = {{ {{ \"{}\", {item} {} }} }} }})\ntrack(\"d\", a):steps(\"{STEPS}\"):mod(\"x\")\n",
                device.name,
                match device.name {
                    "env" if p.name != "points" => ", points = { {0, 0}, {1, 1} }",
                    "stepper" if p.name != "values" => ", values = { 1, 0.5 }",
                    _ => "",
                }
            );
            song(&src).unwrap_or_else(|e| panic!("{}.{}: {e}\n{src}", device.name, p.name));
        }
    }
    for p in api::MOD_COMMON {
        let (item, _) = api::expand_template(p.template);
        let src = format!(
            "mod(\"x\", {{ pitch = {{ {{ \"ahdsr\", {item} }} }} }})\ntrack(\"d\", a):steps(\"{STEPS}\"):mod(\"x\")\n"
        );
        song(&src).unwrap_or_else(|e| panic!("{}: {e}\n{src}", p.name));
    }
    // Las curvas de la canción en los parámetros que las admiten.
    song(&format!(
        "macro_init(\"m\", 0.5)\n\
         mod(\"x\", {{ vol = {{ {{ \"ahdsr\", d = macro(\"m\", 10, 400), amount = lfo(2, 0.5, 1) }}, {{ \"lfo\", rate = ramp(1, 8, \"1 compás\") }} }} }})\n\
         track(\"d\", a):steps(\"{STEPS}\"):mod(\"x\")\n"
    ))
    .unwrap();
}

#[test]
fn a_synth_track_takes_a_set() {
    let song = song(
        "mod(\"bark\", { cutoff = { { \"ahdsr\", a = 1, d = 150, s = 0.2 }, { \"velocity\", min = 0.6, max = 1 } } })\n\
         track(\"bajo\", synth(\"acid\")):melody(\"c . c! .  eb . c .  . . . .  . . . .\"):mod(\"bark\")\n",
    )
    .unwrap();
    let audio = song.render(RATE, 1).unwrap();
    assert!(audio.iter().all(|x| x.is_finite()));
    assert!(audio.iter().any(|x| x.abs() > 0.01));
}

fn load(host: &mut ScriptHost, c: &mut Controller, src: &str) -> Result<(), String> {
    host.load(c, src, "song.lua", &dir())
        .map(|_| ())
        .map_err(|e| e.message)
}

fn render_left(engine: &mut Engine, frames: usize) -> Vec<f32> {
    left(&offline::render(engine, frames, 2))
}

#[test]
fn errors_say_the_line_and_the_old_song_keeps_playing() {
    let good = format!(
        "{PRE}mod(\"p\", {{ vol = {{ \"ahdsr\", d = 80, s = 0.3 }} }})\ntrack(\"d\", a):steps(\"{STEPS}\"):mod(\"p\")\n"
    );
    let (mut controller, mut engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    load(&mut host, &mut controller, &good).unwrap();
    controller.start().unwrap();
    let mut heard = render_left(&mut engine, BAR + 777);

    let cases: &[(&str, &[&str])] = &[
        (
            "mod(\"x\", { volume = { \"ahdsr\" } })",
            &[
                "song.lua:3:",
                "el dominio «volume» no existe (vol, pan, pitch, cutoff, range)",
            ],
        ),
        (
            "mod(\"x\", { vol = { \"adsr\" } })",
            &[
                "song.lua:3:",
                "el dispositivo «adsr» no existe (ahdsr, env, lfo",
            ],
        ),
        (
            "mod(\"x\", { vol = { \"ahdsr\", attack = 3 } })",
            &[
                "song.lua:3:",
                "parámetro desconocido «attack»",
                "admite: a, h, d",
            ],
        ),
        (
            "mod(\"x\", { vol = { \"ahdsr\", s = 2 } })",
            &["song.lua:3:", "sustain fuera de rango (0 a 1)"],
        ),
        (
            "mod(\"x\", { vol = { \"ahdsr\", d = 0/0 } })",
            &["song.lua:3:", "decay fuera de rango"],
        ),
        (
            "mod(\"x\", { vol = { \"lfo\", rate = math.huge } })",
            &["song.lua:3:", "rate fuera de rango (0.01 a 50 Hz)"],
        ),
        (
            "mod(\"x\", { vol = { \"lfo\", wave = \"noise\" } })",
            &[
                "song.lua:3:",
                "wave «noise» no existe (sine, tri, saw, square, random)",
            ],
        ),
        (
            "mod(\"x\", { vol = { \"env\", points = { {0, 0}, {1, 1} }, loop = \"forward\", sustain = 2 } })",
            &["song.lua:3:", "una envolvente con sustain no va en bucle"],
        ),
        (
            "mod(\"x\", { vol = { \"env\", points = { {0, 0}, {2, 1} } } })",
            &["song.lua:3:", "de 0 a 1"],
        ),
        (
            "mod(\"x\", { vol = { \"stepper\", values = {} } })",
            &["song.lua:3:", "values lleva al menos un número"],
        ),
        (
            "mod(\"x\", { vol = { \"stepper\", values = { 1 }, step = 20 } })",
            &["song.lua:3:", "step es un entero de 0 a 16"],
        ),
        (
            "mod(\"x\", { vol = { {\"value\"}, {\"value\"}, {\"value\"}, {\"value\"}, {\"value\"}, {\"value\"}, {\"value\"}, {\"value\"}, {\"value\"} } })",
            &[
                "song.lua:3:",
                "demasiados dispositivos (máximo 8 por cadena)",
            ],
        ),
        (
            "mod(\"x\", { vol = { \"value\", op = \"pow\" } })",
            &["song.lua:3:", "op «pow» no existe (add, sub, mul, div)"],
        ),
        (
            "mod(\"x\", { pitch = { \"value\" }, range = 99 })",
            &["song.lua:3:", "range del pitch fuera de rango"],
        ),
        (
            "mod(\"p\", { vol = { \"value\" } })\nmod(\"p\", { vol = { \"value\" } })",
            &["song.lua:4:", "ya hay un mod set con ese nombre"],
        ),
        (
            "track(\"d\", a):steps(\"1 . . .  . . . .  . . . .  . . . .\"):mod(\"nadie\")",
            &["song.lua:3:", "no hay ningún mod set «nadie»"],
        ),
        (
            "track(\"d\", a):steps(\"1{mod=nadie} . . .  . . . .  . . . .  . . . .\")",
            &["song.lua:3:", "{mod=nadie}: no hay ningún mod set «nadie»"],
        ),
        (
            "mod(\"q\", { vol = { \"value\" } })\na:mod(9, \"q\")",
            &["song.lua:4:", "el corte 9 no existe"],
        ),
    ];
    for (line, fragments) in cases {
        let src = format!("{PRE}{line}\n");
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
