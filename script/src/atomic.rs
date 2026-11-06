//! Guardar un archivo sin dejarlo nunca a medias: se escribe a un
//! temporal en la misma carpeta, se vuelca al disco (`sync_all`) y se
//! renombra sobre el destino, que es atómico en el mismo sistema de
//! archivos. Si algo falla antes del renombrado, el archivo de antes queda
//! intacto y el temporal se borra.
//!
//! - **Permisos**: los del archivo de antes pasan al nuevo (un script de
//!   solo lectura para el grupo lo sigue siendo). Un archivo de solo
//!   lectura no se sustituye (da `PermissionDenied`, como `fs::write`). El
//!   dueño no se toca: se guarda como el usuario que corre el programa.
//! - **Enlaces simbólicos**: se sigue el enlace y se escribe en su destino
//!   (con el temporal junto al destino), así que el enlace sigue siendo un
//!   enlace y apunta a lo nuevo. Un enlace roto crea su destino.
//! - **Temporales huérfanos** (el programa murió a mitad): se llaman
//!   `.<nombre>.<pid>-<n>.sampler-tmp`; antes de cada guardado se borran
//!   los del mismo archivo con más de [`ORPHAN_AGE`].
//!
//! Lo usan el editor (el script y su copia sin guardar), los proyectos, la
//! biblioteca de bloques, las ranuras, la configuración, el historial y las
//! exportaciones a WAV.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// Lo que tiene que tener un temporal para que se considere huérfano (un
/// guardado normal dura milisegundos; una exportación larga, segundos).
pub const ORPHAN_AGE: Duration = Duration::from_secs(60);

/// Terminación de los temporales.
const SUFFIX: &str = ".sampler-tmp";

/// Escribe `bytes` en `path` sin dejarlo nunca a medias.
pub fn write(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    write_with(path, |tmp| {
        let mut file = File::options().write(true).truncate(true).open(tmp)?;
        file.write_all(bytes.as_ref())
    })
}

/// Como [`write`], para lo que se escribe con su propia función (un WAV):
/// `fill` recibe la ruta del temporal, ya creado y vacío, y lo llena.
pub fn write_with(
    path: impl AsRef<Path>,
    fill: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<()> {
    replace(path.as_ref(), fill, |from, to| fs::rename(from, to))
}

/// El guardado entero, con el renombrado aparte (los tests lo hacen fallar).
fn replace(
    path: &Path,
    fill: impl FnOnce(&Path) -> io::Result<()>,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
) -> io::Result<()> {
    let target = resolve(path)?;
    let dir = match target.parent() {
        Some(dir) if !dir.as_os_str().is_empty() => dir.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "la ruta no es un archivo"))?
        .to_string_lossy()
        .into_owned();
    clean_orphans(&dir, &name);
    let previous = fs::metadata(&target).ok();
    if previous
        .as_ref()
        .is_some_and(|m| m.permissions().readonly())
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} es de solo lectura", target.display()),
        ));
    }
    let tmp = create_temp(&dir, &name)?;
    let result = (|| {
        fill(&tmp)?;
        let file = File::options().write(true).open(&tmp)?;
        if let Some(meta) = &previous {
            file.set_permissions(meta.permissions())?;
        }
        file.sync_all()?;
        drop(file);
        rename(&tmp, &target)
    })();
    if let Err(e) = result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    sync_dir(&dir);
    Ok(())
}

/// La ruta donde se escribe de verdad: la de `path` o, si es un enlace
/// simbólico, la de su destino (siguiendo los enlaces en cadena).
fn resolve(path: &Path) -> io::Result<PathBuf> {
    let mut current = path.to_path_buf();
    // Como mucho 40 saltos, como el núcleo (ELOOP).
    for _ in 0..40 {
        match fs::symlink_metadata(&current) {
            Ok(meta) if meta.file_type().is_symlink() => {
                let link = fs::read_link(&current)?;
                current = if link.is_absolute() {
                    link
                } else {
                    current.parent().unwrap_or(Path::new(".")).join(link)
                };
            }
            _ => return Ok(current),
        }
    }
    Err(io::Error::other(format!(
        "{}: demasiados enlaces simbólicos seguidos",
        path.display()
    )))
}

/// Un temporal nuevo y vacío junto a `name` en `dir` (nunca pisa otro).
fn create_temp(dir: &Path, name: &str) -> io::Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = dir.join(format!(".{name}.{}-{n}{SUFFIX}", std::process::id()));
        match File::options().write(true).create_new(true).open(&tmp) {
            Ok(_) => return Ok(tmp),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
}

/// Borra los temporales de `name` en `dir` que llevan ahí más de
/// [`ORPHAN_AGE`] (de un guardado que no terminó). Sin errores: si no se
/// puede, se quedan.
pub fn clean_orphans(dir: &Path, name: &str) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let prefix = format!(".{name}.");
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let file = entry.file_name();
        let file = file.to_string_lossy();
        if !(file.starts_with(&prefix) && file.ends_with(SUFFIX)) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age >= ORPHAN_AGE);
        if old {
            let _ = fs::remove_file(entry.path());
        }
    }
}

/// Vuelca la carpeta al disco para que el renombrado sobreviva a un corte
/// (en Unix; donde no se puede abrir una carpeta, nada).
fn sync_dir(dir: &Path) {
    if let Ok(dir) = File::open(dir) {
        let _ = dir.sync_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(test: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("sampler-atomic-{}-{test}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn temps(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(SUFFIX))
            .collect()
    }

    #[test]
    fn writes_and_replaces() {
        let d = dir("writes");
        let path = d.join("cancion.lua");
        write(&path, "bpm(120)\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "bpm(120)\n");
        write(&path, "bpm(170)\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "bpm(170)\n");
        assert!(temps(&d).is_empty());
    }

    /// El disco se llena a mitad de escribir: el archivo de antes queda
    /// como estaba y no queda ningún temporal.
    #[test]
    fn a_failure_while_writing_keeps_the_original() {
        let d = dir("full");
        let path = d.join("cancion.lua");
        fs::write(&path, "lo de antes\n").unwrap();
        let err = write_with(&path, |tmp| {
            fs::write(tmp, "la mitad de lo nue")?;
            Err(io::Error::new(io::ErrorKind::StorageFull, "disco lleno"))
        })
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::StorageFull);
        assert_eq!(fs::read_to_string(&path).unwrap(), "lo de antes\n");
        assert!(temps(&d).is_empty(), "{:?}", temps(&d));
    }

    /// El renombrado falla (permiso denegado): igual.
    #[test]
    fn a_failed_rename_keeps_the_original() {
        let d = dir("rename");
        let path = d.join("proyecto.sampler");
        fs::write(&path, "script = a.lua\n").unwrap();
        let err = replace(
            &path,
            |tmp| fs::write(tmp, "script = b.lua\n"),
            |_, _| Err(io::Error::new(io::ErrorKind::PermissionDenied, "no")),
        )
        .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(&path).unwrap(), "script = a.lua\n");
        assert!(temps(&d).is_empty());
    }

    /// Sin permiso para escribir en la carpeta, de verdad: ni se crea el
    /// temporal ni se toca el archivo.
    #[cfg(unix)]
    #[test]
    fn a_read_only_folder_keeps_the_original() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("readonly");
        let path = d.join("cancion.lua");
        fs::write(&path, "lo de antes\n").unwrap();
        fs::set_permissions(&d, fs::Permissions::from_mode(0o555)).unwrap();
        let result = write(&path, "lo nuevo\n");
        fs::set_permissions(&d, fs::Permissions::from_mode(0o755)).unwrap();
        // Como root la carpeta se puede escribir igual: no hay nada que ver.
        if result.is_ok() {
            return;
        }
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(&path).unwrap(), "lo de antes\n");
    }

    #[cfg(unix)]
    #[test]
    fn keeps_the_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("mode");
        let path = d.join("cancion.lua");
        fs::write(&path, "a").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        write(&path, "b").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }

    /// Un archivo de solo lectura no se sustituye.
    #[test]
    fn a_read_only_file_is_not_replaced() {
        let d = dir("rofile");
        let path = d.join("cancion.lua");
        fs::write(&path, "lo de antes\n").unwrap();
        let mut perms = fs::metadata(&path).unwrap().permissions();
        perms.set_readonly(true);
        fs::set_permissions(&path, perms.clone()).unwrap();
        let err = write(&path, "lo nuevo\n").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(&path).unwrap(), "lo de antes\n");
        assert!(temps(&d).is_empty());
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        fs::set_permissions(&path, perms).unwrap();
    }

    /// Por un enlace: se escribe en el destino y el enlace sigue ahí.
    #[cfg(unix)]
    #[test]
    fn follows_symlinks() {
        let d = dir("link");
        fs::create_dir_all(d.join("de_verdad")).unwrap();
        let real = d.join("de_verdad/cancion.lua");
        fs::write(&real, "a").unwrap();
        let link = d.join("cancion.lua");
        std::os::unix::fs::symlink("de_verdad/cancion.lua", &link).unwrap();
        write(&link, "b").unwrap();
        assert!(
            fs::symlink_metadata(&link)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_to_string(&real).unwrap(), "b");
        assert!(temps(&d).is_empty() && temps(&d.join("de_verdad")).is_empty());
        // Un enlace roto crea su destino.
        let broken = d.join("roto.lua");
        std::os::unix::fs::symlink("de_verdad/nuevo.lua", &broken).unwrap();
        write(&broken, "c").unwrap();
        assert_eq!(
            fs::read_to_string(d.join("de_verdad/nuevo.lua")).unwrap(),
            "c"
        );
    }

    /// Los temporales viejos del mismo archivo se borran al guardar; los
    /// recientes (de otro guardado en curso) y los de otros archivos, no.
    #[test]
    fn cleans_old_orphans() {
        let d = dir("orphans");
        let path = d.join("cancion.lua");
        let old = d.join(format!(".cancion.lua.1-0{SUFFIX}"));
        let fresh = d.join(format!(".cancion.lua.2-0{SUFFIX}"));
        let other = d.join(format!(".otra.lua.1-0{SUFFIX}"));
        for p in [&old, &fresh, &other] {
            fs::write(p, "a medias").unwrap();
        }
        let long_ago = SystemTime::now() - 2 * ORPHAN_AGE;
        for p in [&old, &other] {
            File::options()
                .write(true)
                .open(p)
                .unwrap()
                .set_modified(long_ago)
                .unwrap();
        }
        write(&path, "x").unwrap();
        assert!(!old.exists());
        assert!(fresh.exists() && other.exists());
    }
}
