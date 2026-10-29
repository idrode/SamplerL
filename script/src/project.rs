//! Proyectos: un archivo de texto (`.sampler`) con el script, la carpeta
//! de los samples y el tempo. Las rutas se guardan relativas al proyecto.
//!
//! ```text
//! # proyecto del sampler
//! script = canción.lua
//! samples = samples
//! bpm = 170
//! etiqueta = amen.wav 8 1 kick
//! corte = amen.wav 8 3 412.5
//! ```
//!
//! Cada `etiqueta` es: sample (como en `sample("...")`), número de cortes
//! (la `n` de `slice(n)`, o `rejillaN` para `slice_grid(N)`), corte
//! (desde 1) y nombre. Cada `corte` es un ajuste manual: el inicio de ese
//! corte en ms, que manda sobre el detector. `partir = amen.wav 8 412.5` añade un
//! corte que empieza ahí y `unir = amen.wav 8 3` quita el corte 3 (el 2 se
//! alarga); se aplican en orden, después de detectar y antes de los ajustes
//! y las etiquetas, que van con la numeración que queda. Cada `mando` es un
//! mando MIDI asignado (MIDI learn; ver [`crate::learn`]):
//! `mando = 1 21 drums lowpass.cutoff 200 8000 log`.

use std::fmt;
use std::path::{Component, Path, PathBuf};

/// Extensión de los archivos de proyecto.
pub const EXTENSION: &str = "sampler";

/// Cómo se cortó un sample: `slice(n)` (detector) o `slice_grid(n)`
/// (rejilla). Las etiquetas, los ajustes y las ediciones del proyecto son
/// de un sample cortado de una forma concreta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cut {
    pub count: usize,
    pub grid: bool,
}

impl Cut {
    pub fn detect(count: usize) -> Cut {
        Cut { count, grid: false }
    }

    pub fn grid(count: usize) -> Cut {
        Cut { count, grid: true }
    }

    /// La llamada que corta así: `slice(8)` o `slice_grid(16)`.
    pub fn call(&self) -> String {
        if self.grid {
            format!("slice_grid({})", self.count)
        } else {
            format!("slice({})", self.count)
        }
    }

    /// Lee `8` o `rejilla16`.
    fn parse(text: &str) -> Result<Cut, String> {
        let (grid, number) = match text.strip_prefix("rejilla") {
            Some(n) => (true, n),
            None => (false, text),
        };
        number
            .parse()
            .ok()
            .filter(|n| (1..=64).contains(n))
            .map(|count| Cut { count, grid })
            .ok_or_else(|| format!("número de cortes no válido: {text} (8, o rejilla16)"))
    }
}

impl fmt::Display for Cut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.grid {
            write!(f, "rejilla{}", self.count)
        } else {
            write!(f, "{}", self.count)
        }
    }
}

/// Edición de los cortes guardada en el proyecto.
#[derive(Debug, Clone, PartialEq)]
pub struct SliceEdit {
    pub sample: String,
    pub cut: Cut,
    pub kind: EditKind,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EditKind {
    /// Un corte nuevo que empieza en esos ms.
    Split(f64),
    /// Quitar el corte con ese número (desde 2): el anterior se alarga.
    Join(usize),
}

impl SliceEdit {
    /// Lee `amen.wav 8 412.5` (partir) o `amen.wav 8 3` (unir).
    fn parse(value: &str, split: bool) -> Result<SliceEdit, String> {
        let usage = if split {
            "se esperaba «partir = sample cortes ms»"
        } else {
            "se esperaba «unir = sample cortes corte»"
        };
        let mut words = value.rsplitn(3, char::is_whitespace);
        let at = words.next().ok_or(usage)?;
        let cut = Cut::parse(words.next().ok_or(usage)?)?;
        let sample = words
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(usage)?;
        let kind = if split {
            EditKind::Split(
                at.parse()
                    .ok()
                    .filter(|m: &f64| m.is_finite() && *m >= 0.0)
                    .ok_or_else(|| format!("posición no válida: {at} (ms desde el principio)"))?,
            )
        } else {
            EditKind::Join(
                at.parse()
                    .ok()
                    .filter(|i| (2..=64).contains(i))
                    .ok_or_else(|| {
                        format!("corte no válido: {at} (de 2 a 64: se une con el anterior)")
                    })?,
            )
        };
        Ok(SliceEdit {
            sample: sample.to_string(),
            cut,
            kind,
        })
    }
}

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
    /// Ajustes manuales de cortes.
    pub positions: Vec<SlicePosition>,
    /// Cortes añadidos y quitados a mano, en orden.
    pub edits: Vec<SliceEdit>,
    /// Mandos MIDI asignados (MIDI learn).
    pub learned: Vec<crate::learn::Learned>,
}

/// Ajuste manual de un corte guardado en el proyecto: dónde empieza, en
/// ms desde el principio del sample. Manda sobre el detector.
#[derive(Debug, Clone, PartialEq)]
pub struct SlicePosition {
    /// El sample tal como se escribe en `sample("...")`.
    pub sample: String,
    /// Cómo se cortó (la `n` de `slice(n)` o de `slice_grid(n)`).
    pub slices: Cut,
    /// Corte, desde 1.
    pub slice: usize,
    /// Inicio en ms.
    pub ms: f64,
}

impl SlicePosition {
    /// Lee `amen.wav 8 3 412.5` (el sample puede tener espacios).
    fn parse(value: &str) -> Result<SlicePosition, String> {
        let usage = "se esperaba «corte = sample cortes corte ms»";
        let mut words = value.rsplitn(4, char::is_whitespace);
        let ms = words.next().ok_or(usage)?;
        let slice = words.next().ok_or(usage)?;
        let slices = words.next().ok_or(usage)?;
        let sample = words
            .next()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or(usage)?;
        let slices = Cut::parse(slices)?;
        let slice: usize = slice
            .parse()
            .ok()
            .filter(|i| (1..=64).contains(i))
            .ok_or_else(|| format!("corte no válido: {slice} (de 1 a 64)"))?;
        let ms: f64 = ms
            .parse()
            .ok()
            .filter(|m: &f64| m.is_finite() && *m >= 0.0)
            .ok_or_else(|| format!("posición no válida: {ms} (ms desde el principio)"))?;
        Ok(SlicePosition {
            sample: sample.to_string(),
            slices,
            slice,
            ms,
        })
    }
}

/// Etiqueta de un corte guardada en el proyecto.
#[derive(Debug, Clone, PartialEq)]
pub struct SliceLabel {
    /// El sample tal como se escribe en `sample("...")`.
    pub sample: String,
    /// Cómo se cortó (la `n` de `slice(n)` o de `slice_grid(n)`): cortado
    /// de otra forma no se aplica.
    pub slices: Cut,
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
        let slices = Cut::parse(slices)?;
        let slice: usize = slice
            .parse()
            .ok()
            .filter(|i| (1..=64).contains(i))
            .ok_or_else(|| format!("corte no válido: {slice} (de 1 a 64)"))?;
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
        let mut positions: Vec<SlicePosition> = Vec::new();
        let mut edits: Vec<SliceEdit> = Vec::new();
        let mut learned = Vec::new();
        // Línea de cada etiqueta y ajuste, para comprobar al final que su
        // corte existe (contando los que añade `partir`).
        let mut numbered: Vec<(usize, String, Cut, usize)> = Vec::new();
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
                    numbered.push((i + 1, label.sample.clone(), label.slices, label.slice));
                    labels.push(label);
                }
                "corte" => {
                    let position = SlicePosition::parse(value).map_err(|e| err(i + 1, e))?;
                    if positions.iter().any(|p| {
                        p.sample == position.sample
                            && p.slices == position.slices
                            && p.slice == position.slice
                    }) {
                        return Err(err(
                            i + 1,
                            format!("el corte {} ya tiene un ajuste", position.slice),
                        ));
                    }
                    numbered.push((
                        i + 1,
                        position.sample.clone(),
                        position.slices,
                        position.slice,
                    ));
                    positions.push(position);
                }
                "partir" | "unir" => {
                    edits.push(SliceEdit::parse(value, key == "partir").map_err(|e| err(i + 1, e))?)
                }
                "mando" => {
                    learned.push(crate::learn::Learned::parse(value).map_err(|e| err(i + 1, e))?)
                }
                other => {
                    return Err(err(
                        i + 1,
                        format!(
                            "clave desconocida «{other}» (admite: script, samples, bpm, etiqueta, \
                             corte, partir, unir, mando)"
                        ),
                    ));
                }
            }
        }
        for (line, sample, cut, slice) in numbered {
            let splits = edits
                .iter()
                .filter(|e| e.sample == sample && e.cut == cut)
                .filter(|e| matches!(e.kind, EditKind::Split(_)))
                .count();
            let max = cut.count + splits;
            if slice > max {
                return Err(err(
                    line,
                    format!("corte no válido: {slice} (de 1 a {max})"),
                ));
            }
        }
        let script = script
            .ok_or_else(|| ProjectError(format!("{}: falta «script = ...»", path.display())))?;
        Ok(Project {
            script,
            samples,
            bpm,
            labels,
            positions,
            edits,
            learned,
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
        // Las ediciones van antes: se aplican antes que los ajustes.
        for e in &self.edits {
            match e.kind {
                EditKind::Split(ms) => {
                    text.push_str(&format!("partir = {} {} {ms}\n", e.sample, e.cut))
                }
                EditKind::Join(slice) => {
                    text.push_str(&format!("unir = {} {} {slice}\n", e.sample, e.cut))
                }
            }
        }
        for p in &self.positions {
            text.push_str(&format!(
                "corte = {} {} {} {}\n",
                p.sample, p.slices, p.slice, p.ms
            ));
        }
        for l in &self.learned {
            text.push_str(&format!("mando = {}\n", l.to_line()));
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
