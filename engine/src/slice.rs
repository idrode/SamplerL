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
