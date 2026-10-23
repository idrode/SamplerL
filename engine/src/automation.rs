//! Automatización de parámetros de efecto: curvas (`ramp`, `lfo`) que
//! dependen solo de la posición de la canción, así que son deterministas y
//! quedan alineadas aunque se recargue el script. Van en el patrón
//! (`Pattern::with_automation`) y entran con él al empezar el compás.
//!
//! El motor las evalúa cada 32 frames y suaviza el valor (un polo de 5 ms,
//! en escala logarítmica para las frecuencias) antes de pasarlo al efecto:
//! nada salta de golpe ni hace clic. Los valores por paso (`3~800`, el
//! corte del primer filtro de la pista) se suavizan igual.

use crate::fx::Fx;

/// Automatizaciones por pista como máximo.
pub const MAX_AUTOMATIONS: usize = 8;

/// Velocidad de un LFO.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LfoRate {
    /// Ciclos por segundo.
    Hz(f32),
    /// Pasos que dura un ciclo (sigue el tempo).
    Steps(f64),
}

/// Cómo cambia un valor con el tiempo.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Curve {
    /// De `from` a `to` en `steps` pasos, y vuelta a empezar: una rampa en
    /// diente de sierra alineada con la canción (`ramp(200, 4000, "4
    /// compases")` sube en los compases 0-3, 4-7...).
    Ramp { from: f32, to: f32, steps: f64 },
    /// Seno entre `min` y `max` que empieza en `min` cuando arranca la
    /// canción.
    Lfo { min: f32, max: f32, rate: LfoRate },
}

impl Curve {
    /// Valor en la posición `steps` de la canción (pasos con decimales
    /// desde que arrancó) y `seconds` (segundos desde que arrancó). Con
    /// `log`, se interpola en escala logarítmica (frecuencias).
    pub fn value(&self, steps: f64, seconds: f64, log: bool) -> f32 {
        let (a, b, t) = match *self {
            Curve::Ramp {
                from,
                to,
                steps: len,
            } => (from, to, (steps / len).rem_euclid(1.0)),
            Curve::Lfo { min, max, rate } => {
                let phase = match rate {
                    LfoRate::Hz(hz) => seconds * hz as f64,
                    LfoRate::Steps(len) => steps / len,
                };
                let t = 0.5 - 0.5 * (std::f64::consts::TAU * phase).cos();
                (min, max, t)
            }
        };
        mix(a, b, t as f32, log)
    }

    /// Los dos extremos de la curva.
    pub fn bounds(&self) -> (f32, f32) {
        match *self {
            Curve::Ramp { from, to, .. } => (from, to),
            Curve::Lfo { min, max, .. } => (min, max),
        }
    }
}

/// `a + t·(b − a)`, o en escala logarítmica.
pub(crate) fn mix(a: f32, b: f32, t: f32, log: bool) -> f32 {
    // Los extremos, exactos.
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    if log && a > 0.0 && b > 0.0 {
        (a.ln() + t * (b.ln() - a.ln())).exp()
    } else {
        a + t * (b - a)
    }
}

/// Un parámetro automatizado: el `param` (índice de [`Fx::params`]) del
/// efecto `fx` (índice en la cadena de la pista) sigue `curve`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Automation {
    pub fx: usize,
    pub param: usize,
    pub curve: Curve,
}

impl Automation {
    /// Comprueba que el efecto y el parámetro existen y que la curva no se
    /// sale de su rango.
    pub fn validate(&self, chain: &[Option<Fx>]) -> Result<(), &'static str> {
        let fx = chain
            .get(self.fx)
            .copied()
            .flatten()
            .ok_or("automatización de un efecto que no está en la pista")?;
        let (lo, hi, _) = fx
            .param_range(self.param)
            .ok_or("ese parámetro del efecto no se puede automatizar")?;
        let (a, b) = self.curve.bounds();
        if ![a, b].iter().all(|v| (lo..=hi).contains(v)) {
            return Err("la automatización se sale del rango del parámetro");
        }
        check_curve(&self.curve)
    }
}

/// Comprueba la duración de una rampa y la velocidad de un LFO.
pub(crate) fn check_curve(curve: &Curve) -> Result<(), &'static str> {
    match *curve {
        Curve::Ramp { steps, .. } if !(steps >= 1.0 && steps.is_finite()) => {
            Err("la rampa tiene que durar al menos un paso")
        }
        Curve::Lfo {
            rate: LfoRate::Hz(hz),
            ..
        } if !(0.01..=50.0).contains(&hz) => Err("el LFO va de 0.01 a 50 Hz"),
        Curve::Lfo {
            rate: LfoRate::Steps(steps),
            ..
        } if !(steps >= 1.0 && steps.is_finite()) => {
            Err("un ciclo del LFO tiene que durar al menos un paso")
        }
        _ => Ok(()),
    }
}

/// Parámetros de mezcla de una pista que pueden seguir una curva
/// ([`crate::Pattern::with_mix_automation`]): volumen, pan y el envío a
/// cada bus. Van después de los efectos y se suavizan como el volumen de
/// siempre (un polo de ~5 ms por muestra); la curva se evalúa cada
/// [`crate::engine::MOD_BLOCK`] frames.
pub const MIX_GAIN: usize = 0;
pub const MIX_PAN: usize = 1;
/// El envío al bus `b` es `MIX_SEND + b`.
pub const MIX_SEND: usize = 2;
pub const MIX_PARAMS: usize = MIX_SEND + crate::bus::BUSES;

/// Una curva (o ninguna) por parámetro de mezcla.
pub type MixCurves = [Option<Curve>; MIX_PARAMS];

/// Rango de cada parámetro de mezcla.
pub(crate) fn mix_range(param: usize) -> (f32, f32) {
    match param {
        MIX_GAIN => (0.0, crate::sequencer::MAX_GAIN),
        MIX_PAN => (-1.0, 1.0),
        _ => (0.0, crate::bus::MAX_SEND),
    }
}

/// Comprueba que cada curva cabe en el rango de su parámetro.
pub(crate) fn check_mix(curves: &MixCurves) -> Result<(), &'static str> {
    for (param, curve) in curves.iter().enumerate() {
        let Some(curve) = curve else { continue };
        let (lo, hi) = mix_range(param);
        let (a, b) = curve.bounds();
        if ![a, b].iter().all(|v| (lo..=hi).contains(v)) {
            return Err(match param {
                MIX_GAIN => "la curva del volumen se sale de 0 a 4",
                MIX_PAN => "la curva del pan se sale de -1 a 1",
                _ => "la curva del envío se sale de 0 a 1",
            });
        }
        check_curve(curve)?;
    }
    Ok(())
}
