//! Cada bloque ```lua de docs/GUIA.md y cada receta de scripts/ejemplos/
//! se carga sin errores contra el break sintético (como `amen.wav`), y la
//! sección de la API coincide con el registro (`script::api`).

#[path = "../../engine/tests/support/synth_break.rs"]
mod synth;

use std::path::{Path, PathBuf};

use engine::{new_engine, offline};
use script::ScriptHost;

const RATE: u32 = 48_000;
const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/..");

/// Carpeta con el break sintético como `amen.wav`. Una por test: si dos
/// tests escriben el mismo archivo a la vez, uno puede leerlo a medias.
fn samples(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("guide_samples_{test}"));
    std::fs::create_dir_all(&dir).unwrap();
    let (audio, _) = synth::synth_break();
    offline::write_wav(dir.join("amen.wav"), &audio, 1, synth::RATE).unwrap();
    dir
}

/// Bloques ```lua con la línea donde empiezan.
fn lua_blocks(markdown: &str) -> Vec<(usize, String)> {
    let mut blocks = Vec::new();
    let mut current: Option<(usize, String)> = None;
    for (i, line) in markdown.lines().enumerate() {
        match &mut current {
            None if line.trim() == "```lua" => current = Some((i + 2, String::new())),
            Some(_) if line.trim() == "```" => blocks.push(current.take().unwrap()),
            Some((_, text)) => {
                text.push_str(line);
                text.push('\n');
            }
            None => {}
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
    assert_eq!(examples.len(), 8, "una por receta");
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
