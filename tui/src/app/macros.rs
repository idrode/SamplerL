//! Macros en vivo: el panel «Macros» de la paleta (↑↓ elige, ←→ mueve de
//! 0,05 en 0,05, `-` y `+` de 0,01 en 0,01, 0-9 salta a 0, 0,1... 0,9,
//! Inicio y Fin a 0 y 1) y los controles MIDI de `[midi] macros`. Cada
//! cambio va ya al motor (`ScriptHost::set_macro`); recargar el script deja
//! las macros donde estén.

use ratatui_textarea::{Input, Key};

use super::{App, Flow};

/// Lo que mueven ←→ y `-`/`+`.
const STEP: f32 = 0.05;
const FINE: f32 = 0.01;

impl App {
    /// Abre el panel de las macros (si el script tiene alguna).
    pub fn open_macros(&mut self) {
        if self.host.macros().is_empty() {
            self.message =
                "no hay macros: ponlas en el script con macro(\"nombre\", desde, hasta)".into();
            return;
        }
        self.macro_panel = Some(0);
        self.message = "macros: ↑↓ elegir · ←→ ±0.05 · -+ ±0.01 · 0-9 · Inicio/Fin · Esc".into();
    }

    /// Una tecla con el panel de las macros abierto.
    pub(super) fn handle_macros(&mut self, input: Input) -> Flow {
        let count = self.host.macros().len();
        let Some(selected) = self.macro_panel.filter(|&i| i < count) else {
            self.macro_panel = None;
            return Flow::Continue;
        };
        let value = self.host.macros()[selected].value;
        let target = match input.key {
            Key::Up => {
                self.macro_panel = Some(selected.saturating_sub(1));
                None
            }
            Key::Down => {
                self.macro_panel = Some((selected + 1).min(count - 1));
                None
            }
            Key::Left => Some(value - STEP),
            Key::Right => Some(value + STEP),
            Key::Char('-') => Some(value - FINE),
            Key::Char('+') => Some(value + FINE),
            Key::Char(c @ '0'..='9') if !input.ctrl && !input.alt => {
                Some((c as u8 - b'0') as f32 / 10.0)
            }
            Key::Home => Some(0.0),
            Key::End => Some(1.0),
            Key::Esc | Key::Enter => {
                self.macro_panel = None;
                self.message.clear();
                None
            }
            _ => None,
        };
        if let Some(value) = target {
            self.set_macro(selected, value);
        }
        Flow::Continue
    }

    /// Mueve la macro `index` a `value` (se queda entre 0 y 1).
    pub fn set_macro(&mut self, index: usize, value: f32) {
        // Redondeado a centésimas: los pasos de 0,05 no acumulan error.
        let value = (value.clamp(0.0, 1.0) * 100.0).round() / 100.0;
        match self.host.set_macro(&mut self.controller, index, value) {
            Ok(()) => {
                let name = &self.host.macros()[index].name;
                self.message = format!("macro {} «{name}»: {value:.2}", index + 1);
            }
            Err(e) => self.message = format!("macro: {e}"),
        }
    }

    /// Un controlador MIDI: si `[midi] macros` lo asigna a una macro, la
    /// mueve (de 0 a 127 → de 0 a 1) y devuelve `true`.
    pub(super) fn midi_macro(&mut self, channel: u8, number: u8, value: u8) -> bool {
        let Some(index) = self.midi.macro_for(channel, number) else {
            return false;
        };
        if index < self.host.macros().len() {
            let value = value.min(127) as f32 / 127.0;
            if let Err(e) = self.host.set_macro(&mut self.controller, index, value) {
                self.message = format!("macro: {e}");
            }
        }
        true
    }
}
