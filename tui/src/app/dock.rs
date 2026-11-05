//! Dónde va cada cosa que se puede mover a mano: el ancho del explorador
//! y (ver más abajo) el alto del editor. Lo que se cambia con el ratón o
//! las teclas se guarda en `[interfaz]` de `config.toml`, sin tocar lo
//! demás, para la próxima vez.

use super::App;

impl App {
    /// Guarda en la configuración lo que se ha movido (si hay archivo de
    /// configuración: en los tests, no).
    pub(super) fn persist_layout(&mut self) {
        let Some(path) = self.devices_config.clone() else {
            return;
        };
        let values = [
            ("explorador", self.files.shown.to_string()),
            ("ancho_explorador", self.files.width.to_string()),
        ];
        if let Err(e) = crate::config::save_keys(&path, "interfaz", &values) {
            self.message = format!("no se pudo guardar en {}: {e}", path.display());
        }
    }
}
