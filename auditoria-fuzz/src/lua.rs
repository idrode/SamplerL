//! Canciones de verdad (las recetas de `scripts/ejemplos/` y los bloques de
//! Lua de la guía y del tutorial), mutadas: números cambiados por valores
//! límite, pasos cambiados por otros de la notación, líneas borradas,
//! duplicadas o traídas de otra canción. Lo que carga se toca en un motor
//! de verdad, con bloques de tamaños raros, y se mira cada muestra.
//!
//! Con `vivo`, además, entre bloque y bloque se mandan órdenes del modo en
//! vivo (macros, mandos, pánico, mute, saltos, arrancar en un compás).

use std::path::Path;
use std::time::Instant;

use crate::{Cuenta, Rng, carpeta};

const RATE: u32 = 48_000;

/// La canción con la que se comprueba que el motor sigue sano.
const LIMPIA: &str = "bpm(120)\nlocal amen = sample(\"amen.wav\"):slice(8)\ntrack(\"drums\", amen):steps(\"1 . 2 . 3 . 4 . 5 . 6 . 7 . 8 .\"):send(\"reverb\", 0.3)\n";

const NUMEROS: &[&str] = &[
    "0", "1", "-1", "2", "0.5", "0.0001", "1e-9", "1e-30", "1e9", "1e30", "16", "64", "65", "127", "255", "256",
    "20", "20000", "20000.1", "19.9", "24000", "(0/0)", "(1/0)", "(-1/0)", "(2^53)", "(2^63)", "-0.0", "3.999",
    "4", "4.0001", "0.999999", "1.000001", "400", "12", "-12", "36", "-36", "37", "48000", "44100",
];

const PASOS: &[&str] = &[
    "1", "x", ".", "_", "8", "1r16", "3'^36", "x%-12", "5@4", "1~20", "1~20000", "2{lp=20>20000:64}", "1!/",
    "8^7^12^-36", "3{crush=1}", "4{dur=1/64}", "4{dur=16b,fit=loop}", "4{dur=1ms,fit=stretch}", "1%12r16'",
    "2@0", "1^0.01", "6{hp=20000}", "7{lp=20,q=40}", "1{dist=40}", "3{delay=1}", "c", "c1", "g9", "eb/", "c!",
];

fn corpus() -> Vec<String> {
    let raiz = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut out = Vec::new();
    if let Ok(dir) = std::fs::read_dir(raiz.join("scripts/ejemplos")) {
        let mut rutas: Vec<_> = dir.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e == "lua")).collect();
        rutas.sort();
        for ruta in rutas {
            out.extend(std::fs::read_to_string(ruta));
        }
    }
    for doc in ["docs/GUIA.md", "docs/PRIMERA-CANCION.md"] {
        let Ok(texto) = std::fs::read_to_string(raiz.join(doc)) else { continue };
        let mut bloque: Option<String> = None;
        for linea in texto.lines() {
            match &mut bloque {
                None if linea.starts_with("```lua") => bloque = Some(String::new()),
                Some(b) if linea.starts_with("```") => {
                    if b.contains("track(") {
                        out.push(std::mem::take(b));
                    }
                    bloque = None;
                }
                Some(b) => {
                    b.push_str(linea);
                    b.push('\n');
                }
                None => {}
            }
        }
    }
    out
}

/// Posiciones (inicio, fin) de los números escritos en `texto`.
fn numeros(texto: &str) -> Vec<(usize, usize)> {
    let b = texto.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let previo = i.checked_sub(1).map(|j| b[j]);
        if b[i].is_ascii_digit() && !previo.is_some_and(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.') {
            let start = i;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            out.push((start, i));
        } else {
            i += 1;
        }
    }
    out
}

fn mutar(rng: &mut Rng, corpus: &[String]) -> String {
    let mut texto = rng.pick(corpus).clone();
    for _ in 0..1 + rng.below(3) {
        match rng.below(10) {
            // Un número por un valor límite.
            0..=4 => {
                let n = numeros(&texto);
                if n.is_empty() {
                    continue;
                }
                let (a, b) = *rng.pick(&n);
                // Dentro de una cadena (un paso) no vale una expresión.
                let dentro = texto[..a].bytes().filter(|&c| c == b'"').count() % 2 == 1;
                let nuevo = loop {
                    let v = *rng.pick(NUMEROS);
                    if !(dentro && v.starts_with('(')) {
                        break v;
                    }
                };
                texto.replace_range(a..b, nuevo);
            }
            // Un paso de un patrón por otro.
            5 | 6 => {
                let comillas: Vec<usize> = texto.match_indices('"').map(|(i, _)| i).collect();
                if comillas.len() < 2 {
                    continue;
                }
                let k = rng.below(comillas.len() / 2) * 2;
                let (a, b) = (comillas[k] + 1, comillas[k + 1]);
                let dentro = &texto[a..b];
                let palabras: Vec<(usize, &str)> = dentro.split(' ').scan(0usize, |pos, w| {
                    let at = *pos;
                    *pos += w.len() + 1;
                    Some((at, w))
                }).filter(|(_, w)| !w.is_empty()).collect();
                if palabras.len() < 4 {
                    continue;
                }
                let (at, w) = *rng.pick(&palabras);
                let (start, end) = (a + at, a + at + w.len());
                texto.replace_range(start..end, rng.pick(PASOS));
            }
            // Borrar, duplicar o traer una línea.
            7 => {
                let mut lineas: Vec<&str> = texto.lines().collect();
                if lineas.len() > 2 {
                    lineas.remove(rng.below(lineas.len()));
                }
                texto = lineas.join("\n");
            }
            8 => {
                let mut lineas: Vec<&str> = texto.lines().collect();
                if !lineas.is_empty() {
                    let k = rng.below(lineas.len());
                    lineas.insert(k, lineas[k]);
                }
                texto = lineas.join("\n");
            }
            _ => {
                let otra = rng.pick(corpus);
                let otras: Vec<&str> = otra.lines().filter(|l| l.trim_start().starts_with(':')).collect();
                let mut lineas: Vec<&str> = texto.lines().collect();
                if !otras.is_empty() && !lineas.is_empty() {
                    // Tras una línea que sigue una cadena de métodos.
                    let candidatas: Vec<usize> =
                        (0..lineas.len()).filter(|&i| lineas[i].trim_start().starts_with(':')).collect();
                    if !candidatas.is_empty() {
                        let k = *rng.pick(&candidatas);
                        lineas.insert(k, rng.pick(&otras));
                    }
                }
                texto = lineas.join("\n");
            }
        }
    }
    texto
}

pub fn correr(rng: &mut Rng, cuenta: &mut Cuenta, fin: Instant, vivo: bool) {
    let objetivo = if vivo { "motor" } else { "lua" };
    let corpus = corpus();
    assert!(!corpus.is_empty(), "no hay canciones de partida");
    let dir = carpeta("tmp").join("lua");
    std::fs::create_dir_all(dir.join("breaks")).unwrap();
    let amen = Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/ejemplos/amen.wav");
    for nombre in ["amen.wav", "perc.wav", "nota.wav", "x.wav", "break.wav", "breaks/amen.wav", "archivo.wav"] {
        let _ = std::fs::copy(&amen, dir.join(nombre));
    }
    eprintln!("{objetivo}: {} canciones de partida", corpus.len());
    let mut host = script::ScriptHost::new(RATE);
    host.set_seed(Some(7));
    host.set_time_limit(std::time::Duration::from_millis(1500));
    while Instant::now() < fin {
        let texto = mutar(rng, &corpus);
        let semilla = rng.next();
        let (mut nan, mut fuera) = (false, false);
        let mut envenenado = false;
        cuenta.correr(objetivo, texto.as_bytes(), || {
            let (mut controller, mut engine) = engine::new_engine(RATE);
            if host.load(&mut controller, &texto, "fuzz.lua", &dir).is_err() {
                return false;
            }
            let mut r = Rng(semilla | 1);
            let _ = controller.start();
            let canales = *r.pick(&[2usize, 2, 2, 1, 6]);
            let mut bloque = vec![0.0f32; 4096 * canales];
            // Unos 3 s de audio (a 170 BPM, algo más de dos compases).
            let mut quedan = 3 * RATE as usize;
            while quedan > 0 {
                let frames = (*r.pick(&[1usize, 2, 7, 64, 127, 256, 512, 512, 1024, 4096])).min(quedan);
                if vivo && r.chance(0.3) {
                    let pista = r.below(9);
                    let _ = match r.below(14) {
                        0 => controller.set_macro(r.below(9), r.below(101) as f32 / 100.0),
                        1 => controller.move_knob(r.below(17), r.below(101) as f32 / 100.0),
                        2 => controller.panic(r.chance(0.5)),
                        3 => controller.resume(),
                        4 => controller.set_mute(pista, r.chance(0.5)),
                        5 => controller.set_solo(pista, r.chance(0.5)),
                        6 => controller.start_at(*r.pick(&[0u64, 1, 2, 63, 64, 1000, 65_535, 65_536, 65_537])),
                        7 => controller.stop(),
                        8 => controller.start(),
                        9 => controller.jump_to(r.below(300)),
                        10 => controller.loop_section(r.chance(0.5)),
                        11 => controller.set_voice_limit(*r.pick(&[0usize, 1, 2, 64, 256, 257])),
                        12 => controller.set_metronome(r.chance(0.5)),
                        _ => host.load(&mut controller, &texto, "fuzz.lua", &dir).map(|_| ()).map_err(|_| engine::SendError::QueueFull),
                    };
                }
                let out = &mut bloque[..frames * canales];
                crate::audio(|| engine.process(out, canales));
                for &s in out.iter() {
                    nan |= !s.is_finite();
                    fuera |= s.abs() > 1.0;
                }
                quedan -= frames;
                controller.collect_garbage();
            }
            // ¿Se recupera el motor? El limitador convierte un NaN en
            // silencio, así que un bus o un filtro con NaN dentro no se ve
            // en las muestras: se carga una canción limpia en el mismo
            // motor y tiene que sonar.
            let _ = controller.resume();
            let _ = controller.set_voice_limit(64);
            for t in 0..8 {
                let _ = controller.set_mute(t, false);
                let _ = controller.set_solo(t, false);
            }
            if host.load(&mut controller, LIMPIA, "limpia.lua", &dir).is_err() {
                panic!("la canción limpia no carga después de la mutada");
            }
            let _ = controller.stop();
            for _ in 0..4 {
                engine.process(&mut bloque[..512 * canales], canales);
            }
            let _ = controller.start();
            let mut suena = false;
            for i in 0..(4 * RATE as usize / 512) {
                let out = &mut bloque[..512 * canales];
                // Sin contar reservas: quitar las lanes de la mutada pasa siempre por
                // engine.rs:3024 (N6) y taparía otros sitios.
                engine.process(out, canales);
                // Solo cuenta el último segundo.
                if i >= 3 * RATE as usize / 512 {
                    suena |= out.iter().any(|&s| s != 0.0 && s.abs() < 0.9999);
                }
                controller.collect_garbage();
            }
            envenenado = !suena;
            true
        });
        if envenenado {
            cuenta.no_finitas += 1;
            cuenta.guardar(objetivo, "MOTOR ENVENENADO: después de esta canción, una limpia ya no suena", texto.as_bytes());
            continue;
        }
        if nan {
            cuenta.no_finitas += 1;
            cuenta.guardar(objetivo, "MUESTRAS NO FINITAS (NaN o infinito) en la salida", texto.as_bytes());
        } else if fuera {
            cuenta.fuera += 1;
            cuenta.guardar(objetivo, "MUESTRAS FUERA DE ±1 en la salida", texto.as_bytes());
        }
    }
}
