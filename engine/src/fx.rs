//! Efectos de pista: filtro paso bajo/alto, bitcrusher, compresor y
//! distorsión.
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
    /// Compresor estéreo enlazado: por encima de `threshold_db` el nivel
    /// sube `1/ratio` dB por cada dB de entrada; la reducción llega en
    /// `attack_ms` y se va en `release_ms`; `makeup_db` sube la salida.
    Compressor {
        threshold_db: f32,
        ratio: f32,
        attack_ms: f32,
        release_ms: f32,
        makeup_db: f32,
    },
    /// Saturación con `tanh(drive · x)`, mezclada con la señal limpia
    /// (`mix` 1 = solo la saturada).
    Distortion { drive: f32, mix: f32 },
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

    /// Compresor con ataque de 5 ms, release de 100 ms y sin makeup.
    pub fn compressor(threshold_db: f32, ratio: f32) -> Self {
        Fx::Compressor {
            threshold_db,
            ratio,
            attack_ms: 5.0,
            release_ms: 100.0,
            makeup_db: 0.0,
        }
    }

    pub fn distortion(drive: f32, mix: f32) -> Self {
        Fx::Distortion { drive, mix }
    }

    /// Parámetros que se pueden automatizar (con `ramp`, `lfo` o, el corte,
    /// por paso): su nombre en el script, en el orden de su índice.
    pub fn params(&self) -> &'static [&'static str] {
        match self {
            Fx::Filter { .. } => &["cutoff", "q"],
            Fx::Compressor { .. } => &["threshold", "makeup"],
            Fx::Distortion { .. } => &["drive", "mix"],
            Fx::Bitcrush { .. } => &[],
        }
    }

    /// Índice del parámetro automatizable `name`.
    pub fn param_index(&self, name: &str) -> Option<usize> {
        self.params().iter().position(|p| *p == name)
    }

    /// Rango válido del parámetro `index` y si se mueve en escala
    /// logarítmica (las frecuencias).
    pub fn param_range(&self, index: usize) -> Option<(f32, f32, bool)> {
        Some(match (self, index) {
            (Fx::Filter { .. }, 0) => (20.0, 20_000.0, true),
            (Fx::Filter { .. }, 1) => (0.1, 20.0, true),
            (Fx::Compressor { .. }, 0) => (-60.0, 0.0, false),
            (Fx::Compressor { .. }, 1) => (0.0, 24.0, false),
            (Fx::Distortion { .. }, 0) => (1.0, 50.0, true),
            (Fx::Distortion { .. }, 1) => (0.0, 1.0, false),
            _ => return None,
        })
    }

    /// Valor fijo del parámetro `index` (el que se escribió en la tabla).
    pub fn param_value(&self, index: usize) -> Option<f32> {
        Some(match (*self, index) {
            (Fx::Filter { cutoff_hz, .. }, 0) => cutoff_hz,
            (Fx::Filter { q, .. }, 1) => q,
            (Fx::Compressor { threshold_db, .. }, 0) => threshold_db,
            (Fx::Compressor { makeup_db, .. }, 1) => makeup_db,
            (Fx::Distortion { drive, .. }, 0) => drive,
            (Fx::Distortion { mix, .. }, 1) => mix,
            _ => return None,
        })
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
            Fx::Compressor {
                threshold_db,
                ratio,
                attack_ms,
                release_ms,
                makeup_db,
            } => {
                if !(-60.0..=0.0).contains(&threshold_db) {
                    return Err("threshold fuera de rango (-60 a 0 dB)");
                }
                if !(1.0..=20.0).contains(&ratio) {
                    return Err("ratio fuera de rango (1 a 20)");
                }
                if !(0.1..=200.0).contains(&attack_ms) {
                    return Err("attack fuera de rango (0.1 a 200 ms)");
                }
                if !(1.0..=2000.0).contains(&release_ms) {
                    return Err("release fuera de rango (1 a 2000 ms)");
                }
                if !(0.0..=24.0).contains(&makeup_db) {
                    return Err("makeup fuera de rango (0 a 24 dB)");
                }
            }
            Fx::Distortion { drive, mix } => {
                if !(1.0..=50.0).contains(&drive) {
                    return Err("drive fuera de rango (1 a 50)");
                }
                if !(0.0..=1.0).contains(&mix) {
                    return Err("mix fuera de rango (0 a 1)");
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
        cutoff_hz: f32,
        q: f32,
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
    Comp {
        threshold_db: f32,
        /// dB que se quitan por dB por encima del umbral: `1 - 1/ratio`.
        slope: f32,
        /// Coeficientes de la envolvente (por muestra) al subir y al bajar.
        attack: f32,
        release: f32,
        makeup_db: f32,
        /// Reducción actual, en dB (0 o más).
        reduction: f32,
    },
    Drive {
        drive: f32,
        mix: f32,
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
                FxState::Biquad {
                    kind,
                    cutoff_hz,
                    q,
                    coefs,
                    z,
                }
            }
            (
                Some(Fx::Compressor {
                    threshold_db,
                    ratio,
                    attack_ms,
                    release_ms,
                    makeup_db,
                }),
                previous,
            ) => {
                let coef = |ms: f32| (-1.0 / (ms / 1000.0 * sample_rate as f32)).exp();
                FxState::Comp {
                    threshold_db,
                    slope: 1.0 - 1.0 / ratio,
                    attack: coef(attack_ms),
                    release: coef(release_ms),
                    makeup_db,
                    // Otro compresor en la misma ranura: sigue su envolvente.
                    reduction: match previous {
                        FxState::Comp { reduction, .. } => reduction,
                        _ => 0.0,
                    },
                }
            }
            (Some(Fx::Distortion { drive, mix }), _) => FxState::Drive { drive, mix },
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

    /// Cambia el parámetro automatizable `index` (ver [`Fx::params`]) sin
    /// tocar la memoria del efecto. Solo recalcula lo necesario.
    pub fn set_param(&mut self, index: usize, value: f32, sample_rate: u32) {
        match (self, index) {
            (
                FxState::Biquad {
                    kind,
                    cutoff_hz,
                    q,
                    coefs,
                    ..
                },
                0 | 1,
            ) => {
                let target = if index == 0 { &mut *cutoff_hz } else { &mut *q };
                if *target != value {
                    *target = value;
                    *coefs = Biquad::new(*kind, *cutoff_hz, *q, sample_rate);
                }
            }
            (FxState::Comp { threshold_db, .. }, 0) => *threshold_db = value,
            (FxState::Comp { makeup_db, .. }, 1) => *makeup_db = value,
            (FxState::Drive { drive, .. }, 0) => *drive = value,
            (FxState::Drive { mix, .. }, 1) => *mix = value,
            _ => {}
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
            FxState::Comp { reduction, .. } => *reduction = 0.0,
            FxState::Drive { .. } => {}
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
            FxState::Comp {
                threshold_db,
                slope,
                attack,
                release,
                makeup_db,
                reduction,
            } => {
                for frame in buf.as_chunks_mut::<2>().0 {
                    // Pico de los dos canales (estéreo enlazado), en dB.
                    let level = frame[0].abs().max(frame[1].abs()).max(1e-9);
                    let over = 20.0 * level.log10() - *threshold_db;
                    let target = if over > 0.0 { over * *slope } else { 0.0 };
                    let coef = if target > *reduction {
                        *attack
                    } else {
                        *release
                    };
                    *reduction = target + coef * (*reduction - target);
                    let gain = 10f32.powf((*makeup_db - *reduction) / 20.0);
                    frame[0] *= gain;
                    frame[1] *= gain;
                }
            }
            FxState::Drive { drive, mix } => {
                for x in buf.iter_mut() {
                    let wet = (*drive * *x).tanh();
                    *x += *mix * (wet - *x);
                }
            }
        }
    }
}
