//! La red de pánicos de la carga: un fallo del programa dentro de una
//! función de la API es un error en el editor con la línea, sigue sonando
//! lo anterior y la siguiente carga funciona.

use super::*;

const GOOD: &str = "bpm(120)\n";

fn host() -> (Controller, ScriptHost) {
    let (controller, engine) = engine::new_engine(48_000);
    // El motor no hace falta para cargar: basta con que la cola exista.
    std::mem::forget(engine);
    (controller, ScriptHost::new(48_000))
}

#[test]
fn a_panic_in_the_api_is_an_error_with_the_line_and_the_old_song_stays() {
    let (mut controller, mut host) = host();
    host.load(&mut controller, GOOD, "song.lua", Path::new("."))
        .map_err(|e| e.message)
        .unwrap();
    let error = host
        .load(
            &mut controller,
            "bpm(130)\nlocal x = 1\n__panico(\"roto\")\n",
            "song.lua",
            Path::new("."),
        )
        .err()
        .expect("la carga tenía que fallar")
        .message;
    assert!(error.starts_with("song.lua:3: error interno"), "{error}");
    assert!(error.contains("roto"), "{error}");
    assert!(error.contains("sigue sonando lo anterior"), "{error}");
    // Sigue la de antes.
    assert_eq!(host.song().map(|s| s.bpm), Some(Some(120.0)));
    // Y la siguiente carga, bien.
    host.load(&mut controller, "bpm(140)\n", "song.lua", Path::new("."))
        .map_err(|e| e.message)
        .unwrap();
    assert_eq!(host.song().map(|s| s.bpm), Some(Some(140.0)));
}

#[test]
fn pcall_does_not_swallow_a_panic() {
    let (mut controller, mut host) = host();
    let error = host
        .load(
            &mut controller,
            "pcall(__panico, \"dentro\")\nbpm(120)\n",
            "song.lua",
            Path::new("."),
        )
        .err()
        .expect("la carga tenía que fallar")
        .message;
    assert!(
        error.contains("error interno") && error.contains("dentro"),
        "{error}"
    );
    let error = host
        .load(
            &mut controller,
            "xpcall(__panico, function(e) return e end, \"x\")\nbpm(120)\n",
            "song.lua",
            Path::new("."),
        )
        .err()
        .expect("la carga tenía que fallar")
        .message;
    assert!(error.contains("error interno"), "{error}");
}

#[test]
fn a_panic_inside_a_section_body_is_caught_too() {
    let (mut controller, mut host) = host();
    let error = host
        .load(
            &mut controller,
            "section(\"a\", function()\n  __panico(\"en la sección\")\nend)\n",
            "song.lua",
            Path::new("."),
        )
        .err()
        .expect("la carga tenía que fallar")
        .message;
    assert!(
        error.contains("error interno") && error.contains("en la sección"),
        "{error}"
    );
    assert!(error.starts_with("song.lua:2:"), "{error}");
}

/// Con la canción sonando, una carga que entra en pánico no cambia nada:
/// el compás siguiente suena igual que antes.
#[test]
fn after_a_panic_the_old_song_keeps_sounding() {
    const RATE: u32 = 48_000;
    const BAR: usize = 96_000; // 4/4 a 120
    let dir = std::env::temp_dir().join(format!("sampler-panico-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    engine::offline::write_wav(dir.join("a.wav"), &[0.5; 1000], 1, RATE).unwrap();
    let (mut controller, mut engine) = engine::new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    let song = "bpm(120)\nlocal a = sample(\"a.wav\"):slice(1)\n\
                track(\"x\", a):steps(\"1 . . .  . . . .  . . . .  . . . .\")\n";
    host.load(&mut controller, song, "song.lua", &dir)
        .map_err(|e| e.message)
        .unwrap();
    controller.start().unwrap();
    let mut audio = engine::offline::render(&mut engine, BAR / 2, 2);
    let broken = format!("{song}track(\"y\", a):steps(\"1\")\n__panico(\"roto\")\n");
    assert!(
        host.load(&mut controller, &broken, "song.lua", &dir)
            .is_err()
    );
    audio.extend(engine::offline::render(&mut engine, BAR / 2 + BAR, 2));
    // El golpe del compás 0 y el del 1, iguales.
    assert_eq!(audio[2 * 500], 0.5);
    assert_eq!(audio[2 * (BAR + 500)], 0.5);
    // Y la siguiente carga correcta entra al compás siguiente.
    let louder = song.replace("track(\"x\", a)", "track(\"x\", a):gain(0.5)");
    host.load(&mut controller, &louder, "song.lua", &dir)
        .map_err(|e| e.message)
        .unwrap();
    let audio = engine::offline::render(&mut engine, 2 * BAR, 2);
    // (La carga cae justo en el borde: entra en el compás siguiente.)
    assert_eq!(audio[2 * (BAR + 500)], 0.25);
    let _ = std::fs::remove_dir_all(&dir);
}
