//! Detección de transitorios y división del sample en cortes.

use std::ops::Range;

use crate::sample::Sample;

/// Tamaño del salto del análisis (~5 ms a 48 kHz).
const HOP: usize = 256;
/// Compresión logarítmica: ln(1 + K·amplitud).
const LOG_K: f32 = 100.0;

/// Fuerza mínima (subida de la envolvente) para el detector por energía.
const MIN_ENERGY_RISE: f32 = 0.5;

/// Tamaño de la FFT del detector por flujo espectral (~10,7 ms a 48 kHz).
/// La ventana del salto `h` cubre los saltos `h - 1` y `h`.
const FFT_SIZE: usize = 2 * HOP;
/// Bandas por octava del flujo espectral.
const BANDS_PER_OCTAVE: f32 = 3.0;
/// Ancho mínimo de banda en bins: con un solo bin, el nivel de un ruido
/// fluctúa tanto entre ventanas que parece una sucesión de golpes.
const MIN_BAND_BINS: usize = 3;
/// Saltos entre las dos ventanas que se comparan. Con 2, las ventanas no se
/// solapan y la subida de un golpe no se reparte entre dos saltos.
const FLUX_LAG: usize = 2;
/// Fuerza mínima (subida media por banda) para el detector por flujo.
const MIN_FLUX: f32 = 0.15;
/// Ventana del afinado fino sobre la señal de agudos (~0,7 ms a 48 kHz).
const FINE: usize = 32;
/// Ventanas finas anteriores con las que se compara el nivel.
const FINE_HISTORY: usize = 4;

/// Separación mínima por defecto entre dos transitorios.
pub const DEFAULT_MIN_GAP_SECONDS: f32 = 0.040;

/// Función de detección de transitorios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Detector {
    /// Energía total por salto. Rápido, pero la cola de un bombo grave
    /// oscila y un golpe agudo y débil encima no se distingue.
    Energy,
    /// Flujo espectral por bandas con compresión logarítmica: cada banda
    /// cuenta igual, así un charles débil sobre un bombo se ve en agudos.
    #[default]
    Flux,
}

/// Parámetros de la detección de transitorios.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DetectOptions {
    pub detector: Detector,
    /// Separación mínima entre dos transitorios, en segundos. De dos golpes
    /// más cercanos se queda el más fuerte.
    pub min_gap_seconds: f32,
}

impl Default for DetectOptions {
    fn default() -> Self {
        DetectOptions {
            detector: Detector::default(),
            min_gap_seconds: DEFAULT_MIN_GAP_SECONDS,
        }
    }
}

/// Cortes de un sample: rangos de frames contiguos, ordenados.
/// El corte `i` va desde su transitorio hasta el siguiente (o el final del sample).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Slices(Vec<Range<usize>>);

impl Slices {
    /// Busca hasta `n` transitorios (los más fuertes) y corta el sample en ellos.
    /// Si hay menos transitorios que `n`, devuelve menos cortes; si no hay
    /// ninguno, un único corte con el sample entero. Lo que haya antes del
    /// primer transitorio no pertenece a ningún corte.
    pub fn detect(sample: &Sample, n: usize) -> Self {
        Self::detect_with(sample, n, &DetectOptions::default())
    }

    /// Como [`Slices::detect`], con parámetros de detección propios.
    pub fn detect_with(sample: &Sample, n: usize, options: &DetectOptions) -> Self {
        let frames = sample.frames();
        let mut starts = detect_onsets(sample, n, options);
        if starts.is_empty() && frames > 0 && n > 0 {
            starts.push(0);
        }
        let ranges = starts
            .iter()
            .enumerate()
            .map(|(i, &start)| start..starts.get(i + 1).copied().unwrap_or(frames))
            .collect();
        Slices(ranges)
    }

    /// Los mismos cortes en un sample estirado en el tiempo por `ratio`
    /// (ver [`Sample::stretched`]) que dura `frames`: cada límite pasa a
    /// `round(límite · ratio)` (los mismos cortes, en el mismo orden).
    pub fn scaled(&self, ratio: f64, frames: usize) -> Slices {
        let scale = |f: usize| ((f as f64 * ratio).round() as usize).min(frames);
        Slices(
            self.0
                .iter()
                .map(|r| scale(r.start)..scale(r.end))
                .collect(),
        )
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<Range<usize>> {
        self.0.get(index).cloned()
    }

    /// Solo el corte `index`, como un conjunto de un único corte.
    pub fn only(&self, index: usize) -> Option<Slices> {
        self.get(index).map(|range| Slices(vec![range]))
    }

    pub fn iter(&self) -> impl Iterator<Item = Range<usize>> + '_ {
        self.0.iter().cloned()
    }

    /// `n` cortes iguales que cubren el sample entero (rejilla): el corte
    /// `k` empieza en `round(k · frames / n)`.
    pub fn grid(sample: &Sample, n: usize) -> Self {
        let frames = sample.frames();
        let at = |k: usize| ((k as f64 * frames as f64 / n as f64).round() as usize).min(frames);
        Slices(
            (0..n.min(frames))
                .map(|k| at(k)..at(k + 1))
                .filter(|r| r.start < r.end)
                .collect(),
        )
    }

    /// Los mismos cortes con uno más que empieza en `frame`: parte en dos
    /// el corte que lo contiene (o, antes del primero, añade uno que llega
    /// hasta él). Tiene que quedar a más de `min_gap` frames de los
    /// límites que ya hay; si no, `Err` con los límites válidos del corte
    /// donde cae. Devuelve también el índice del corte nuevo.
    pub fn split(&self, frame: usize, min_gap: usize) -> Result<(Slices, usize), Range<usize>> {
        let mut ranges = self.0.clone();
        let Some(first) = ranges.first().cloned() else {
            return Err(0..0);
        };
        if frame < first.start {
            if first.start - frame <= min_gap {
                return Err(0..first.start.saturating_sub(min_gap));
            }
            ranges.insert(0, frame..first.start);
            return Ok((Slices(ranges), 0));
        }
        let i = ranges
            .iter()
            .position(|r| r.contains(&frame))
            .ok_or(first.start..ranges.last().map_or(0, |r| r.end))?;
        let r = ranges[i].clone();
        let (low, high) = (r.start + min_gap, r.end.saturating_sub(min_gap));
        if !(low < frame && frame < high) {
            return Err(low..high);
        }
        ranges[i].end = frame;
        ranges.insert(i + 1, frame..r.end);
        Ok((Slices(ranges), i + 1))
    }

    /// Los mismos cortes sin la marca del corte `index` (desde 0, al menos
    /// el segundo): el anterior se alarga hasta donde acababa este.
    pub fn without(&self, index: usize) -> Option<Slices> {
        if index == 0 || index >= self.0.len() {
            return None;
        }
        let mut ranges = self.0.clone();
        let removed = ranges.remove(index);
        ranges[index - 1].end = removed.end;
        Some(Slices(ranges))
    }

    /// Los mismos cortes con el inicio del corte `index` en `start` (ajuste
    /// manual): el corte anterior acaba donde empieza este. Tiene que
    /// quedar a más de `min_gap` frames de sus vecinos (y del final del
    /// sample); si no, `Err` con los límites válidos.
    pub fn with_start(
        &self,
        index: usize,
        start: usize,
        min_gap: usize,
    ) -> Result<Slices, Range<usize>> {
        let current = self.0.get(index).ok_or(0..0)?;
        let low = match index {
            0 => 0,
            i => self.0[i - 1].start + min_gap,
        };
        let high = current.end.saturating_sub(min_gap);
        if !(low..=high).contains(&start) || low > high {
            return Err(low..high);
        }
        let mut ranges = self.0.clone();
        ranges[index].start = start;
        if index > 0 {
            ranges[index - 1].end = start;
        }
        Ok(Slices(ranges))
    }

    /// Los mismos cortes con cada inicio llevado al cruce por cero más
    /// cercano (ver [`nearest_zero_crossing`]) a no más de `max` frames, sin
    /// acercarse a menos de `min_gap` de sus vecinos ni del final de su
    /// corte. El anterior acaba donde empieza el siguiente, como siempre.
    /// Con `max` = 0, los mismos cortes.
    pub fn snapped(&self, sample: &Sample, max: usize, min_gap: usize) -> Slices {
        let mut ranges = self.0.clone();
        if max == 0 {
            return Slices(ranges);
        }
        for i in 0..ranges.len() {
            let low = if i == 0 {
                0
            } else {
                ranges[i - 1].start + min_gap
            };
            let high = ranges[i].end.saturating_sub(min_gap);
            if low > high {
                continue;
            }
            let start = snap_frame(sample, ranges[i].start, max, low..high + 1);
            ranges[i].start = start;
            if i > 0 {
                ranges[i - 1].end = start;
            }
        }
        Slices(ranges)
    }
}

/// Lleva `frame` al cruce por cero más cercano dentro de `bounds`, a no más
/// de `max` frames; si no hay ninguno, lo deja donde estaba.
pub fn snap_frame(sample: &Sample, frame: usize, max: usize, bounds: Range<usize>) -> usize {
    nearest_zero_crossing(sample, frame, max, bounds).unwrap_or(frame)
}

/// El cruce por cero más cercano a `frame` (a no más de `max` frames y
/// dentro de `bounds`): el frame donde la mezcla mono `(l + r) / 2` cambia
/// de signo (o vale 0), el de los dos lados del cambio que más cerca de
/// cero queda. A igual distancia, el de antes. `None` si no hay ninguno.
pub fn nearest_zero_crossing(
    sample: &Sample,
    frame: usize,
    max: usize,
    bounds: Range<usize>,
) -> Option<usize> {
    let frames = sample.frames();
    let end = bounds.end.min(frames);
    let mono = |i: usize| {
        let (l, r) = sample.frame(i);
        0.5 * (l + r)
    };
    let changes = |a: f32, b: f32| (a < 0.0) != (b < 0.0);
    // `f` es un cruce si vale 0, o si la señal cambia de signo entre él y
    // un vecino y él es el que más se acerca a cero de los dos.
    let is_zero = |f: usize| {
        let x = mono(f);
        x == 0.0
            || (f > 0 && changes(mono(f - 1), x) && x.abs() <= mono(f - 1).abs())
            || (f + 1 < frames && changes(x, mono(f + 1)) && x.abs() < mono(f + 1).abs())
    };
    let inside = |f: usize| bounds.start <= f && f < end;
    (0..=max)
        .flat_map(|d| [frame.checked_sub(d), frame.checked_add(d)])
        .flatten()
        .find(|&f| inside(f) && is_zero(f))
}

/// Posiciones (en frames) de hasta `n` transitorios, ordenadas.
fn detect_onsets(sample: &Sample, n: usize, options: &DetectOptions) -> Vec<usize> {
    let frames = sample.frames();
    if n == 0 || frames == 0 {
        return Vec::new();
    }
    let mono: Vec<f32> = (0..frames)
        .map(|i| {
            let (l, r) = sample.frame(i);
            0.5 * (l + r)
        })
        .collect();

    // Función de detección por salto y umbral según el detector.
    let (odf, min_strength) = match options.detector {
        Detector::Energy => (energy_odf(&mono), MIN_ENERGY_RISE),
        Detector::Flux => (flux_odf(&mono), MIN_FLUX),
    };
    let hops = odf.len();

    // Candidatos: máximos locales por encima del umbral.
    let mut candidates: Vec<usize> = (0..hops)
        .filter(|&h| {
            odf[h] >= min_strength
                && (h == 0 || odf[h] >= odf[h - 1])
                && (h + 1 == hops || odf[h] > odf[h + 1])
        })
        .collect();
    candidates.sort_by(|&a, &b| odf[b].total_cmp(&odf[a]));

    // Para el afinado del flujo: diferencia entre muestras, que realza los
    // agudos y apaga las colas graves.
    let high: Vec<f32> = match options.detector {
        Detector::Energy => Vec::new(),
        Detector::Flux => (0..frames)
            .map(|i| mono[i] - if i > 0 { mono[i - 1] } else { 0.0 })
            .collect(),
    };

    // Los más fuertes primero, respetando la separación mínima. Se compara
    // la posición ya afinada, en frames, para que la separación sea exacta.
    let min_gap_frames = (options.min_gap_seconds.max(0.0) * sample.sample_rate() as f32) as usize;
    let min_gap_frames = min_gap_frames.max(1);
    let mut onsets: Vec<usize> = Vec::with_capacity(n);
    for h in candidates {
        if onsets.len() == n {
            break;
        }
        let onset = match options.detector {
            Detector::Energy => refine_energy(h, &mono),
            Detector::Flux => refine_flux(h, &high),
        };
        if onsets.iter().all(|&o| o.abs_diff(onset) >= min_gap_frames) {
            onsets.push(onset);
        }
    }
    onsets.sort_unstable();
    onsets
}

/// Detector por energía: subida positiva del RMS por salto, comprimido
/// logarítmicamente. Antes del sample se asume silencio, así que un golpe en
/// el frame 0 cuenta.
fn energy_odf(mono: &[f32]) -> Vec<f32> {
    let envelope: Vec<f32> = mono
        .chunks(HOP)
        .map(|hop| {
            let rms = (hop.iter().map(|s| s * s).sum::<f32>() / hop.len() as f32).sqrt();
            (1.0 + LOG_K * rms).ln()
        })
        .collect();
    rises(&envelope)
}

fn rises(envelope: &[f32]) -> Vec<f32> {
    (0..envelope.len())
        .map(|h| {
            let prev = if h > 0 { envelope[h - 1] } else { 0.0 };
            (envelope[h] - prev).max(0.0)
        })
        .collect()
}

/// Detector por flujo espectral: para cada salto, FFT con ventana de Hann
/// de los dos últimos saltos, amplitud por bandas de tercio de octava y
/// compresión logarítmica. La subida de cada banda se mide respecto al
/// máximo de esa banda y sus vecinas `FLUX_LAG` saltos antes (como en
/// SuperFlux), para no confundir fluctuaciones del ruido con golpes. El
/// resultado es la media de las subidas positivas por banda.
fn flux_odf(mono: &[f32]) -> Vec<f32> {
    use rustfft::FftPlanner;
    use rustfft::num_complex::Complex;

    let hops = mono.len().div_ceil(HOP);
    let fft = FftPlanner::<f32>::new().plan_fft_forward(FFT_SIZE);
    let window: Vec<f32> = (0..FFT_SIZE)
        .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT_SIZE as f32).cos())
        .collect();
    // Normalización para que un seno de amplitud A dé ~A en su bin.
    let norm = 2.0 / window.iter().sum::<f32>();
    let bands = band_edges();

    let mut buffer = vec![Complex::new(0.0f32, 0.0); FFT_SIZE];
    let mut scratch = vec![Complex::new(0.0f32, 0.0); fft.get_inplace_scratch_len()];
    // Niveles por banda de cada salto; antes del sample, silencio.
    let mut levels: Vec<Vec<f32>> = Vec::with_capacity(hops);
    let silence = vec![0.0f32; bands.len()];
    let mut odf = Vec::with_capacity(hops);

    for h in 0..hops {
        // Ventana que acaba al final del salto `h`; fuera del sample, silencio.
        let end = (h + 1) * HOP;
        for (i, slot) in buffer.iter_mut().enumerate() {
            let pos = (end + i).checked_sub(FFT_SIZE);
            let x = pos.and_then(|p| mono.get(p)).copied().unwrap_or(0.0);
            *slot = Complex::new(x * window[i], 0.0);
        }
        fft.process_with_scratch(&mut buffer, &mut scratch);

        let current: Vec<f32> = bands
            .iter()
            .map(|band| {
                let power = buffer[band.clone()]
                    .iter()
                    .map(|c| c.norm_sqr())
                    .sum::<f32>()
                    / band.len() as f32;
                (1.0 + LOG_K * norm * power.sqrt()).ln()
            })
            .collect();
        let reference = h.checked_sub(FLUX_LAG).map_or(&silence, |r| &levels[r]);
        let rise: f32 = (0..bands.len())
            .map(|b| {
                let neighbours = b.saturating_sub(1)..(b + 2).min(bands.len());
                let before = reference[neighbours].iter().copied().fold(0.0f32, f32::max);
                (current[b] - before).max(0.0)
            })
            .sum();
        odf.push(rise / bands.len() as f32);
        levels.push(current);
    }
    odf
}

/// Bins de cada banda: tercios de octava desde el bin 0 hasta la frecuencia
/// de Nyquist, con al menos `MIN_BAND_BINS` bins cada una.
fn band_edges() -> Vec<Range<usize>> {
    let nyquist = FFT_SIZE / 2;
    let mut bands = Vec::new();
    let mut start = 0;
    while start <= nyquist {
        let next = ((start as f32 * 2f32.powf(1.0 / BANDS_PER_OCTAVE)).round() as usize)
            .max(start + MIN_BAND_BINS)
            .min(nyquist + 1);
        bands.push(start..next);
        start = next;
    }
    bands
}

/// Afina un transitorio del detector por energía hasta el frame donde empieza
/// el ataque: el primero que supera una fracción del pico local y claramente
/// el nivel previo.
fn refine_energy(h: usize, mono: &[f32]) -> usize {
    let frames = mono.len();
    let peak_in = |range: Range<usize>| mono[range].iter().fold(0.0f32, |m, s| m.max(s.abs()));

    // La subida está entre el final del salto anterior y el final de este.
    let start = h.saturating_sub(1) * HOP;
    let end = ((h + 1) * HOP).min(frames);
    let peak = peak_in(start..end);
    let before = if h >= 2 {
        peak_in((h - 2) * HOP..(h - 1) * HOP)
    } else {
        0.0
    };
    let threshold = (0.1 * peak).max(2.0 * before).min(0.5 * peak);

    (start..end)
        .find(|&i| mono[i].abs() >= threshold)
        .unwrap_or(h * HOP)
}

/// Afina un transitorio del detector por flujo sobre la señal de agudos
/// `high`. En ventanas de `FINE` frames busca la mayor subida de nivel
/// respecto a las anteriores, y dentro de ella el primer frame que supera
/// claramente el nivel previo.
fn refine_flux(h: usize, high: &[f32]) -> usize {
    let frames = high.len();
    let peak_in = |range: Range<usize>| {
        high[range.start.min(frames)..range.end.min(frames)]
            .iter()
            .fold(0.0f32, |m, s| m.max(s.abs()))
    };
    let level = |w: usize| {
        let range = w * FINE..((w + 1) * FINE).min(frames);
        let len = range.len().max(1) as f32;
        let rms = (high[range].iter().map(|s| s * s).sum::<f32>() / len).sqrt();
        (1.0 + LOG_K * rms).ln()
    };

    // La ventana de la FFT del salto `h` cubre los saltos `h - 1` y `h`; se
    // busca desde un salto antes por si el ataque empezó al borde.
    let first = h.saturating_sub(2) * HOP / FINE;
    let last = (((h + 1) * HOP).min(frames)).div_ceil(FINE);
    let Some(best) = (first..last).max_by(|&a, &b| {
        let rise = |w: usize| {
            let before = (w.saturating_sub(FINE_HISTORY)..w)
                .map(level)
                .fold(0.0f32, f32::max);
            level(w) - before
        };
        // En caso de empate gana la ventana más temprana.
        rise(a).total_cmp(&rise(b)).then(b.cmp(&a))
    }) else {
        return h * HOP;
    };

    let before = peak_in(best.saturating_sub(FINE_HISTORY) * FINE..best.saturating_sub(1) * FINE);
    let peak = peak_in(best * FINE..(best + 2) * FINE);
    let threshold = (0.1 * peak).max(2.0 * before).min(0.5 * peak);
    let start = best.saturating_sub(1) * FINE;
    let end = ((best + 1) * FINE).min(frames);
    (start..end)
        .find(|&i| high[i].abs() >= threshold && high[i] != 0.0)
        .unwrap_or(best * FINE)
}
