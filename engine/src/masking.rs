//! Choques de frecuencia entre pistas (masking), offline: el espectro de
//! cada pista en tercios de octava, ventana a ventana, y una puntuación por
//! pareja de pistas y banda de cuánto se pisan. Se calcula en el hilo de
//! control con el audio ya renderizado; nada de esto toca el motor.
//!
//! Criterio: en cada ventana, una pista está *activa* en una banda si su
//! nivel no queda más de [`MaskingOptions::active_range_db`] por debajo de
//! su propio máximo (el de esa pista en cualquier banda y ventana). Si las
//! dos lo están, su *cercanía* vale 1 con el mismo nivel y baja en línea
//! recta hasta 0 cuando las separan [`MaskingOptions::level_gap_db`]. La
//! puntuación de la banda es la media de la cercanía ponderada por la
//! energía de las dos pistas en cada ventana (de 0 a 1). Las bandas en
//! las que una de las dos casi no suena
//! ([`MaskingOptions::empty_band_db`]) no se puntúan.

use rustfft::FftPlanner;
use rustfft::num_complex::Complex;

/// Bandas de tercio de octava: de 25 Hz a 20 kHz (centros exactos
/// `1000 · 10^(k/10)`).
pub const BANDS: usize = 30;

/// Nombre corto del centro de cada banda (el nominal de la norma).
pub const BAND_NAMES: [&str; BANDS] = [
    "25", "31.5", "40", "50", "63", "80", "100", "125", "160", "200", "250", "315", "400", "500",
    "630", "800", "1k", "1.25k", "1.6k", "2k", "2.5k", "3.15k", "4k", "5k", "6.3k", "8k", "10k",
    "12.5k", "16k", "20k",
];

/// Centro exacto de la banda `band` en Hz.
pub fn band_center(band: usize) -> f32 {
    1000.0 * 10f32.powf((band as f32 - 16.0) / 10.0)
}

/// Bordes de la banda `band` (un sexto de octava a cada lado del centro).
pub fn band_edges(band: usize) -> (f32, f32) {
    let c = band_center(band);
    (c * 10f32.powf(-0.05), c * 10f32.powf(0.05))
}

/// Parámetros del análisis. Los valores por defecto ([`Default`]) son los
/// documentados en la guía.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaskingOptions {
    /// Ventana de análisis (Hann), en ms.
    pub window_ms: f32,
    /// Salto entre ventanas, en ms (la mitad: se solapan).
    pub hop_ms: f32,
    /// Una pista está activa en una banda si no queda más de estos dB por
    /// debajo de su propio máximo.
    pub active_range_db: f32,
    /// Diferencia de nivel a la que dos pistas activas dejan de chocar
    /// (la cercanía es `1 − diferencia / esto`).
    pub level_gap_db: f32,
    /// Una banda en la que una de las dos pistas casi no suena (su energía
    /// en todo el análisis queda más de estos dB por debajo de la de su
    /// propia banda más fuerte) no se puntúa: para chocar, las dos tienen
    /// que sonar ahí de verdad, no solo con el chasquido de un ataque.
    pub empty_band_db: f32,
    /// Puntuación a partir de la cual se informa de un choque (y se
    /// sugiere un recorte).
    pub report_score: f32,
}

impl Default for MaskingOptions {
    fn default() -> Self {
        MaskingOptions {
            window_ms: 50.0,
            hop_ms: 25.0,
            active_range_db: 30.0,
            level_gap_db: 9.0,
            empty_band_db: 30.0,
            report_score: 0.25,
        }
    }
}

/// Mínimo de puntos de la FFT: a 48 kHz, bins cada 5,9 Hz, para que los
/// tercios de octava graves tengan algún bin propio (la ventana de 50 ms
/// los emborrona igual por debajo de unos 60 Hz).
const MIN_FFT: usize = 8192;

/// Potencia por banda de cada ventana de una pista.
#[derive(Debug, Clone, PartialEq)]
pub struct BandSpectrum {
    /// `power[ventana][banda]`, potencia lineal (unidades arbitrarias pero
    /// las mismas para todas las pistas analizadas con las mismas opciones
    /// y frecuencia).
    pub power: Vec<[f32; BANDS]>,
}

impl BandSpectrum {
    /// Analiza `audio` (estéreo entrelazado; se mezcla a mono) a
    /// `sample_rate`, con ventanas hasta el final (lo que pasa del final,
    /// silencio). Las bandas por encima de Nyquist quedan a cero.
    pub fn analyze(audio: &[f32], sample_rate: u32, options: &MaskingOptions) -> Self {
        Self::analyze_cycle(audio, audio.len() / 2, sample_rate, options)
    }

    /// Como [`BandSpectrum::analyze`], con ventanas que empiezan en los
    /// primeros `cycle` frames: si `audio` sigue después (lo que suena al
    /// volver a empezar el ciclo), las últimas lo leen en vez de acabar en
    /// silencio de golpe, que esparciría energía por todas las bandas.
    pub fn analyze_cycle(
        audio: &[f32],
        cycle: usize,
        sample_rate: u32,
        options: &MaskingOptions,
    ) -> Self {
        let mono: Vec<f32> = audio
            .as_chunks::<2>()
            .0
            .iter()
            .map(|[l, r]| 0.5 * (l + r))
            .collect();
        let rate = sample_rate as f32;
        let window_len = ((options.window_ms / 1000.0 * rate).round() as usize).max(16);
        let hop = ((options.hop_ms / 1000.0 * rate).round() as usize).max(1);
        let size = window_len.next_power_of_two().max(MIN_FFT);
        let window: Vec<f32> = (0..window_len)
            .map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / (window_len - 1) as f32).cos())
            .collect();
        let bins = band_bins(size, rate);
        let fft = FftPlanner::<f32>::new().plan_fft_forward(size);
        let mut buffer = vec![Complex::new(0.0f32, 0.0); size];
        let mut scratch = vec![Complex::new(0.0f32, 0.0); fft.get_inplace_scratch_len()];
        let mut power = Vec::new();
        let mut start = 0;
        while start < cycle {
            buffer.fill(Complex::new(0.0, 0.0));
            for (i, (slot, w)) in buffer.iter_mut().zip(&window).enumerate() {
                *slot = Complex::new(mono.get(start + i).copied().unwrap_or(0.0) * w, 0.0);
            }
            fft.process_with_scratch(&mut buffer, &mut scratch);
            let mut row = [0.0f32; BANDS];
            for (band, range) in bins.iter().enumerate() {
                row[band] = buffer[range.clone()].iter().map(|c| c.norm_sqr()).sum();
            }
            power.push(row);
            start += hop;
        }
        BandSpectrum { power }
    }

    /// Ventanas analizadas.
    pub fn windows(&self) -> usize {
        self.power.len()
    }

    /// Máxima potencia en cualquier banda y ventana.
    pub fn peak(&self) -> f32 {
        self.power
            .iter()
            .flat_map(|row| row.iter())
            .fold(0.0, |a, &b| a.max(b))
    }

    /// Energía total de la banda `band`.
    pub fn band_energy(&self, band: usize) -> f32 {
        self.power.iter().map(|row| row[band]).sum()
    }
}

/// Bins de la FFT de `size` puntos que caen en cada banda; si en una no
/// cae ninguno, el más cercano a su centro. Vacío por encima de Nyquist.
fn band_bins(size: usize, rate: f32) -> Vec<std::ops::Range<usize>> {
    let spacing = rate / size as f32;
    let nyquist = size / 2;
    (0..BANDS)
        .map(|band| {
            let (lo, hi) = band_edges(band);
            if lo >= rate / 2.0 {
                return 0..0;
            }
            let first = (lo / spacing).ceil() as usize;
            let last = ((hi / spacing).ceil() as usize).min(nyquist);
            if first < last {
                first..last
            } else {
                let nearest = ((band_center(band) / spacing).round() as usize).clamp(1, nyquist);
                nearest..nearest + 1
            }
        })
        .collect()
}

/// Choque de dos pistas en una banda.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BandClash {
    /// De 0 (no se pisan) a 1 (siempre las dos, al mismo nivel).
    pub score: f32,
    /// Fracción del tiempo (ventanas) en que las dos están activas.
    pub overlap: f32,
    /// Energía de cada pista en la banda, en dB relativos a la banda más
    /// fuerte de la pareja (0 la más fuerte).
    pub level_db: [f32; 2],
}

/// Choques de las pistas `a` y `b`, banda a banda: `None` en las bandas
/// en las que una de las dos casi no suena. Si una es más corta, lo que le
/// falta cuenta como silencio.
pub fn clashes(
    a: &BandSpectrum,
    b: &BandSpectrum,
    options: &MaskingOptions,
) -> [Option<BandClash>; BANDS] {
    let windows = a.windows().max(b.windows());
    let row = |s: &BandSpectrum, t: usize| s.power.get(t).copied().unwrap_or([0.0; BANDS]);
    let db = |p: f32| 10.0 * p.max(1e-30).log10();
    // Una pista muda nunca está activa.
    let floor = |s: &BandSpectrum| {
        let peak = s.peak();
        (peak > 0.0).then(|| db(peak) - options.active_range_db)
    };
    let (floor_a, floor_b) = (floor(a), floor(b));
    let energy: Vec<[f32; 2]> = (0..BANDS)
        .map(|band| [a.band_energy(band), b.band_energy(band)])
        .collect();
    let loudest = energy.iter().map(|e| e[0] + e[1]).fold(0.0, f32::max);
    // La banda más fuerte de cada pista.
    let strongest = |i: usize| energy.iter().map(|e| e[i]).fold(0.0, f32::max);
    let (strongest_a, strongest_b) = (strongest(0), strongest(1));
    let sounds =
        |e: f32, strongest: f32| strongest > 0.0 && db(e) >= db(strongest) - options.empty_band_db;
    std::array::from_fn(|band| {
        let [ea, eb] = energy[band];
        if !sounds(ea, strongest_a) || !sounds(eb, strongest_b) {
            return None;
        }
        let (mut weighted, mut total, mut both) = (0.0f64, 0.0f64, 0usize);
        for t in 0..windows {
            let (pa, pb) = (row(a, t)[band], row(b, t)[band]);
            let weight = (pa + pb) as f64;
            total += weight;
            let active = |p: f32, floor: Option<f32>| floor.is_some_and(|f| db(p) >= f);
            if active(pa, floor_a) && active(pb, floor_b) {
                both += 1;
                let closeness = (1.0 - (db(pa) - db(pb)).abs() / options.level_gap_db).max(0.0);
                weighted += weight * closeness as f64;
            }
        }
        if total <= 0.0 {
            return None;
        }
        Some(BandClash {
            score: (weighted / total) as f32,
            overlap: both as f32 / windows.max(1) as f32,
            level_db: [db(ea) - db(loudest), db(eb) - db(loudest)],
        })
    })
}
