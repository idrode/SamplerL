//! Cada bloque ```lua de docs/GUIA.md y docs/PRIMERA-CANCION.md y cada receta de scripts/ejemplos/
//! se carga sin errores contra el break sintético (como `amen.wav`), la
//! canción del tutorial crece paso a paso tal como se cuenta, y la sección
//! de la API coincide con el registro (`script::api`).

#[path = "../../engine/tests/support/synth_break.rs"]
mod synth;

use std::path::{Path, PathBuf};

use engine::{new_engine, offline};
use script::ScriptHost;

const RATE: u32 = 48_000;
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

/// La guía de referencia y el tutorial (otro archivo).
const GUIDE: &str = "GUIA.md";
const TUTORIAL: &str = "PRIMERA-CANCION.md";

fn doc(name: &str) -> String {
    std::fs::read_to_string(format!("{ROOT}/docs/{name}")).unwrap()
}

/// Carpeta con el break sintético como `amen.wav` y, para el tutorial,
/// otro break a 120 BPM (`perc.wav`) y dos notas sintéticas: un do central
/// (`nota.wav`) y un do grave (`bajo.wav`). Una por test: si dos tests
/// escriben el mismo archivo a la vez, uno puede leerlo a medias.
fn samples(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("guide_samples_{test}"));
    std::fs::create_dir_all(&dir).unwrap();
    let (audio, _) = synth::synth_break();
    offline::write_wav(dir.join("amen.wav"), &audio, 1, synth::RATE).unwrap();
    let eighth = (synth::RATE as f32 * 60.0 / 120.0 / 2.0) as usize;
    use synth::Hit::{Hat, Kick, Snare};
    let hits: Vec<_> = [Hat, Hat, Snare, Hat, Hat, Kick, Snare, Hat]
        .into_iter()
        .enumerate()
        .map(|(n, hit)| (hit, n * eighth, 0.8))
        .collect();
    let perc = synth::render_hits(synth::RATE, &hits, eighth * hits.len());
    offline::write_wav(dir.join("perc.wav"), &perc, 1, synth::RATE).unwrap();
    offline::write_wav(dir.join("nota.wav"), &note(261.63), 1, synth::RATE).unwrap();
    offline::write_wav(dir.join("bajo.wav"), &note(65.41), 1, synth::RATE).unwrap();
    dir
}

/// Dos segundos de una nota con cuatro armónicos, que se apaga despacio.
fn note(freq: f32) -> Vec<f32> {
    let rate = synth::RATE as f32;
    (0..2 * synth::RATE as usize)
        .map(|i| {
            let t = i as f32 / rate;
            let tone: f32 = (1..=4)
                .map(|k| (std::f32::consts::TAU * freq * k as f32 * t).sin() / k as f32)
                .sum();
            0.4 * tone * (t / 0.005).min(1.0) * (-0.5 * t).exp()
        })
        .collect()
}

/// Bloques ```lua con la línea donde empiezan.
fn lua_blocks(markdown: &str) -> Vec<(usize, String)> {
    tagged_blocks(markdown)
        .into_iter()
        .filter(|(_, tag, _)| tag.is_empty())
        .map(|(line, _, text)| (line, text))
        .collect()
}

/// Bloques ```lua con la línea donde empiezan y lo que lleva la apertura
/// tras `lua` (vacío en los de siempre; `añade`, `cambia` o `entero` en
/// los del tutorial).
fn tagged_blocks(markdown: &str) -> Vec<(usize, String, String)> {
    let mut blocks = Vec::new();
    let mut current: Option<(usize, String, String)> = None;
    for (i, line) in markdown.lines().enumerate() {
        match &mut current {
            None => {
                if let Some(tag) = line.trim().strip_prefix("```lua") {
                    current = Some((i + 2, tag.trim().to_string(), String::new()));
                }
            }
            Some(_) if line.trim() == "```" => blocks.push(current.take().unwrap()),
            Some((_, _, text)) => {
                text.push_str(line);
                text.push('\n');
            }
        }
    }
    blocks
}

fn check(source: &str, name: &str, dir: &Path) {
    let (mut controller, _engine) = new_engine(RATE);
    let mut host = ScriptHost::new(RATE);
    host.set_sample_dir(Some(dir.to_path_buf()));
    host.set_seed(Some(1));
    let song = host
        .load(&mut controller, source, name, dir)
        .unwrap_or_else(|e| panic!("{name}: {e}\n{source}"))
        .clone();
    let audio = song.render(RATE, song.length_bars(1).min(8)).unwrap();
    assert!(
        audio.iter().all(|s| s.is_finite() && s.abs() <= 1.0),
        "{name}"
    );
    if !song.tracks.is_empty() {
        assert!(audio.iter().any(|s| s.abs() > 0.01), "{name}: no suena");
    }
}

#[test]
fn every_lua_block_in_the_guide_runs() {
    let dir = samples("blocks");
    let mut total = 0;
    for file in [GUIDE, TUTORIAL] {
        for (line, source) in &lua_blocks(&doc(file)) {
            check(source, &format!("{file}:{line}"), &dir);
            total += 1;
        }
    }
    assert!(total >= 20, "solo {total} bloques");
}

/// Sustituye en `song` el párrafo (hasta la línea en blanco) que empieza
/// como el bloque: su primera línea hasta el primer `)`.
fn replace_paragraph(song: &str, block: &str, name: &str) -> String {
    let first = block.lines().next().unwrap();
    let key = &first[..first.find(')').map_or(first.len(), |i| i + 1)];
    let lines: Vec<&str> = song.lines().collect();
    let start = lines
        .iter()
        .position(|l| l.starts_with(key))
        .unwrap_or_else(|| panic!("{name}: no hay ningún párrafo que empiece por {key}\n{song}"));
    let end = lines[start..]
        .iter()
        .position(|l| l.trim().is_empty())
        .map_or(lines.len(), |n| start + n);
    let mut out = String::new();
    for line in &lines[..start] {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(block);
    for line in &lines[end..] {
        out.push_str(line);
        out.push('\n');
    }
    out
}

/// «Tu primera canción»: el script se escribe entero una vez (`lua
/// entero`) y luego crece con párrafos que se añaden al final (`lua
/// añade`) o que sustituyen a uno que ya está (`lua cambia`). Cada estado
/// se carga y suena, y cada `lua entero` posterior es exactamente lo que
/// lleva quien ha seguido los pasos.
#[test]
fn the_tutorial_song_grows_step_by_step() {
    let dir = samples("tutorial");
    let tutorial = doc(TUTORIAL);
    let mut song = String::new();
    let (mut changes, mut wholes) = (0, 0);
    for (line, tag, block) in tagged_blocks(&tutorial) {
        let name = format!("{TUTORIAL}:{line}");
        match tag.as_str() {
            "entero" if song.is_empty() => song = block,
            "entero" => {
                assert_eq!(
                    song, block,
                    "{name}: el script entero no es lo que llevan los pasos"
                );
                wholes += 1;
            }
            "añade" => {
                song.push('\n');
                song.push_str(&block);
                changes += 1;
            }
            "cambia" => {
                song = replace_paragraph(&song, &block, &name);
                changes += 1;
            }
            "" => continue,
            other => panic!("{name}: bloque ```lua {other} desconocido"),
        }
        check(&song, &name, &dir);
    }
    assert!(
        changes >= 21 && wholes >= 4,
        "{changes} cambios, {wholes} scripts enteros"
    );
}

#[test]
fn every_example_runs_and_matches_the_guide() {
    let dir = samples("examples");
    let blocks: Vec<String> = [GUIDE, TUTORIAL]
        .into_iter()
        .flat_map(|file| lua_blocks(&doc(file)))
        .map(|(_, b)| b)
        .collect();
    let mut examples: Vec<PathBuf> = std::fs::read_dir(format!("{ROOT}/scripts/ejemplos"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "lua"))
        .collect();
    examples.sort();
    assert_eq!(examples.len(), 16, "una por receta");
    for path in &examples {
        let source = std::fs::read_to_string(path).unwrap();
        let name = path.file_name().unwrap().to_string_lossy();
        assert!(
            blocks.contains(&source),
            "{name} no coincide con su bloque de la guía"
        );
        check(&source, &name, &dir);
    }
    let link = std::fs::read_link(format!("{ROOT}/scripts/ejemplos/amen.wav")).unwrap();
    assert_eq!(link, Path::new("../../renders/synth_break.wav"));
}

#[test]
fn the_api_section_of_the_guide_comes_from_the_registry() {
    let guide = std::fs::read_to_string(format!("{ROOT}/docs/GUIA.md")).unwrap();
    let updated = script::api::update_guide(&guide).expect("faltan las marcas api:inicio/api:fin");
    assert!(
        updated == guide,
        "docs/GUIA.md no coincide con script/src/api.rs: \
         ejecuta `cargo run -p script --example guia`"
    );
}

/// Ancla de GitHub de un título: minúsculas, sin signos (se quedan letras,
/// cifras, `_` y `-`) y los espacios como guiones.
fn slug(heading: &str) -> String {
    heading
        .trim()
        .to_lowercase()
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(c, '_' | '-' | ' '))
        .map(|c| if c == ' ' { '-' } else { c })
        .collect()
}

/// Las anclas de un Markdown (los títulos fuera de los bloques de código;
/// los repetidos llevan `-1`, `-2`...).
fn anchors(markdown: &str) -> Vec<String> {
    let mut seen: std::collections::HashMap<String, usize> = Default::default();
    let mut fenced = false;
    let mut out = Vec::new();
    for line in markdown.lines() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
        }
        let Some(title) = line.strip_prefix('#').filter(|_| !fenced) else {
            continue;
        };
        let Some(title) = title.trim_start_matches('#').strip_prefix(' ') else {
            continue;
        };
        let base = slug(title);
        let n = seen.entry(base.clone()).or_default();
        out.push(if *n == 0 { base } else { format!("{base}-{n}") });
        *n += 1;
    }
    out
}

/// Los destinos de los enlaces `](destino)` fuera de los bloques de código
/// y del código en línea, con su línea.
fn links(markdown: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut fenced = false;
    for (i, line) in markdown.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        // Sin lo que va entre comillas invertidas.
        let text: String = line
            .split('`')
            .enumerate()
            .filter(|(k, _)| k % 2 == 0)
            .map(|(_, t)| t)
            .collect::<Vec<_>>()
            .join(" ");
        let mut rest = text.as_str();
        while let Some(at) = rest.find("](") {
            rest = &rest[at + 2..];
            if let Some(end) = rest.find(')') {
                out.push((i + 1, rest[..end].to_string()));
                rest = &rest[end..];
            }
        }
    }
    out
}

/// Todo enlace interno de la guía y del tutorial, dentro de cada archivo y
/// entre los dos, apunta a un archivo y a un ancla que existen.
#[test]
fn every_internal_link_points_to_an_existing_anchor() {
    let files = [GUIDE, TUTORIAL];
    let docs: Vec<(&str, String)> = files.iter().map(|f| (*f, doc(f))).collect();
    let mut checked = 0;
    let mut across = (0, 0);
    for (file, text) in &docs {
        for (line, target) in links(text) {
            if target.contains("://") {
                continue;
            }
            let (path, anchor) = match target.split_once('#') {
                Some((path, anchor)) => (path, Some(anchor)),
                None => (target.as_str(), None),
            };
            let other = if path.is_empty() { *file } else { path };
            let Some((_, destination)) = docs.iter().find(|(f, _)| *f == other) else {
                // Otro archivo de docs/ (ESTADO.md...): tiene que existir.
                assert!(
                    Path::new(&format!("{ROOT}/docs/{path}")).exists(),
                    "{file}:{line}: el enlace {target} apunta a un archivo que no existe"
                );
                continue;
            };
            if let Some(anchor) = anchor {
                assert!(
                    anchors(destination).iter().any(|a| a == anchor),
                    "{file}:{line}: el enlace {target} apunta a un ancla que no existe en {other}"
                );
            }
            checked += 1;
            if other != *file {
                if *file == TUTORIAL {
                    across.0 += 1;
                } else {
                    across.1 += 1;
                }
            }
        }
    }
    assert!(checked >= 100, "solo {checked} enlaces comprobados");
    assert!(
        across.0 >= 13 && across.1 >= 13,
        "faltan enlaces entre los dos archivos: {across:?}"
    );
}

/// Cada paso del tutorial tiene su línea de referencia a la guía, y cada
/// paso al que enlaza la guía la enlaza de vuelta (algún enlace de la guía
/// apunta a cada paso).
#[test]
fn every_tutorial_step_links_to_the_guide_and_back() {
    let tutorial = doc(TUTORIAL);
    let guide = doc(GUIDE);
    let steps: Vec<String> = anchors(&tutorial)
        .into_iter()
        .filter(|a| a.starts_with("paso-"))
        .collect();
    assert_eq!(steps.len(), 13);
    let from_guide: Vec<String> = links(&guide)
        .into_iter()
        .filter_map(|(_, t)| t.strip_prefix("PRIMERA-CANCION.md#").map(String::from))
        .collect();
    for step in &steps {
        assert!(from_guide.contains(step), "la guía no enlaza con {step}");
        let heading = tutorial
            .lines()
            .position(|l| l.starts_with("## Paso ") && step.starts_with(&slug(&l[3..9])))
            .unwrap();
        let next = tutorial.lines().skip(heading + 1).find(|l| !l.is_empty());
        assert!(
            next.is_some_and(|l| l.starts_with("> **Referencia:** ") && l.contains("(GUIA.md#")),
            "{step}: falta su línea de referencia a la guía"
        );
    }
}
