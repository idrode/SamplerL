//! Efectos de pista: filtro paso bajo/alto y bitcrusher.
//!
//! La configuración ([`Fx`]) va dentro del `Pattern` y entra con él al
//! empezar el compás. El estado ([`FxState`]) vive en la pista, en el hilo
//! de audio, en tamaño fijo: aplicar un efecto no reserva memoria.

use std::f32::consts::TAU;

/// Efectos por pista como máximo (se aplican en orden).
pub const MAX_FX: usize = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    LowPass,
    HighPass,
}

/// Un efecto de pista.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Fx {
    /// Biquad de 12 dB/octava. `q` = 0.707 es la respuesta más plana.
    Filter {
        kind: FilterKind,
        cutoff_hz: f32,
        q: f32,
    },
    /// Reduce la resolución a `bits` y repite cada muestra `downsample`
    /// veces (reducción de frecuencia de muestreo sin filtro).
    Bitcrush { bits: u32, downsample: u32 },
}

pub const DEFAULT_Q: f32 = std::f32::consts::FRAC_1_SQRT_2;

impl Fx {
    pub fn lowpass(cutoff_hz: f32) -> Self {
        Fx::Filter {
            kind: FilterKind::LowPass,
            cutoff_hz,
            q: DEFAULT_Q,
        }
    }

    pub fn highpass(cutoff_hz: f32) -> Self {
        Fx::Filter {
            kind: FilterKind::HighPass,
            cutoff_hz,
            q: DEFAULT_Q,
        }
    }

    pub fn bitcrush(bits: u32, downsample: u32) -> Self {
        Fx::Bitcrush { bits, downsample }
    }

    /// Comprueba los rangos. El corte se limita además por debajo de la
    /// frecuencia de Nyquist al aplicarse.
    pub fn validate(&self) -> Result<(), &'static str> {
        match *self {
            Fx::Filter { cutoff_hz, q, .. } => {
                if !(20.0..=20_000.0).contains(&cutoff_hz) {
                    return Err("frecuencia de corte fuera de rango (20 a 20000 Hz)");
                }
                if !(0.1..=20.0).contains(&q) {
                    return Err("resonancia (q) fuera de rango (0.1 a 20)");
                }
            }
            Fx::Bitcrush { bits, downsample } => {
                if !(1..=16).contains(&bits) {
                    return Err("bits fuera de rango (1 a 16)");
                }
                if !(1..=64).contains(&downsample) {
                    return Err("downsample fuera de rango (1 a 64)");
                }
            }
        }
        Ok(())
    }
}

/// Estado de un efecto en el hilo de audio.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) enum FxState {
    #[default]
    Off,
    Biquad {
        kind: FilterKind,
        coefs: Biquad,
        /// Memoria (z1, z2) de cada canal.
        z: [[f32; 2]; 2],
    },
    Crush {
        levels: f32,
        downsample: u32,
        counter: u32,
        held: [f32; 2],
    },
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
}

impl Biquad {
    /// Coeficientes RBJ (Audio EQ Cookbook), normalizados por a0.
    fn new(kind: FilterKind, cutoff_hz: f32, q: f32, sample_rate: u32) -> Self {
        let cutoff = cutoff_hz.min(0.45 * sample_rate as f32);
        let w0 = TAU * cutoff / sample_rate as f32;
        let (sin, cos) = w0.sin_cos();
        let alpha = sin / (2.0 * q);
        let a0 = 1.0 + alpha;
        let (b0, b1, b2) = match kind {
            FilterKind::LowPass => ((1.0 - cos) / 2.0, 1.0 - cos, (1.0 - cos) / 2.0),
            FilterKind::HighPass => ((1.0 + cos) / 2.0, -(1.0 + cos), (1.0 + cos) / 2.0),
        };
        Biquad {
            b0: b0 / a0,
            b1: b1 / a0,
            b2: b2 / a0,
            a1: -2.0 * cos / a0,
            a2: (1.0 - alpha) / a0,
        }
    }
}

impl FxState {
    /// Estado para `fx`. Si el estado anterior es del mismo tipo, conserva
    /// su memoria (así cambiar el corte de un filtro no hace clic).
    pub fn update(self, fx: Option<Fx>, sample_rate: u32) -> Self {
        match (fx, self) {
            (None, _) => FxState::Off,
            (Some(Fx::Filter { kind, cutoff_hz, q }), previous) => {
                let coefs = Biquad::new(kind, cutoff_hz, q, sample_rate);
                let z = match previous {
                    FxState::Biquad { kind: k, z, .. } if k == kind => z,
                    _ => [[0.0; 2]; 2],
                };
                FxState::Biquad { kind, coefs, z }
            }
            (Some(Fx::Bitcrush { bits, downsample }), previous) => {
                let (counter, held) = match previous {
                    FxState::Crush { counter, held, .. } => (counter % downsample, held),
                    _ => (0, [0.0; 2]),
                };
                FxState::Crush {
                    levels: (1u32 << (bits - 1)) as f32,
                    downsample,
                    counter,
                    held,
                }
            }
        }
    }

    /// Vacía la memoria (pista en silencio).
    pub fn reset(&mut self) {
        match self {
            FxState::Off => {}
            FxState::Biquad { z, .. } => *z = [[0.0; 2]; 2],
            FxState::Crush { counter, held, .. } => {
                *counter = 0;
                *held = [0.0; 2];
            }
        }
    }

    /// Procesa `buf` (estéreo entrelazado) en su sitio.
    pub fn process(&mut self, buf: &mut [f32]) {
        match self {
            FxState::Off => {}
            FxState::Biquad { coefs, z, .. } => {
                let c = *coefs;
                for frame in buf.as_chunks_mut::<2>().0 {
                    for (x, z) in frame.iter_mut().zip(z.iter_mut()) {
                        // Forma directa II transpuesta.
                        let y = c.b0 * *x + z[0];
                        z[0] = c.b1 * *x - c.a1 * y + z[1];
                        z[1] = c.b2 * *x - c.a2 * y;
                        *x = y;
                    }
                }
            }
            FxState::Crush {
                levels,
                downsample,
                counter,
                held,
            } => {
                for frame in buf.as_chunks_mut::<2>().0 {
                    if *counter == 0 {
                        for (h, x) in held.iter_mut().zip(frame.iter()) {
                            *h = (*x * *levels).round() / *levels;
                        }
                    }
                    *counter = (*counter + 1) % *downsample;
                    *frame = *held;
                }
            }
        }
    }
}
