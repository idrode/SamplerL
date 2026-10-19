//! Choques de frecuencia entre las pistas de una canción: cada pista
//! renderizada en solo (offline, en un motor aparte) durante un ciclo
//! entero de la canción, su espectro por tercios de octava y la puntuación
//! de cada pareja en cada banda ([`engine::masking`]). Todo en el hilo que
//! lo pide; el motor que suena no se toca.

use engine::masking::{BAND_NAMES, BANDS, BandClash, BandSpectrum, MaskingOptions, clashes};

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
        let spectra = tracks
            .iter()
            .map(|name| {
                let audio = self
                    .render_track(sample_rate, bars, name)
                    .expect("la pista es de la canción")?;
                Ok(BandSpectrum::analyze(&audio, sample_rate, options))
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
        Ok(MaskingReport {
            bars,
            tracks,
            options: *options,
            pairs,
        })
    }
}

impl MaskingReport {
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
        let strength = if c.score >= 0.6 {
            "fuerte"
        } else if c.score >= 0.4 {
            "medio"
        } else {
            "leve"
        };
        let [la, lb] = c.level_db;
        let gap = (la - lb).abs();
        let louder = if gap < 1.0 {
            "las dos igual de fuertes".to_string()
        } else {
            let name = if la > lb { clash.a } else { clash.b };
            format!("{} {gap:.0} dB más fuerte", self.tracks[name])
        };
        format!(
            "{} {strength}: a la vez el {:.0} % del tiempo, {louder}",
            score_text(c.score),
            100.0 * c.overlap
        )
    }

    /// Informe en texto (para la línea de órdenes): cabecera, una línea
    /// por choque y una tabla por pareja con las bandas que chocan.
    pub fn text(&self) -> String {
        let mut out = format!(
            "choques de frecuencia: {} pistas, {} compases (ventanas de {} ms; activa a menos de {} dB de su máximo; choque hasta {} dB de diferencia; informe desde {})\n",
            self.tracks.len(),
            self.bars,
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
        out
    }

    /// Tabla pareja × banda: una fila por pareja con algún choque y una
    /// columna por banda que choca en alguna pareja (`·` sin choque, `-`
    /// banda vacía).
    pub fn table(&self) -> String {
        let clashes = self.clashes();
        let mut bands: Vec<usize> = clashes.iter().map(|c| c.band).collect();
        bands.sort_unstable();
        bands.dedup();
        let mut pairs: Vec<(usize, usize)> = clashes.iter().map(|c| (c.a, c.b)).collect();
        pairs.sort_unstable();
        pairs.dedup();
        let names: Vec<String> = pairs
            .iter()
            .map(|&(a, b)| format!("{} × {}", self.tracks[a], self.tracks[b]))
            .collect();
        let width = names.iter().map(|n| n.chars().count()).max().unwrap_or(0);
        let mut out = format!("{:width$}", "");
        for &band in &bands {
            out.push_str(&format!(" {:>5}", BAND_NAMES[band]));
        }
        out.push('\n');
        for (name, &(a, b)) in names.iter().zip(&pairs) {
            let pair = self.pairs.iter().find(|p| (p.a, p.b) == (a, b)).unwrap();
            out.push_str(&format!("{name:<width$}"));
            for &band in &bands {
                let cell = match pair.bands[band] {
                    None => "-".to_string(),
                    Some(c) if c.score < self.options.report_score => "·".to_string(),
                    Some(c) => score_text(c.score),
                };
                out.push_str(&format!(" {cell:>5}"));
            }
            out.push('\n');
        }
        out
    }
}

/// Puntuación con dos decimales: `0.82`.
pub fn score_text(score: f32) -> String {
    format!("{score:.2}")
}
