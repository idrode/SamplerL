//! Qué abre una ventana aparte (editor, dispositivos, piano roll): la
//! principal la lanza con la ruta completa y canónica de su script o
//! proyecto, porque la terminal nueva puede arrancar en otra carpeta
//! (`kitty --directory ~`, un servidor de terminal, o a mano); y la
//! ventana se niega a abrir lo que no existe en vez de poner la plantilla.

use std::path::{Path, PathBuf};

/// La ruta completa y canónica de `path` (sin `..` ni enlaces). Si el
/// archivo aún no existe, la de su carpeta y su nombre; si tampoco está la
/// carpeta, la absoluta respecto a la carpeta actual.
pub fn full_path(path: &Path) -> PathBuf {
    if let Ok(full) = std::fs::canonicalize(path) {
        return full;
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    if let (Ok(dir), Some(name)) = (std::fs::canonicalize(parent), path.file_name()) {
        return dir.join(name);
    }
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Lo que abre una ventana aparte con `path` (un script o un proyecto):
/// su ruta canónica y la del script. Falla con el motivo si el archivo no
/// existe (y, con `script_needed`, si no existe el script del proyecto),
/// diciendo desde qué carpeta se buscó si la ruta era relativa.
pub fn window_target(path: &Path, script_needed: bool) -> Result<(PathBuf, PathBuf), String> {
    let missing = |what: &Path, e: std::io::Error| {
        let from = if what.is_relative() {
            std::env::current_dir()
                .map(|d| format!(" (buscado desde {})", d.display()))
                .unwrap_or_default()
        } else {
            String::new()
        };
        format!(
            "no existe «{}»{from}: {e}. La ventana aparte no crea un archivo nuevo ni abre la \
             plantilla; ábrela desde la ventana principal (menú Ventanas), que le pasa la ruta \
             completa",
            what.display()
        )
    };
    let full = std::fs::canonicalize(path).map_err(|e| missing(path, e))?;
    let is_project = full
        .extension()
        .is_some_and(|e| e == script::project::EXTENSION);
    if !is_project {
        return Ok((full.clone(), full));
    }
    let project = script::Project::load(&full).map_err(|e| e.to_string())?;
    if !script_needed {
        return Ok((full, full_path(&project.script)));
    }
    let script = std::fs::canonicalize(&project.script).map_err(|e| {
        format!(
            "{} (el script del proyecto {})",
            missing(&project.script, e),
            full.display()
        )
    })?;
    Ok((full, script))
}

/// Opciones con que las terminales conocidas fijan su carpeta de trabajo.
const DIRECTORY_FLAGS: [&str; 5] = [
    "--directory",
    "--working-directory",
    "--workdir",
    "--cwd",
    "--dir",
];

/// Si el comando de la terminal (`[editor] terminal` y compañía) fija una
/// carpeta que no existe, el aviso: la terminal fallaría o abriría otra
/// carpeta sin decir nada.
pub fn terminal_problem(words: &[String]) -> Option<String> {
    let mut words = words.iter();
    while let Some(word) = words.next() {
        let dir = if let Some((flag, dir)) = word.split_once('=') {
            if !DIRECTORY_FLAGS.contains(&flag) {
                continue;
            }
            dir.to_string()
        } else if DIRECTORY_FLAGS.contains(&word.as_str()) {
            match words.next() {
                Some(dir) => dir.clone(),
                None => continue,
            }
        } else {
            continue;
        };
        let expanded = match dir.strip_prefix("~/").or((dir == "~").then_some("")) {
            Some(rest) => match std::env::var_os("HOME") {
                Some(home) => Path::new(&home).join(rest),
                None => PathBuf::from(&dir),
            },
            None => PathBuf::from(&dir),
        };
        if !expanded.is_dir() {
            return Some(format!(
                "la carpeta de la terminal no existe: {dir} (en {} de config.toml); \
                 corrígela o quítala, la ventana no la necesita: va con la ruta completa",
                word.split('=').next().unwrap_or(word)
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_file_takes_its_folder_and_name() {
        let dir = std::env::temp_dir().canonicalize().unwrap();
        let path = dir.join("no-existe-todavia.lua");
        assert_eq!(full_path(&path), path);
        assert!(full_path(Path::new("no-existe.lua")).is_absolute());
    }

    #[test]
    fn terminal_directories_are_checked() {
        let words = |w: &[&str]| w.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(terminal_problem(&words(&["kitty", "-e"])), None);
        assert_eq!(
            terminal_problem(&words(&["kitty", "--directory", "/", "-e"])),
            None
        );
        assert!(terminal_problem(&words(&["foot", "--working-directory=/no/hay"])).is_some());
        let problem = terminal_problem(&words(&["kitty", "--directory", "/no/hay", "-e"]));
        assert!(problem.unwrap().contains("/no/hay"));
    }
}
