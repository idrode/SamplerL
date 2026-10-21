//! Colores de la interfaz: un tema (los de siempre por defecto, o los de
//! `[colores]` del archivo de configuración). Los widgets leen el tema del
//! hilo que dibuja; `main` lo pone al arrancar.
//!
//! Sin truecolor (`COLORTERM` no dice `truecolor` ni `24bit`), los colores
//! `#rrggbb` se cambian por el más cercano de la paleta de 256 colores: se
//! ven parecidos en cualquier terminal.

use std::cell::Cell;
use std::str::FromStr;

use ratatui::style::Color;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Theme {
    /// Bordes, la pista elegida, la paleta...
    pub accent: Color,
    /// Texto secundario.
    pub dim: Color,
    pub error: Color,
    /// Fondo de la línea con error.
    pub error_line: Color,
    /// Paso con error en `steps("...")`.
    pub diagnostic: Color,
    pub warning: Color,
    /// Forma de onda y marcas de los cortes.
    pub wave: Color,
    pub slice: Color,
    /// Fondo del paso que suena en el tracker.
    pub playing: Color,
    /// Medidores de nivel.
    pub meter: Color,
    /// Resaltado en vivo del código al sonar: un color por pista (en el
    /// orden del script, y vuelta a empezar) y su versión apagada, para
    /// las pistas que no están seleccionadas.
    pub highlight: [Color; HIGHLIGHT_COLORS],
    pub highlight_dim: [Color; HIGHLIGHT_COLORS],
}

/// Colores del resaltado en vivo (uno por pista del motor).
pub const HIGHLIGHT_COLORS: usize = engine::MAX_TRACKS;

/// Los de siempre: rosa, celeste, amarillo, verde, lila, naranja, azul y
/// rojo de la paleta de 256 (se ven en cualquier terminal).
const HIGHLIGHT: [Color; HIGHLIGHT_COLORS] = [
    Color::Indexed(205),
    Color::Indexed(81),
    Color::Indexed(221),
    Color::Indexed(120),
    Color::Indexed(141),
    Color::Indexed(209),
    Color::Indexed(75),
    Color::Indexed(167),
];

impl Default for Theme {
    fn default() -> Self {
        Theme {
            accent: Color::Cyan,
            dim: Color::DarkGray,
            error: Color::Red,
            error_line: Color::Indexed(52),
            diagnostic: Color::LightRed,
            warning: Color::Yellow,
            wave: Color::Gray,
            slice: Color::Yellow,
            playing: Color::Blue,
            meter: Color::Green,
            highlight: HIGHLIGHT,
            highlight_dim: HIGHLIGHT.map(|c| dimmed(c, false)),
        }
    }
}

/// Nombres de los colores del tema en `[colores]`, con su campo.
pub const NAMES: &[&str] = &[
    "acento",
    "tenue",
    "error",
    "linea_error",
    "aviso_pasos",
    "aviso",
    "onda",
    "cortes",
    "sonando",
    "nivel",
];

/// Qué pinta cada color de [`NAMES`] (para la guía).
#[cfg(test)]
pub const DESCRIPTIONS: &[&str] = &[
    "bordes, la pista elegida, la paleta",
    "texto secundario",
    "errores",
    "fondo de la línea con error",
    "paso con error en `steps(\"...\")`",
    "avisos (limitador, cambios fuera del editor)",
    "forma de onda",
    "marcas de los cortes",
    "fondo del paso que suena en el tracker",
    "medidores de nivel",
];

impl Theme {
    pub fn set(&mut self, name: &str, color: Color) -> bool {
        let slot = match name {
            "acento" => &mut self.accent,
            "tenue" => &mut self.dim,
            "error" => &mut self.error,
            "linea_error" => &mut self.error_line,
            "aviso_pasos" => &mut self.diagnostic,
            "aviso" => &mut self.warning,
            "onda" => &mut self.wave,
            "cortes" => &mut self.slice,
            "sonando" => &mut self.playing,
            "nivel" => &mut self.meter,
            _ => return false,
        };
        *slot = color;
        true
    }
}

thread_local! {
    static THEME: Cell<Theme> = Cell::new(Theme::default());
}

/// Pone el tema de este hilo (el que dibuja).
pub fn set(theme: Theme) {
    THEME.with(|t| t.set(theme));
}

pub fn get() -> Theme {
    THEME.with(Cell::get)
}

/// Si la terminal anuncia truecolor.
pub fn truecolor() -> bool {
    std::env::var("COLORTERM").is_ok_and(|v| v == "truecolor" || v == "24bit")
}

/// Lee un color: un nombre (`cyan`, `lightred`...), `#rrggbb` o un número
/// de la paleta de 256. Sin `truecolor`, un `#rrggbb` pasa al más cercano
/// de la paleta.
pub fn parse(text: &str, truecolor: bool) -> Option<Color> {
    let color = Color::from_str(text.trim()).ok()?;
    Some(if truecolor { color } else { downgrade(color) })
}

/// Un color más apagado (a menos de la mitad de luz), para el fondo de lo
/// que suena en las pistas que no están seleccionadas. Sin `truecolor`,
/// el más cercano de la paleta de 256.
pub fn dimmed(color: Color, truecolor: bool) -> Color {
    let Some((r, g, b)) = rgb(color) else {
        return color;
    };
    let scale = |v: u8| (v as f32 * 0.45).round() as u8;
    let dim = Color::Rgb(scale(r), scale(g), scale(b));
    if truecolor { dim } else { downgrade(dim) }
}

/// El RGB de un color: los de la paleta de 256 (los 16 básicos con los
/// valores de xterm) y los `#rrggbb`; `None` para el de la terminal.
fn rgb(color: Color) -> Option<(u8, u8, u8)> {
    const BASIC: [(u8, u8, u8); 16] = [
        (0, 0, 0),
        (205, 0, 0),
        (0, 205, 0),
        (205, 205, 0),
        (0, 0, 238),
        (205, 0, 205),
        (0, 205, 205),
        (229, 229, 229),
        (127, 127, 127),
        (255, 0, 0),
        (0, 255, 0),
        (255, 255, 0),
        (92, 92, 255),
        (255, 0, 255),
        (0, 255, 255),
        (255, 255, 255),
    ];
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let index = match color {
        Color::Rgb(r, g, b) => return Some((r, g, b)),
        Color::Indexed(i) => i,
        Color::Black => 0,
        Color::Red => 1,
        Color::Green => 2,
        Color::Yellow => 3,
        Color::Blue => 4,
        Color::Magenta => 5,
        Color::Cyan => 6,
        Color::Gray => 7,
        Color::DarkGray => 8,
        Color::LightRed => 9,
        Color::LightGreen => 10,
        Color::LightYellow => 11,
        Color::LightBlue => 12,
        Color::LightMagenta => 13,
        Color::LightCyan => 14,
        Color::White => 15,
        Color::Reset => return None,
    };
    Some(match index {
        0..=15 => BASIC[index as usize],
        16..=231 => {
            let i = index - 16;
            (
                LEVELS[(i / 36) as usize],
                LEVELS[(i / 6 % 6) as usize],
                LEVELS[(i % 6) as usize],
            )
        }
        _ => {
            let level = 8 + 10 * (index - 232);
            (level, level, level)
        }
    })
}

/// El color de la paleta de 256 más cercano a un RGB (el cubo 6×6×6 o la
/// escala de grises); los demás, igual.
pub fn downgrade(color: Color) -> Color {
    let Color::Rgb(r, g, b) = color else {
        return color;
    };
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let nearest = |v: u8| {
        (0..6)
            .min_by_key(|&i| (LEVELS[i] as i32 - v as i32).abs())
            .unwrap()
    };
    let (ri, gi, bi) = (nearest(r), nearest(g), nearest(b));
    let cube = (LEVELS[ri], LEVELS[gi], LEVELS[bi]);
    let distance = |(x, y, z): (u8, u8, u8)| {
        let d = |a: u8, b: u8| (a as i32 - b as i32).pow(2);
        d(x, r) + d(y, g) + d(z, b)
    };
    // Escala de grises: 232..=255, de 8 a 238 en pasos de 10.
    let average = (r as u32 + g as u32 + b as u32) / 3;
    let gray_index = ((average.saturating_sub(8) + 5) / 10).min(23) as u8;
    let gray_level = 8 + 10 * gray_index;
    if distance((gray_level, gray_level, gray_level)) < distance(cube) {
        Color::Indexed(232 + gray_index)
    } else {
        Color::Indexed(16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors_parse_and_degrade_without_truecolor() {
        assert_eq!(parse("cyan", false), Some(Color::Cyan));
        assert_eq!(parse("45", false), Some(Color::Indexed(45)));
        assert_eq!(parse("#00d7ff", true), Some(Color::Rgb(0, 215, 255)));
        // En 256 colores: el mismo, que está en el cubo (0, 5·, 5) → 45.
        assert_eq!(parse("#00d7ff", false), Some(Color::Indexed(45)));
        // Un gris va a la escala de grises.
        assert_eq!(downgrade(Color::Rgb(128, 128, 128)), Color::Indexed(244));
        assert_eq!(downgrade(Color::Rgb(255, 0, 0)), Color::Indexed(196));
        assert_eq!(parse("rosa chicle", false), None);
    }

    #[test]
    fn dimmed_colors_keep_their_hue_with_less_light() {
        assert_eq!(dimmed(Color::Rgb(200, 100, 0), true), Color::Rgb(90, 45, 0));
        // 205 es (255, 95, 175): apagado, (115, 43, 79) → el 53 del cubo
        // (95, 0, 95).
        assert_eq!(dimmed(Color::Indexed(205), false), Color::Indexed(53));
        assert_eq!(rgb(Color::Indexed(244)), Some((128, 128, 128)));
        assert_eq!(rgb(Color::Cyan), Some((0, 205, 205)));
        assert_eq!(dimmed(Color::Reset, false), Color::Reset);
    }
}
