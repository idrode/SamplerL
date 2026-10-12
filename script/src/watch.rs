//! Recarga automática del script al guardarlo.
//!
//! [`ScriptWatcher::poll`] lee el archivo y, si su contenido cambió, lo
//! carga con [`ScriptHost::load`]: mismas garantías que una carga a mano
//! (todo entra junto en el siguiente compás; si falla, sigue sonando lo
//! anterior). Se compara el contenido, no la fecha de modificación: así da
//! igual la resolución del reloj del sistema de archivos o que el editor
//! guarde en varios pasos (un guardado a medias da un error y el siguiente
//! sondeo recoge la versión completa).

use std::path::{Path, PathBuf};

use engine::Controller;

use crate::{ScriptError, ScriptHost};

/// Qué pasó en un sondeo.
#[derive(Debug, Clone, PartialEq)]
pub enum WatchEvent {
    /// El archivo no cambió (o el error de lectura ya se había informado).
    Unchanged,
    /// Cambió y se cargó: entra en el siguiente compás.
    Reloaded,
    /// Cambió pero falló: sigue sonando lo anterior.
    Failed(ScriptError),
}

pub struct ScriptWatcher {
    path: PathBuf,
    /// Contenido del último intento de carga (válido o no).
    last_source: Option<String>,
    /// Último error de lectura informado, para no repetirlo en cada sondeo.
    last_read_error: Option<String>,
}

impl ScriptWatcher {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        ScriptWatcher {
            path: path.into(),
            last_source: None,
            last_read_error: None,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Mira si el script cambió y, si es así, lo carga.
    pub fn poll(&mut self, host: &mut ScriptHost, controller: &mut Controller) -> WatchEvent {
        let source = match std::fs::read_to_string(&self.path) {
            Ok(source) => source,
            Err(e) => {
                let message = format!("{}: {e}", self.path.display());
                if self.last_read_error.as_ref() == Some(&message) {
                    return WatchEvent::Unchanged;
                }
                self.last_read_error = Some(message.clone());
                return WatchEvent::Failed(ScriptError { message });
            }
        };
        self.last_read_error = None;
        if self.last_source.as_ref() == Some(&source) {
            return WatchEvent::Unchanged;
        }

        let name = self.path.file_name().map_or_else(
            || self.path.display().to_string(),
            |n| n.to_string_lossy().into(),
        );
        let base = self.path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let result = host.load(controller, &source, &name, &base).map(|_| ());
        // Se recuerda aunque falle: no se reintenta hasta que vuelva a cambiar.
        self.last_source = Some(source);
        match result {
            Ok(()) => WatchEvent::Reloaded,
            Err(e) => WatchEvent::Failed(e),
        }
    }
}
