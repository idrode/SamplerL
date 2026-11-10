//! Los parsers de texto: mini-notación (pasos, notas, duraciones, llaves),
//! puntos de las lanes, proyectos `.sampler` y bloques.

use std::time::Instant;

use crate::{Cuenta, Rng, carpeta};

const TROZOS: &[&str] = &[
    "1", "2", "3", "8", "64", "65", "0", "x", ".", "_", "kick", "snare", "'", "^", "^7", "^-12", "^0.5", "^36",
    "^37", "^1e9", "^nan", "^inf", "%", "%7", "%-12", "@", "@0.5", "@4", "@4.1", "@-1", "@1e30", "r", "r1", "r4",
    "r16", "r17", "r0", "~", "~800", "~0", "~1e9", "!", "/", "{", "}", "{lp=800}", "{lp=800,q=4}", "{crush=6}",
    "{lp=200>4000}", "{lp=200>4000:4}", "{lp=200>4000:0}", "{lp=200>4000:1e9}", "{dur=1/8}", "{dur=0/0}",
    "{dur=1/0}", "{dur=65535/1}", "{dur=2b}", "{dur=120ms}", "{fit=loop}", "{sub=0.5,sub_div=3}", "{type=highpass}",
    "{=}", "{,}", "{lp=}", "{lp==}", "{ lp = 800 , q = 4 }", "=", ",", ">", ":", " ", "  ", "\t", "\n", "c", "c#",
    "eb3", "g1", "bb-1", "c9", "c99999", "[", "]", "*", "é", "ñ", "🥁", "\u{0}", "\u{feff}", "-", "+", "1e308",
    "99999999999999999999", "0x10", "1/8", "1/16t", "3b", "250ms", "ms", "b",
];

fn cadena(rng: &mut Rng, max: usize) -> String {
    let mut s = String::new();
    let n = 1 + rng.below(max);
    for _ in 0..n {
        if rng.chance(0.03) {
            // Un carácter cualquiera.
            if let Some(c) = char::from_u32((rng.next() % 0x3000) as u32) {
                s.push(c);
            }
        } else {
            s.push_str(rng.pick(TROZOS));
        }
        if rng.chance(0.45) {
            s.push(' ');
        }
    }
    s
}

pub fn notacion(rng: &mut Rng, cuenta: &mut Cuenta, fin: Instant) {
    let etiquetas = vec![Some("kick".to_string()), None, Some("snare".to_string()), Some("é".to_string())];
    let fx: &dyn Fn(&[engine::notation::WrittenFx]) -> Result<engine::notation::StepBraces, String> =
        &|w| script::step_fx::resolve(w);
    while Instant::now() < fin {
        let texto = cadena(rng, 40);
        let largo = *rng.pick(&[1usize, 3, 12, 16, 16, 16, 32, 64, 0, 65, 1000]);
        let k = rng.next();
        cuenta.correr("notacion", texto.as_bytes(), || {
            let mut ok = false;
            let _ = engine::notation::step_tokens(&texto);
            let textos = engine::notation::step_texts(&texto);
            for t in textos.iter().take(64) {
                ok |= engine::notation::parse_step(t).is_ok();
                let _ = engine::notation::parse_step_labeled(t, &etiquetas);
                let _ = engine::notation::split_label(t);
                let _ = engine::notation::check_label(t);
                let _ = script::patterns::note_step(t, Some(60), None);
                let _ = script::step_fx::dur_text(t);
                let _ = script::patterns::note_number(t);
            }
            ok |= engine::notation::parse_pattern_labeled(&texto, largo, &etiquetas).is_ok();
            ok |= engine::notation::parse_pattern_with(&texto, largo, &etiquetas, Some(fx)).is_ok();
            let _ = engine::notation::parse_steps(&texto);
            let _ = script::patterns::rotate(&texto, (k % 200) as i64 - 100);
            // `rotate(texto, i64::MIN)` da pánico siempre con comprobación de desbordes
            // (patterns.rs:77, `i - n`): anotado aparte para que no tape lo demás.
            let _ = script::patterns::reverse_pattern(&texto);
            let mut r = script::patterns::Rng::new(k);
            let _ = script::patterns::mutate(&texto, (k % 100) as f64 / 99.0, &mut r);
            let _ = script::patterns::shuffle(&texto, &mut r);
            let _ = script::patterns::melody(&[0, 2, 3, 5, 7, 8, 10], (k % 128) as i32, &texto, "1");
            let _ = script::patterns::notes(&texto, (k % 128) as i32, None);
            let _ = script::patterns::euclid((k % 70) as i64 - 3, ((k >> 8) % 70) as i64 - 3, &texto, None);
            // Los puntos de una lane, tal como se escriben.
            let puntos = puntos(&texto);
            if let Some(p) = script::parse_points(&puntos) {
                let _ = script::points_text(&p, &puntos);
                ok = true;
            }
            let _ = script::blocks::parse(&texto);
            let _ = script::blocks::parse_tags(&texto);
            let _ = script::blocks::check_name(&texto);
            let _ = script::blocks::fold(&texto);
            ok
        });
    }
}

/// El mismo texto con forma de tabla de puntos: `{ {0, 200}, {1, 2000, "exp"} }`.
fn puntos(texto: &str) -> String {
    let mut s = String::from("{ ");
    for (i, palabra) in texto.split_whitespace().enumerate() {
        let n: String = palabra.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | 'e')).collect();
        match i % 4 {
            0 => s.push_str(&format!("{{{}, ", if n.is_empty() { "0" } else { &n })),
            1 => s.push_str(&format!("{}}}, ", if n.is_empty() { "1" } else { &n })),
            2 => s.push_str(&format!("{{{}, 5, \"{}\"}}, ", if n.is_empty() { "2" } else { &n }, palabra)),
            _ => s.push_str(palabra),
        }
    }
    s.push('}');
    s
}

const LINEAS: &[&str] = &[
    "script = cancion.lua", "script = ../../../../etc/passwd", "script = /dev/zero", "script =", "samples = .",
    "samples = /", "samples = ../..", "bpm = 170", "bpm = 0", "bpm = nan", "bpm = inf", "bpm = 1e999", "bpm = -1",
    "etiqueta = amen.wav 8 1 kick", "etiqueta = amen.wav 8 2 snare", "etiqueta = amen.wav g16 1 kick",
    "etiqueta = amen.wav 8 0 kick", "etiqueta = amen.wav 8 99999999999999999999 kick", "etiqueta = 8 1 kick",
    "etiqueta = un sample con espacios.wav 8 3 hat", "etiqueta =    ", "corte = amen.wav 8 3 412.5",
    "corte = amen.wav 8 3 -1", "corte = amen.wav 8 3 nan", "corte = amen.wav 8 3 1e308", "corte = amen.wav 8 0 1",
    "corte = amen.wav 8 65 1", "corte = amen.wav 0 1 1", "corte = amen.wav 18446744073709551615 1 1",
    "partir = amen.wav 8 412.5", "partir = amen.wav 8 nan", "partir = amen.wav 8 -5", "partir = amen.wav 8 1e30",
    "unir = amen.wav 8 3", "unir = amen.wav 8 0", "unir = amen.wav 8 1", "unir = amen.wav 8 99",
    "mando = 1 74 drums filter.cutoff", "mando = 16 127 drums gain", "mando = 0 0", "mando = 17 128 x y z",
    "mando = 1 74 drums filter.cutoff 20 20000 exp", "mando =", "# comentario", "", "sin igual", "= = =",
    "clave_rara = 1", "script = a\u{0}b.lua", "bpm = 170 # no es comentario",
];

pub fn proyecto(rng: &mut Rng, cuenta: &mut Cuenta, fin: Instant) {
    let dir = carpeta("tmp").join("proyecto");
    std::fs::create_dir_all(&dir).unwrap();
    let amen = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../scripts/ejemplos/amen.wav");
    let _ = std::fs::copy(amen, dir.join("amen.wav"));
    std::fs::write(
        dir.join("cancion.lua"),
        "bpm(170)\nlocal a = sample(\"amen.wav\"):slice(8)\ntrack(\"drums\", a):steps(\"kick . 2 . 3 . 4 . 5 . 6 . 7 . 8 .\")\n  :fx(\"filter\", { cutoff = 800 })\n",
    )
    .unwrap();
    let path = dir.join("fuzz.sampler");
    while Instant::now() < fin {
        let mut texto = String::new();
        for _ in 0..rng.below(12) {
            let mut linea = rng.pick(LINEAS).to_string();
            if rng.chance(0.15) && !linea.is_empty() {
                // Un byte cambiado.
                let mut b = linea.into_bytes();
                let at = rng.below(b.len());
                b[at] = b" =.0-9eA\n#"[rng.below(10)];
                linea = String::from_utf8_lossy(&b).into_owned();
            }
            texto.push_str(&linea);
            texto.push('\n');
        }
        std::fs::write(&path, &texto).unwrap();
        cuenta.correr("proyecto", texto.as_bytes(), || {
            let Ok(project) = script::Project::load(&path) else {
                return false;
            };
            // Lo leído se aplica a una carga de verdad (los ajustes de
            // cortes, las etiquetas y los mandos) y se vuelve a guardar.
            if project.script.ends_with("cancion.lua") {
                let mut host = script::ScriptHost::new(48_000);
                host.set_seed(Some(7));
                host.set_labels(project.labels.clone());
                host.set_positions(project.positions.clone());
                host.set_edits(project.edits.clone());
                host.set_default_bpm(project.bpm);
                let (mut controller, _engine) = engine::new_engine(48_000);
                if let Ok(song) = host.load_file(&mut controller, &project.script) {
                    let _ = song.render(48_000, 1);
                }
            }
            let copia = dir.join("copia.sampler");
            if project.save(&copia).is_ok() {
                // Guardar y leer da lo mismo.
                let otra = script::Project::load(&copia).expect("lo guardado se lee");
                assert_eq!(otra.labels, project.labels, "las etiquetas no sobreviven a guardar");
                assert_eq!(otra.positions, project.positions, "los ajustes no sobreviven a guardar");
                assert_eq!(otra.edits, project.edits, "partir/unir no sobreviven a guardar");
            }
            true
        });
    }
}

pub fn bloques(rng: &mut Rng, cuenta: &mut Cuenta, fin: Instant) {
    let dir = carpeta("tmp").join("bloques");
    let cabeceras = [
        "-- nombre: x", "-- nombre: ../../fuera", "-- nombre: a/b", "-- nombre:", "-- descripcion: d",
        "-- etiquetas: a, b,, ,c", "-- etiquetas:", "--", "-- nombre: \u{0}", "-- nombre: con espacios y ñ",
        "track(\"d\", a):steps(\"${patron}\")", "${", "${}", "${a}${a}${b", "", "-- nombre: .", "-- nombre: ..",
        "-- nombre: CON", "-- nombre: muy-largo-muy-largo-muy-largo-muy-largo-muy-largo-muy-largo-muy-largo",
    ];
    while Instant::now() < fin {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("global")).unwrap();
        let mut entrada = String::new();
        for i in 0..rng.below(5) {
            let mut texto = String::new();
            for _ in 0..rng.below(6) {
                texto.push_str(rng.pick(&cabeceras));
                texto.push('\n');
            }
            let nombre = *rng.pick(&["a.lua", "b.lua", "ñ.lua", ".lua", "c.txt", "d.LUA", "e e.lua"]);
            let _ = std::fs::write(dir.join("global").join(format!("{i}{nombre}")), &texto);
            entrada.push_str(&format!("=== {i}{nombre}\n{texto}"));
        }
        let nombre = cabeceras[rng.below(cabeceras.len())].trim_start_matches("-- nombre:").trim().to_string();
        let cuerpo = rng.pick(&cabeceras).to_string();
        entrada.push_str(&format!("=== guardar «{nombre}»\n{cuerpo}\n"));
        cuenta.correr("bloques", entrada.as_bytes(), || {
            let mut lib = script::blocks::Library::open(Some(dir.join("global")), Some(dir.join("proyecto")));
            lib.reload();
            let _ = lib.filter("a");
            let guardado = lib.save(script::blocks::Scope::Global, &nombre, "d", &["t".to_string()], &cuerpo, true);
            if let Ok(block) = &guardado {
                // Nunca fuera de su carpeta.
                let real = std::fs::canonicalize(&block.path).expect("el bloque guardado existe");
                let base = std::fs::canonicalize(dir.join("global")).unwrap();
                assert!(real.starts_with(&base), "el bloque se escribió fuera de su carpeta: {}", real.display());
                let _ = lib.update(&block.name, "otro cuerpo");
                let _ = lib.delete(&block.name);
            }
            guardado.is_ok()
        });
    }
}
