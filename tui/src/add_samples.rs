//! Añadir samples al proyecto: copiarlos a `samples/` junto al script, sin
//! mover ni tocar nunca el original. Cada copia se comprueba antes (que se
//! pueda cargar, que el nombre sea normal, que quepa en el disco) y se
//! escribe con [`script::atomic`] (temporal, `sync_all`, renombrado), en
//! un hilo aparte que cuenta los bytes copiados para la barra de progreso.
//!
//! Si en `samples/` ya hay un archivo con ese nombre: con el mismo
//! contenido, se reutiliza; distinto, nunca se sobrescribe: la copia se
//! llama `amen-2.wav` (o el primer número libre).

use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, channel};

use script::sample_path::same_content;

/// Lo más largo que se acepta en un nombre (bytes, como los sistemas de
/// archivos de Linux) y en una ruta entera.
const MAX_NAME: usize = 255;
const MAX_PATH: usize = 4096;
/// Archivos que se copian como mucho de una carpeta (con sus subcarpetas)
/// y lo hondo que se baja.
pub const MAX_FILES: usize = 2000;
const MAX_DEPTH: usize = 16;
/// Lo que se deja libre en el disco además de la copia.
const SPACE_MARGIN: u64 = 16 << 20;

/// Una copia pedida: el original y dónde va, respecto a `samples/`.
#[derive(Debug, Clone, PartialEq)]
pub struct Copy {
    pub from: PathBuf,
    pub to: PathBuf,
}

/// Cómo acabó cada copia.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// Copiado a `to` (respecto a `samples/`).
    Copied { from: PathBuf, to: PathBuf },
    /// Ya había uno igual en `to`: se usa ese.
    Reused { from: PathBuf, to: PathBuf },
    /// Había otro distinto con ese nombre: copiado como `to`.
    Renamed {
        from: PathBuf,
        wanted: PathBuf,
        to: PathBuf,
    },
    /// No se copió (el motivo); el original queda como estaba.
    Failed { from: PathBuf, reason: String },
}

impl Outcome {
    /// Dónde quedó (respecto a `samples/`), si quedó.
    pub fn placed(&self) -> Option<(&Path, &Path)> {
        match self {
            Outcome::Copied { from, to }
            | Outcome::Reused { from, to }
            | Outcome::Renamed { from, to, .. } => Some((from, to)),
            Outcome::Failed { .. } => None,
        }
    }

    /// Una línea para el mensaje.
    pub fn describe(&self) -> String {
        let name = |p: &Path| p.display().to_string();
        match self {
            Outcome::Copied { to, .. } => format!("{} copiado", name(to)),
            Outcome::Reused { to, .. } => format!("{} ya estaba (igual): se usa ese", name(to)),
            Outcome::Renamed { wanted, to, .. } => format!(
                "ya había otro {} distinto: copiado como {}",
                name(wanted),
                name(to)
            ),
            Outcome::Failed { from, reason } => format!(
                "{}: {reason}",
                from.file_name().unwrap_or_default().to_string_lossy()
            ),
        }
    }
}

/// Un nombre de archivo que se puede copiar: texto normal, sin control,
/// sin ser oculto y no demasiado largo.
pub fn check_name(name: &std::ffi::OsStr) -> Result<&str, String> {
    let Some(text) = name.to_str() else {
        return Err("el nombre no es texto válido (UTF-8): renómbralo".into());
    };
    if text.is_empty() || text == "." || text == ".." {
        return Err("sin nombre".into());
    }
    if text.starts_with('.') {
        return Err(format!("«{text}» es un archivo oculto"));
    }
    if text.chars().any(|c| c.is_control()) {
        return Err(format!(
            "«{}» tiene caracteres de control",
            text.escape_debug()
        ));
    }
    if text.len() > MAX_NAME {
        return Err(format!("nombre de más de {MAX_NAME} bytes"));
    }
    Ok(text)
}

/// `rel` sin `..`, sin raíz y con cada parte válida.
fn check_relative(rel: &Path) -> Result<(), String> {
    for part in rel.components() {
        match part {
            Component::Normal(name) => {
                check_name(name)?;
            }
            _ => return Err(format!("{}: ruta no permitida", rel.display())),
        }
    }
    Ok(())
}

/// Lo que se copia al añadir `paths` (archivos o carpetas): cada archivo
/// de audio que el motor carga, con su sitio dentro de `samples/` (el
/// nombre del archivo; dentro de una carpeta, la carpeta y sus
/// subcarpetas). De una carpeta no se siguen los enlaces que salen de ella
/// ni los que vuelven atrás. Devuelve también lo que se dejó y por qué.
pub fn plan(paths: &[PathBuf]) -> (Vec<Copy>, Vec<String>) {
    let mut copies = Vec::new();
    let mut skipped = Vec::new();
    for path in paths {
        let Some(name) = path.file_name() else {
            skipped.push(format!("{}: sin nombre", path.display()));
            continue;
        };
        if path.is_dir() {
            let Ok(top) = std::fs::canonicalize(path) else {
                skipped.push(format!("{}: no se puede leer", path.display()));
                continue;
            };
            let mut seen = vec![top.clone()];
            walk(
                &top,
                &top,
                Path::new(name),
                0,
                &mut seen,
                &mut copies,
                &mut skipped,
            );
        } else if is_audio(path) {
            copies.push(Copy {
                from: path.clone(),
                to: PathBuf::from(name),
            });
        } else {
            skipped.push(format!(
                "{}: no es un formato que se cargue ({})",
                name.to_string_lossy(),
                crate::files::AUDIO.join(", ")
            ));
        }
    }
    (copies, skipped)
}

fn is_audio(path: &Path) -> bool {
    crate::files::Kind::of(path, false) == Some(crate::files::Kind::Audio)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    top: &Path,
    dir: &Path,
    rel: &Path,
    depth: usize,
    seen: &mut Vec<PathBuf>,
    copies: &mut Vec<Copy>,
    skipped: &mut Vec<String>,
) {
    if depth > MAX_DEPTH {
        skipped.push(format!("{}: demasiado honda", rel.display()));
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        skipped.push(format!("{}: no se puede leer", rel.display()));
        return;
    };
    let mut entries: Vec<_> = entries.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for entry in entries {
        if copies.len() >= MAX_FILES {
            skipped.push(format!("más de {MAX_FILES} archivos: el resto no"));
            return;
        }
        let name = entry.file_name();
        if name.to_string_lossy().starts_with('.') {
            continue;
        }
        let path = entry.path();
        // Un enlace se mira por dónde lleva: fuera de la carpeta que se
        // añade, o a una ya vista, no se sigue.
        let Ok(real) = std::fs::canonicalize(&path) else {
            continue;
        };
        let link = entry.file_type().is_ok_and(|t| t.is_symlink());
        if link && !real.starts_with(top) {
            skipped.push(format!(
                "{}: enlace fuera de la carpeta, no se sigue",
                rel.join(&name).display()
            ));
            continue;
        }
        if real.is_dir() {
            if seen.contains(&real) {
                continue;
            }
            seen.push(real.clone());
            walk(
                top,
                &real,
                &rel.join(&name),
                depth + 1,
                seen,
                copies,
                skipped,
            );
        } else if is_audio(&path) {
            copies.push(Copy {
                from: path,
                to: rel.join(&name),
            });
        }
    }
}

/// Bytes libres para este usuario en el disco de `dir` (`None`: no se
/// sabe).
pub fn free_space(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` es una cadena C válida y `stat`, memoria nuestra del
    // tamaño que espera `statvfs`.
    let result = unsafe { libc::statvfs(path.as_ptr(), &mut stat) };
    (result == 0).then(|| stat.f_bavail as u64 * stat.f_frsize as u64)
}

/// El primer `nombre-N.ext` libre o igual a `from`, a partir de 2.
fn free_name(dir: &Path, wanted: &Path, from: &Path) -> Option<(PathBuf, bool)> {
    let stem = wanted.file_stem()?.to_string_lossy().into_owned();
    let ext = wanted
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let parent = wanted.parent().unwrap_or(Path::new(""));
    for n in 2..1000 {
        let rel = parent.join(format!("{stem}-{n}{ext}"));
        let full = dir.join(&rel);
        if !full.exists() {
            return Some((rel, false));
        }
        if same_content(from, &full).unwrap_or(false) {
            return Some((rel, true));
        }
    }
    None
}

/// El trabajo en marcha: cuánto va (para la barra) y lo que va acabando.
pub struct Running {
    pub total: u64,
    pub copied: Arc<AtomicU64>,
    pub files: usize,
    pub done_files: Arc<AtomicU64>,
    results: Receiver<Vec<Outcome>>,
    cancel: Arc<AtomicBool>,
}

impl Running {
    /// Lo que salió, si ya acabó.
    pub fn finished(&self) -> Option<Vec<Outcome>> {
        self.results.try_recv().ok()
    }

    /// Fracción copiada (0 a 1).
    pub fn fraction(&self) -> f64 {
        if self.total == 0 {
            return 1.0;
        }
        (self.copied.load(Ordering::Relaxed) as f64 / self.total as f64).min(1.0)
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

/// Copia `copies` a `dest` (la carpeta `samples/`, que se crea si hace
/// falta) en un hilo aparte. `rate`: la frecuencia con que se prueba a
/// cargar cada uno antes de copiarlo.
pub fn start(dest: PathBuf, copies: Vec<Copy>, rate: u32) -> Running {
    let total = copies
        .iter()
        .filter_map(|c| std::fs::metadata(&c.from).ok())
        .map(|m| m.len())
        .sum();
    let copied = Arc::new(AtomicU64::new(0));
    let done_files = Arc::new(AtomicU64::new(0));
    let cancel = Arc::new(AtomicBool::new(false));
    let (send, results) = channel();
    let files = copies.len();
    {
        let (copied, done_files, cancel) = (copied.clone(), done_files.clone(), cancel.clone());
        std::thread::spawn(move || {
            // Un fallo raro (un pánico al leer un archivo) no tumba nada: es
            // el error de esa copia.
            let mut out = Vec::new();
            for copy in copies {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let from = copy.from.clone();
                let outcome = script::panics::catch(|| copy_one(&dest, copy, rate, &copied))
                    .unwrap_or_else(|panic| Outcome::Failed {
                        from,
                        reason: format!("error interno: {panic}"),
                    });
                out.push(outcome);
                done_files.fetch_add(1, Ordering::Relaxed);
            }
            let _ = send.send(out);
        });
    }
    Running {
        total,
        copied,
        files,
        done_files,
        results,
        cancel,
    }
}

/// Copia uno (en el hilo aparte).
fn copy_one(dest: &Path, copy: Copy, rate: u32, copied: &AtomicU64) -> Outcome {
    let fail = |reason: String| Outcome::Failed {
        from: copy.from.clone(),
        reason,
    };
    if let Err(e) = check_relative(&copy.to) {
        return fail(e);
    }
    let len = match std::fs::metadata(&copy.from) {
        Ok(m) => m.len(),
        Err(e) => return fail(format!("no se puede leer: {e}")),
    };
    // Que sea un sample que se puede cargar (los WAV rotos, aquí).
    if let Err(e) = engine::Sample::load_wav(&copy.from, rate) {
        copied.fetch_add(len, Ordering::Relaxed);
        return fail(format!("no se puede cargar: {e}"));
    }
    let (wanted, full) = (copy.to.clone(), dest.join(&copy.to));
    if full.as_os_str().len() > MAX_PATH {
        return fail("la ruta de destino es demasiado larga".into());
    }
    let (to, renamed) = if full.exists() {
        if same_content(&copy.from, &full).unwrap_or(false) {
            copied.fetch_add(len, Ordering::Relaxed);
            return Outcome::Reused {
                from: copy.from,
                to: wanted,
            };
        }
        match free_name(dest, &wanted, &copy.from) {
            Some((rel, true)) => {
                copied.fetch_add(len, Ordering::Relaxed);
                return Outcome::Reused {
                    from: copy.from,
                    to: rel,
                };
            }
            Some((rel, false)) => (rel, true),
            None => return fail("no queda un nombre libre (hasta -999)".into()),
        }
    } else {
        (wanted.clone(), false)
    };
    let target = dest.join(&to);
    let dir = target.parent().unwrap_or(dest).to_path_buf();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        return fail(format!("no se puede crear {}: {e}", dir.display()));
    }
    if let Some(free) = free_space(&dir)
        && free < len + SPACE_MARGIN
    {
        return fail(format!(
            "no queda espacio en el disco: hacen falta {} MB y hay {} MB",
            (len + SPACE_MARGIN).div_ceil(1 << 20),
            free >> 20
        ));
    }
    let written = script::atomic::write_with(&target, |tmp| {
        let mut input = std::fs::File::open(&copy.from)?;
        let mut output = std::fs::File::options().write(true).open(tmp)?;
        let mut buffer = vec![0u8; 1 << 20];
        loop {
            let n = input.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            output.write_all(&buffer[..n])?;
            copied.fetch_add(n as u64, Ordering::Relaxed);
        }
        output.flush()
    });
    match written {
        Ok(()) if renamed => Outcome::Renamed {
            from: copy.from,
            wanted,
            to,
        },
        Ok(()) => Outcome::Copied {
            from: copy.from,
            to,
        },
        Err(e) => fail(format!("no se pudo copiar: {e} (el original está intacto)")),
    }
}

/// Las llamadas `sample("...")` con un texto literal de `text`: el texto
/// sin comillas y su sitio (bytes, sin las comillas); y cuántas no tienen
/// un literal (generadas por código). Se saltan los comentarios `--`.
pub fn sample_literals(text: &str) -> (Vec<(String, std::ops::Range<usize>)>, usize) {
    let mut out = Vec::new();
    let mut generated = 0;
    let bytes = text.as_bytes();
    let mut line_start = 0;
    for line in text.split_inclusive('\n') {
        let code_end = line.find("--").unwrap_or(line.len());
        let code = &line[..code_end];
        let mut from = 0;
        while let Some(at) = code[from..].find("sample") {
            let start = from + at;
            from = start + "sample".len();
            // Que sea la palabra entera (no `resample` ni `sample_x`).
            let before = code[..start].chars().next_back();
            if before.is_some_and(|c| c.is_alphanumeric() || c == '_' || c == ':' || c == '.') {
                continue;
            }
            let rest = &code[from..];
            let trimmed = rest.trim_start();
            let Some(args) = trimmed.strip_prefix('(') else {
                continue;
            };
            let args_trim = args.trim_start();
            let quote = match args_trim.chars().next() {
                Some(q @ ('"' | '\'')) => q,
                _ => {
                    generated += 1;
                    continue;
                }
            };
            let open = code.len() - args_trim.len();
            let Some(close) = code[open + 1..].find(quote) else {
                generated += 1;
                continue;
            };
            let value = &code[open + 1..open + 1 + close];
            if value.contains('\\') {
                generated += 1;
                continue;
            }
            let range = line_start + open + 1..line_start + open + 1 + close;
            debug_assert_eq!(&bytes[range.clone()], value.as_bytes());
            out.push((value.to_string(), range));
        }
        line_start += line.len();
    }
    (out, generated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_checked() {
        use std::ffi::OsStr;
        assert!(check_name(OsStr::new("amen.wav")).is_ok());
        assert!(check_name(OsStr::new(".oculto.wav")).is_err());
        assert!(check_name(OsStr::new("a\nb.wav")).is_err());
        assert!(check_name(OsStr::new(&"x".repeat(300))).is_err());
        assert!(check_relative(Path::new("breaks/amen.wav")).is_ok());
        assert!(check_relative(Path::new("../amen.wav")).is_err());
        assert!(check_relative(Path::new("/tmp/amen.wav")).is_err());
    }

    #[test]
    fn the_copy_counts_its_bytes_and_keeps_the_original() {
        let dir = Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/tmp/add_progress"
        ));
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir.join("fuera")).unwrap();
        let mono: Vec<f32> = (0..48_000).map(|i| (i as f32 * 0.01).sin() * 0.5).collect();
        engine::offline::write_wav(dir.join("fuera/largo.wav"), &mono, 1, 48_000).unwrap();
        let (copies, skipped) = plan(&[dir.join("fuera/largo.wav")]);
        assert!(skipped.is_empty());
        let running = start(dir.join("samples"), copies, 48_000);
        let outcomes = loop {
            if let Some(done) = running.finished() {
                break done;
            }
            std::thread::sleep(std::time::Duration::from_millis(2));
        };
        assert_eq!(running.fraction(), 1.0);
        assert_eq!(
            running.copied.load(Ordering::Relaxed),
            std::fs::metadata(dir.join("fuera/largo.wav"))
                .unwrap()
                .len()
        );
        assert!(
            matches!(outcomes[0], Outcome::Copied { .. }),
            "{outcomes:?}"
        );
        assert_eq!(
            std::fs::read(dir.join("samples/largo.wav")).unwrap(),
            std::fs::read(dir.join("fuera/largo.wav")).unwrap()
        );
    }

    #[test]
    fn free_space_is_known() {
        assert!(free_space(&std::env::temp_dir()).is_some_and(|b| b > 0));
    }

    #[test]
    fn sample_literals_are_found_outside_comments() {
        let text = "local a = sample(\"amen.wav\"):slice(8)\n\
                    -- sample(\"comentado.wav\")\n\
                    local b = sample( 'b/x.wav', { gain = 1 })\n\
                    local c = sample(nombre)\n\
                    t:resample(\"no.wav\")\n";
        let (found, generated) = sample_literals(text);
        let names: Vec<&str> = found.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["amen.wav", "b/x.wav"]);
        for (name, range) in &found {
            assert_eq!(&text[range.clone()], name);
        }
        assert_eq!(generated, 1);
    }
}
