//! Proyectos: un archivo de texto (`.sampler`) con el script, la carpeta
//! de los samples y el tempo. Las rutas se guardan relativas al proyecto.
//!
//! ```text
//! # proyecto del sampler
//! script = canción.lua
//! samples = samples
//! bpm = 170
//! ```

use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Extensión de los archivos de proyecto.
pub const EXTENSION: &str = "sampler";

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// Script (ruta absoluta o relativa a la carpeta actual).
    pub script: PathBuf,
    /// Carpeta de los samples; `None`: la del script.
    pub samples: Option<PathBuf>,
    /// Tempo si el script no llama a `bpm()`.
    pub bpm: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectError(pub String);

impl fmt::Display for ProjectError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ProjectError {}

impl Project {
    /// Lee un proyecto. Las rutas se resuelven respecto a su carpeta.
    pub fn load(path: &Path) -> Result<Project, ProjectError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| ProjectError(format!("{}: {e}", path.display())))?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let err = |line: usize, message: String| {
            ProjectError(format!("{}:{line}: {message}", path.display()))
        };

        let (mut script, mut samples, mut bpm) = (None, None, None);
        for (i, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                return Err(err(i + 1, format!("se esperaba «clave = valor»: {line}")));
            };
            let (key, value) = (key.trim(), value.trim());
            match key {
                "script" => script = Some(dir.join(value)),
                "samples" => samples = Some(dir.join(value)),
                "bpm" => {
                    let value: f64 = value
                        .parse()
                        .ok()
                        .filter(|&b| engine::valid_bpm(b))
                        .ok_or_else(|| err(i + 1, format!("bpm no válido: {value}")))?;
                    bpm = Some(value);
                }
                other => {
                    return Err(err(
                        i + 1,
                        format!("clave desconocida «{other}» (admite: script, samples, bpm)"),
                    ));
                }
            }
        }
        let script = script
            .ok_or_else(|| ProjectError(format!("{}: falta «script = ...»", path.display())))?;
        Ok(Project {
            script,
            samples,
            bpm,
        })
    }

    /// Guarda el proyecto con las rutas relativas a su carpeta (si se puede).
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut text = String::from("# proyecto del sampler\n");
        text.push_str(&format!(
            "script = {}\n",
            relative(&self.script, dir).display()
        ));
        if let Some(samples) = &self.samples {
            text.push_str(&format!("samples = {}\n", relative(samples, dir).display()));
        }
        if let Some(bpm) = self.bpm {
            text.push_str(&format!("bpm = {bpm}\n"));
        }
        std::fs::write(path, text)
    }
}

/// `path` relativa a `base` si las dos se pueden comparar; si no, tal cual.
fn relative(path: &Path, base: &Path) -> PathBuf {
    let absolute = |p: &Path| std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
    let (path, base) = (absolute(path), absolute(base));
    fn normal(p: &Path) -> Vec<Component<'_>> {
        p.components().filter(|c| *c != Component::CurDir).collect()
    }
    let (p, b) = (normal(&path), normal(&base));
    let common = p.iter().zip(&b).take_while(|(x, y)| x == y).count();
    if common == 0 {
        return path;
    }
    let mut out = PathBuf::new();
    for _ in common..b.len() {
        out.push("..");
    }
    for c in &p[common..] {
        out.push(c.as_os_str());
    }
    if out.as_os_str().is_empty() {
        out.push(".");
    }
    out
}
