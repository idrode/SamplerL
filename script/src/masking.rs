//! Choques de frecuencia entre las pistas de una canción: cada pista
//! renderizada en solo (offline, en un motor aparte) durante un ciclo
//! entero de la canción, su espectro por tercios de octava y la puntuación
//! de cada pareja en cada banda ([`engine::masking`]). Todo en el hilo que
//! lo pide; el motor que suena no se toca.

use engine::masking::{
    BAND_NAMES, BANDS, BandClash, BandSpectrum, MaskingOptions, band_center, band_edges, clashes,
};

use crate::{MAX_SONG_BARS, ScriptError, Song};

/// Resultado del análisis de una canción.
#[derive(Debug, Clone)]
pub struct MaskingReport {
    /// Compases analizados: el ciclo entero de la canción (lo que tarda en
    /// repetirse, o la línea de tiempo), como mucho [`MAX_SONG_BARS`], el
    /// mismo tope que la exportación.
    pub bars: u64,
    /// Pistas, en el orden de la canción.
    pub tracks: Vec<String>,
    pub options: MaskingOptions,
    /// Cada pareja de pistas (`a` < `b`, índices de `tracks`).
    pub pairs: Vec<PairClashes>,
    /// `priority()` de la canción: la primera manda.
    pub priority: Vec<String>,
    /// Recortes sugeridos: un EQ por pista que cede, en el orden de las
    /// pistas.
    pub suggestions: Vec<Suggestion>,
}

/// Recortes como mucho por pista: las bandas de un EQ.
pub const MAX_CUTS: usize = engine::EQ_BANDS;
/// Profundidad de un recorte: de 2 dB (un choque en el umbral de informe)
/// a 4 dB (desde [`FULL_CUT_SCORE`]).
pub const MIN_CUT_DB: f32 = 2.0;
pub const MAX_CUT_DB: f32 = 4.0;
pub const FULL_CUT_SCORE: f32 = 0.75;
/// Q de un recorte: una banda de tercio de octava da 4,3; varias seguidas,
/// menos (más ancho), sin bajar de 0,7.
pub const MIN_CUT_Q: f32 = 0.7;

/// El EQ sugerido para una pista que cede en uno o más choques.
#[derive(Debug, Clone, PartialEq)]
pub struct Suggestion {
    pub track: usize,
    /// Como mucho [`MAX_CUTS`], de grave a agudo.
    pub cuts: Vec<Cut>,
}

/// Un recorte en campana sobre una o varias bandas seguidas.
#[derive(Debug, Clone, PartialEq)]
pub struct Cut {
    /// Bandas que cubre (primera y última).
    pub bands: (usize, usize),
    pub freq_hz: f32,
    pub q: f32,
    /// Negativa: de −[`MIN_CUT_DB`] a −[`MAX_CUT_DB`], en medios dB.
    pub gain_db: f32,
    /// La puntuación más alta de los choques que lo piden.
    pub score: f32,
    /// Las pistas que mandan en esas bandas.
    pub against: Vec<usize>,
}

/// Choques de una pareja de pistas, banda a banda.
#[derive(Debug, Clone)]
pub struct PairClashes {
    pub a: usize,
    pub b: usize,
    /// `None`: banda casi vacía para la pareja.
    pub bands: [Option<BandClash>; BANDS],
}

/// Un choque por encima del umbral de informe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Clash {
    pub a: usize,
    pub b: usize,
    pub band: usize,
    pub clash: BandClash,
}

impl Song {
    /// Analiza los choques de frecuencia entre sus pistas a `sample_rate`.
    pub fn analyze_masking(
        &self,
        sample_rate: u32,
        options: &MaskingOptions,
    ) -> Result<MaskingReport, ScriptError> {
        let bars = self.length_bars(1).min(MAX_SONG_BARS);
        let tracks: Vec<String> = self.track_names().into_iter().map(String::from).collect();
        // Un compás más: las últimas ventanas del ciclo leen cómo vuelve a
        // empezar, en vez de un corte en seco.
        let cycle = self.frames(sample_rate, bars) as usize;
        let spectra = tracks
            .iter()
            .map(|name| {
                let audio = self
                    .render_track(sample_rate, bars + 1, name)
                    .expect("la pista es de la canción")?;
                Ok(BandSpectrum::analyze_cycle(
                    &audio,
                    cycle,
                    sample_rate,
                    options,
                ))
            })
            .collect::<Result<Vec<_>, ScriptError>>()?;
        let mut pairs = Vec::new();
        for a in 0..tracks.len() {
            for b in a + 1..tracks.len() {
                pairs.push(PairClashes {
                    a,
                    b,
                    bands: clashes(&spectra[a], &spectra[b], options),
                });
            }
        }
        let mut report = MaskingReport {
            bars,
            tracks,
            options: *options,
            pairs,
            priority: self.priority.clone(),
            suggestions: Vec::new(),
        };
        report.suggestions = report.suggest();
        Ok(report)
    }
}

impl MaskingReport {
    /// La pista que se queda la banda en un choque: la de más prioridad
    /// (`priority()`); si ninguna está o solo una, la de la lista; sin
    /// lista, la que suena más en la banda (y con menos de medio dB de
    /// diferencia, la que se definió antes).
    pub fn winner(&self, clash: &Clash) -> usize {
        let rank = |t: usize| self.priority.iter().position(|n| *n == self.tracks[t]);
        match (rank(clash.a), rank(clash.b)) {
            (Some(x), Some(y)) => {
                if x < y {
                    clash.a
                } else {
                    clash.b
                }
            }
            (Some(_), None) => clash.a,
            (None, Some(_)) => clash.b,
            (None, None) => {
                let [la, lb] = clash.clash.level_db;
                if (la - lb).abs() < 0.5 || la > lb {
                    clash.a
                } else {
                    clash.b
                }
            }
        }
    }

    /// La pista que cede en un choque (la otra).
    pub fn loser(&self, clash: &Clash) -> usize {
        if self.winner(clash) == clash.a {
            clash.b
        } else {
            clash.a
        }
    }

    /// Los recortes de cada pista que cede: por banda, el choque más
    /// fuerte; las bandas seguidas, un solo recorte más ancho; como mucho
    /// [`MAX_CUTS`] por pista (los de más puntuación).
    fn suggest(&self) -> Vec<Suggestion> {
        let mut out = Vec::new();
        let clashes = self.clashes();
        for track in 0..self.tracks.len() {
            let mut bands: [Option<(f32, Vec<usize>)>; BANDS] = std::array::from_fn(|_| None);
            for clash in clashes.iter().filter(|c| self.loser(c) == track) {
                let winner = self.winner(clash);
                let entry = bands[clash.band].get_or_insert((0.0, Vec::new()));
                entry.0 = entry.0.max(clash.clash.score);
                if !entry.1.contains(&winner) {
                    entry.1.push(winner);
                }
            }
            let mut cuts = Vec::new();
            let mut band = 0;
            while band < BANDS {
                if bands[band].is_none() {
                    band += 1;
                    continue;
                }
                let first = band;
                let (mut score, mut against) = (0.0f32, Vec::new());
                while let Some(Some((s, w))) = bands.get(band) {
                    score = score.max(*s);
                    for t in w {
                        if !against.contains(t) {
                            against.push(*t);
                        }
                    }
                    band += 1;
                }
                against.sort_unstable();
                cuts.push(self.cut(first, band - 1, score, against));
            }
            if cuts.is_empty() {
                continue;
            }
            // Los más fuertes (y, a igual puntuación, los más anchos).
            cuts.sort_by(|x, y| {
                y.score
                    .total_cmp(&x.score)
                    .then((y.bands.1 - y.bands.0).cmp(&(x.bands.1 - x.bands.0)))
            });
            cuts.truncate(MAX_CUTS);
            cuts.sort_by(|x, y| x.freq_hz.total_cmp(&y.freq_hz));
            out.push(Suggestion { track, cuts });
        }
        out
    }

    /// Recorte en campana de la banda `first` a la `last`.
    fn cut(&self, first: usize, last: usize, score: f32, against: Vec<usize>) -> Cut {
        let (lo, hi) = (band_edges(first).0, band_edges(last).1);
        // Una sola banda: su frecuencia nominal (250, no 251,2).
        let centre = if first == last {
            nominal(first)
        } else {
            (lo * hi).sqrt()
        };
        let q = ((lo * hi).sqrt() / (hi - lo)).max(MIN_CUT_Q);
        let report = self.options.report_score;
        let t = ((score - report) / (FULL_CUT_SCORE - report)).clamp(0.0, 1.0);
        let depth = MIN_CUT_DB + t * (MAX_CUT_DB - MIN_CUT_DB);
        Cut {
            bands: (first, last),
            freq_hz: round_freq(centre),
            q: (q * 10.0).round() / 10.0,
            gain_db: -(depth * 2.0).round() / 2.0,
            score,
            against,
        }
    }

    /// La sugerencia para la pista `name`, si cede en algún choque.
    pub fn suggestion(&self, name: &str) -> Option<&Suggestion> {
        self.suggestions
            .iter()
            .find(|s| self.tracks[s.track] == name)
    }

    /// El `fx("eq", {...})` de una sugerencia, como se escribe en Lua.
    pub fn eq_table(&self, suggestion: &Suggestion) -> String {
        let fields: Vec<String> = suggestion
            .cuts
            .iter()
            .enumerate()
            .map(|(i, c)| {
                let n = i + 1;
                format!(
                    "freq{n} = {}, gain{n} = {}, q{n} = {}",
                    number(c.freq_hz),
                    number(c.gain_db),
                    number(c.q)
                )
            })
            .collect();
        format!("fx(\"eq\", {{ {} }})", fields.join(", "))
    }

    /// La línea de Lua que aplica una sugerencia:
    /// `track("b"):fx("eq", { ... })`.
    pub fn lua_line(&self, suggestion: &Suggestion) -> String {
        format!(
            "track(\"{}\"):{}",
            self.tracks[suggestion.track],
            self.eq_table(suggestion)
        )
    }

    /// Una línea por recorte: dónde, cuánto y frente a qué.
    pub fn cut_lines(&self, suggestion: &Suggestion) -> Vec<String> {
        suggestion
            .cuts
            .iter()
            .map(|c| {
                let range = if c.bands.0 == c.bands.1 {
                    format!("{} Hz", BAND_NAMES[c.bands.0])
                } else {
                    format!("{}-{} Hz", BAND_NAMES[c.bands.0], BAND_NAMES[c.bands.1])
                };
                let against: Vec<&str> =
                    c.against.iter().map(|&t| self.tracks[t].as_str()).collect();
                format!(
                    "{range}: {} dB, q {} (frente a {}, {})",
                    number(c.gain_db),
                    number(c.q),
                    against.join(", "),
                    score_text(c.score)
                )
            })
            .collect()
    }
    /// Puntuación de la pareja (por nombre, en cualquier orden) en la banda
    /// `band`; `None` si no existe o la banda está vacía.
    pub fn score(&self, a: &str, b: &str, band: usize) -> Option<f32> {
        let index = |n: &str| self.tracks.iter().position(|t| t == n);
        let (a, b) = (index(a)?, index(b)?);
        let pair = self
            .pairs
            .iter()
            .find(|p| (p.a, p.b) == (a.min(b), a.max(b)))?;
        pair.bands[band].map(|c| c.score)
    }

    /// Los choques que pasan de [`MaskingOptions::report_score`], del más
    /// fuerte al más débil.
    pub fn clashes(&self) -> Vec<Clash> {
        let mut out: Vec<Clash> = self
            .pairs
            .iter()
            .flat_map(|p| {
                p.bands.iter().enumerate().filter_map(move |(band, c)| {
                    c.filter(|c| c.score >= self.options.report_score)
                        .map(|clash| Clash {
                            a: p.a,
                            b: p.b,
                            band,
                            clash,
                        })
                })
            })
            .collect();
        out.sort_by(|x, y| y.clash.score.total_cmp(&x.clash.score));
        out
    }

    /// Una línea por choque: pareja, banda, puntuación y un resumen.
    pub fn clash_line(&self, clash: &Clash) -> (String, String, String) {
        let pair = format!("{} × {}", self.tracks[clash.a], self.tracks[clash.b]);
        let band = format!("{} Hz", BAND_NAMES[clash.band]);
        (pair, band, self.summary(clash))
    }

    /// Resumen en texto de un choque: lo fuerte que es, cuánto tiempo
    /// coinciden y cuál suena más en la banda.
    pub fn summary(&self, clash: &Clash) -> String {
        let c = clash.clash;
        let strength = strength(c.score);
        let [la, lb] = c.level_db;
        let gap = (la - lb).abs();
        let louder = if gap < 1.0 {
            "las dos igual de fuertes".to_string()
        } else {
            let name = if la > lb { clash.a } else { clash.b };
            format!("{} {gap:.0} dB más fuerte", self.tracks[name])
        };
        format!(
            "{} {strength}: a la vez el {:.0} % del tiempo, {louder}; cede {}",
            score_text(c.score),
            100.0 * c.overlap,
            self.tracks[self.loser(clash)]
        )
    }

    /// Informe en texto (para la línea de órdenes): cabecera, una línea
    /// por choque y una tabla por pareja con las bandas que chocan.
    pub fn text(&self) -> String {
        let mut out = format!(
            "choques de frecuencia: {} pistas, {} (ventanas de {} ms; activa a menos de {} dB de su máximo; choque hasta {} dB de diferencia; informe desde {})\n",
            self.tracks.len(),
            crate::bars_text(self.bars),
            self.options.window_ms,
            self.options.active_range_db,
            self.options.level_gap_db,
            score_text(self.options.report_score)
        );
        let clashes = self.clashes();
        if self.tracks.len() < 2 {
            out.push_str("hace falta más de una pista\n");
            return out;
        }
        if clashes.is_empty() {
            out.push_str("sin choques\n");
            return out;
        }
        let rows: Vec<(String, String, String)> =
            clashes.iter().map(|c| self.clash_line(c)).collect();
        let pair_width = rows.iter().map(|r| r.0.chars().count()).max().unwrap_or(0);
        let band_width = rows.iter().map(|r| r.1.chars().count()).max().unwrap_or(0);
        for (pair, band, summary) in &rows {
            out.push_str(&format!(
                "{pair:<pair_width$}  {band:>band_width$}  {summary}\n"
            ));
        }
        out.push('\n');
        out.push_str(&self.table());
        out.push('\n');
        out.push_str(&self.suggestions_text());
        out
    }

    /// Las sugerencias en texto: con qué prioridad, y por cada pista que
    /// cede, su línea de Lua y sus recortes.
    pub fn suggestions_text(&self) -> String {
        let mut out = if self.priority.is_empty() {
            "sugerencias (sin priority(): en cada banda manda la pista más fuerte):\n".to_string()
        } else {
            format!(
                "sugerencias (priority: {}; las demás, por su nivel):\n",
                self.priority.join(", ")
            )
        };
        for suggestion in &self.suggestions {
            out.push_str(&format!("  {}\n", self.lua_line(suggestion)));
            for line in self.cut_lines(suggestion) {
                out.push_str(&format!("    -- {line}\n"));
            }
        }
        out
    }

    /// Tabla pareja × banda: las bandas que chocan en alguna pareja (de
    /// grave a agudo) y una fila por pareja con algún choque, con una celda
    /// por cada una de esas bandas.
    pub fn grid(&self) -> (Vec<usize>, Vec<GridRow>) {
        let clashes = self.clashes();
        let mut bands: Vec<usize> = clashes.iter().map(|c| c.band).collect();
        bands.sort_unstable();
        bands.dedup();
        let mut pairs: Vec<(usize, usize)> = clashes.iter().map(|c| (c.a, c.b)).collect();
        pairs.sort_unstable();
        pairs.dedup();
        let rows = pairs
            .into_iter()
            .map(|(a, b)| {
                let pair = self.pairs.iter().find(|p| (p.a, p.b) == (a, b)).unwrap();
                let cells = bands
                    .iter()
                    .map(|&band| match pair.bands[band] {
                        None => Cell::Empty,
                        Some(c) if c.score < self.options.report_score => Cell::Quiet,
                        Some(c) => Cell::Clash(c.score),
                    })
                    .collect();
                ((a, b), cells)
            })
            .collect();
        (bands, rows)
    }

    /// La tabla de [`MaskingReport::grid`] en texto (`·` sin choque, `-`
    /// banda vacía).
    pub fn table(&self) -> String {
        let (bands, rows) = self.grid();
        let names: Vec<String> = rows
            .iter()
            .map(|&((a, b), _)| format!("{} × {}", self.tracks[a], self.tracks[b]))
            .collect();
        let width = names.iter().map(|n| n.chars().count()).max().unwrap_or(0);
        let mut out = format!("{:width$}", "");
        for &band in &bands {
            out.push_str(&format!(" {:>5}", BAND_NAMES[band]));
        }
        out.push('\n');
        for (name, (_, cells)) in names.iter().zip(&rows) {
            out.push_str(&format!("{name:<width$}"));
            for cell in cells {
                out.push_str(&format!(" {:>5}", cell.text()));
            }
            out.push('\n');
        }
        out
    }
}

/// Fila de [`MaskingReport::grid`]: la pareja (índices de pistas) y sus
/// celdas.
pub type GridRow = ((usize, usize), Vec<Cell>);

/// Celda de la tabla pareja × banda.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Cell {
    /// Banda casi vacía para la pareja.
    Empty,
    /// Suenan las dos, pero por debajo del umbral de informe.
    Quiet,
    /// Choque con su puntuación.
    Clash(f32),
}

impl Cell {
    /// `-`, `·` o la puntuación.
    pub fn text(&self) -> String {
        match self {
            Cell::Empty => "-".into(),
            Cell::Quiet => "·".into(),
            Cell::Clash(score) => score_text(*score),
        }
    }
}

/// Lo fuerte que es un choque: «fuerte» desde 0,6, «medio» desde 0,4 y
/// «leve» por debajo.
pub fn strength(score: f32) -> &'static str {
    if score >= 0.6 {
        "fuerte"
    } else if score >= 0.4 {
        "medio"
    } else {
        "leve"
    }
}

/// Frecuencia nominal de la banda `band` (la de su nombre: `1.25k` →
/// 1250).
fn nominal(band: usize) -> f32 {
    let name = BAND_NAMES[band];
    match name.strip_suffix('k') {
        Some(k) => k.parse::<f32>().map_or(band_center(band), |k| k * 1000.0),
        None => name.parse().unwrap_or(band_center(band)),
    }
}

/// Frecuencia redondeada para escribirla: al hercio por debajo de 1 kHz y
/// a la decena por encima.
fn round_freq(hz: f32) -> f32 {
    if hz < 1000.0 {
        hz.round()
    } else {
        (hz / 10.0).round() * 10.0
    }
}

/// Número para Lua, sin decimales de más: `-4`, `-3.5`, `4.3`.
fn number(x: f32) -> String {
    let text = format!("{x:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.into()
    }
}

/// Puntuación con dos decimales: `0.82`.
pub fn score_text(score: f32) -> String {
    format!("{score:.2}")
}
