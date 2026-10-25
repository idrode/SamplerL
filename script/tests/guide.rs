//! Cada bloque ```lua de docs/GUIA.md y cada receta de scripts/ejemplos/
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
    let guide = std::fs::read_to_string(format!("{ROOT}/docs/GUIA.md")).unwrap();
    let blocks = lua_blocks(&guide);
    assert!(blocks.len() >= 20, "solo {} bloques", blocks.len());
    for (line, source) in &blocks {
        check(source, &format!("GUIA.md:{line}"), &dir);
    }
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
    let guide = std::fs::read_to_string(format!("{ROOT}/docs/GUIA.md")).unwrap();
    let start = guide
        .find("\n## 8. Tu primera canción")
        .expect("falta la sección del tutorial")
        + 1;
    let end = guide[start..]
        .find("\n## ")
        .map_or(guide.len(), |e| start + e);
    let offset = guide[..start].lines().count();
    let mut song = String::new();
    let (mut changes, mut wholes) = (0, 0);
    for (line, tag, block) in tagged_blocks(&guide[start..end]) {
        let name = format!("GUIA.md:{} (tutorial)", line + offset);
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
    let guide = std::fs::read_to_string(format!("{ROOT}/docs/GUIA.md")).unwrap();
    let blocks: Vec<String> = lua_blocks(&guide).into_iter().map(|(_, b)| b).collect();
    let mut examples: Vec<PathBuf> = std::fs::read_dir(format!("{ROOT}/scripts/ejemplos"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "lua"))
        .collect();
    examples.sort();
    assert_eq!(examples.len(), 13, "una por receta");
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
