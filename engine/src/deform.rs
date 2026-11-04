//! Deformaciones de un corte que se calculan antes de sonar: `warp` (una
//! curva de tiempo, sin cambiar el tono) y `smear` (difuminarlo con
//! granos). Son caras para el hilo de audio, así que el script las hace al
//! cargar, en su hilo y con su memoria ([`render`]), y el buffer terminado
//! viaja en el patrón (`Pattern::with_rendered`), que entra al compás en
//! su lote como siempre: mientras tanto suena lo anterior y, si algo
//! falla, el patrón anterior sigue. El hilo de audio solo lee el buffer
//! (y suelta su `Arc` por la cola de basura).
//!
//! No hay técnicas nuevas: `warp` son los granos alineados del pitch sin
//! cambio de duración ([`crate::shift`]) con el ancla de cada grano movida
//! por la curva, y `smear` es una nube de granos ([`crate::grain`]) que
//! recorre el corte a velocidad 1 con su posición al azar, mezclada con el
//! original.

use std::ops::Range;
use std::sync::Arc;

use crate::grain::{Cloud, Granular, Knob, Position};
use crate::sample::Sample;
use crate::shift::Shift;

/// Puntos de control de un `warp`, como mucho.
pub const MAX_WARP_POINTS: usize = 6;

/// Curva de tiempo de un corte: en la fracción `x` de lo que dura (0 a 1)
/// suena lo que había en la fracción `y` del original. Pasa siempre por
/// (0, 0) y (1, 1) y, entre medias, por sus puntos (recta entre ellos):
/// `(0.5, 0.3)` empieza despacio (en la primera mitad lee el 30 %) y
/// acelera. La duración no cambia, ni el tono.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Warp {
    points: [(f32, f32); MAX_WARP_POINTS],
    len: u8,
}

impl Warp {
    /// Con estos puntos interiores (`x` e `y` dentro de 0..1, los dos
    /// crecientes: el tiempo no vuelve atrás).
    pub fn new(points: &[(f32, f32)]) -> Result<Warp, &'static str> {
        if points.is_empty() || points.len() > MAX_WARP_POINTS {
            return Err("warp lleva de 1 a 6 puntos {x, y}");
        }
        let inside = |v: f32| v > 0.0 && v < 1.0;
        if !points.iter().all(|&(x, y)| inside(x) && inside(y)) {
            return Err("los puntos de warp van entre 0 y 1 (sin llegar): {x, y}");
        }
        if points
            .windows(2)
            .any(|w| w[1].0 <= w[0].0 || w[1].1 <= w[0].1)
        {
            return Err("los puntos de warp van en orden, con x e y creciendo");
        }
        let mut out = [(0.0, 0.0); MAX_WARP_POINTS];
        out[..points.len()].copy_from_slice(points);
        Ok(Warp {
            points: out,
            len: points.len() as u8,
        })
    }

    /// Un solo punto en la mitad: en la primera mitad del tiempo suena la
    /// fracción `y` del corte (menos de 0.5, empieza despacio y acelera).
    pub fn midpoint(y: f32) -> Result<Warp, &'static str> {
        Warp::new(&[(0.5, y)])
    }

    pub fn points(&self) -> &[(f32, f32)] {
        &self.points[..self.len as usize]
    }

    /// La fracción del original que suena en la fracción `x` del tiempo.
    pub fn at(&self, x: f64) -> f64 {
        let x = x.clamp(0.0, 1.0);
        let mut from = (0.0f64, 0.0f64);
        for &(px, py) in self.points().iter().chain(&[(1.0, 1.0)]) {
            let (px, py) = (px as f64, py as f64);
            if x <= px {
                let t = (x - from.0) / (px - from.0);
                return from.1 + t * (py - from.1);
            }
            from = (px, py);
        }
        1.0
    }
}

/// Lo que se calcula de un corte: su rango en el sample y sus
/// deformaciones. Dos golpes con la misma clave comparten el buffer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderKey {
    pub start: usize,
    pub end: usize,
    pub warp: Option<Warp>,
    /// De 0 (nada) a 1 (todo granos).
    pub smear: f32,
}

impl RenderKey {
    pub fn range(&self) -> Range<usize> {
        self.start..self.end
    }
}

/// Granos del `smear`: duración, densidad (con solape 4) y hasta cuánto
/// se desplaza su posición al azar con `smear = 1`.
const SMEAR_GRAIN_MS: f32 = 60.0;
const SMEAR_DENSITY: f32 = 66.0;
const SMEAR_JITTER_MS: f32 = 80.0;
/// Semilla fija: el mismo corte con el mismo `smear` suena igual siempre.
const SMEAR_SEED: u64 = 0x5eed_c0de;

/// El corte `key.range()` de `sample` con sus deformaciones: primero el
/// `warp` y luego el `smear`. Dura lo mismo que el corte. Reserva memoria:
/// es para el hilo de control (al cargar), nunca para el de audio.
pub fn render(sample: &Sample, key: &RenderKey) -> Sample {
    let end = key.end.min(sample.frames());
    let start = key.start.min(end);
    let rate = sample.sample_rate();
    let mut audio: Vec<f32> = match key.warp {
        Some(warp) => warped(sample, start..end, warp),
        None => (start..end)
            .flat_map(|i| {
                let (l, r) = sample.frame(i);
                [l, r]
            })
            .collect(),
    };
    if key.smear > 0.0 && audio.len() >= 4 {
        let dry = Arc::new(stereo(&audio, rate));
        audio = smeared(&dry, key.smear.min(1.0));
    }
    stereo(&audio, rate)
}

fn stereo(audio: &[f32], rate: u32) -> Sample {
    if audio.is_empty() {
        return Sample::silence(rate);
    }
    Sample::from_interleaved(audio, 2, rate, rate).expect("estéreo con frames")
}

/// `warp`: los granos alineados del pitch sin cambio de duración (a
/// velocidad 1, el tono no cambia) con el ancla de cada grano donde dice
/// la curva.
fn warped(sample: &Sample, range: Range<usize>, warp: Warp) -> Vec<f32> {
    let (start, end) = (range.start, range.end);
    let frames = end - start;
    let mut out = Vec::with_capacity(frames * 2);
    if frames < 2 {
        for i in range {
            let (l, r) = sample.frame(i);
            out.extend([l, r]);
        }
        return out;
    }
    let span = (frames - 1) as f64;
    let mut shift = Shift::new(1.0, sample.sample_rate(), None).with_warp(warp, span, span);
    let fit = Some(span - shift.reach());
    for n in 0..frames {
        let grains = shift.grains(n as u64, fit, |offset| {
            crate::shift::mono_at(sample, start as f64 + offset, start, end)
        });
        let (mut l, mut r) = (0.0, 0.0);
        for (offset, weight) in grains {
            if weight == 0.0 || offset > span {
                continue;
            }
            let (gl, gr) = crate::interp::read(sample, start as f64 + offset, start, end, 1.0);
            l += weight * gl;
            r += weight * gr;
        }
        out.extend([l, r]);
    }
    out
}

/// `smear`: una nube de granos de Hann que recorre el corte a su
/// velocidad, cada uno desplazado al azar hasta `amount · 80 ms`,
/// igualada en nivel (RMS) al original y mezclada con él en esa
/// proporción. Dura lo mismo que el corte.
fn smeared(dry: &Arc<Sample>, amount: f32) -> Vec<f32> {
    let frames = dry.frames();
    let rate = dry.sample_rate();
    let mut granular = Granular::new(SMEAR_SEED);
    granular.size_ms = Knob::Fixed(SMEAR_GRAIN_MS);
    granular.density = Knob::Fixed(SMEAR_DENSITY);
    granular.jitter_ms = Knob::Fixed(amount * SMEAR_JITTER_MS);
    let mut cloud = Cloud::default();
    let _ = cloud.open(dry.clone(), 0, frames, 1.0, false, 1.0, frames as u64);
    let mut wet = vec![0.0f32; frames * 2];
    const BLOCK: usize = 512;
    for (b, chunk) in wet.chunks_mut(BLOCK * 2).enumerate() {
        let at = Position {
            frame: (b * BLOCK) as u64,
            steps: 0.0,
            seconds: 0.0,
            frames_per_step: 1.0,
            sample_rate: rate,
            macros: [0.0; crate::automation::MAX_MACROS],
        };
        let _ = cloud.mix_into(chunk, &granular, &at, 1);
    }
    let rms = |x: &mut dyn Iterator<Item = f32>| {
        let (sum, n) = x.fold((0.0f64, 0usize), |(s, n), v| {
            (s + (v as f64).powi(2), n + 1)
        });
        (sum / n.max(1) as f64).sqrt()
    };
    let dry_rms = rms(&mut (0..frames).flat_map(|i| {
        let (l, r) = dry.frame(i);
        [l, r]
    }));
    let wet_rms = rms(&mut wet.iter().copied());
    let level = if wet_rms > 1e-9 {
        (dry_rms / wet_rms) as f32
    } else {
        0.0
    };
    // Lo último se funde (la nube se corta al acabar el corte).
    let fade = ((0.005 * rate as f64) as usize).clamp(1, frames);
    let mut out = Vec::with_capacity(frames * 2);
    for i in 0..frames {
        let (l, r) = dry.frame(i);
        let tail = ((frames - i) as f32 / fade as f32).min(1.0);
        let w = level * tail;
        out.push((1.0 - amount) * l + amount * w * wet[2 * i]);
        out.push((1.0 - amount) * r + amount * w * wet[2 * i + 1]);
    }
    out
}
