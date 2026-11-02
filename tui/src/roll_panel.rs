//! El panel del piano roll: el mismo en la ventana principal y en la suya
//! (`tui --window-roll`). Dibuja una [`RollView`], que es solo datos (la
//! calcula la principal desde su canción y se manda tal cual por el
//! socket), y [`layout`] dice dónde cae cada paso y cada nota: lo usan el
//! dibujo y el ratón, así que coinciden siempre.
//!
//! Resolución: con un semitono por fila y un paso por columna (zoom 1/16)
//! o dos columnas por paso (1/32), cada nota es un bloque con su cabeza
//! (`█▓▒░` por su volumen, o la marca de su retrigger, reverse...) y `━`
//! mientras dura. Si la tesitura no cabe, dos semitonos por fila con medias
//! celdas (`▀▄`); con el zoom 1/8 o 1/4, dos o cuatro pasos por columna
//! con cuadrantes (`▘▝▖▗▌▐`...).

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, Widget};

use crate::roll::{Note, Scale, Style as NoteStyle};
use crate::theme;

/// Cuánto tiempo es una columna.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Zoom {
    /// Fusa: dos columnas por paso.
    Z32,
    /// Semicorchea: una columna por paso.
    #[default]
    Z16,
    /// Corchea: dos pasos por columna.
    Z8,
    /// Negra: cuatro pasos por columna.
    Z4,
}

impl Zoom {
    pub const ALL: [Zoom; 4] = [Zoom::Z32, Zoom::Z16, Zoom::Z8, Zoom::Z4];

    pub fn name(self) -> &'static str {
        match self {
            Zoom::Z32 => "1/32",
            Zoom::Z16 => "1/16",
            Zoom::Z8 => "1/8",
            Zoom::Z4 => "1/4",
        }
    }

    pub fn from_name(name: &str) -> Option<Zoom> {
        Zoom::ALL.into_iter().find(|z| z.name() == name)
    }

    /// Más detalle (`true`) o menos.
    pub fn step(self, closer: bool) -> Zoom {
        let i = Zoom::ALL.iter().position(|z| *z == self).unwrap_or(1);
        let i = if closer {
            i.saturating_sub(1)
        } else {
            (i + 1).min(Zoom::ALL.len() - 1)
        };
        Zoom::ALL[i]
    }

    /// Pasos por columna (las de 1/32 son media: 0).
    fn steps_per_cell(self) -> usize {
        match self {
            Zoom::Z32 | Zoom::Z16 => 1,
            Zoom::Z8 => 2,
            Zoom::Z4 => 4,
        }
    }

    /// Columnas por paso.
    fn cells_per_step(self) -> usize {
        if self == Zoom::Z32 { 2 } else { 1 }
    }
}

/// Todo lo que hace falta para dibujar el piano roll.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RollView {
    /// La pista y lo que es (` bajo · sh101 · 2 vueltas de 16`).
    pub title: String,
    /// Notas (`true`) o cortes en vertical.
    pub notes_axis: bool,
    /// Columnas del ciclo, pasos de una vuelta y de un compás.
    pub steps: usize,
    pub length: usize,
    pub bar_steps: usize,
    /// El nombre de cada corte (en una pista de cortes).
    pub labels: Vec<String>,
    pub notes: Vec<Note>,
    pub scale: Option<Scale>,
    /// Lo más grave y lo más agudo que suena, y lo que se puede escribir.
    pub range: Option<(i32, i32)>,
    pub bounds: (i32, i32),
    /// El cursor (columna y nota), si el piano roll tiene las teclas.
    pub cursor: Option<(usize, i32)>,
    pub selection: Option<(usize, usize)>,
    /// La columna que se oye, con el secuenciador en marcha.
    pub playhead: Option<usize>,
    pub zoom: Zoom,
    /// El zoom lo elige el panel: el más detallado con el que cabe el
    /// ciclo entero (si no cabe ni a 1/4, 1/16 y se desplaza).
    pub zoom_auto: bool,
    /// Primera columna que se ve y nota de arriba (`None`: la tesitura).
    pub first: usize,
    pub top: Option<i32>,
    /// La línea de estado (el token del cursor, o por qué no se edita) y
    /// la de la rejilla.
    pub status: String,
    pub snap: String,
    /// El color de la pista (su orden en el script).
    pub color: usize,
}

/// Dónde cae cada cosa del piano roll en un área.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RollLayout {
    /// La cuadrícula (sin la cabecera, los nombres ni la línea de estado).
    pub grid: Rect,
    /// Ancho de los nombres de las filas.
    pub label: u16,
    /// Semitonos (o cortes) por fila: 1, o 2 con medias celdas.
    pub per_row: i32,
    /// La nota de la fila de arriba.
    pub top: i32,
    /// Primera columna del ciclo que se ve.
    pub first: usize,
    pub zoom: Zoom,
}

impl RollLayout {
    /// Columnas del ciclo que caben.
    pub fn shown(&self) -> usize {
        let cells = self.grid.width as usize;
        match self.zoom {
            Zoom::Z32 => cells / 2,
            z => cells * z.steps_per_cell(),
        }
    }

    /// Filas (notas) que caben.
    pub fn rows(&self) -> i32 {
        self.grid.height as i32 * self.per_row
    }

    /// La columna y la nota de una celda de la pantalla (de la fila de
    /// arriba de la pareja en media celda), si está en la cuadrícula.
    pub fn hit(&self, x: u16, y: u16) -> Option<(usize, i32)> {
        let g = self.grid;
        if x < g.x || x >= g.right() || y < g.y || y >= g.bottom() {
            return None;
        }
        let cell = (x - g.x) as usize;
        let col = self.first
            + match self.zoom {
                Zoom::Z32 => cell / 2,
                z => cell * z.steps_per_cell(),
            };
        let pitch = self.top - (y - g.y) as i32 * self.per_row;
        Some((col, pitch))
    }

    /// La celda de la pantalla de una columna (su x), si se ve.
    pub fn x_of(&self, col: usize) -> Option<u16> {
        if col < self.first || col >= self.first + self.shown() {
            return None;
        }
        let d = col - self.first;
        let cell = match self.zoom {
            Zoom::Z32 => d * 2,
            z => d / z.steps_per_cell(),
        };
        Some(self.grid.x + cell as u16)
    }

    /// La fila de la pantalla de una nota, si se ve.
    pub fn y_of(&self, pitch: i32) -> Option<u16> {
        let d = self.top - pitch;
        (d >= 0 && d < self.rows()).then(|| self.grid.y + (d / self.per_row) as u16)
    }
}

/// Lo que se ve alrededor de la tesitura: una nota de margen y, al
/// menos, una octava.
fn wanted(view: &RollView) -> (i32, i32) {
    let (lo, hi) = view.bounds;
    if !view.notes_axis {
        return (lo, hi);
    }
    let (a, b) = view.range.unwrap_or((60, 71));
    let (mut a, mut b) = (a - 1, b + 1);
    if b - a < 11 {
        let pad = 11 - (b - a);
        a -= pad / 2;
        b += pad - pad / 2;
    }
    (a.max(lo), b.min(hi))
}

/// Dónde cae cada cosa del piano roll de `view` en `area` (el panel entero,
/// con sus bordes). Sigue al cursor (o, sin él, al paso que se oye).
pub fn layout(view: &RollView, area: Rect) -> RollLayout {
    let inner = Block::bordered().inner(area);
    let label = if view.notes_axis {
        5
    } else {
        view.labels
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(2)
            .clamp(2, 10) as u16
            + 1
    };
    let label = label.min(inner.width / 3);
    // La cabecera arriba y la línea de estado abajo.
    let grid = Rect {
        x: inner.x + label,
        y: inner.y + 1.min(inner.height),
        width: inner.width.saturating_sub(label),
        height: inner.height.saturating_sub(2),
    };
    let zoom = if view.zoom_auto {
        let width = grid.width as usize;
        let steps = view.steps.max(1);
        if steps * 2 <= width {
            Zoom::Z32
        } else if steps <= width {
            Zoom::Z16
        } else if steps <= width * 2 {
            Zoom::Z8
        } else if steps <= width * 4 {
            Zoom::Z4
        } else {
            Zoom::Z16
        }
    } else {
        view.zoom
    };
    let (lo, hi) = wanted(view);
    let span = hi - lo + 1;
    let height = grid.height.max(1) as i32;
    let per_row = if view.notes_axis && span > height { 2 } else { 1 };
    let rows = height * per_row;
    let (min, max) = view.bounds;
    // Arriba: lo pedido o lo más agudo de la tesitura (las notas, centradas
    // si sobra sitio; los cortes, desde el de más arriba).
    let mut top = view.top.unwrap_or_else(|| {
        if span >= rows || !view.notes_axis {
            hi
        } else {
            hi + (rows - span) / 2
        }
    });
    if let Some((_, p)) = view.cursor {
        if p > top {
            top = p;
        }
        if p <= top - rows {
            top = p + rows - 1;
        }
    }
    // En media celda, cada fila es una nota impar y la de debajo (C# y C,
    // D# y D, F y E...): las parejas no cambian al desplazarse.
    if per_row == 2 && top.rem_euclid(2) == 0 {
        top += 1;
    }
    let floor = if max - min + 1 >= rows {
        min + rows - 1
    } else {
        min
    };
    top = top.min(max + per_row - 1).max(floor);
    if let Some((_, p)) = view.cursor {
        while p <= top - rows {
            top -= per_row;
        }
        while p > top {
            top += per_row;
        }
    }
    let mut out = RollLayout {
        grid,
        label,
        per_row,
        top,
        first: 0,
        zoom,
    };
    let shown = out.shown().max(1);
    let align = zoom.steps_per_cell();
    let focus = view.cursor.map(|(c, _)| c).or(view.playhead);
    let mut first = view.first.min(view.steps.saturating_sub(1));
    if let Some(f) = focus {
        if f < first {
            first = f;
        } else if f >= first + shown {
            first = f + 1 - shown;
        }
    }
    if first + shown > view.steps && view.steps >= shown {
        first = view.steps - shown;
    } else if view.steps < shown {
        first = 0;
    }
    out.first = first - first % align;
    out
}

/// El panel del piano roll.
pub struct RollPanel<'a>(pub &'a RollView);

/// Colores del piano roll (de `[colores]`).
fn note_color(style: NoteStyle, track: Color) -> Style {
    let t = theme::get();
    match style {
        NoteStyle::Scale => Style::new().fg(track),
        NoteStyle::Outside => Style::new().fg(t.roll_outside),
        NoteStyle::Variation => Style::new().fg(t.roll_variation).add_modifier(Modifier::ITALIC),
        NoteStyle::Generated => Style::new().fg(t.roll_generated),
        NoteStyle::Dropped => Style::new().fg(t.dim),
    }
}

/// Un cuadrante por bit: arriba a la izquierda, arriba a la derecha, abajo
/// a la izquierda y abajo a la derecha.
fn quadrant(bits: u8) -> char {
    const GLYPHS: [char; 16] = [
        ' ', '▘', '▝', '▀', '▖', '▌', '▞', '▛', '▗', '▚', '▐', '▜', '▄', '▙', '▟', '█',
    ];
    GLYPHS[(bits & 15) as usize]
}

/// La cabeza de una nota: su marca o el sombreado de su volumen.
fn head_glyph(note: &Note) -> char {
    if note.style == NoteStyle::Dropped {
        return '×';
    }
    match note.mark {
        ' ' => ['░', '▒', '▓', '█'][note.shade.min(3) as usize],
        m => m,
    }
}

impl Widget for RollPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let view = self.0;
        let t = theme::get();
        let track = t.highlight[view.color % t.highlight.len()];
        let lay = layout(view, area);
        let shown = lay.shown();
        let last = (lay.first + shown).min(view.steps);
        let mut title = format!(
            " piano roll · {} · {}{}",
            view.title,
            if view.zoom_auto { "auto " } else { "" },
            lay.zoom.name()
        );
        if shown < view.steps && view.steps > 0 {
            title.push_str(&format!(" · pasos {}–{} de {}", lay.first + 1, last, view.steps));
        }
        title.push(' ');
        let border = if view.cursor.is_some() {
            Style::new().fg(t.accent)
        } else {
            Style::new()
        };
        Block::bordered()
            .title(title)
            .border_style(border)
            .render(area, buf);
        let inner = Block::bordered().inner(area);
        if inner.height < 3 || lay.grid.width == 0 {
            if inner.height > 0 {
                buf.set_stringn(inner.x, inner.y, "(sin sitio)", inner.width as usize, Style::new().fg(t.dim));
            }
            return;
        }
        let g = lay.grid;
        let bar = view.bar_steps.max(1);
        let length = view.length.max(1);
        // Cabecera: compases, tiempos y `▼` en el paso que se oye.
        for col in lay.first..last {
            let Some(x) = lay.x_of(col) else { continue };
            if col % lay.zoom.steps_per_cell() != 0 {
                continue;
            }
            let (text, style) = if view
                .playhead
                .is_some_and(|p| p >= col && p < col + lay.zoom.steps_per_cell())
            {
                ("▼".to_string(), Style::new().fg(t.playing).bold())
            } else if col % length == 0 && length != bar {
                (format!("v{}", col / length + 1), Style::new().fg(t.accent))
            } else if col % bar == 0 {
                ((col / bar + 1).to_string(), Style::new().fg(t.accent))
            } else if col % 4 == 0 && lay.zoom != Zoom::Z4 {
                (":".to_string(), Style::new().fg(t.dim))
            } else {
                continue;
            };
            if x < g.right() {
                buf.set_stringn(x, inner.y, &text, (g.right() - x) as usize, style);
            }
        }
        // Las filas: nombre y fondo (tónica y escala).
        let row_bg = |pitch: i32| -> Option<Color> {
            if !view.notes_axis {
                return None;
            }
            match view.scale {
                Some(s) if pitch.rem_euclid(12) == s.tonic => Some(t.roll_tonic),
                Some(s) if s.contains(pitch) => Some(t.roll_scale),
                Some(_) => None,
                None if pitch.rem_euclid(12) == 0 => Some(t.roll_tonic),
                None => None,
            }
        };
        for r in 0..g.height {
            let y = g.y + r;
            let upper = lay.top - r as i32 * lay.per_row;
            let name = if view.notes_axis && lay.per_row == 2 {
                // La nota natural de la pareja (la de abajo si lo son las
                // dos) y en qué mitad está: `C4▄`, `G3▀`.
                let lower = upper - 1;
                let natural = |p: i32| !matches!(p.rem_euclid(12), 1 | 3 | 6 | 8 | 10);
                if natural(lower) {
                    format!("{}▄", engine::note_name(lower))
                } else {
                    format!("{}▀", engine::note_name(upper))
                }
            } else if view.notes_axis {
                engine::note_name(upper)
            } else {
                view.labels
                    .get(upper.max(0) as usize)
                    .cloned()
                    .unwrap_or_default()
            };
            let mut style = Style::new().fg(t.dim);
            if view.notes_axis
                && (upper.rem_euclid(12) == 0
                    || (lay.per_row == 2 && (upper - 1).rem_euclid(12) == 0))
            {
                style = Style::new().fg(t.accent);
            }
            if view.scale.is_some_and(|s| upper.rem_euclid(12) == s.tonic) {
                style = style.add_modifier(Modifier::BOLD);
            }
            if upper < view.bounds.0 - (lay.per_row - 1) || upper > view.bounds.1 {
                continue;
            }
            buf.set_stringn(inner.x, y, &name, lay.label.saturating_sub(1) as usize, style);
            // El fondo de la fila y las marcas de tiempo.
            for cx in 0..g.width {
                let x = g.x + cx;
                let col = lay.first
                    + match lay.zoom {
                        Zoom::Z32 => cx as usize / 2,
                        z => cx as usize * z.steps_per_cell(),
                    };
                if col >= view.steps {
                    break;
                }
                let bg = row_bg(upper).or_else(|| {
                    (lay.per_row == 2).then(|| row_bg(upper - 1)).flatten()
                });
                let mut cell = Style::new();
                if let Some(bg) = bg {
                    cell = cell.bg(bg);
                }
                let first_half = lay.zoom != Zoom::Z32 || cx % 2 == 0;
                let symbol = if !first_half {
                    " "
                } else if col.is_multiple_of(length) {
                    "┆"
                } else if col.is_multiple_of(bar) {
                    "┊"
                } else if col.is_multiple_of(4) && matches!(lay.zoom, Zoom::Z16 | Zoom::Z32) {
                    "·"
                } else {
                    " "
                };
                buf[(x, y)].set_symbol(symbol).set_style(cell.fg(t.dim));
            }
        }
        // La selección y el paso que se oye, de fondo.
        let tint = |buf: &mut Buffer, col: usize, color: Color| {
            if let Some(x) = lay.x_of(col) {
                let w = lay.zoom.cells_per_step() as u16;
                for dx in 0..w {
                    for y in g.y..g.bottom() {
                        if x + dx < g.right() {
                            buf[(x + dx, y)].set_bg(color);
                        }
                    }
                }
            }
        };
        if let Some((a, b)) = view.selection {
            for col in a..=b {
                tint(buf, col, t.roll_selection);
            }
        }
        if let Some(p) = view.playhead {
            tint(buf, p, t.playing);
        }
        // Las notas.
        let rich = lay.per_row == 1 && lay.zoom.steps_per_cell() == 1;
        if rich {
            for note in &view.notes {
                let Some(y) = lay.y_of(note.pitch) else { continue };
                let style = note_color(note.style, track);
                for k in 0..note.len {
                    let col = note.col + k;
                    let Some(x) = lay.x_of(col) else { continue };
                    let glyphs: [char; 2] = if k == 0 {
                        [head_glyph(note), if note.style == NoteStyle::Dropped { ' ' } else { '█' }]
                    } else {
                        ['━', '━']
                    };
                    for (dx, glyph) in glyphs.iter().enumerate().take(lay.zoom.cells_per_step()) {
                        let x = x + dx as u16;
                        if x < g.right() {
                            let mut s = style;
                            if k == 0 {
                                s = s.add_modifier(Modifier::BOLD);
                            }
                            buf[(x, y)].set_char(*glyph).set_style(s);
                        }
                    }
                }
            }
        } else {
            // Medias celdas y cuadrantes: por celda, qué cuartos ocupa
            // alguna nota y el estilo de la de más peso.
            let cells = g.width as usize * g.height as usize;
            let mut bits = vec![0u8; cells];
            let mut styles: Vec<Option<NoteStyle>> = vec![None; cells];
            let sub = match lay.zoom {
                Zoom::Z8 => 1,
                Zoom::Z4 => 2,
                _ => 0,
            };
            for note in &view.notes {
                if note.style == NoteStyle::Dropped {
                    continue;
                }
                let d = lay.top - note.pitch;
                if d < 0 || d >= lay.rows() {
                    continue;
                }
                let row = (d / lay.per_row) as usize;
                // Arriba (1) y abajo (2) de la celda.
                let vmask: u8 = match (lay.per_row, d % 2) {
                    (1, _) => 0b11,
                    (_, 0) => 0b01,
                    _ => 0b10,
                };
                for k in 0..note.len {
                    let col = note.col + k;
                    let Some(x) = lay.x_of(col) else { continue };
                    let cx = (x - g.x) as usize;
                    // Izquierda (1) y derecha (2): con 1/8, el paso; con
                    // 1/4, la pareja de pasos; si no, las dos.
                    let hmask: u8 = if sub == 0 {
                        0b11
                    } else {
                        let within = (col - lay.first) % lay.zoom.steps_per_cell();
                        if within < sub { 0b01 } else { 0b10 }
                    };
                    let mut mask = 0u8;
                    for v in 0..2 {
                        for h in 0..2 {
                            if vmask & (1 << v) != 0 && hmask & (1 << h) != 0 {
                                mask |= 1 << (v * 2 + h);
                            }
                        }
                    }
                    let i = row * g.width as usize + cx;
                    let w = lay.zoom.cells_per_step();
                    for dx in 0..w {
                        if cx + dx < g.width as usize {
                            bits[i + dx] |= mask;
                            let weight = |s: Option<NoteStyle>| match s {
                                Some(NoteStyle::Outside) => 4,
                                Some(NoteStyle::Scale) => 3,
                                Some(NoteStyle::Variation) => 2,
                                Some(NoteStyle::Generated) => 1,
                                _ => 0,
                            };
                            if weight(Some(note.style)) > weight(styles[i + dx]) {
                                styles[i + dx] = Some(note.style);
                            }
                        }
                    }
                }
            }
            for (i, b) in bits.iter().enumerate() {
                if *b == 0 {
                    continue;
                }
                let x = g.x + (i % g.width as usize) as u16;
                let y = g.y + (i / g.width as usize) as u16;
                let style = note_color(styles[i].unwrap_or(NoteStyle::Scale), track);
                buf[(x, y)].set_char(quadrant(*b)).set_fg(style.fg.unwrap_or(track));
            }
        }
        // El cursor, invertido.
        if let Some((col, pitch)) = view.cursor
            && let (Some(x), Some(y)) = (lay.x_of(col), lay.y_of(pitch))
        {
            for dx in 0..lay.zoom.cells_per_step() as u16 {
                if x + dx < g.right() {
                    let cell = &mut buf[(x + dx, y)];
                    if cell.symbol() == " " {
                        cell.set_symbol("▒");
                    }
                    cell.set_style(Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD));
                }
            }
        }
        // La línea de estado: el paso del cursor y la rejilla.
        let y = inner.bottom() - 1;
        let right = format!(" {} ", view.snap);
        let room = (inner.width as usize).saturating_sub(right.chars().count());
        buf.set_stringn(inner.x, y, &view.status, room, Style::new());
        let rx = inner.x + room as u16;
        if rx < inner.right() {
            buf.set_stringn(rx, y, &right, inner.right() as usize - rx as usize, Style::new().fg(t.dim));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quadrants_cover_every_combination() {
        assert_eq!(quadrant(0b0000), ' ');
        assert_eq!(quadrant(0b0011), '▀');
        assert_eq!(quadrant(0b1100), '▄');
        assert_eq!(quadrant(0b0101), '▌');
        assert_eq!(quadrant(0b1010), '▐');
        assert_eq!(quadrant(0b1111), '█');
    }

    #[test]
    fn zoom_goes_both_ways_and_stops() {
        assert_eq!(Zoom::Z16.step(true), Zoom::Z32);
        assert_eq!(Zoom::Z32.step(true), Zoom::Z32);
        assert_eq!(Zoom::Z16.step(false), Zoom::Z8);
        assert_eq!(Zoom::Z4.step(false), Zoom::Z4);
        assert_eq!(Zoom::from_name("1/8"), Some(Zoom::Z8));
    }

    fn view(steps: usize, range: (i32, i32)) -> RollView {
        RollView {
            notes_axis: true,
            steps,
            length: 16,
            bar_steps: 16,
            range: Some(range),
            bounds: (24, 96),
            ..RollView::default()
        }
    }

    #[test]
    fn the_layout_maps_cells_both_ways() {
        let v = view(32, (48, 60));
        let lay = layout(&v, Rect::new(0, 0, 40, 20));
        // 18 filas para 15 notas: una por semitono.
        assert_eq!(lay.per_row, 1);
        let (x, y) = (lay.x_of(5).unwrap(), lay.y_of(55).unwrap());
        assert_eq!(lay.hit(x, y), Some((5, 55)));
        // Con poca altura, dos semitonos por fila.
        let lay = layout(&v, Rect::new(0, 0, 40, 8));
        assert_eq!(lay.per_row, 2);
        // Cada fila, una nota impar y la de debajo.
        assert_eq!(lay.top.rem_euclid(2), 1);
        let top = lay.top;
        assert_eq!(lay.y_of(top), lay.y_of(top - 1));
        assert_ne!(lay.y_of(top - 1), lay.y_of(top - 2));
    }

    #[test]
    fn the_layout_follows_the_cursor() {
        let mut v = view(64, (48, 60));
        v.cursor = Some((60, 50));
        let lay = layout(&v, Rect::new(0, 0, 30, 20));
        assert!(lay.x_of(60).is_some());
        v.cursor = Some((0, 90));
        let lay = layout(&v, Rect::new(0, 0, 30, 10));
        assert!(lay.x_of(0).is_some() && lay.y_of(90).is_some());
        v.zoom = Zoom::Z4;
        let lay = layout(&v, Rect::new(0, 0, 30, 10));
        assert_eq!(lay.first, 0);
        assert!(lay.shown() >= 64);
    }
}
