//! Lanes de automatización: una curva de puntos que lleva un parámetro a lo
//! largo del progreso de la pista, como la envolvente dibujada de un DAW.
//!
//! Una lane son hasta [`MAX_LANE_POINTS`] puntos `(posición, valor, curva
//! hacia el siguiente)` sobre una duración propia (`length`, en pasos),
//! independiente de la del patrón: una lane de 3 compases sobre un patrón
//! de 4 hace polirritmo de automatización. Su valor depende solo de la
//! posición de la canción (como `ramp` y `lfo`), calculada en cada bloque
//! desde el contador de frames del transporte: no acumula nada y no deriva.
//!
//! Las lanes viven en una tabla de capacidad fija del motor ([`Lanes`],
//! [`MAX_LANES`]), que entra entera por el lote al compás; un parámetro las
//! sigue con [`crate::Curve::Lane`] (su índice en la tabla), así que valen
//! donde vale cualquier curva y se suavizan igual. Sin reservas en el hilo
//! de audio: la tabla se hace en el hilo de control y el motor solo la lee.

/// Lanes que caben en la tabla del motor. Fácil de subir: la tabla ocupa
/// unos `MAX_LANES · MAX_LANE_POINTS · 16` bytes.
pub const MAX_LANES: usize = 32;
/// Puntos de una lane, como mucho.
pub const MAX_LANE_POINTS: usize = 64;
/// Lo más que dura una lane, en pasos (64 compases de 16).
pub const MAX_LANE_STEPS: f64 = 1024.0;

/// Cómo va un punto hacia el siguiente.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LaneCurve {
    /// Recta.
    #[default]
    Linear,
    /// Exponencial: el valor se multiplica por lo mismo cada tramo igual de
    /// tiempo (recta en escala logarítmica; con un extremo en 0 o
    /// negativo, recta).
    Exp,
    /// Suave al salir y al llegar (smoothstep).
    Smooth,
    /// Se queda en el valor del punto hasta el siguiente, y salta.
    Step,
}

/// Un punto de una lane: `at` pasos desde el inicio de la lane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LanePoint {
    pub at: f64,
    pub value: f32,
    pub curve: LaneCurve,
}

impl LanePoint {
    const ZERO: LanePoint = LanePoint {
        at: 0.0,
        value: 0.0,
        curve: LaneCurve::Linear,
    };
}

/// Una lane: sus puntos (crecientes en `at`), lo que dura y si interpola en
/// escala logarítmica (las frecuencias: una recta ahí sube igual por
/// octavas).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lane {
    points: [LanePoint; MAX_LANE_POINTS],
    count: u8,
    length: f64,
    log: bool,
}

impl Lane {
    /// Con estos puntos (de 1 a [`MAX_LANE_POINTS`], con `at` creciente y
    /// dentro de `0..=length`) sobre `length` pasos (más de 0, hasta
    /// [`MAX_LANE_STEPS`]).
    pub fn new(points: &[LanePoint], length: f64, log: bool) -> Result<Lane, &'static str> {
        if points.is_empty() || points.len() > MAX_LANE_POINTS {
            return Err("una lane lleva de 1 a 64 puntos");
        }
        if !(length > 0.0 && length <= MAX_LANE_STEPS) {
            return Err("una lane dura más de 0 y hasta 64 compases");
        }
        if points
            .iter()
            .any(|p| !(p.at.is_finite() && (0.0..=length).contains(&p.at) && p.value.is_finite()))
        {
            return Err("los puntos de una lane van dentro de su duración (len)");
        }
        if points.windows(2).any(|w| w[1].at <= w[0].at) {
            return Err("los puntos de una lane van en orden, sin repetir posición");
        }
        if log && points.iter().any(|p| p.value <= 0.0) {
            return Err("una lane en escala log necesita valores mayores que 0");
        }
        let mut stored = [LanePoint::ZERO; MAX_LANE_POINTS];
        stored[..points.len()].copy_from_slice(points);
        Ok(Lane {
            points: stored,
            count: points.len() as u8,
            length,
            log,
        })
    }

    pub fn points(&self) -> &[LanePoint] {
        &self.points[..self.count as usize]
    }

    /// Lo que dura, en pasos.
    pub fn length(&self) -> f64 {
        self.length
    }

    /// El valor más bajo y el más alto de sus puntos (los extremos de la
    /// curva: entre dos puntos nunca se sale de ellos).
    pub fn bounds(&self) -> (f32, f32) {
        self.points()
            .iter()
            .fold((f32::MAX, f32::MIN), |(lo, hi), p| {
                (lo.min(p.value), hi.max(p.value))
            })
    }

    /// El valor en la posición `t` de la lane (pasos desde su inicio, ya
    /// dentro de `0..=length`): antes del primer punto, el del primero;
    /// después del último, el del último.
    pub fn at(&self, t: f64) -> f32 {
        let points = self.points();
        // El primer punto que está después de `t`.
        let next = points.partition_point(|p| p.at <= t);
        if next == 0 {
            return points[0].value;
        }
        let a = points[next - 1];
        let Some(b) = points.get(next) else {
            return a.value;
        };
        let u = ((t - a.at) / (b.at - a.at)).clamp(0.0, 1.0) as f32;
        let (from, to) = (a.value, b.value);
        // En los puntos, su valor exacto (sin pasar por logaritmos).
        if u <= 0.0 || a.curve == LaneCurve::Step {
            return from;
        }
        let geometric = |u: f32| (from.ln() + u * (to.ln() - from.ln())).exp();
        let line = |u: f32| from + u * (to - from);
        let positive = from > 0.0 && to > 0.0;
        match a.curve {
            LaneCurve::Step => from,
            _ if u >= 1.0 => to,
            LaneCurve::Exp if positive => geometric(u),
            LaneCurve::Exp => line(u),
            LaneCurve::Smooth | LaneCurve::Linear => {
                let u = if a.curve == LaneCurve::Smooth {
                    u * u * (3.0 - 2.0 * u)
                } else {
                    u
                };
                if self.log && positive {
                    geometric(u)
                } else {
                    line(u)
                }
            }
        }
    }

    /// El valor en la posición `steps` de la canción: la lane se repite
    /// cada `length` pasos desde el inicio.
    pub fn value(&self, steps: f64) -> f32 {
        self.at(steps.rem_euclid(self.length))
    }
}

/// La tabla de lanes del motor: [`MAX_LANES`] huecos, cada uno con su lane
/// o vacío. Se hace en el hilo de control y entra entera al compás.
#[derive(Debug, Clone, PartialEq)]
pub struct Lanes {
    lanes: [Option<Lane>; MAX_LANES],
}

impl Lanes {
    /// Sin ninguna lane.
    pub const EMPTY: Lanes = Lanes {
        lanes: [None; MAX_LANES],
    };

    pub fn new() -> Self {
        Lanes::EMPTY
    }

    /// Pone la lane `index` (de 0 a [`MAX_LANES`] − 1).
    pub fn set(&mut self, index: usize, lane: Option<Lane>) -> Result<(), &'static str> {
        let slot = self
            .lanes
            .get_mut(index)
            .ok_or("no caben más lanes (hay 32)")?;
        *slot = lane;
        Ok(())
    }

    pub fn get(&self, index: usize) -> Option<&Lane> {
        self.lanes.get(index).and_then(Option::as_ref)
    }

    pub fn is_empty(&self) -> bool {
        self.lanes.iter().all(Option::is_none)
    }
}

impl Default for Lanes {
    fn default() -> Self {
        Lanes::EMPTY
    }
}

/// Una tabla vacía, para evaluar sin motor (tests, precálculo).
pub(crate) static NO_LANES: Lanes = Lanes::EMPTY;
