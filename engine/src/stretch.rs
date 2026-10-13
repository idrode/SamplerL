//! Time-stretch sin cambiar el tono: WSOLA (overlap-add con búsqueda de
//! similitud de forma de onda) por tramos entre transitorios, en Rust puro
//! y sin dependencias.
//!
//! Se hace al cargar, en el hilo de control: el motor solo recibe el sample
//! ya estirado. Primero se buscan los transitorios (el mismo detector que
//! corta los samples) y el audio se parte en tramos de golpe a golpe; cada
//! tramo `a..b` pasa a ocupar `round(a·r)..round(b·r)`, así cada golpe
//! empieza exactamente en su sitio escalado (como hace ReCycle con los
//! cortes), y [`crate::Slices::scaled`] da los mismos límites.
//!
//! Dentro de cada tramo, WSOLA: ventanas de Hann de ~32 ms que se solapan
//! a la mitad; cada una se toma del sitio que le toca en la entrada (según
//! la proporción) desplazada hasta ±8 ms para que encaje con la
//! continuación natural de la anterior (la correlación más alta). La forma
//! de onda se copia, no se resintetiza: el tono no cambia. La primera
//! ventana de cada tramo empieza en su golpe, sin buscar, y nada se lee de
//! fuera del tramo (no se adelanta el golpe siguiente). Al estirar mucho
//! (más de ×1,5) las colas largas se repiten o se emborronan un poco; para
//! breaks, entre ×0,7 y ×1,4 va bien.

use crate::sample::Sample;
use crate::slice::Slices;

/// Proporción mínima y máxima (duración nueva / original).
pub const MIN_STRETCH: f64 = 0.25;
pub const MAX_STRETCH: f64 = 4.0;

/// Duración de la ventana y de la búsqueda.
const WINDOW_SECONDS: f64 = 0.032;
const SEARCH_SECONDS: f64 = 0.008;
/// La correlación se calcula cada tantas muestras (basta para encontrar el
/// encaje y la búsqueda va mucho más rápida); luego se afina a 1.
const STRIDE: usize = 4;
/// Transitorios que se buscan como mucho para partir el sample.
const MAX_ONSETS: usize = 128;

impl Sample {
    /// El sample estirado (o encogido) en el tiempo sin cambiar su tono:
    /// dura `round(frames · ratio)` frames. `ratio` de [`MIN_STRETCH`] a
    /// [`MAX_STRETCH`] (se recorta); con 1 devuelve una copia exacta.
    pub fn stretched(&self, ratio: f64) -> Sample {
        self.stretched_with(ratio, &[])
    }

    /// Como [`Sample::stretched`], con `bounds` (frames de la entrada) como
    /// límites de tramo además de los transitorios: los inicios de los
    /// cortes, para que cada corte empiece justo en su sitio escalado.
    pub fn stretched_with(&self, ratio: f64, bounds: &[usize]) -> Sample {
        let ratio = ratio.clamp(MIN_STRETCH, MAX_STRETCH);
        let frames = self.frames();
        let rate = self.sample_rate();
        let out_frames = (frames as f64 * ratio).round() as usize;
        if ratio == 1.0 || frames < 2 {
            let data = (0..frames)
                .flat_map(|i| {
                    let (l, r) = self.frame(i);
                    [l, r]
                })
                .collect();
            return Sample::from_raw(data, rate);
        }
        let mut bounds: Vec<usize> = [0]
            .into_iter()
            .chain(bounds.iter().copied().filter(|&b| b < frames))
            .collect();
        bounds.extend(Slices::detect(self, MAX_ONSETS).iter().map(|r| r.start));
        bounds.push(frames);
        bounds.sort_unstable();
        bounds.dedup();

        let wsola = Wsola::new(self, rate);
        let mut out = vec![0.0f32; 2 * out_frames];
        let scale = |f: usize| ((f as f64 * ratio).round() as usize).min(out_frames);
        for pair in bounds.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            let (oa, ob) = (scale(a), scale(b));
            wsola.segment(a..b, &mut out[2 * oa..2 * ob]);
        }
        Sample::from_raw(out, rate)
    }
}

struct Wsola<'a> {
    sample: &'a Sample,
    mono: Vec<f32>,
    window: usize,
    hop: usize,
    search: usize,
    hann: Vec<f32>,
}

impl<'a> Wsola<'a> {
    fn new(sample: &'a Sample, rate: u32) -> Self {
        let window = ((WINDOW_SECONDS * rate as f64) as usize / 2 * 2).max(4);
        Wsola {
            sample,
            mono: (0..sample.frames())
                .map(|i| {
                    let (l, r) = sample.frame(i);
                    0.5 * (l + r)
                })
                .collect(),
            window,
            hop: window / 2,
            search: ((SEARCH_SECONDS * rate as f64) as usize).max(1),
            hann: (0..window)
                .map(|n| 0.5 - 0.5 * (std::f32::consts::TAU * n as f32 / window as f32).cos())
                .collect(),
        }
    }

    /// Estira el tramo `input` de la entrada a lo que mide `out` (estéreo
    /// entrelazado), leyendo solo de ese tramo.
    fn segment(&self, input: std::ops::Range<usize>, out: &mut [f32]) {
        let out_frames = out.len() / 2;
        if out_frames == 0 || input.is_empty() {
            return;
        }
        let (start, len) = (input.start as isize, input.len() as isize);
        let ratio = out_frames as f64 / input.len() as f64;
        let at = |i: isize| -> f32 {
            if (0..len).contains(&i) {
                self.mono[(start + i) as usize]
            } else {
                0.0
            }
        };
        let window = self.window;
        // Correlación (normalizada por la energía de la candidata) entre lo
        // que seguiría a la ventana anterior y la ventana candidata.
        let similarity = |natural: isize, candidate: isize, stride: usize| -> f32 {
            let (mut xy, mut yy) = (0.0f32, 1e-9f32);
            for n in (0..window).step_by(stride) {
                let (x, y) = (at(natural + n as isize), at(candidate + n as isize));
                xy += x * y;
                yy += y * y;
            }
            xy / yy.sqrt()
        };
        let best = |natural: isize, center: isize, radius: isize, step: usize| {
            (center - radius..=center + radius)
                .step_by(step)
                .filter(|c| (0..len).contains(c))
                .map(|c| (similarity(natural, c, STRIDE), c))
                .fold(None, |best: Option<(f32, isize)>, (s, c)| match best {
                    Some((bs, _)) if bs >= s => best,
                    _ => Some((s, c)),
                })
                .map(|(_, c)| c)
        };

        let mut acc = vec![0.0f32; 2 * (out_frames + window)];
        let mut weight = vec![0.0f32; out_frames + window];
        let mut previous: isize = 0;
        let mut k = 0usize;
        while k * self.hop < out_frames {
            let nominal = (k as f64 * self.hop as f64 / ratio).round() as isize;
            let chosen = if k == 0 {
                // La primera ventana, en el golpe: sin buscar.
                0
            } else {
                let natural = previous + self.hop as isize;
                let coarse = best(natural, nominal, self.search as isize, STRIDE)
                    .unwrap_or(nominal.clamp(0, len - 1));
                best(natural, coarse, STRIDE as isize, 1).unwrap_or(coarse)
            };
            let first = k * self.hop;
            for n in 0..window {
                let i = chosen + n as isize;
                if !(0..len).contains(&i) {
                    continue;
                }
                let (l, r) = self.sample.frame((start + i) as usize);
                let w = self.hann[n];
                acc[2 * (first + n)] += w * l;
                acc[2 * (first + n) + 1] += w * r;
                weight[first + n] += w;
            }
            previous = chosen;
            k += 1;
        }
        for ((dst, src), &w) in out
            .as_chunks_mut::<2>()
            .0
            .iter_mut()
            .zip(acc.as_chunks::<2>().0)
            .zip(&weight)
        {
            // Donde las ventanas no suman 1 (el principio y el final del
            // tramo) se compensa; donde no hay nada, silencio.
            if w > 1e-3 {
                dst[0] = src[0] / w;
                dst[1] = src[1] / w;
            }
        }
    }
}
