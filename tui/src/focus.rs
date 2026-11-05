//! El panel con el foco: el que se queda las teclas y lleva el borde
//! resaltado. No es estado nuevo: sale de lo que ya decide a dónde van
//! las teclas (el modo editar o tocar y los cursores del tracker, el piano
//! roll, la ocupación y las lanes). Solo la mezcla, los efectos y las
//! visualizaciones del master, que no tienen cursor, se apuntan aparte.

/// Un panel de la ventana principal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Panel {
    /// El editor de código.
    #[default]
    Code,
    /// La forma de onda del sample (el modo tocar).
    Wave,
    /// La secuencia: el tracker o, abierto, el piano roll.
    Tracker,
    Mixer,
    Fx,
    Lanes,
    Scope,
    Spectrum,
    Occupancy,
    /// El explorador de archivos (Alt-0).
    Files,
}

impl Panel {
    /// Los que tienen tecla de foco, en su orden (Alt-1, Alt-2...).
    pub const NUMBERED: [Panel; 9] = [
        Panel::Code,
        Panel::Wave,
        Panel::Tracker,
        Panel::Mixer,
        Panel::Fx,
        Panel::Lanes,
        Panel::Scope,
        Panel::Spectrum,
        Panel::Occupancy,
    ];

    /// Su número (el de Alt-n y el de su título; el explorador, el 0).
    pub fn number(self) -> usize {
        Self::NUMBERED
            .iter()
            .position(|p| *p == self)
            .map_or(0, |i| i + 1)
    }

    /// Nombre corto, para títulos y pestañas.
    pub fn name(self) -> &'static str {
        match self {
            Panel::Code => "código",
            Panel::Wave => "onda",
            Panel::Tracker => "tracker",
            Panel::Mixer => "mezcla",
            Panel::Fx => "efectos",
            Panel::Lanes => "lanes",
            Panel::Scope => "scope",
            Panel::Spectrum => "espectro",
            Panel::Occupancy => "ocupación",
            Panel::Files => "archivos",
        }
    }

    /// Nombre de las pestañas cuando no cabe el corto.
    pub fn short(self) -> &'static str {
        match self {
            Panel::Code => "cód",
            Panel::Wave => "onda",
            Panel::Tracker => "trk",
            Panel::Mixer => "mez",
            Panel::Fx => "fx",
            Panel::Lanes => "lan",
            Panel::Scope => "sco",
            Panel::Spectrum => "esp",
            Panel::Occupancy => "ocu",
            Panel::Files => "arch",
        }
    }

    /// Título de su borde: `1 código`.
    pub fn title(self) -> String {
        format!("{} {}", self.number(), self.name())
    }
}

/// Lo que dice a dónde van las teclas, para deducir el foco.
#[derive(Debug, Clone, Copy, Default)]
pub struct Inputs {
    pub playing_mode: bool,
    pub tracker_cursor: bool,
    pub roll_cursor: bool,
    pub occupancy_cursor: bool,
    pub lane_point: bool,
    /// El foco puesto a mano en un panel sin cursor propio.
    pub chosen: Option<Panel>,
}

/// El panel con el foco: un cursor abierto manda; si no, el elegido a
/// mano; si no, el editor o la onda según el modo.
pub fn derive(inputs: Inputs) -> Panel {
    if inputs.tracker_cursor || inputs.roll_cursor {
        Panel::Tracker
    } else if inputs.occupancy_cursor {
        Panel::Occupancy
    } else if inputs.lane_point {
        Panel::Lanes
    } else if let Some(panel) = inputs.chosen {
        panel
    } else if inputs.playing_mode {
        Panel::Wave
    } else {
        Panel::Code
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_focus_follows_the_cursors_then_the_choice_then_the_mode() {
        assert_eq!(derive(Inputs::default()), Panel::Code);
        let play = Inputs {
            playing_mode: true,
            ..Default::default()
        };
        assert_eq!(derive(play), Panel::Wave);
        let mixer = Inputs {
            chosen: Some(Panel::Mixer),
            ..play
        };
        assert_eq!(derive(mixer), Panel::Mixer);
        for (inputs, panel) in [
            (
                Inputs {
                    tracker_cursor: true,
                    ..mixer
                },
                Panel::Tracker,
            ),
            (
                Inputs {
                    roll_cursor: true,
                    ..mixer
                },
                Panel::Tracker,
            ),
            (
                Inputs {
                    occupancy_cursor: true,
                    ..mixer
                },
                Panel::Occupancy,
            ),
            (
                Inputs {
                    lane_point: true,
                    ..mixer
                },
                Panel::Lanes,
            ),
        ] {
            assert_eq!(derive(inputs), panel);
        }
    }

    #[test]
    fn numbers_and_titles() {
        assert_eq!(Panel::Code.number(), 1);
        assert_eq!(Panel::Occupancy.number(), 9);
        assert_eq!(Panel::Mixer.title(), "4 mezcla");
    }
}
