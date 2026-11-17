//! La línea del compás: bajo la cabecera, una línea horizontal que crece
//! de izquierda a derecha a lo largo del compás que suena (de 0 a 100 %,
//! con el compás de la canción, no 16 pasos fijos) y vuelve a cero al
//! cambiar de compás.
//!
//! - Lo lleno, con un trazo grueso (`━`) en el color `regla` (el acento);
//!   lo vacío, con uno fino (`─`) en `regla_vacia` (el tenue).
//! - La celda del frente se llena por partes ([`Subcell`]): por defecto en
//!   medias celdas con `╾` (grueso a la izquierda y fino a la derecha:
//!   la línea sigue siendo una), o en octavos con los bloques `▏▎▍▌▋▊▉`.
//! - En ASCII: `=====>-----`.
//! - Marcas de los tiempos o de los pasos ([`Marks`], apagadas por
//!   defecto): una muesca (`┷` en lo lleno, `┴` en lo vacío).
//!
//! La posición sale de lo que se oye ([`engine::Status::heard`], en pasos
//! con decimales), que el motor publica una vez por bloque de audio: el
//! [`Smoother`] la sigue con el reloj y el tempo entre bloque y bloque,
//! sin volver atrás, y se resincroniza ante un salto (cue, arranque) o un
//! tempo nuevo. Todo es de la interfaz: no lee nada nuevo del motor.

// Hasta que la regla de la cabecera la use (en el commit siguiente).
#![allow(dead_code)]

use std::time::Instant;

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;

/// Por debajo de este ancho la línea no se dibuja (la línea de tiempo de
/// las secciones, si la hay, se queda con todo).
pub const MIN_WIDTH: u16 = 8;

/// El ancho que se le deja como mínimo junto a la línea de tiempo (el de
/// la regla de antes: la línea de tiempo empieza donde empezaba).
pub const MIN_WITH_TIMELINE: u16 = 16;

/// Columnas entre la línea y la línea de tiempo.
pub const GAP: u16 = 3;

/// Marcas sobre la línea (`[interfaz] regla_marcas`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Marks {
    /// Sin marcas (`"no"`).
    #[default]
    None,
    /// Una por tiempo del compás (`"tiempos"`).
    Beats,
    /// Una por paso (`"pasos"`); si no caben, las de los tiempos.
    Steps,
}

impl Marks {
    pub const NAMES: [(&str, Marks); 3] = [
        ("no", Marks::None),
        ("tiempos", Marks::Beats),
        ("pasos", Marks::Steps),
    ];

    pub fn from_name(name: &str) -> Option<Marks> {
        Self::NAMES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, m)| *m)
    }
}

/// Cómo se llena la celda del frente (`[interfaz] regla_subcelda`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Subcell {
    /// Medias celdas con `╾` (`"media"`): la línea no cambia de alto.
    #[default]
    Half,
    /// Octavos con los bloques `▏▎▍▌▋▊▉` (`"octavos"`): más pasos, pero
    /// el frente es un bloque de todo el alto de la fila.
    Eighths,
}

impl Subcell {
    pub const NAMES: [(&str, Subcell); 2] =
        [("media", Subcell::Half), ("octavos", Subcell::Eighths)];

    pub fn from_name(name: &str) -> Option<Subcell> {
        Self::NAMES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, s)| *s)
    }
}

/// Lo que dibuja la línea (en el modelo de vista).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BarLine {
    /// Por dónde va el compás, de 0 a 1; `None` sin transporte (parado o
    /// sin información): la línea vacía.
    pub fraction: Option<f64>,
    /// Pasos del compás que suena y pasos de cada tiempo (para las marcas).
    pub bar_steps: usize,
    pub beat: usize,
    pub marks: Marks,
    pub subcell: Subcell,
    /// Grabando: lo lleno, en el color de error (el de `● REC`).
    pub recording: bool,
}

/// Qué es cada celda de la línea.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Full,
    /// La del frente, a medias (el carácter dice cuánto).
    Head,
    Empty,
}

/// Las celdas de una línea de `width` al `fraction` (de 0 a 1): su carácter
/// y qué parte son. Puro (para los tests y para la ventana que lo quiera).
pub fn cells(line: &BarLine, width: u16, ascii: bool) -> Vec<(char, Part)> {
    let width = width as usize;
    let fraction = line.fraction.unwrap_or(0.0).clamp(0.0, 1.0);
    let mut out = Vec::with_capacity(width);
    if ascii {
        let full = ((fraction * width as f64).floor() as usize).min(width);
        for i in 0..width {
            out.push(if i < full {
                ('=', Part::Full)
            } else if i == full && fraction > 0.0 {
                ('>', Part::Head)
            } else {
                ('-', Part::Empty)
            });
        }
    } else {
        let parts = match line.subcell {
            Subcell::Half => 2,
            Subcell::Eighths => 8,
        };
        let units = ((fraction * (width * parts) as f64).floor() as usize).min(width * parts);
        let (full, rest) = (units / parts, units % parts);
        for i in 0..width {
            out.push(if i < full {
                ('━', Part::Full)
            } else if i == full && rest > 0 {
                let c = match line.subcell {
                    Subcell::Half => '╾',
                    Subcell::Eighths => ['▏', '▎', '▍', '▌', '▋', '▊', '▉'][rest - 1],
                };
                (c, Part::Head)
            } else {
                ('─', Part::Empty)
            });
        }
    }
    // Las marcas: nunca encima del frente ni en la primera celda.
    for at in mark_cells(line, width) {
        let part = out[at].1;
        let mark = match (part, ascii) {
            (Part::Head, _) => continue,
            (_, true) => '|',
            (Part::Full, false) => '┷',
            (Part::Empty, false) => '┴',
        };
        out[at] = (mark, part);
    }
    out
}

/// Las celdas con marca: una por tiempo o por paso, si caben con al menos
/// una celda de línea entre dos (si no, las de los tiempos; si tampoco,
/// ninguna).
pub fn mark_cells(line: &BarLine, width: usize) -> Vec<usize> {
    let bar = line.bar_steps.max(1);
    let beat = line.beat.max(1);
    let beats = if bar.is_multiple_of(beat) {
        bar / beat
    } else {
        0
    };
    let wanted = match line.marks {
        Marks::None => return Vec::new(),
        Marks::Beats => [beats, 0],
        Marks::Steps => [bar, beats],
    };
    let Some(n) = wanted.into_iter().find(|&n| n > 1 && width >= n * 2) else {
        return Vec::new();
    };
    (1..n)
        .map(|k| k * width / n)
        .filter(|&c| c > 0 && c < width)
        .collect()
}

impl Widget for BarLine {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < MIN_WIDTH || area.height == 0 {
            return;
        }
        let theme = crate::theme::get();
        let full = if self.recording {
            theme.error
        } else {
            theme.ruler.unwrap_or(theme.accent)
        };
        let empty = theme.ruler_empty.unwrap_or(theme.dim);
        let ascii = crate::theme::ascii();
        for (i, (c, part)) in cells(&self, area.width, ascii).into_iter().enumerate() {
            let color: Color = match part {
                Part::Full | Part::Head => full,
                Part::Empty => empty,
            };
            buf[(area.x + i as u16, area.y)]
                .set_char(c)
                .set_style(Style::new().fg(color));
        }
    }
}

/// Por dónde va el compás (de 0 a 1) cuando se oye el paso `heard` (con
/// decimales), sabiendo que el último paso disparado fue el `step` del
/// compás y el `global` de la canción, con compases de `bar_steps`.
pub fn fraction(heard: f64, step: usize, global: u64, bar_steps: usize) -> f64 {
    let bar = bar_steps.max(1) as f64;
    let into = step as f64 + (heard - global as f64);
    (into.rem_euclid(bar) / bar).clamp(0.0, 1.0)
}

/// Más de esto (en pasos) entre lo previsto y lo que dice el motor es un
/// salto del transporte: se resincroniza.
const JUMP: f64 = 1.0;

/// Lo más que lo previsto puede ir por delante de lo que dice el motor (en
/// pasos): más que un bloque de audio grande a un tempo alto (1024 frames
/// a 200 BPM son 0,28 pasos). Si el motor deja de avanzar, la línea se
/// para ahí.
const AHEAD: f64 = 0.5;

/// La posición que se ve, suave entre los bloques del motor.
#[derive(Debug, Clone, Copy, Default)]
pub struct Smoother {
    /// Lo último que se enseñó, cuándo y con qué tempo.
    last: Option<(f64, Instant, f64)>,
}

impl Smoother {
    /// Lo que se ve en `now` si el motor dice `heard` (en pasos, `None`
    /// parado) con el tempo `bpm`: lo previsto con el reloj desde el cuadro
    /// anterior, entre `heard` (lo que dice el motor llega con un bloque de
    /// retraso como mucho: nunca se queda por detrás) y `heard` +
    /// [`AHEAD`], y nunca hacia atrás. Un salto (más de [`JUMP`] pasos de
    /// diferencia) o un tempo nuevo resincronizan.
    pub fn update(&mut self, heard: Option<f64>, bpm: f64, now: Instant) -> Option<f64> {
        let Some(heard) = heard.filter(|h| h.is_finite()) else {
            self.last = None;
            return None;
        };
        let shown = match self.last {
            Some((prev, at, prev_bpm)) if prev_bpm == bpm => {
                let per_second = bpm * 4.0 / 60.0;
                let elapsed = now.saturating_duration_since(at).as_secs_f64();
                let predicted = prev + elapsed * per_second;
                if (heard - predicted).abs() > JUMP {
                    heard
                } else {
                    predicted.clamp(heard, heard + AHEAD).max(prev)
                }
            }
            _ => heard,
        };
        self.last = Some((shown, now, bpm));
        Some(shown)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn line(fraction: Option<f64>) -> BarLine {
        BarLine {
            fraction,
            bar_steps: 16,
            beat: 4,
            ..BarLine::default()
        }
    }

    fn text(line: &BarLine, width: u16, ascii: bool) -> String {
        cells(line, width, ascii)
            .into_iter()
            .map(|(c, _)| c)
            .collect()
    }

    #[test]
    fn empty_half_and_full() {
        assert_eq!(text(&line(Some(0.0)), 10, false), "──────────");
        assert_eq!(text(&line(None), 10, false), "──────────");
        assert_eq!(text(&line(Some(0.5)), 10, false), "━━━━━─────");
        assert_eq!(text(&line(Some(1.0)), 10, false), "━━━━━━━━━━");
        assert_eq!(text(&line(Some(1.7)), 4, false), "━━━━");
    }

    #[test]
    fn the_head_grows_by_halves_or_eighths() {
        // Entre el paso 8 y el 9 de 16 en 16 celdas: la celda 8, a medias.
        let mut l = line(Some(8.5 / 16.0));
        assert_eq!(text(&l, 16, false), "━━━━━━━━╾───────");
        l.subcell = Subcell::Eighths;
        let heads: Vec<char> = (1..8)
            .map(|k| {
                l.fraction = Some((8.0 + k as f64 / 8.0) / 16.0);
                cells(&l, 16, false)[8].0
            })
            .collect();
        assert_eq!(heads, ['▏', '▎', '▍', '▌', '▋', '▊', '▉']);
        // Y crece sin volver atrás: cada octavo, una unidad más.
        let mut before = 0;
        for k in 0..=64 {
            l.fraction = Some(k as f64 / 64.0);
            let units: usize = cells(&l, 8, false)
                .iter()
                .map(|(c, _)| match c {
                    '━' => 8,
                    '─' => 0,
                    c => {
                        ['▏', '▎', '▍', '▌', '▋', '▊', '▉']
                            .iter()
                            .position(|x| x == c)
                            .unwrap()
                            + 1
                    }
                })
                .sum();
            assert_eq!(units, k);
            assert!(units >= before);
            before = units;
        }
    }

    #[test]
    fn ascii_has_an_arrow_head() {
        assert_eq!(text(&line(Some(0.0)), 10, true), "----------");
        assert_eq!(text(&line(Some(0.55)), 10, true), "=====>----");
        assert_eq!(text(&line(Some(1.0)), 10, true), "==========");
    }

    #[test]
    fn marks_follow_the_meter_and_never_cover_the_head() {
        let mut l = line(Some(0.0));
        l.marks = Marks::Beats;
        assert_eq!(text(&l, 16, false), "────┴───┴───┴───");
        // 3/4: 12 pasos, tres tiempos.
        l.bar_steps = 12;
        assert_eq!(mark_cells(&l, 12), [4, 8]);
        // 7/8: 14 pasos de 2, siete tiempos.
        l.bar_steps = 14;
        l.beat = 2;
        assert_eq!(mark_cells(&l, 28), [4, 8, 12, 16, 20, 24]);
        // Los pasos, si caben; si no, los tiempos.
        l.marks = Marks::Steps;
        assert_eq!(mark_cells(&l, 28).len(), 13);
        assert_eq!(mark_cells(&l, 20).len(), 6);
        assert!(mark_cells(&l, 10).is_empty());
        // En lo lleno, la muesca gruesa; el frente, sin marca.
        let mut l = line(Some(0.5 + 1.0 / 32.0));
        l.marks = Marks::Beats;
        assert_eq!(text(&l, 16, false), "━━━━┷━━━╾───┴───");
        assert_eq!(text(&l, 16, true), "====|===>---|---");
    }

    #[test]
    fn no_squares_anywhere() {
        for subcell in [Subcell::Half, Subcell::Eighths] {
            for k in 0..=40 {
                let l = BarLine {
                    subcell,
                    marks: Marks::Steps,
                    ..line(Some(k as f64 / 40.0))
                };
                let s = text(&l, 33, false);
                assert!(!s.contains('▰') && !s.contains('▱'), "{s}");
            }
        }
    }

    #[test]
    fn the_fraction_is_inside_the_bar() {
        // 4/4: paso 8 de la canción 40 (compás 2), oyendo 40,0: la mitad.
        assert_eq!(fraction(40.0, 8, 40, 16), 0.5);
        // Se oye aún el final del compás anterior (el motor ya disparó el 0).
        assert_eq!(fraction(31.5, 0, 32, 16), 15.5 / 16.0);
        // 3/4 y 7/8.
        assert_eq!(fraction(30.0, 6, 30, 12), 0.5);
        assert!((fraction(21.0, 7, 21, 14) - 0.5).abs() < 1e-9);
        // Al cambiar de compás, vuelve a cero.
        assert_eq!(fraction(48.0, 0, 48, 16), 0.0);
    }

    #[test]
    fn the_smoother_moves_with_the_clock_and_never_goes_back() {
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        let mut s = Smoother::default();
        // 150 BPM: 10 pasos por segundo.
        assert_eq!(s.update(Some(4.0), 150.0, t0), Some(4.0));
        // El motor aún dice lo mismo (bloque sin terminar): se avanza solo,
        // hasta medio paso por delante como mucho.
        let a = s.update(Some(4.0), 150.0, ms(33)).unwrap();
        assert!((a - 4.33).abs() < 0.01, "{a}");
        assert_eq!(s.update(Some(4.0), 150.0, ms(80)), Some(4.5));
        // Un bloque que dice menos de lo que se ve no hace retroceder.
        let b = s.update(Some(4.2), 150.0, ms(81)).unwrap();
        assert!(b >= 4.5, "{b}");
        // Uno que dice más: se le alcanza ya.
        assert_eq!(s.update(Some(5.2), 150.0, ms(82)), Some(5.2));
        // Un salto (cue): se resincroniza.
        assert_eq!(s.update(Some(128.0), 150.0, ms(230)), Some(128.0));
        // Un tempo nuevo: también.
        assert_eq!(s.update(Some(128.2), 170.0, ms(250)), Some(128.2));
        // Parado: nada, y al volver empieza de nuevo.
        assert_eq!(s.update(None, 170.0, ms(260)), None);
        assert_eq!(s.update(Some(0.0), 170.0, ms(500)), Some(0.0));
    }
}
