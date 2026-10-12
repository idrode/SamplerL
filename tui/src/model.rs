//! Modelo de vista: todo lo que dibujan los paneles. Se construye a partir
//! del modelo del script ([`script::Song`]), la instantánea del motor
//! ([`engine::Status`]) y el estado del editor. Los paneles solo leen esto.

use std::ops::Range;

#[derive(Debug, Clone, Default)]
pub struct ViewModel {
    /// Nombre del archivo del script.
    pub file: String,
    /// Hay cambios en el editor sin guardar.
    pub dirty: bool,
    /// Mensaje informativo de la última acción.
    pub message: String,
    /// Error de la última carga (sigue sonando lo anterior).
    pub error: Option<ErrorView>,
    pub transport: TransportView,
    /// Pistas en el orden del script.
    pub tracks: Vec<TrackView>,
    /// Pista seleccionada (índice en `tracks`).
    pub selected: Option<usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ErrorView {
    /// Línea del script (desde 1), si el error la indica.
    pub line: Option<usize>,
    pub message: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TransportView {
    pub running: bool,
    pub bpm: f64,
    pub bar: u64,
    /// Último paso disparado.
    pub step: Option<usize>,
}

#[derive(Debug, Clone)]
pub struct TrackView {
    pub name: String,
    /// Pista del motor asignada.
    pub slot: Option<usize>,
    pub slices: Vec<Range<usize>>,
    /// Compases del ciclo (`every`/`chance`).
    pub cycle: usize,
    /// Corte que sonó por última vez (índice desde 0).
    pub playing_slice: Option<usize>,
    /// Medidor de nivel (pico con caída), de 0 a 1.
    pub level: f32,
}

/// Línea (desde 1) de un mensaje de error de Lua (`archivo:línea: ...`).
pub fn error_line(message: &str, file: &str) -> Option<usize> {
    let rest = message.strip_prefix(file)?.strip_prefix(':')?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_error_line() {
        assert_eq!(error_line("song.lua:12: steps: ...", "song.lua"), Some(12));
        assert_eq!(error_line("otro.lua:3: x", "song.lua"), None);
        assert_eq!(error_line("song.lua: no existe", "song.lua"), None);
    }
}
