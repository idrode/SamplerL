//! API de Lua con render offline: patrones, varias pistas, recarga en
//! caliente cuantizada al compás y errores que no rompen lo que suena.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use engine::{Controller, Engine, new_engine, offline};
use script::ScriptHost;

const RATE: u32 = 48_000;
/// A 120 BPM: 6000 frames por paso, 96000 por compás.
const STEP: usize = 6000;
const BAR: usize = 16 * STEP;
/// Pasado el fundido de entrada (≤ 2 ms).
const SETTLE: usize = 200;
/// El WAV de prueba se guarda en 16 bits: 0.5 queda en 16383/32767.
const EPS: f32 = 1e-3;

/// Carpeta propia para cada test, con `break.wav`: dos cortes, un bloque
/// constante de 800 frames a 0.5 en 0 y otro a 0.25 en 3000.
fn fixture(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(test);
    std::fs::create_dir_all(&dir).unwrap();
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    mono[3000..3800].fill(0.25);
    offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
    dir
}

fn setup(test: &str) -> (PathBuf, ScriptHost, Controller, Engine) {
    let (controller, engine) = new_engine(RATE);
    (fixture(test), ScriptHost::new(RATE), controller, engine)
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

fn level(left: &[f32], bar: usize, step: usize) -> f32 {
    left[bar * BAR + step * STEP + SETTLE]
}

fn assert_level(left: &[f32], bar: usize, step: usize, expected: f32) {
    let actual = level(left, bar, step);
    assert!(
        (actual - expected).abs() < EPS,
        "compás {bar}, paso {step}: {actual} (esperado {expected})"
    );
}

const SONG_A: &str = r#"
bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 . 2 .  . . . .  . . . .  . . . .")
track("kick", amen):steps("x . . .  x . . .  . . . .  . . . .")
"#;

#[test]
fn api_from_claude_md_renders_two_tracks() {
    let (dir, mut host, mut controller, mut engine) = setup("api");
    load(&mut host, &mut controller, &dir, SONG_A).unwrap();
    controller.start().unwrap();
    let l = render_left(&mut engine, BAR);

    // Paso 0: corte 1 en las dos pistas (0.5 + 0.5, por el limitador). Paso 2: corte 2 en
    // "drums". Paso 4: "x" (= corte 1) en "kick". Paso 1: silencio.
    assert_level(&l, 0, 0, engine::soft_limit(1.0));
    assert_level(&l, 0, 1, 0.0);
    assert_level(&l, 0, 2, 0.25);
    assert_level(&l, 0, 4, 0.5);

    let song = host.song().unwrap();
    assert_eq!(song.bpm, Some(120.0));
    let names: Vec<&str> = song.tracks.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["drums", "kick"]);
    assert_eq!(song.tracks[0].slices.len(), 2);
}

#[test]
fn track_by_name_and_effects_from_the_notation() {
    let (dir, mut host, mut controller, mut engine) = setup("by_name");
    let src = r#"
local amen = sample("break.wav"):slice(2)
track("drums", amen)
track("drums"):steps("1@0.5 . 2' .  . . . .  . . . .  . . . .")
"#;
    load(&mut host, &mut controller, &dir, src).unwrap();
    controller.start().unwrap();
    let l = render_left(&mut engine, BAR);
    assert_level(&l, 0, 0, 0.25);
    // Corte 2 al revés: el bloque está al final de sus 3000 frames.
    assert!(l[2 * STEP..2 * STEP + 2200].iter().all(|&s| s == 0.0));
    assert!((l[2 * STEP + 2600] - 0.25).abs() < EPS);
}

#[test]
fn reload_enters_at_the_next_bar() {
    let (dir, mut host, mut controller, mut engine) = setup("reload");
    let every_step = r#"
bpm(120)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 1 1 1  1 1 1 1  1 1 1 1  1 1 1 1")
"#;
    load(&mut host, &mut controller, &dir, every_step).unwrap();
    controller.start().unwrap();
    let mut l = render_left(&mut engine, BAR / 2 + 77);

    // Recarga a mitad de compás: otro patrón y 60 BPM (pasos de 12000).
    let changed = r#"
bpm(60)
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("2 . . .  2 . . .  . . . .  . . . .")
"#;
    load(&mut host, &mut controller, &dir, changed).unwrap();
    l.extend(render_left(&mut engine, BAR / 2 - 77 + BAR));

    for step in 0..16 {
        assert_level(&l, 0, step, 0.5);
    }
    // Compás 1: corte 2 en el paso 0 (frame 96000) y en el paso 4, que a
    // 60 BPM cae en 96000 + 4 · 12000; en el paso 1 a 120 BPM, nada.
    assert!((l[BAR + SETTLE] - 0.25).abs() < EPS);
    assert_eq!(l[BAR + STEP + SETTLE], 0.0);
    assert!((l[BAR + 4 * 2 * STEP + SETTLE] - 0.25).abs() < EPS);
    assert_eq!(host.song().unwrap().bpm, Some(60.0));
}

#[test]
fn errors_keep_the_previous_pattern_playing() {
    let (dir, mut host, mut controller, mut engine) = setup("errors");
    host.set_time_limit(Duration::from_millis(100));
    load(&mut host, &mut controller, &dir, SONG_A).unwrap();
    controller.start().unwrap();
    let mut audio = render_left(&mut engine, BAR + 1234);

    let nine_tracks: String =
        std::iter::once("local a = sample(\"break.wav\"):slice(2)\n".to_string())
            .chain((0..9).map(|i| format!("track(\"t{i}\", a)\n")))
            .collect();
    let bad: Vec<(String, &[&str])> = vec![
        ("bpm(120\n".into(), &["song.lua:"]),
        ("local x = nil\nx.y = 1\n".into(), &["song.lua:2:"]),
        (
            "local a = sample(\"break.wav\"):slice(2)\n\ntrack(\"d\", a):steps(\"1 . 2^ .  . . . .  . . . .  . . . .\")\n".into(),
            &["song.lua:3:", "paso 3 (\"2^\")", "falta el número después de ^"],
        ),
        (
            "local a = sample(\"break.wav\"):slice(2)\ntrack(\"d\", a):steps(\"5 . . .  . . . .  . . . .  . . . .\")\n".into(),
            &["song.lua:2:", "el corte 5 no existe", "tiene 2"],
        ),
        (
            "local a = sample(\"break.wav\"):slice(2)\ntrack(\"d\", a):steps(\"1@9 . . .  . . . .  . . . .  . . . .\")\n".into(),
            &["song.lua:2:", "ganancia"],
        ),
        ("sample(\"nope.wav\")\n".into(), &["song.lua:1:", "nope.wav"]),
        ("track(\"hats\"):steps(\"1\")\n".into(), &["song.lua:1:", "\"hats\" no existe"]),
        ("bpm(1000)\n".into(), &["song.lua:1:", "BPM 1000"]),
        ("while true do end\n".into(), &["tarda"]),
        (nine_tracks, &["el máximo es 8"]),
        ("io.open(\"x\")\n".into(), &["song.lua:1:", "io"]),
    ];
    for (src, fragments) in &bad {
        let err = load(&mut host, &mut controller, &dir, src).unwrap_err();
        for fragment in *fragments {
            assert!(
                err.contains(fragment),
                "error \"{err}\" sin \"{fragment}\"\n{src}"
            );
        }
    }
    audio.extend(render_left(&mut engine, 2 * BAR - 1234));

    // Lo que suena es idéntico a no haber intentado ninguna recarga.
    let (mut controller2, mut engine2) = new_engine(RATE);
    let mut host2 = ScriptHost::new(RATE);
    load(&mut host2, &mut controller2, &dir, SONG_A).unwrap();
    controller2.start().unwrap();
    let reference = render_left(&mut engine2, 3 * BAR);
    assert_eq!(audio, reference);

    // El modelo sigue siendo el de la última carga válida.
    assert_eq!(host.song().unwrap().tracks.len(), 2);
}

/// Los límites del script (memoria y tiempo): lo que se desboca se corta
/// pronto, con su error y su línea, sigue sonando lo anterior y la carga
/// siguiente entra bien. Cubre también la memoria que se reserva desde
/// las funciones de Rust (`scale` crea sus tablas en el intérprete).
#[test]
fn runaway_scripts_hit_the_limits_and_the_previous_keeps_playing() {
    let (dir, mut host, mut controller, mut engine) = setup("limits");
    let limit = Duration::from_millis(500);
    load(&mut host, &mut controller, &dir, SONG_A).unwrap();
    controller.start().unwrap();
    let mut audio = render_left(&mut engine, BAR / 2);

    let ms = Duration::from_millis;
    // Los casos de memoria llevan un límite de tiempo holgado: con la
    // máquina cargada, el de 0,5 s podía llegar antes que el de memoria
    // (N11 de `docs/AUDITORIA-2.md`).
    let roomy = Duration::from_secs(30);
    // (script, límite de memoria en MB, de tiempo, lo que dice el error)
    let bad: [(&str, usize, Duration, &str); 5] = [
        // Cadenas grandes que se guardan.
        (
            "local t = {}\nwhile true do t[#t + 1] = string.rep(\"x\", 100000) .. #t end\n",
            32,
            roomy,
            "el script usa más de 32 MB de memoria",
        ),
        // Tablas que crea Rust (cada `scale` es una tabla nueva).
        (
            "local t = {}\nwhile true do t[#t + 1] = scale(\"minor\", \"c\") end\n",
            4,
            roomy,
            "el script usa más de 4 MB de memoria",
        ),
        // Concatenar en un bucle.
        (
            "local s = \"\"\nwhile true do s = s .. string.rep(\"y\", 100000) end\n",
            32,
            roomy,
            "el script usa más de 32 MB de memoria",
        ),
        // Instrucciones caras sin guardar nada: solo el tiempo las corta.
        (
            "local s = string.rep(\"z\", 100000)\nwhile true do local _ = s:upper() end\n",
            32,
            limit,
            "tarda más de 0.5 s",
        ),
        ("\nwhile true do end\n", 32, limit, "tarda más de 0.5 s"),
    ];
    for (src, mb, time, expected) in bad {
        host.set_memory_limit(mb << 20);
        host.set_time_limit(time);
        let start = std::time::Instant::now();
        let err = load(&mut host, &mut controller, &dir, src).unwrap_err();
        let took = start.elapsed();
        assert!(
            err.contains(expected),
            "\"{err}\" sin \"{expected}\"\n{src}"
        );
        assert!(
            err.contains("song.lua:2:"),
            "sin la línea: \"{err}\"\n{src}"
        );
        assert!(err.contains("[script]"), "sin el ajuste: \"{err}\"");
        // Un límite de tiempo corta poco después de pasarlo; el margen es
        // absoluto y amplio (con la máquina cargada se tardaba casi el
        // doble), pero un límite que no corta lo sigue rompiendo.
        if time == limit {
            assert!(took >= limit, "{src}: cortó antes del límite ({took:?})");
            assert!(took < limit + ms(2000), "{src}: tardó {took:?}");
        }
    }
    audio.extend(render_left(&mut engine, BAR / 2 + BAR));

    // Lo que suena es lo de antes, como si no hubiera habido nada.
    let (mut controller2, mut engine2) = new_engine(RATE);
    let mut host2 = ScriptHost::new(RATE);
    load(&mut host2, &mut controller2, &dir, SONG_A).unwrap();
    controller2.start().unwrap();
    assert_eq!(audio, render_left(&mut engine2, 2 * BAR));

    // Y la carga siguiente, buena, entra.
    let only_kick = r#"
local amen = sample("break.wav"):slice(2)
track("kick", amen):steps("x . . .  x . . .  . . . .  . . . .")
"#;
    load(&mut host, &mut controller, &dir, only_kick).unwrap();
    assert_eq!(host.song().unwrap().tracks.len(), 1);
    assert!(host.memory_peak() < 1 << 20);
}

/// Con el límite de memoria de siempre, el que reserva sin parar también
/// se corta (y no se lleva por delante el proceso).
#[test]
fn the_default_memory_limit_holds() {
    let (dir, mut host, mut controller, _engine) = setup("limits_default");
    let err = load(
        &mut host,
        &mut controller,
        &dir,
        "local t = {}\nwhile true do t[#t + 1] = string.rep(\"x\", 10000000) .. #t end\n",
    )
    .unwrap_err();
    let mb = script::DEFAULT_MEMORY_LIMIT >> 20;
    assert!(err.contains(&format!("más de {mb} MB")), "{err}");
}

/// Lo que cuesta comprobar el tiempo cada N instrucciones, en un bucle de
/// Lua que solo hace cuentas (lo peor para el hook): `cargo test
/// --release -p script --test lua medida_del_hook -- --ignored
/// --nocapture`.
#[test]
#[ignore = "medida: cargo test --release -p script --test lua medida_del_hook -- --ignored --nocapture"]
fn medida_del_hook() {
    use mlua::debug::DebugEvent;
    use mlua::{HookTriggers, Lua, VmState};
    // Cuentas sin llamadas (lo peor para el hook de instrucciones) y
    // llamadas sin nada más (lo peor para el de llamadas).
    const LOOP: &str = "local s = 0 for i = 1, 30000000 do s = s + i % 7 end return s";
    const CALLS: &str = "local function f(x) return x + 1 end local s = 0 for i = 1, 10000000 do s = f(s) end return s";
    let run = |src: &str, every: Option<u32>, calls: bool| {
        let lua = Lua::new();
        if let Some(n) = every {
            let deadline = std::time::Instant::now() + Duration::from_secs(60);
            let line = std::rc::Rc::new(std::cell::Cell::new(0usize));
            let peak = std::rc::Rc::new(std::cell::Cell::new(0usize));
            let mut triggers = HookTriggers::new().every_nth_instruction(n);
            if calls {
                triggers = triggers.on_calls();
            }
            let half = 128usize << 20;
            lua.set_hook(triggers, move |lua, debug| {
                let used = lua.used_memory();
                if debug.event() == DebugEvent::Count {
                    if let Some(l) = debug.current_line() {
                        line.set(l);
                    }
                    peak.set(peak.get().max(used));
                    if std::time::Instant::now() > deadline {
                        return Ok(VmState::Yield);
                    }
                } else if used > half
                    && let Some(Some(l)) = lua.inspect_stack(1, |d| d.current_line())
                {
                    line.set(l);
                }
                Ok(VmState::Continue)
            })
            .unwrap();
        }
        (0..5)
            .map(|_| {
                let start = std::time::Instant::now();
                let _: i64 = lua.load(src).eval().unwrap();
                start.elapsed()
            })
            .min()
            .unwrap()
    };
    for (name, src) in [("cuentas", LOOP), ("llamadas", CALLS)] {
        let base = run(src, None, false);
        println!("{name}: sin hook {:.1} ms", base.as_secs_f64() * 1e3);
        let pct = |t: Duration| 100.0 * (t.as_secs_f64() / base.as_secs_f64() - 1.0);
        for n in [100u32, 1000, 10_000] {
            let t = run(src, Some(n), false);
            let c = run(src, Some(n), true);
            println!(
                "  cada {n:>6}: +{:.1} %; y en cada llamada: +{:.1} %",
                pct(t),
                pct(c)
            );
        }
    }
}

#[test]
fn reload_reuses_loaded_samples_and_keeps_track_slots() {
    let (dir, mut host, mut controller, mut engine) = setup("slots");
    load(&mut host, &mut controller, &dir, SONG_A).unwrap();
    let first = host.song().unwrap().tracks[1].sample.clone();
    let kick_slot = host.track_slot("kick").unwrap();
    controller.start().unwrap();
    let mut l = render_left(&mut engine, BAR / 2);

    // Se quita "drums": "kick" conserva su pista y "drums" calla en el
    // compás siguiente.
    let only_kick = r#"
local amen = sample("break.wav"):slice(2)
track("kick", amen):steps("x . . .  x . . .  . . . .  . . . .")
"#;
    load(&mut host, &mut controller, &dir, only_kick).unwrap();
    l.extend(render_left(&mut engine, BAR / 2 + BAR));

    assert!(Arc::ptr_eq(&first, &host.song().unwrap().tracks[0].sample));
    assert_eq!(host.track_slot("kick"), Some(kick_slot));
    assert_eq!(host.track_slot("drums"), None);
    assert_level(&l, 0, 0, engine::soft_limit(1.0));
    assert_level(&l, 0, 2, 0.25);
    assert_level(&l, 1, 0, 0.5);
    assert_level(&l, 1, 2, 0.0);
    assert_level(&l, 1, 4, 0.5);
}

#[test]
fn load_file_resolves_samples_next_to_the_script() {
    let (dir, mut host, mut controller, mut engine) = setup("file");
    let path = dir.join("song.lua");
    std::fs::write(&path, SONG_A).unwrap();
    host.load_file(&mut controller, &path).unwrap();
    controller.start().unwrap();
    let l = render_left(&mut engine, BAR);
    assert_level(&l, 0, 0, engine::soft_limit(1.0));

    std::fs::write(&path, "bpm(120)\n\nbpm(\"rápido\")\n").unwrap();
    let err = host.load_file(&mut controller, &path).err().unwrap();
    assert!(err.message.contains("song.lua:3:"), "{err}");
}

#[test]
fn track_gain_from_lua() {
    let (dir, mut host, mut controller, mut engine) = setup("gain");
    let src = r#"
local amen = sample("break.wav"):slice(2)
track("drums", amen):steps("1 . . .  . . . .  . . . .  . . . ."):gain(0.5)
"#;
    load(&mut host, &mut controller, &dir, src).unwrap();
    controller.start().unwrap();
    let l = render_left(&mut engine, BAR);
    assert_level(&l, 0, 0, 0.25);
    assert_eq!(host.song().unwrap().tracks[0].gain, 0.5);

    let bad = "local a = sample(\"break.wav\"):slice(2)\ntrack(\"d\", a):gain(5)\n";
    let err = load(&mut host, &mut controller, &dir, bad).unwrap_err();
    assert!(
        err.contains("song.lua:2:") && err.contains("volumen de pista 5"),
        "{err}"
    );
}

/// Los rodeos del límite de tiempo (N4 de `docs/AUDITORIA-2.md`): un
/// `pcall` o un `xpcall` en un bucle que se traga el error del límite, un
/// patrón de `string.find` con vuelta atrás (C, donde el hook no llega) y
/// un `__gc` con un bucle (Lua apaga los hooks en los finalizadores). Cada
/// uno colgaba la carga para siempre; ahora acaba poco después del límite,
/// con su error, y la carga siguiente entra. Cada caso corre en su hilo
/// con su propio host: si no vuelve, el test falla en vez de colgarse.
#[test]
fn the_time_limit_cannot_be_dodged() {
    let limit = Duration::from_millis(500);
    let cases: [(&str, &str); 8] = [
        (
            "pcall",
            "bpm(120)\nwhile true do pcall(function() while true do end end) end\n",
        ),
        (
            "pcall anidado",
            "bpm(120)\nwhile true do pcall(pcall, function() while true do end end) end\n",
        ),
        (
            "xpcall",
            "bpm(120)\nwhile true do xpcall(function() while true do end end, function(e) return e end) end\n",
        ),
        (
            "pcall con error()",
            "bpm(120)\nwhile true do pcall(error, \"x\") end\n",
        ),
        (
            "string.find",
            "bpm(120)\nlocal s = (\"a\"):rep(5000)\nlocal r = s:find((\"a-\"):rep(25) .. \"b\")\n",
        ),
        // Un patrón que el filtro deja pasar (dos cuantificadores, coste
        // justo en el máximo) pero que tarda minutos en C: lo para el
        // vigilante.
        (
            "patrón lento en C",
            "bpm(120)\nlocal s = (\"a\"):rep(10000)\nlocal r = s:find(\".-.-x\")\n",
        ),
        (
            "__gc",
            "bpm(120)\nsetmetatable({}, { __gc = function() while true do end end })\ncollectgarbage()\n",
        ),
        (
            "__tostring del error",
            "error(setmetatable({}, { __tostring = function() while true do end end }))\n",
        ),
    ];
    let dir = fixture("dodge");
    for (what, source) in cases {
        let (tx, rx) = std::sync::mpsc::channel();
        let (source, dir_) = (source.to_string(), dir.clone());
        let started = std::time::Instant::now();
        std::thread::spawn(move || {
            let (mut controller, _engine) = new_engine(RATE);
            let mut host = ScriptHost::new(RATE);
            host.set_time_limit(limit);
            load(&mut host, &mut controller, &dir_, SONG_A).unwrap();
            let error = load(&mut host, &mut controller, &dir_, &source).err();
            // Sigue la de antes y la siguiente entra.
            let kept = host.song().map(|s| s.tracks.len());
            let next = load(&mut host, &mut controller, &dir_, SONG_A);
            let _ = tx.send((error, kept, next));
        });
        let (error, kept, next) = rx
            .recv_timeout(limit + Duration::from_secs(5))
            .unwrap_or_else(|_| panic!("{what}: la carga no vuelve"));
        let took = started.elapsed();
        let error = error.unwrap_or_else(|| panic!("{what}: cargó sin error"));
        assert!(
            ["tarda más de", "patrón", "__gc", "se abandona"]
                .iter()
                .any(|m| error.contains(m)),
            "{what}: {error}"
        );
        assert_eq!(kept, Some(2), "{what}: no sigue lo de antes");
        assert_eq!(next, Ok(()), "{what}: la siguiente no entra");
        assert!(
            took < limit + Duration::from_secs(3),
            "{what}: tardó {took:?}"
        );
    }
}

/// Lo que cuesta evaluar en un hilo vigilado (N4): una evaluación de una
/// canción corta y, aparte, lo que cuesta solo crear el hilo (con su pila
/// de 16 MB) y recibir el resultado: `cargo test --release -p script --test
/// lua medida_del_vigilante -- --ignored --nocapture`.
#[test]
#[ignore = "medida: cargo test --release -p script --test lua medida_del_vigilante -- --ignored --nocapture"]
fn medida_del_vigilante() {
    let dir = fixture("vigilante");
    let mut host = ScriptHost::new(RATE);
    host.eval(SONG_A, "song.lua", &dir).unwrap();
    let rounds = 500;
    let start = std::time::Instant::now();
    for _ in 0..rounds {
        host.eval(SONG_A, "song.lua", &dir).unwrap();
    }
    let eval = start.elapsed() / rounds;
    let start = std::time::Instant::now();
    for _ in 0..rounds {
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .stack_size(16 << 20)
            .spawn(move || tx.send(1).unwrap())
            .unwrap();
        rx.recv().unwrap();
    }
    let thread = start.elapsed() / rounds;
    println!("evaluar SONG_A: {eval:?} por carga; el hilo solo: {thread:?}");
}

/// La biblioteca base sin `dofile` ni `loadfile` (leían cualquier archivo
/// como Lua), y `print` en los avisos de la carga en vez de en la salida
/// estándar, que ensuciaba la pantalla de la interfaz (N9 de
/// `docs/AUDITORIA-2.md`).
#[test]
fn print_goes_to_the_warnings_and_there_is_no_dofile() {
    let (dir, mut host, mut controller, _engine) = setup("print");
    std::fs::write(dir.join("otro.lua"), "bpm(99)\n").unwrap();
    for source in ["dofile(\"otro.lua\")\n", "loadfile(\"otro.lua\")()\n"] {
        let error = load(&mut host, &mut controller, &dir, source).unwrap_err();
        assert!(error.contains("nil"), "{source}: {error}");
    }
    let source = format!("{SONG_A}print(\"hola\", 3, nil)\nfor i = 1, 30 do print(i) end\n");
    host.load(&mut controller, &source, "song.lua", &dir)
        .unwrap();
    let warnings = &host.song().unwrap().warnings;
    assert!(
        warnings.contains(&"song.lua:6: print: hola\t3\tnil".to_string()),
        "{warnings:?}"
    );
    let prints = warnings.iter().filter(|w| w.contains("print:")).count();
    assert_eq!(prints, script::sandbox::MAX_PRINTS + 1, "{warnings:?}");
    assert!(warnings.iter().any(|w| w.contains("hay más líneas")));
}
