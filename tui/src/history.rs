//! Historial de versiones del script: una copia con marca de tiempo cada
//! vez que una carga sale bien, en una carpeta por script fuera del
//! repositorio (`~/.local/state/sampler/historial/<script>-<id>/`).
//!
//! Cada versión es un archivo `<segundos>-<nanos>-<hash>.lua` (el nombre
//! ordena por fecha y dice su contenido): no se guarda otra igual a la
//! última (mismo hash) y, si pasan de [`MAX_VERSIONS`] o de [`MAX_BYTES`],
//! se borran las más antiguas. Todo lo que toca el disco devuelve un error
//! en vez de fallar: la interfaz lo avisa y sigue.

use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Versiones que se guardan como mucho por script.
pub const MAX_VERSIONS: usize = 200;
/// Tamaño máximo del historial de un script.
pub const MAX_BYTES: u64 = 20 * 1024 * 1024;

/// El historial de un script.
#[derive(Debug, Clone)]
pub struct History {
    dir: PathBuf,
    max_versions: usize,
    max_bytes: u64,
}

/// Una versión guardada.
#[derive(Debug, Clone, PartialEq)]
pub struct Version {
    pub path: PathBuf,
    /// Cuándo se guardó.
    pub time: SystemTime,
    pub hash: u64,
    pub bytes: u64,
}

/// Carpeta de los historiales: `$XDG_STATE_HOME/sampler/historial` o
/// `~/.local/state/sampler/historial`.
pub fn root() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("sampler").join("historial"))
}

/// FNV-1a de 64 bits: estable entre versiones de Rust (el de la
/// biblioteca estándar no lo es) y sin dependencias.
pub fn hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100_0000_01b3)
    })
}

impl History {
    /// El historial del script `script` dentro de `root`: una carpeta con
    /// su nombre y un identificador de su ruta (dos `song.lua` de dos
    /// proyectos no se mezclan).
    pub fn for_script(root: &Path, script: &Path) -> History {
        let full = std::fs::canonicalize(script).unwrap_or_else(|_| script.to_path_buf());
        let stem = script.file_stem().map_or_else(
            || "script".to_string(),
            |s| s.to_string_lossy().into_owned(),
        );
        let id = hash(&full.to_string_lossy());
        History::new(root.join(format!("{stem}-{:08x}", id as u32)))
    }

    pub fn new(dir: PathBuf) -> History {
        History {
            dir,
            max_versions: MAX_VERSIONS,
            max_bytes: MAX_BYTES,
        }
    }

    /// Con otros límites (para probarlos).
    #[cfg(test)]
    pub fn with_limits(mut self, max_versions: usize, max_bytes: u64) -> History {
        self.max_versions = max_versions.max(1);
        self.max_bytes = max_bytes;
        self
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Guarda `text` como versión de `now`, salvo que sea igual a la última
    /// (devuelve si la guardó). Luego borra las más antiguas que pasen de
    /// los límites (nunca la que acaba de guardar).
    pub fn record(&self, text: &str, now: SystemTime) -> io::Result<bool> {
        let hash = hash(text);
        let versions = self.list()?;
        if versions.first().is_some_and(|v| v.hash == hash) {
            return Ok(false);
        }
        std::fs::create_dir_all(&self.dir)?;
        let since = now.duration_since(UNIX_EPOCH).unwrap_or_default();
        // Dos cargas en el mismo nanosegundo no deberían pasar; si pasa, la
        // segunda va justo detrás.
        let mut stamp = since;
        let mut path;
        loop {
            path = self.dir.join(format!(
                "{:011}-{:09}-{hash:016x}.lua",
                stamp.as_secs(),
                stamp.subsec_nanos()
            ));
            if !path.exists() {
                break;
            }
            stamp += Duration::from_nanos(1);
        }
        // Primero a un temporal y luego se renombra: nunca queda una versión
        // a medias.
        let partial = path.with_extension("lua.part");
        std::fs::write(&partial, text)?;
        std::fs::rename(&partial, &path)?;
        self.prune(&path)?;
        Ok(true)
    }

    /// Borra las más antiguas hasta quedar en los límites, sin tocar `keep`.
    fn prune(&self, keep: &Path) -> io::Result<()> {
        let mut versions = self.list()?;
        let mut total: u64 = versions.iter().map(|v| v.bytes).sum();
        while versions.len() > self.max_versions || total > self.max_bytes {
            let Some(oldest) = versions.pop() else { break };
            if oldest.path == keep {
                break;
            }
            std::fs::remove_file(&oldest.path)?;
            total -= oldest.bytes;
        }
        Ok(())
    }

    /// Las versiones, de la más nueva a la más antigua. Sin carpeta, ninguna.
    pub fn list(&self) -> io::Result<Vec<Version>> {
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(e),
        };
        let mut versions = Vec::new();
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let Some((secs, nanos, hash)) = parse_name(name) else {
                continue;
            };
            versions.push(Version {
                time: UNIX_EPOCH + Duration::new(secs, nanos),
                hash,
                bytes: entry.metadata()?.len(),
                path,
            });
        }
        versions.sort_by(|a, b| b.time.cmp(&a.time).then(b.path.cmp(&a.path)));
        Ok(versions)
    }

    pub fn read(&self, version: &Version) -> io::Result<String> {
        std::fs::read_to_string(&version.path)
    }
}

/// `<segundos>-<nanos>-<hash>.lua` → sus partes.
fn parse_name(name: &str) -> Option<(u64, u32, u64)> {
    let stem = name.strip_suffix(".lua")?;
    let mut parts = stem.split('-');
    let secs = parts.next()?.parse().ok()?;
    let nanos = parts.next()?.parse().ok().filter(|n| *n < 1_000_000_000)?;
    let hash = u64::from_str_radix(parts.next()?, 16).ok()?;
    parts.next().is_none().then_some((secs, nanos, hash))
}

/// Fecha y hora (UTC) de una versión: `2026-10-18 19:42:05 UTC`.
pub fn date(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let (days, rest) = (secs / 86_400, secs % 86_400);
    // Días desde 1970 a fecha civil (Howard Hinnant).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// Cuánto hace, corto: `hace 40 s`, `hace 5 min`, `hace 3 h`, `hace 2 días`.
pub fn ago(time: SystemTime, now: SystemTime) -> String {
    let secs = now.duration_since(time).unwrap_or_default().as_secs();
    match secs {
        0..60 => format!("hace {secs} s"),
        60..3600 => format!("hace {} min", secs / 60),
        3600..86_400 => format!("hace {} h", secs / 3600),
        _ if secs < 2 * 86_400 => "hace 1 día".into(),
        _ => format!("hace {} días", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(test: &str) -> PathBuf {
        let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../target/tmp"))
            .join(format!("historial_{test}"));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn at(secs: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(secs)
    }

    #[test]
    fn records_without_duplicates_newest_first() {
        let history = History::new(dir("record"));
        assert!(history.list().unwrap().is_empty());
        assert!(history.record("bpm(120)\n", at(1000)).unwrap());
        // Lo mismo otra vez: no se guarda.
        assert!(!history.record("bpm(120)\n", at(1010)).unwrap());
        assert!(history.record("bpm(130)\n", at(1020)).unwrap());
        // Volver a lo de antes sí es una versión nueva.
        assert!(history.record("bpm(120)\n", at(1030)).unwrap());
        let versions = history.list().unwrap();
        assert_eq!(versions.len(), 3);
        assert_eq!(versions[0].time, at(1030));
        assert_eq!(history.read(&versions[1]).unwrap(), "bpm(130)\n");
        assert_eq!(versions[2].hash, hash("bpm(120)\n"));
    }

    #[test]
    fn the_oldest_go_first_when_it_fills_up() {
        let history = History::new(dir("prune")).with_limits(3, 1000);
        for i in 0..5u64 {
            history.record(&format!("v{i}\n"), at(100 + i)).unwrap();
        }
        let texts: Vec<String> = history
            .list()
            .unwrap()
            .iter()
            .map(|v| history.read(v).unwrap())
            .collect();
        assert_eq!(texts, ["v4\n", "v3\n", "v2\n"]);
        // Por tamaño: 40 bytes caben en 100; la tercera no.
        let history = History::new(dir("prune_bytes")).with_limits(100, 100);
        for i in 0..3u64 {
            history
                .record(&format!("{i}{}\n", "x".repeat(38)), at(i))
                .unwrap();
        }
        assert_eq!(history.list().unwrap().len(), 2);
        // La que se acaba de guardar se queda aunque sola ya no quepa.
        history.record(&"y".repeat(500), at(10)).unwrap();
        assert_eq!(history.list().unwrap().len(), 1);
    }

    #[test]
    fn a_broken_disk_is_an_error_not_a_panic() {
        // La «carpeta» es un archivo.
        let path = dir("broken");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "no soy una carpeta").unwrap();
        let history = History::new(path.clone());
        assert!(history.record("bpm(120)\n", at(1)).is_err());
        assert!(history.list().is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn one_folder_per_script_and_readable_dates() {
        let root = dir("root");
        let a = History::for_script(&root, Path::new("/uno/song.lua"));
        let b = History::for_script(&root, Path::new("/dos/song.lua"));
        assert_ne!(a.dir(), b.dir());
        assert!(
            a.dir()
                .file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("song-")
        );
        assert_eq!(date(at(0)), "1970-01-01 00:00:00 UTC");
        assert_eq!(date(at(1_792_345_325)), "2026-10-18 17:42:05 UTC");
        assert_eq!(ago(at(0), at(90)), "hace 1 min");
        assert_eq!(ago(at(0), at(3 * 86_400)), "hace 3 días");
    }
}
