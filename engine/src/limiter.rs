//! Limitador suave al final de la mezcla.
//!
//! Es un recortador suave sin memoria: lineal hasta `LIMIT_THRESHOLD` y,
//! por encima, una curva tanh que se acerca a 1 sin pasarlo y con derivada
//! continua en el umbral. Por debajo del umbral la señal no cambia.

/// Nivel a partir del cual empieza a comprimir.
pub const LIMIT_THRESHOLD: f32 = 0.9;

/// Limita una muestra a ±1 de forma suave.
#[inline]
pub fn soft_limit(x: f32) -> f32 {
    // NaN (no debería llegar) se queda en silencio en vez de propagarse.
    if x.is_nan() {
        return 0.0;
    }
    let a = x.abs();
    if a <= LIMIT_THRESHOLD {
        return x;
    }
    let headroom = 1.0 - LIMIT_THRESHOLD;
    // `min(1.0)`: tanh en f32 ya da como mucho 1, pero así queda explícito.
    let y = (LIMIT_THRESHOLD + headroom * ((a - LIMIT_THRESHOLD) / headroom).tanh()).min(1.0);
    y.copysign(x)
}
