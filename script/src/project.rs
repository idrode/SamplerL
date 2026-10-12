//! Proyectos: un archivo de texto (`.sampler`) con el script, la carpeta
//! de los samples y el tempo. Las rutas se guardan relativas al proyecto.
//!
//! ```text
//! # proyecto del sampler
//! script = canción.lua
//! samples = samples
//! bpm = 170
//! etiqueta = amen.wav 8 1 kick
//! ```
//!
//! Cada `etiqueta` es: sample (como en `sample("...")`), número de cortes
//! (la `n` de `slice(n)`), corte (desde 1) y nombre.

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
    /// Etiquetas de corte.
    pub labels: Vec<SliceLabel>,
}

/// Etiqueta de un corte guardada en el proyecto.
#[derive(Debug, Clone, PartialEq)]
pub struct SliceLabel {
    /// El sample tal como se escribe en `sample("...")`.
    pub sample: String,
    /// La `n` de `slice(n)`: con otro número de cortes no se aplica.
    pub slices: usize,
    /// Corte, desde 1.
    pub slice: usize,
    pub name: String,
}

impl SliceLabel {
    /// Lee `amen.wav 8 1 kick` (el sample puede tener espacios).
    fn parse(value: &str) -> Result<SliceLabel, String> {
        let usage = "se esperaba «etiqueta = sample cortes corte nombre»";
        let mut words = value.rsplitn(4, char::is_whitespace);
        let name = words.next().ok_or(usage)?;
        let slice = words.next().ok_or(usage)?;
        let slices = words.next().ok_or(usage)?;
        let sample = words
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(usage)?;
        let slices: usize = slices
            .parse()
            .ok()
            .filter(|n| (1..=64).contains(n))
            .ok_or_else(|| format!("número de cortes no válido: {slices}"))?;
        let slice: usize = slice
            .parse()
            .ok()
            .filter(|i| (1..=slices).contains(i))
            .ok_or_else(|| format!("corte no válido: {slice} (de 1 a {slices})"))?;
        engine::notation::check_label(name)?;
        Ok(SliceLabel {
            sample: sample.to_string(),
            slices,
            slice,
            name: name.to_string(),
        })
    }
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
        let mut labels: Vec<SliceLabel> = Vec::new();
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
                "etiqueta" => {
                    let label = SliceLabel::parse(value).map_err(|e| err(i + 1, e))?;
                    let same =
                        |l: &&SliceLabel| l.sample == label.sample && l.slices == label.slices;
                    if let Some(other) = labels.iter().filter(same).find(|l| l.slice == label.slice)
                    {
                        return Err(err(
                            i + 1,
                            format!(
                                "el corte {} ya tiene la etiqueta «{}»",
                                label.slice, other.name
                            ),
                        ));
                    }
                    if let Some(other) = labels.iter().filter(same).find(|l| l.name == label.name) {
                        return Err(err(
                            i + 1,
                            format!(
                                "la etiqueta «{}» ya es del corte {}",
                                label.name, other.slice
                            ),
                        ));
                    }
                    labels.push(label);
                }
                other => {
                    return Err(err(
                        i + 1,
                        format!(
                            "clave desconocida «{other}» (admite: script, samples, bpm, etiqueta)"
                        ),
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
            labels,
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
        for l in &self.labels {
            text.push_str(&format!(
                "etiqueta = {} {} {} {}\n",
                l.sample, l.slices, l.slice, l.name
            ));
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
