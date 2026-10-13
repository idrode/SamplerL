//! Biblioteca de bloques: formato, carpetas global y del proyecto, guardar,
//! actualizar y borrar, y que un bloque roto no afecta a nada.

use std::path::{Path, PathBuf};

use engine::{new_engine, offline};
use script::ScriptHost;
use script::blocks::{self, Library, SaveError, Scope};

const RATE: u32 = 48_000;

fn dir(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("blocks_{test}"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

const GHOST: &str = "-- nombre: ghost_suave\n\
                     -- descripción: Ghost notes en contratiempos\n\
                     -- etiquetas: ghost, #hats hats\n\
                     \n\
                     track(\"${ghost}\", amen:slice_at(${3}))\n  \
                     :steps(\". x . x  . x . .  . x . x  . . x .\")\n";

#[test]
fn parses_the_header_and_the_body() {
    let (name, description, tags, body) = blocks::parse(GHOST);
    assert_eq!(name.as_deref(), Some("ghost_suave"));
    assert_eq!(description, "Ghost notes en contratiempos");
    assert_eq!(tags, ["ghost", "hats"]);
    assert!(body.starts_with("track(\"${ghost}\""), "{body}");
    assert!(body.ends_with(".\")\n"), "{body:?}");

    // Sin cabecera: todo es cuerpo, también un comentario normal.
    let (name, _, tags, body) = blocks::parse("-- un comentario\nbpm(170)");
    assert_eq!((name, tags.len()), (None, 0));
    assert_eq!(body, "-- un comentario\nbpm(170)\n");
    // Claves sin tilde y en inglés.
    let (name, description, tags, _) =
        blocks::parse("-- name: x\n-- descripcion: d\n-- tags: a,b\nbpm(1)\n");
    assert_eq!(
        (name.as_deref(), description.as_str(), tags.len()),
        (Some("x"), "d", 2)
    );
}

#[test]
fn missing_folders_are_created() {
    let dir = dir("create");
    let global = dir.join("config/sampler/blocks");
    let project = dir.join("proyecto/blocks");
    let library = Library::open(Some(global.clone()), Some(project.clone()));
    assert!(library.blocks.is_empty());
    assert!(library.warnings.is_empty(), "{:?}", library.warnings);
    // La global se crea al leer; la del proyecto, al guardar en ella.
    assert!(global.is_dir() && !project.is_dir());
    let mut library = Library::open(None, Some(project.clone()));
    assert!(library.warnings.is_empty());
    library
        .save(Scope::Project, "uno", "", &[], "bpm(170)", false)
        .unwrap();
    assert!(project.join("uno.lua").is_file());
    // Sin carpeta global (sin $HOME), guardar en ella da un error claro.
    let global_save = library.save(Scope::Global, "dos", "", &[], "x()", false);
    assert!(matches!(global_save, Err(SaveError::Invalid(_))));
}

#[test]
fn saves_updates_and_deletes() {
    let dir = dir("save");
    let mut library = Library::open(Some(dir.join("global")), Some(dir.join("blocks")));
    let tags = vec!["ghost".to_string(), "hats".to_string()];
    let block = library
        .save(
            Scope::Global,
            "ghost_suave",
            "Ghost notes",
            &tags,
            "track(\"g\", amen)\n\n\n",
            false,
        )
        .unwrap();
    assert_eq!(block.path, dir.join("global/ghost_suave.lua"));
    let file = std::fs::read_to_string(&block.path).unwrap();
    assert_eq!(
        file,
        "-- nombre: ghost_suave\n-- descripción: Ghost notes\n-- etiquetas: ghost, hats\n\
         \ntrack(\"g\", amen)\n"
    );
    assert_eq!(library.get("ghost_suave"), Some(&block));

    // Otra vez con el mismo nombre: no sin permiso.
    let again = library.save(Scope::Global, "ghost_suave", "", &[], "x()", false);
    assert!(matches!(again, Err(SaveError::Exists(_))), "{again:?}");
    let replaced = library
        .save(Scope::Global, "ghost_suave", "", &[], "x()", true)
        .unwrap();
    assert_eq!(replaced.body, "x()\n");
    assert!(replaced.tags.is_empty());

    // Errores.
    for (name, body) in [("con espacio", "x()"), ("1a", "x()"), ("vacio", "  \n")] {
        let result = library.save(Scope::Global, name, "", &[], body, false);
        assert!(matches!(result, Err(SaveError::Invalid(_))), "{name}");
    }

    // Actualizar conserva la cabecera y el archivo.
    library
        .save(Scope::Global, "ghost_suave", "Ghost", &tags, "a()", true)
        .unwrap();
    let updated = library.update("ghost_suave", "b()\n").unwrap();
    assert_eq!(
        (updated.description.as_str(), &updated.tags),
        ("Ghost", &tags)
    );
    assert_eq!(library.get("ghost_suave").unwrap().body, "b()\n");
    assert!(library.update("nada", "b()").is_err());

    // Borrar.
    library.delete("ghost_suave").unwrap();
    assert!(library.get("ghost_suave").is_none());
    assert!(!dir.join("global/ghost_suave.lua").exists());
    assert!(library.delete("ghost_suave").is_err());
}

#[test]
fn project_blocks_hide_global_ones_with_the_same_name() {
    let dir = dir("shadow");
    let mut library = Library::open(Some(dir.join("global")), Some(dir.join("blocks")));
    library
        .save(Scope::Global, "fill", "global", &[], "global()", false)
        .unwrap();
    library
        .save(Scope::Global, "solo_global", "", &[], "g()", false)
        .unwrap();
    library
        .save(
            Scope::Project,
            "fill",
            "del proyecto",
            &[],
            "proyecto()",
            false,
        )
        .unwrap();
    let names: Vec<(&str, Scope)> = library
        .blocks
        .iter()
        .map(|b| (b.name.as_str(), b.scope))
        .collect();
    assert_eq!(
        names,
        [("fill", Scope::Project), ("solo_global", Scope::Global)]
    );
    // Al borrar el del proyecto, vuelve el global.
    library.delete("fill").unwrap();
    assert_eq!(library.get("fill").unwrap().scope, Scope::Global);
    assert!(library.warnings.is_empty(), "{:?}", library.warnings);
}

#[test]
fn broken_files_are_reported_and_the_rest_still_load() {
    let dir = dir("broken");
    let blocks_dir = dir.join("blocks");
    std::fs::create_dir_all(&blocks_dir).unwrap();
    std::fs::write(blocks_dir.join("bueno.lua"), "bpm(170)\n").unwrap();
    std::fs::write(blocks_dir.join("mal nombre.lua"), "bpm(170)\n").unwrap();
    std::fs::write(blocks_dir.join("vacio.lua"), "-- nombre: vacio\n\n").unwrap();
    std::fs::write(blocks_dir.join("binario.lua"), [0xff, 0xfe, 0x00]).unwrap();
    std::fs::write(blocks_dir.join("repetido.lua"), "-- nombre: bueno\nx()\n").unwrap();
    std::fs::write(blocks_dir.join("notas.txt"), "no es un bloque").unwrap();
    // Lua no válido y un campo sin cerrar: es texto, se lee igual.
    std::fs::write(
        blocks_dir.join("roto.lua"),
        "-- nombre: roto\ntrack(\"${pista\", :steps(\n",
    )
    .unwrap();
    let library = Library::open(None, Some(blocks_dir));
    let names: Vec<&str> = library.blocks.iter().map(|b| b.name.as_str()).collect();
    assert_eq!(names, ["bueno", "roto"]);
    assert_eq!(library.warnings.len(), 4, "{:?}", library.warnings);
    assert_eq!(
        library.get("roto").unwrap().preview(),
        "track(\"${pista\", :steps(\n"
    );
}

#[test]
fn search_looks_at_name_tags_and_description_without_accents() {
    let dir = dir("search");
    let mut library = Library::open(None, Some(dir.join("blocks")));
    let tags = |t: &str| blocks::parse_tags(t);
    library
        .save(
            Scope::Project,
            "ghost",
            "Golpes suaves",
            &tags("hats"),
            "a()",
            false,
        )
        .unwrap();
    library
        .save(
            Scope::Project,
            "intro",
            "Pista filtrada más baja",
            &tags("intro, filtro"),
            "b()",
            false,
        )
        .unwrap();
    let found = |q: &str| -> Vec<String> {
        library
            .filter(q)
            .into_iter()
            .map(|b| b.name.clone())
            .collect()
    };
    assert_eq!(found(""), ["ghost", "intro"]);
    assert_eq!(found("HATS"), ["ghost"]);
    assert_eq!(found("#filtro"), ["intro"]);
    assert_eq!(found("mas baja"), ["intro"]);
    assert_eq!(found("suaves gho"), ["ghost"]);
    assert!(found("ghost intro").is_empty());
}

/// Los bloques nunca se cargan con el script: una carpeta `blocks/` con
/// Lua roto junto al script no cambia nada.
#[test]
fn the_blocks_folder_is_never_loaded_with_the_script() {
    let dir = dir("never_loaded");
    let mut mono = vec![0.0f32; 6000];
    mono[..800].fill(0.5);
    offline::write_wav(dir.join("break.wav"), &mono, 1, RATE).unwrap();
    let song = "local amen = sample(\"break.wav\"):slice(1)\n\
                track(\"drums\", amen):steps(\"1 . . .  1 . . .  1 . . .  1 . . .\")\n";
    let render = || {
        let (mut controller, _engine) = new_engine(RATE);
        let mut host = ScriptHost::new(RATE);
        let song = host.load(&mut controller, song, "song.lua", &dir).unwrap();
        song.render(RATE, 1).unwrap()
    };
    let before = render();
    std::fs::create_dir_all(dir.join("blocks")).unwrap();
    std::fs::write(dir.join("blocks/roto.lua"), "error(\"boom\")\ntrack(").unwrap();
    assert_eq!(render(), before);
}
