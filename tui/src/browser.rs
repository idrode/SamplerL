//! Explorador de samples, sin terminal: la lista de WAV de la carpeta de
//! muestras (y sus subcarpetas) y el filtro por nombre. La preescucha y la
//! inserción en el editor las hace la aplicación.

use std::path::Path;

/// Subcarpetas que se recorren como mucho.
const MAX_DEPTH: usize = 4;
/// Archivos que se listan como mucho (una carpeta enorme no bloquea).
const MAX_FILES: usize = 2000;

/// WAV de `dir` y sus subcarpetas, con la ruta relativa (con `/`), en
/// orden alfabético. Las carpetas que empiezan por `.` no se miran.
pub fn list(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    walk(dir, "", 0, &mut out);
    out.sort();
    out
}

fn walk(dir: &Path, prefix: &str, depth: usize, out: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = entries.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if out.len() >= MAX_FILES {
            return;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            if depth < MAX_DEPTH {
                walk(&path, &format!("{prefix}{name}/"), depth + 1, out);
            }
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
        {
            out.push(format!("{prefix}{name}"));
        }
    }
}

/// Los que contienen cada palabra de la búsqueda (sin distinguir
/// mayúsculas ni tildes).
pub fn filter<'a>(files: &'a [String], query: &str) -> Vec<&'a String> {
    let words: Vec<String> = query.split_whitespace().map(script::blocks::fold).collect();
    files
        .iter()
        .filter(|f| {
            let name = script::blocks::fold(f);
            words.iter().all(|w| name.contains(w.as_str()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_wavs_recursively_and_filters_them() {
        let dir = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/tmp/browser_list"
        ));
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir.join("breaks/viejos")).unwrap();
        std::fs::create_dir_all(dir.join(".oculta")).unwrap();
        for f in [
            "amen.wav",
            "Think.WAV",
            "notas.txt",
            "breaks/apache.wav",
            "breaks/viejos/funky drummer.wav",
            ".oculta/x.wav",
        ] {
            std::fs::write(dir.join(f), b"").unwrap();
        }
        let files = list(dir);
        assert_eq!(
            files,
            [
                "Think.WAV",
                "amen.wav",
                "breaks/apache.wav",
                "breaks/viejos/funky drummer.wav"
            ]
        );
        let names = |q: &str| -> Vec<String> { filter(&files, q).into_iter().cloned().collect() };
        assert_eq!(names("think"), ["Think.WAV"]);
        assert_eq!(names("breaks funky"), ["breaks/viejos/funky drummer.wav"]);
        assert_eq!(names("").len(), 4);
        assert!(names("zzz").is_empty());
    }
}
