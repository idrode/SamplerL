//! Vista de ocupación por paso: una fila por pista y una columna por
//! semicorchea de la canción, golpe o silencio, y debajo cuántas pistas se
//! oyen en cada paso. Sirve para ver los huecos entre pistas (breakcore,
//! sonidos que se alternan).
//!
//! Lo que se enseña es lo que va a sonar, no el texto: los pasos salen del
//! ciclo de vueltas que recibe el motor ([`script::TrackDef::bars`], con
//! `every`, `fill` y los dados de `chance` ya tirados al cargar), y cada
//! pista se coloca en la página con el último paso que publicó el motor
//! ([`engine::Cursor`]: de qué carga es y en qué punto de su ciclo va), así
//! que valen las longitudes distintas (`:length(n)`), las vueltas, las
//! secciones y el arranque en un compás. Parado, como el tracker: desde el
//! principio de la sección.
//!
//! Una página tiene tantos compases como hagan falta para que todas las
//! pistas vuelvan a coincidir (el mínimo común múltiplo del compás y de
//! `longitud · vueltas` de cada una), como mucho [`MAX_PAGE`] pasos; si no
//! cabe, el de las longitudes solas (una vuelta de `chance` es de 16
//! compases: se ve compás a compás, según suena).

use engine::Step;
use script::{Song, TrackDef};

use crate::highlight::Cursors;

/// Pasos como mucho de una página (cuatro compases de 4/4).
pub const MAX_PAGE: usize = 64;

/// Lo que hay en un paso de una pista.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    /// Nada.
    Silence,
    /// Un golpe: su intensidad (de 0 a 3, por su volumen `@`) y su
    /// retrigger (`rN`; 1 sin él).
    Hit { level: u8, retrig: u8 },
    /// Sigue la nota de un paso anterior (`_`).
    Held,
    /// En el patrón base hay un golpe, pero en esta vuelta no suena
    /// (`chance` con `"mute"`, o `every` que lo quita).
    Dropped,
}

impl Cell {
    /// Suena algo de la pista en este paso.
    pub fn sounds(self) -> bool {
        matches!(self, Cell::Hit { .. } | Cell::Held)
    }

    /// El carácter de la celda: el sombreado por intensidad (`░▒▓█`), la
    /// cifra del retrigger, `─` la nota alargada, `×` el golpe quitado y
    /// `·` el silencio.
    pub fn glyph(self) -> char {
        const SHADES: [char; 4] = ['░', '▒', '▓', '█'];
        match self {
            Cell::Hit { retrig, .. } if retrig > 9 => '+',
            Cell::Hit { retrig, .. } if retrig > 1 => char::from(b'0' + retrig),
            Cell::Hit { level, .. } => SHADES[level.min(3) as usize],
            Cell::Held => '─',
            Cell::Dropped => '×',
            Cell::Silence => '·',
        }
    }
}

/// Intensidad de un golpe por su volumen: hasta 0,3, 0,6, 0,9 y más.
pub fn level(gain: f32) -> u8 {
    match gain {
        g if g <= 0.3 => 0,
        g if g <= 0.6 => 1,
        g if g <= 0.9 => 2,
        _ => 3,
    }
}

/// Una pista de la rejilla.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Row {
    pub name: String,
    /// Su color: el orden de la pista en el script (como el resaltado).
    pub color: usize,
    /// Se oye (sin mute, ni solo de otra).
    pub audible: bool,
    pub cells: Vec<Cell>,
    /// Por columna, la vuelta del ciclo y el paso de la pista que suenan
    /// ahí (para escribir en su token).
    pub at: Vec<(usize, usize)>,
}

/// La página de la rejilla.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Grid {
    /// Compás (desde 0, como `Status::bar`) en que empieza la página.
    pub bar: u64,
    /// Pasos de un compás.
    pub bar_steps: usize,
    pub rows: Vec<Row>,
    /// Pistas que se oyen en cada paso.
    pub counts: Vec<usize>,
    /// Columna del paso que se oye, con el secuenciador en marcha.
    pub current: Option<usize>,
}

impl Grid {
    /// Pasos de la página.
    pub fn steps(&self) -> usize {
        self.counts.len()
    }

    /// Columnas donde no suena ninguna pista.
    pub fn gaps(&self) -> Vec<usize> {
        (0..self.counts.len())
            .filter(|&c| self.counts[c] == 0)
            .collect()
    }
}

/// Lo que hace falta para construir la rejilla.
pub struct Source<'a> {
    pub song: &'a Song,
    /// Las pistas que suenan (las de la sección, si la hay), en su orden.
    pub tracks: &'a [TrackDef],
    pub cursors: &'a Cursors,
    pub status: Option<engine::Status>,
    /// Lo que se oye, en pasos (con el retardo a mano ya restado), como en
    /// el resaltado de la fase M.
    pub heard: Option<f64>,
    /// La pista del motor de cada nombre.
    pub slot: &'a dyn Fn(&str) -> Option<usize>,
    /// Si se oye cada pista de `tracks` (mute/solo en vivo).
    pub audible: &'a [bool],
}

/// Pasos de una página para estas pistas.
pub fn page_len(tracks: &[TrackDef], bar_steps: usize) -> usize {
    let bar = bar_steps.max(1);
    let fit = |page: usize| page <= MAX_PAGE;
    let cycles = tracks.iter().fold(bar, |acc, t| {
        lcm(acc, t.length.max(1) * t.bars.len().max(1))
    });
    if fit(cycles) {
        return cycles;
    }
    let lengths = tracks.iter().fold(bar, |acc, t| lcm(acc, t.length.max(1)));
    if fit(lengths) {
        return lengths;
    }
    (MAX_PAGE / bar).max(1) * bar
}

/// La página que se oye (o, parada, la del último paso).
pub fn grid(src: &Source) -> Grid {
    let bar_steps = src.status.map_or(engine::STEPS, |s| s.bar_steps.max(1));
    let page = page_len(src.tracks, bar_steps);
    let page_bars = (page / bar_steps).max(1) as i64;
    let running = src.status.is_some_and(|s| s.running);
    // El paso que se oye, y dónde empieza su compás.
    let (global, step, bar, section_bar) = src.status.map_or((0, 0, 0, 0), |s| {
        (
            s.global as i64,
            s.step.unwrap_or(0) as i64,
            s.bar as i64,
            s.section.map_or(s.bar as i64, |p| p.bar as i64),
        )
    });
    let anchor = match src.heard.filter(|_| running) {
        Some(h) => (h.floor() as i64).max(0),
        None => global,
    };
    let into = step + (anchor - global);
    let bars_back = into.div_euclid(bar_steps as i64);
    let bar_start = anchor - into.rem_euclid(bar_steps as i64);
    let back = (section_bar + bars_back).rem_euclid(page_bars);
    let start = bar_start - back * bar_steps as i64;
    // Principio de la sección (las pistas cuentan sus vueltas desde ahí).
    let origin = global - (section_bar * bar_steps as i64 + step);
    let names = src.song.track_names();

    let rows: Vec<Row> = src
        .tracks
        .iter()
        .enumerate()
        .map(|(i, def)| {
            let cursor = running
                .then(|| (src.slot)(&def.name))
                .flatten()
                .and_then(|slot| src.cursors.at(slot, anchor).or(src.cursors.last(slot)));
            let played = cursor.and_then(|c| {
                src.song
                    .track_by_tag(c.tag)
                    .filter(|t| t.name == def.name)
                    .map(|t| (t, c))
            });
            let (cells, at) = (0..page as i64)
                .map(|col| {
                    let abs = start + col;
                    match played {
                        Some((t, c)) => {
                            let cycle = (t.length.max(1) * t.bars.len().max(1)) as i64;
                            let index =
                                (c.index as i64 + abs - c.global as i64).rem_euclid(cycle) as usize;
                            cell(t, index)
                        }
                        None => {
                            let (lap, k) = def.lap((abs - origin).max(0) as u64);
                            let laps = def.bars.len().max(1) as u64;
                            cell(def, (lap % laps) as usize * def.length.max(1) + k)
                        }
                    }
                })
                .unzip();
            Row {
                name: def.name.clone(),
                color: names.iter().position(|n| *n == def.name).unwrap_or(i),
                audible: src.audible.get(i).copied().unwrap_or(true),
                cells,
                at,
            }
        })
        .collect();
    let counts = (0..page)
        .map(|c| {
            rows.iter()
                .filter(|r| r.audible && r.cells[c].sounds())
                .count()
        })
        .collect();
    let current = running
        .then_some(anchor - start)
        .filter(|c| (0..page as i64).contains(c))
        .map(|c| c as usize);
    Grid {
        bar: (bar + bars_back - back).max(0) as u64,
        bar_steps,
        rows,
        counts,
        current,
    }
}

/// Sugerencias como mucho de una vez.
pub const MAX_SUGGESTIONS: usize = 4;

/// Una sugerencia de relleno: qué golpe de qué pista iría en un hueco.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    /// Fila (pista) y columna de la página, y el compás en que empieza la
    /// página (para marcarla solo en la suya).
    pub row: usize,
    pub col: usize,
    pub bar: u64,
    /// El golpe, como se escribiría (`x`, `2@0.5`...).
    pub hit: String,
    /// Puntuación: cuanto menos, mejor (ver [`suggest`]).
    pub score: f32,
    /// Por qué, en una frase.
    pub why: String,
}

/// Lo que cuesta que la pista que rellena suene también en un paso vecino:
/// se busca que los sonidos se alternen.
const REPEAT_COST: f32 = 0.5;
/// Lo que cuesta una pista que llena toda la página (por la fracción de
/// pasos en que suena): mejor la que tiene sitio.
const DENSITY_COST: f32 = 0.25;

/// Hasta [`MAX_SUGGESTIONS`] huecos de la página y qué golpe los
/// rellenaría. Para cada hueco (un paso donde no se oye nada), cada pista
/// que se oye y que se puede rellenar ahí (`hit` da su golpe; `None` si no
/// se puede) se puntúa con la suma de: el choque de frecuencia más fuerte
/// con las pistas que suenan en el paso de antes y el de después (`clash`:
/// la puntuación de 0 a 1 del análisis de choques de la pareja; `None` sin
/// análisis, y no cuenta), [`REPEAT_COST`] si ella misma suena en un paso
/// vecino y [`DENSITY_COST`] por la fracción de la página en que suena. En
/// cada hueco, la mejor; de los huecos, los de menos puntuación (y, a
/// igualdad, los primeros), en el orden de la página.
pub fn suggest(
    grid: &Grid,
    hit: impl Fn(usize, usize) -> Option<String>,
    clash: impl Fn(&str, &str) -> Option<f32>,
) -> Vec<Suggestion> {
    let steps = grid.steps();
    if steps == 0 {
        return Vec::new();
    }
    let sounding = |col: usize| -> Vec<usize> {
        (0..grid.rows.len())
            .filter(|&r| grid.rows[r].audible && grid.rows[r].cells[col].sounds())
            .collect()
    };
    let mut best: Vec<Suggestion> = grid
        .gaps()
        .into_iter()
        .filter_map(|col| {
            let before = sounding((col + steps - 1) % steps);
            let after = sounding((col + 1) % steps);
            let neighbours: Vec<usize> = before.iter().chain(&after).copied().collect();
            grid.rows
                .iter()
                .enumerate()
                .filter(|(_, row)| row.audible)
                .filter_map(|(r, row)| {
                    let token = hit(r, col)?;
                    let worst = neighbours
                        .iter()
                        .filter(|&&n| n != r)
                        .filter_map(|&n| clash(&row.name, &grid.rows[n].name).map(|s| (s, n)))
                        .max_by(|a, b| a.0.total_cmp(&b.0));
                    let repeats = neighbours.contains(&r);
                    let density =
                        row.cells.iter().filter(|c| c.sounds()).count() as f32 / steps as f32;
                    let score = worst.map_or(0.0, |(s, _)| s)
                        + if repeats { REPEAT_COST } else { 0.0 }
                        + DENSITY_COST * density;
                    let mut why = Vec::new();
                    let others: Vec<&str> = neighbours
                        .iter()
                        .filter(|&&n| n != r)
                        .map(|&n| grid.rows[n].name.as_str())
                        .fold(Vec::new(), |mut acc, n| {
                            if !acc.contains(&n) {
                                acc.push(n);
                            }
                            acc
                        });
                    if !others.is_empty() {
                        why.push(format!("alterna con {}", others.join(" y ")));
                    }
                    if repeats {
                        why.push("repite su sonido al lado".into());
                    }
                    match worst {
                        Some((s, n)) => {
                            why.push(format!("choque con {} {s:.2}", grid.rows[n].name))
                        }
                        None if !others.is_empty() => why.push("sin choque medido".into()),
                        None => {}
                    }
                    if neighbours.is_empty() {
                        why.push("entre silencios".into());
                    }
                    Some(Suggestion {
                        row: r,
                        col,
                        bar: grid.bar,
                        hit: token,
                        score,
                        why: why.join(", "),
                    })
                })
                .min_by(|a, b| a.score.total_cmp(&b.score))
        })
        .collect();
    best.sort_by(|a, b| a.score.total_cmp(&b.score).then(a.col.cmp(&b.col)));
    best.truncate(MAX_SUGGESTIONS);
    best.sort_by_key(|s| s.col);
    best
}

/// La celda del paso `index` del ciclo aplanado de una pista, con su
/// vuelta y su paso.
fn cell(def: &TrackDef, index: usize) -> (Cell, (usize, usize)) {
    let length = def.length.max(1);
    let (lap, k) = (index / length, index % length);
    let at = (lap, k);
    let Some(bar) = def.bars.get(lap) else {
        return (Cell::Silence, at);
    };
    let cell = match bar.get(k).copied().flatten() {
        Some(step) => hit(&step),
        None if held(bar, k) => Cell::Held,
        None if def.steps.get(k).is_some_and(Option::is_some) => Cell::Dropped,
        None => Cell::Silence,
    };
    (cell, at)
}

fn hit(step: &Step) -> Cell {
    Cell::Hit {
        level: level(step.gain),
        retrig: step.retrig.max(1),
    }
}

/// El paso `k` alarga (`_`) un golpe anterior de la misma vuelta.
fn held(bar: &[Option<Step>], k: usize) -> bool {
    match bar[..k].iter().rposition(Option::is_some) {
        Some(j) => bar[j].is_some_and(|s| s.hold as usize >= k - j),
        None => false,
    }
}

fn lcm(a: usize, b: usize) -> usize {
    fn gcd(a: usize, b: usize) -> usize {
        if b == 0 { a } else { gcd(b, a % b) }
    }
    a / gcd(a, b) * b
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(name: &str, cells: &str) -> Row {
        let cells: Vec<Cell> = cells
            .chars()
            .map(|c| match c {
                'x' => Cell::Hit {
                    level: 3,
                    retrig: 1,
                },
                _ => Cell::Silence,
            })
            .collect();
        Row {
            name: name.into(),
            color: 0,
            audible: true,
            at: (0..cells.len()).map(|k| (0, k)).collect(),
            cells,
        }
    }

    fn page(rows: Vec<Row>) -> Grid {
        let steps = rows[0].cells.len();
        let counts = (0..steps)
            .map(|c| rows.iter().filter(|r| r.cells[c].sounds()).count())
            .collect();
        Grid {
            bar: 0,
            bar_steps: steps,
            rows,
            counts,
            current: None,
        }
    }

    #[test]
    fn suggestions_alternate_sounds_and_avoid_clashes() {
        // kick en 0 y 4, snare en 2 y 6, hats en 2: huecos en 1, 3, 5 y 7.
        let grid = page(vec![
            row("kick", "x...x..."),
            row("snare", "..x...x."),
            row("hats", "..x....."),
        ]);
        let any = |_: usize, _: usize| Some("1".to_string());
        // Sin análisis: en cada hueco, la que no suena al lado y tiene más
        // sitio (hats en 1, 3 y 5; en 7, al lado de snare y kick, hats).
        let got = suggest(&grid, any, |_, _| None);
        assert_eq!(got.len(), 4);
        assert!(got.iter().all(|s| s.row == 2), "{got:?}");
        assert_eq!(got.iter().map(|s| s.col).collect::<Vec<_>>(), [1, 3, 5, 7]);
        assert!(
            got[0].why.contains("alterna con kick y snare"),
            "{}",
            got[0].why
        );
        assert!(got[0].why.contains("sin choque medido"), "{}", got[0].why);
        // Con un choque fuerte entre hats y kick, en los huecos junto a kick
        // gana snare aunque suene más.
        let clash = |a: &str, b: &str| {
            let pair = [a, b];
            Some(if pair.contains(&"hats") && pair.contains(&"kick") {
                0.9
            } else {
                0.1
            })
        };
        let got = suggest(&grid, any, clash);
        let at = |col: usize| got.iter().find(|s| s.col == col).map(|s| s.row);
        assert_eq!(at(1), Some(1), "{got:?}");
        assert!(
            got.iter()
                .find(|s| s.col == 1)
                .unwrap()
                .why
                .contains("choque con")
        );
        // Donde no se puede rellenar, nada; nunca más de 4.
        let none = suggest(&grid, |_, _| None, |_, _| None);
        assert!(none.is_empty());
        let wide = page(vec![
            row("kick", "x.x.x.x.x.x.x.x."),
            row("hats", "................"),
        ]);
        assert_eq!(suggest(&wide, any, |_, _| None).len(), MAX_SUGGESTIONS);
    }

    #[test]
    fn glyphs_show_level_retrig_and_silences() {
        assert_eq!(
            Cell::Hit {
                level: 3,
                retrig: 1
            }
            .glyph(),
            '█'
        );
        assert_eq!(
            Cell::Hit {
                level: 0,
                retrig: 1
            }
            .glyph(),
            '░'
        );
        assert_eq!(
            Cell::Hit {
                level: 2,
                retrig: 4
            }
            .glyph(),
            '4'
        );
        assert_eq!(
            Cell::Hit {
                level: 2,
                retrig: 16
            }
            .glyph(),
            '+'
        );
        assert_eq!(Cell::Held.glyph(), '─');
        assert_eq!(Cell::Dropped.glyph(), '×');
        assert_eq!(Cell::Silence.glyph(), '·');
        assert_eq!(level(1.0), 3);
        assert_eq!(level(0.5), 1);
        assert_eq!(level(0.25), 0);
        assert_eq!(level(0.8), 2);
        assert!(Cell::Held.sounds() && !Cell::Dropped.sounds());
    }
}
