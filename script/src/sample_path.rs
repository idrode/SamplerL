//! Dónde está el archivo de `sample("amen.wav")`. Se busca, en orden:
//!
//! 1. la carpeta de los samples de antes: la de `samples =` del `.sampler`
//!    o la orden `muestras` si la hay; si no, la del script (así un script
//!    de antes carga exactamente los mismos samples que antes);
//! 2. la del script, si la de antes era otra;
//! 3. `samples/` junto al script (la carpeta de samples del proyecto, a
//!    donde copia el explorador);
//! 4. la carpeta de samples global (`[samples] carpeta` en `config.toml`).
//!
//! Gana la primera que lo tenga. Si otra también tiene un archivo con ese
//! nombre y es distinto, se avisa (el nombre es ambiguo). Una ruta
//! absoluta es solo ella. Si no está en ninguna, se dice dónde se buscó.

use std::io::Read;
use std::path::{Path, PathBuf};

/// Carpeta de samples de un proyecto, junto a su script.
pub const PROJECT_SAMPLES: &str = "samples";

/// Hasta qué tamaño se comparan dos archivos byte a byte para saber si un
/// nombre ambiguo lo es de verdad (más grandes, se dan por distintos).
const COMPARE_LIMIT: u64 = 64 << 20;

/// Las carpetas donde se busca, en orden (ver el módulo).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SampleDirs {
    /// La de `samples =` del proyecto (o `muestras`), si la hay.
    pub declared: Option<PathBuf>,
    /// La del script.
    pub script: PathBuf,
    /// La global de la configuración.
    pub global: Option<PathBuf>,
}

impl SampleDirs {
    /// Las carpetas en el orden en que se mira, sin repetir.
    pub fn order(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        let candidates = self
            .declared
            .iter()
            .cloned()
            .chain([self.script.clone(), self.script.join(PROJECT_SAMPLES)])
            .chain(self.global.iter().cloned());
        for dir in candidates {
            if !out.iter().any(|d| same_dir(d, &dir)) {
                out.push(dir);
            }
        }
        out
    }

    /// Dónde está `name`.
    pub fn resolve(&self, name: &str) -> Resolved {
        let given = Path::new(name);
        if given.is_absolute() {
            return if given.is_file() {
                Resolved::Found {
                    path: given.to_path_buf(),
                    others: Vec::new(),
                }
            } else {
                Resolved::Missing {
                    searched: vec![given.to_path_buf()],
                }
            };
        }
        let candidates: Vec<PathBuf> = self.order().iter().map(|d| d.join(given)).collect();
        let Some(first) = candidates.iter().position(|p| p.is_file()) else {
            return Resolved::Missing {
                searched: candidates,
            };
        };
        let path = candidates[first].clone();
        let others = candidates[first + 1..]
            .iter()
            .filter(|p| p.is_file() && !same_file(&path, p))
            .cloned()
            .collect();
        Resolved::Found { path, others }
    }
}

/// Dónde se encontró un sample.
#[derive(Debug, Clone, PartialEq)]
pub enum Resolved {
    /// En `path`; `others`, otros archivos distintos con el mismo nombre
    /// en las carpetas de después (el nombre es ambiguo).
    Found { path: PathBuf, others: Vec<PathBuf> },
    /// En ninguna: dónde se buscó, en orden.
    Missing { searched: Vec<PathBuf> },
}

impl Resolved {
    /// El aviso que merece (`None`: encontrado sin dudas).
    pub fn warning(&self, name: &str) -> Option<String> {
        match self {
            Resolved::Found { others, .. } if others.is_empty() => None,
            Resolved::Found { path, others } => Some(format!(
                "{name}: hay otro distinto en {}; se usa el de {}",
                list(others),
                path.display()
            )),
            Resolved::Missing { searched } => Some(format!(
                "{name}: no está (buscado en {}); suena en silencio hasta que esté",
                list(searched)
            )),
        }
    }
}

fn list(paths: &[PathBuf]) -> String {
    paths
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(", ")
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (std::fs::canonicalize(a), std::fs::canonicalize(b)),
            (Ok(x), Ok(y)) if x == y
        )
}

/// Dos archivos son el mismo o tienen el mismo contenido.
pub fn same_file(a: &Path, b: &Path) -> bool {
    if same_dir(a, b) {
        return true;
    }
    same_content(a, b).unwrap_or(false)
}

/// Mismo contenido byte a byte (hasta [`COMPARE_LIMIT`]; más grandes, no).
pub fn same_content(a: &Path, b: &Path) -> std::io::Result<bool> {
    let (ma, mb) = (std::fs::metadata(a)?, std::fs::metadata(b)?);
    if ma.len() != mb.len() || ma.len() > COMPARE_LIMIT {
        return Ok(false);
    }
    let (mut fa, mut fb) = (std::fs::File::open(a)?, std::fs::File::open(b)?);
    let (mut ba, mut bb) = (vec![0u8; 64 << 10], vec![0u8; 64 << 10]);
    loop {
        let n = fa.read(&mut ba)?;
        if n == 0 {
            return Ok(true);
        }
        fb.read_exact(&mut bb[..n])?;
        if ba[..n] != bb[..n] {
            return Ok(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(test: &str) -> PathBuf {
        let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../target/tmp"))
            .join(format!("sample_path_{test}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("song/samples")).unwrap();
        std::fs::create_dir_all(dir.join("global")).unwrap();
        std::fs::create_dir_all(dir.join("declared")).unwrap();
        dir
    }

    #[test]
    fn the_order_is_declared_script_project_samples_and_global() {
        let d = dir("order");
        let dirs = SampleDirs {
            declared: Some(d.join("declared")),
            script: d.join("song"),
            global: Some(d.join("global")),
        };
        std::fs::write(d.join("global/a.wav"), "g").unwrap();
        let found = |name: &str| match dirs.resolve(name) {
            Resolved::Found { path, .. } => path,
            other => panic!("{other:?}"),
        };
        assert_eq!(found("a.wav"), d.join("global/a.wav"));
        std::fs::write(d.join("song/samples/a.wav"), "s").unwrap();
        assert_eq!(found("a.wav"), d.join("song/samples/a.wav"));
        std::fs::write(d.join("song/a.wav"), "x").unwrap();
        assert_eq!(found("a.wav"), d.join("song/a.wav"));
        std::fs::write(d.join("declared/a.wav"), "d").unwrap();
        assert_eq!(found("a.wav"), d.join("declared/a.wav"));
        // Distintos: ambiguo, con todos los demás en el aviso.
        let resolved = dirs.resolve("a.wav");
        let warning = resolved.warning("a.wav").unwrap();
        assert!(warning.contains("global/a.wav"), "{warning}");
        assert!(warning.contains("se usa el de"), "{warning}");
    }

    #[test]
    fn identical_copies_are_not_ambiguous_and_missing_says_where() {
        let d = dir("same");
        let dirs = SampleDirs {
            declared: None,
            script: d.join("song"),
            global: Some(d.join("global")),
        };
        std::fs::write(d.join("song/samples/b.wav"), "igual").unwrap();
        std::fs::write(d.join("global/b.wav"), "igual").unwrap();
        assert_eq!(dirs.resolve("b.wav").warning("b.wav"), None);
        let missing = dirs.resolve("c.wav");
        let warning = missing.warning("c.wav").unwrap();
        assert!(warning.contains("no está"), "{warning}");
        assert!(warning.contains(&d.join("song/samples/c.wav").display().to_string()));
        assert!(warning.contains(&d.join("global/c.wav").display().to_string()));
        // Sin carpeta declarada ni global: la del script y su samples/.
        assert_eq!(
            SampleDirs {
                script: d.join("song"),
                ..Default::default()
            }
            .order(),
            [d.join("song"), d.join("song/samples")]
        );
    }
}
