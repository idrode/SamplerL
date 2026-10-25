//! Osciladores con antialiasing: sierra y cuadrada con PolyBLEP (el salto
//! de cada periodo se sustituye por un escalón de banda limitada de dos
//! muestras; también el pulso estrecho del sub), triangular con PolyBLAMP (lo mismo con el cambio de
//! pendiente), seno y ruido blanco.
//!
//! La fase va en f64 (de 0 a 1): a 20 Hz y 48 kHz el incremento es 4e-4 y
//! en f32 la afinación derivaría al acumular. Todo es aritmética sin
//! estado oculto: la misma fase inicial da la misma onda.

use super::Wave;

/// Corrección PolyBLEP de un salto de -2 (el de la sierra al volver) en
/// la fase `t` con incremento `dt`.
fn blep(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

/// Corrección PolyBLAMP de un cambio de pendiente en la fase `t` (el
/// residuo de la integral del escalón de [`blep`]).
fn blamp(t: f64, dt: f64) -> f64 {
    if t < dt {
        let x = t / dt - 1.0;
        -x * x * x / 3.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt + 1.0;
        x * x * x / 3.0
    } else {
        0.0
    }
}

/// Un oscilador: su fase (0 a 1).
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct Osc {
    pub phase: f64,
}

/// La onda `wave` en la fase `t` (0 a 1) con incremento `dt` por muestra
/// (para el antialiasing; amplitud ±1; `pw`, el ancho de pulso de la
/// cuadrada, de 0 a 1).
#[inline]
pub(crate) fn shape(wave: Wave, t: f64, dt: f64, pw: f64) -> f64 {
    match wave {
        Wave::Sine => (std::f64::consts::TAU * t).sin(),
        Wave::Saw => 2.0 * t - 1.0 - blep(t, dt),
        Wave::Square => {
            let naive = if t < pw { 1.0 } else { -1.0 };
            let fall = (t - pw).rem_euclid(1.0);
            naive + blep(t, dt) - blep(fall, dt)
        }
        Wave::Pulse25 => {
            // El pulso al 25 % tiene de media -0,5: se le quita.
            shape(Wave::Square, t, dt, 0.25) + 0.5
        }
        Wave::Triangle => {
            // 1 en t = 0, -1 en t = 0,5: los picos se redondean.
            let naive = 4.0 * (t - 0.5).abs() - 1.0;
            let valley = (t + 0.5).rem_euclid(1.0);
            naive - 4.0 * dt * (blamp(t, dt) - blamp(valley, dt))
        }
    }
}

impl Osc {
    /// La muestra de ahora con la forma `wave` (ver [`shape`]) y avanza
    /// `dt` (frecuencia / frecuencia de muestreo, menos de 0,5).
    #[inline]
    pub fn next(&mut self, wave: Wave, dt: f64, pw: f64) -> f32 {
        let t = self.phase;
        let y = shape(wave, t, dt, pw);
        let next = t + dt;
        self.phase = if next >= 1.0 { next - 1.0 } else { next };
        y as f32
    }
}

/// Ruido blanco (xorshift32): determinista desde su semilla.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Noise {
    state: u32,
}

impl Noise {
    pub fn new(seed: u64) -> Self {
        // Nunca 0 (el xorshift se quedaría ahí).
        let mixed = (seed ^ (seed >> 32)) as u32;
        Noise { state: mixed | 1 }
    }

    /// Muestra uniforme en [-1, 1).
    #[inline]
    pub fn next(&mut self) -> f32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        (x >> 8) as f32 / (1u32 << 23) as f32 - 1.0
    }
}

/// Generador de números deterministas (splitmix64) para las fases
/// iniciales y las semillas del ruido de cada voz.
pub(crate) fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// Un número en [0, 1) del generador.
pub(crate) fn unit(state: &mut u64) -> f64 {
    (splitmix(state) >> 11) as f64 / (1u64 << 53) as f64
}

/// Coeficientes del filtro de media banda del diezmado: los de los
/// desplazamientos impares 1, 3… desde el centro (los pares son 0 y el
/// central, 1/2).
const HALF_TAPS: usize = 16;
/// Longitud del filtro (en muestras sobremuestreadas).
const HALF_LEN: usize = 4 * HALF_TAPS - 1;

/// Seno cardinal con ventana de Kaiser (β = 8, unos 80 dB de rechazo),
/// normalizado a ganancia 1 en continua. Se calcula una vez
/// ([`prepare`]), sin memoria dinámica.
static HALFBAND: std::sync::LazyLock<[f32; HALF_TAPS]> = std::sync::LazyLock::new(|| {
    fn bessel_i0(x: f64) -> f64 {
        let (mut sum, mut term) = (1.0, 1.0);
        for k in 1..64 {
            term *= (x / (2.0 * k as f64)).powi(2);
            sum += term;
        }
        sum
    }
    let beta = 8.0;
    let half = (HALF_LEN / 2 + 1) as f64;
    let mut taps = [0.0f64; HALF_TAPS];
    for (i, tap) in taps.iter_mut().enumerate() {
        let n = (2 * i + 1) as f64;
        let sinc = (std::f64::consts::FRAC_PI_2 * n).sin() / (std::f64::consts::PI * n);
        let window = bessel_i0(beta * (1.0 - (n / half).powi(2)).max(0.0).sqrt()) / bessel_i0(beta);
        *tap = sinc * window;
    }
    // Ganancia 1 en continua: 1/2 + 2·Σ = 1.
    let sum: f64 = taps.iter().sum();
    taps.map(|t| (t * 0.25 / sum) as f32)
});

/// Calcula los coeficientes del diezmado (fuera del hilo de audio).
pub(crate) fn prepare() {
    std::sync::LazyLock::force(&HALFBAND);
}

/// Diezmado ×2: entra el doble de muestras y sale una de cada dos, sin
/// lo que hay por encima de la mitad (lo que se plegaría).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Halfband {
    /// Historia doble (cada muestra se escribe dos veces), así la ventana
    /// es siempre contigua.
    buf: [f32; 2 * HALF_LEN],
    pos: usize,
}

impl Default for Halfband {
    fn default() -> Self {
        Halfband {
            buf: [0.0; 2 * HALF_LEN],
            pos: 0,
        }
    }
}

impl Halfband {
    #[inline]
    pub fn push(&mut self, x: f32) {
        self.buf[self.pos] = x;
        self.buf[self.pos + HALF_LEN] = x;
        self.pos = if self.pos + 1 == HALF_LEN {
            0
        } else {
            self.pos + 1
        };
    }

    /// La muestra de salida de la ventana actual (las últimas
    /// [`HALF_LEN`] muestras).
    #[inline]
    pub fn output(&self) -> f32 {
        let w = &self.buf[self.pos..self.pos + HALF_LEN];
        let c = HALF_LEN / 2;
        let taps = &*HALFBAND;
        let mut acc = 0.5 * w[c];
        for (i, &h) in taps.iter().enumerate() {
            let k = 2 * i + 1;
            acc += h * (w[c - k] + w[c + k]);
        }
        acc
    }
}
