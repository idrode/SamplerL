//! Cambiar un número escrito en el código con la rueda, el ratón o el
//! teclado: cuánto se mueve y cómo se escribe. Puro (sin editor ni
//! terminal): lo usa el gesto de `app/adjust.rs`.
//!
//! - **Con el registro** (el número es el valor de un parámetro de
//!   [`script::api`]): su rango, entero o decimal y su escala. Un
//!   deslizamiento largo ([`FULL_RANGE_TICKS`] pasos) recorre el rango
//!   entero; en escala logarítmica (frecuencias, tiempos largos), cada paso
//!   multiplica por lo mismo. Nunca sale del rango.
//! - **Sin él**: un entero cambia de 1 en 1 y un decimal en su última
//!   cifra (`0.25` de 0,01 en 0,01).
//!
//! El valor siempre sale del original y de la posición del gesto (no se
//! acumula redondeo), y se escribe con los decimales que tenía (o los que
//! hagan falta para que el paso se vea), sin notación científica ni ceros
//! de más.

use script::api::Numeric;

/// Pasos de la rueda que recorren el rango entero de un parámetro del
/// registro con `[editor] ajuste_sensibilidad = 1`.
pub const FULL_RANGE_TICKS: f64 = 60.0;
/// Los decimales como mucho.
const MAX_DECIMALS: usize = 6;
/// Lo más grande que se escribe sin registro (más, notación científica).
const FREE_LIMIT: f64 = 1e12;

/// Un número que se está cambiando.
#[derive(Debug, Clone, PartialEq)]
pub struct Stepper {
    original: f64,
    /// El texto de partida (para volver a él tal cual).
    original_text: String,
    decimals: usize,
    /// Se escribió sin el cero (`.5`).
    leading_dot: bool,
    spec: Option<Numeric>,
    sensitivity: f64,
    /// Dónde va el gesto, en pasos (ya con la sensibilidad).
    position: f64,
}

impl Stepper {
    /// Un número escrito como `text` (decimal, quizá negativo), con su
    /// parámetro del registro si se sabe. `None` si no es un número que se
    /// pueda escribir de vuelta.
    pub fn new(text: &str, spec: Option<Numeric>, sensitivity: f64) -> Option<Stepper> {
        let digits = text.strip_prefix('-').unwrap_or(text);
        if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return None;
        }
        let original: f64 = text.parse().ok().filter(|v: &f64| v.is_finite())?;
        let decimals = digits.split_once('.').map_or(0, |(_, d)| d.len());
        Some(Stepper {
            original,
            original_text: text.to_string(),
            decimals,
            leading_dot: digits.starts_with('.'),
            spec,
            sensitivity: if sensitivity.is_finite() && sensitivity > 0.0 {
                sensitivity
            } else {
                1.0
            },
            position: 0.0,
        })
    }

    /// El parámetro del registro, si lo hay.
    pub fn spec(&self) -> Option<Numeric> {
        self.spec
    }

    /// Mueve `ticks` pasos (positivo, sube) y da el texto nuevo.
    pub fn nudge(&mut self, ticks: f64) -> String {
        if ticks.is_finite() {
            self.position += ticks * self.sensitivity;
        }
        // En el borde, la posición se queda en él: volver no cuesta pasos.
        let value = self.raw(self.position);
        if let Some(spec) = self.spec {
            let clamped = value.clamp(spec.min, spec.max);
            if clamped != value {
                self.position = self.position_of(clamped);
            }
        } else if value.abs() > FREE_LIMIT {
            self.position = self.position_of(value.clamp(-FREE_LIMIT, FREE_LIMIT));
        }
        self.text()
    }

    /// El valor de ahora (el del texto: con su redondeo).
    pub fn value(&self) -> f64 {
        self.text().parse().unwrap_or(self.original)
    }

    /// Donde está el valor en su rango (de 0 a 1, en su escala), para la
    /// barra; `None` sin registro.
    pub fn fraction(&self) -> Option<f64> {
        let spec = self.spec?;
        let v = self.value().clamp(spec.min, spec.max);
        Some(if spec.log && spec.min > 0.0 {
            (v / spec.min).ln() / (spec.max / spec.min).ln()
        } else {
            (v - spec.min) / (spec.max - spec.min)
        })
    }

    /// El texto de ahora.
    pub fn text(&self) -> String {
        if self.position == 0.0 {
            return self.original_text.clone();
        }
        let raw = self.raw(self.position);
        let value = match self.spec {
            Some(spec) => raw.clamp(spec.min, spec.max),
            None => raw.clamp(-FREE_LIMIT, FREE_LIMIT),
        };
        let integer = self.spec.is_some_and(|s| s.integer);
        let needed = if integer { 0 } else { self.needed_decimals() };
        let keep = if integer { 0 } else { self.decimals };
        written(
            value,
            keep.max(needed).min(MAX_DECIMALS),
            keep,
            self.leading_dot,
        )
    }

    /// El valor sin redondear en la posición `p`.
    fn raw(&self, p: f64) -> f64 {
        match self.spec {
            Some(spec) if self.logarithmic() => {
                let start = self.original.clamp(spec.min, spec.max);
                start * (spec.max / spec.min).powf(p / FULL_RANGE_TICKS)
            }
            Some(spec) => {
                let start = self.original.clamp(spec.min, spec.max);
                start + p * self.linear_step(spec)
            }
            None => self.original + p * self.free_step(),
        }
    }

    /// La posición en la que el valor sin redondear es `value`.
    fn position_of(&self, value: f64) -> f64 {
        match self.spec {
            Some(spec) if self.logarithmic() => {
                let start = self.original.clamp(spec.min, spec.max);
                (value / start).ln() / (spec.max / spec.min).ln() * FULL_RANGE_TICKS
            }
            Some(spec) => {
                let start = self.original.clamp(spec.min, spec.max);
                (value - start) / self.linear_step(spec)
            }
            None => (value - self.original) / self.free_step(),
        }
    }

    fn logarithmic(&self) -> bool {
        self.spec
            .is_some_and(|s| s.log && s.min > 0.0 && self.original > 0.0)
    }

    /// Lo que mueve un paso en un rango lineal (un entero, al menos 1).
    fn linear_step(&self, spec: Numeric) -> f64 {
        let step = (spec.max - spec.min) / FULL_RANGE_TICKS;
        if spec.integer {
            step.round().max(1.0)
        } else {
            step
        }
    }

    /// Lo que mueve un paso sin registro: 1 o la última cifra escrita.
    fn free_step(&self) -> f64 {
        10f64.powi(-(self.decimals as i32))
    }

    /// Los decimales que hacen falta para que un paso se vea donde está el
    /// valor ahora.
    fn needed_decimals(&self) -> usize {
        let Some(spec) = self.spec else {
            return self.decimals;
        };
        let here = self.raw(self.position).clamp(spec.min, spec.max);
        let step = if self.logarithmic() {
            here * ((spec.max / spec.min).powf(1.0 / FULL_RANGE_TICKS) - 1.0)
        } else {
            self.linear_step(spec)
        } * self.sensitivity;
        if !(step.is_finite() && step > 0.0) {
            return self.decimals;
        }
        // Que medio paso cambie la última cifra.
        let needed = -(step / 2.0).log10();
        if needed <= 0.0 {
            0
        } else {
            (needed.ceil() as usize).min(MAX_DECIMALS)
        }
    }
}

/// `value` escrito con `decimals` decimales, quitando los ceros del final
/// que pasan de `keep` (los que tenía escritos); sin `-0` y, con
/// `leading_dot`, sin el cero delante del punto.
pub fn written(value: f64, decimals: usize, keep: usize, leading_dot: bool) -> String {
    let mut text = format!("{value:.decimals$}");
    if decimals > keep && text.contains('.') {
        let (int, frac) = text.split_once('.').unwrap_or((&text, ""));
        let mut frac = frac.to_string();
        while frac.len() > keep && frac.ends_with('0') {
            frac.pop();
        }
        text = if frac.is_empty() {
            int.to_string()
        } else {
            format!("{int}.{frac}")
        };
    }
    // `-0`, `-0.00`: sin signo.
    if text.starts_with('-') && text[1..].chars().all(|c| c == '0' || c == '.') {
        text.remove(0);
    }
    if leading_dot {
        if let Some(rest) = text.strip_prefix("0.") {
            text = format!(".{rest}");
        } else if let Some(rest) = text.strip_prefix("-0.") {
            text = format!("-.{rest}");
        }
    }
    text
}

/// El valor con su unidad, como se enseña (`3200 Hz`, `-0.25`).
pub fn shown(text: &str, spec: Option<Numeric>) -> String {
    match spec.map(|s| s.unit).filter(|u| !u.is_empty()) {
        Some(unit) => format!("{text} {unit}"),
        None => text.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(min: f64, max: f64, log: bool, integer: bool) -> Option<Numeric> {
        Some(Numeric {
            min,
            max,
            unit: "",
            log,
            integer,
        })
    }

    #[test]
    fn integers_and_decimals_without_registry() {
        let mut s = Stepper::new("170", None, 1.0).unwrap();
        assert_eq!(s.nudge(1.0), "171");
        assert_eq!(s.nudge(-3.0), "168");
        let mut s = Stepper::new("0.25", None, 1.0).unwrap();
        assert_eq!(s.nudge(1.0), "0.26");
        assert_eq!(s.nudge(-27.0), "-0.01");
        assert_eq!(s.nudge(1.0), "0.00");
        let mut s = Stepper::new("-3", None, 1.0).unwrap();
        assert_eq!(s.nudge(4.0), "1");
        // Volver al principio es el texto de antes, tal cual.
        let mut s = Stepper::new(".5", None, 1.0).unwrap();
        assert_eq!(s.nudge(1.0), ".6");
        assert_eq!(s.nudge(-1.0), ".5");
        assert!(Stepper::new("0x10", None, 1.0).is_none());
        assert!(Stepper::new("1e3", None, 1.0).is_none());
        assert!(Stepper::new("-", None, 1.0).is_none());
    }

    #[test]
    fn the_registry_range_is_never_left_and_coming_back_is_immediate() {
        let mut s = Stepper::new("1", spec(0.0, 4.0, false, false), 1.0).unwrap();
        // Un paso: un sesentavo del rango, con los decimales que hacen
        // falta; el cero que sobra no se escribe.
        assert_eq!(s.nudge(1.0), "1.07");
        assert_eq!(s.nudge(-1.0), "1");
        for _ in 0..500 {
            s.nudge(1.0);
        }
        assert_eq!(s.text(), "4");
        assert_eq!(s.value(), 4.0);
        // En el borde: un paso atrás ya baja.
        assert!(s.nudge(-1.0).parse::<f64>().unwrap() < 4.0);
        for _ in 0..500 {
            s.nudge(-1.0);
        }
        assert_eq!(s.text(), "0");
        for t in [f64::NAN, f64::INFINITY, -f64::INFINITY] {
            let text = s.nudge(t);
            assert!(text.parse::<f64>().unwrap().is_finite(), "{t}: {text}");
        }
    }

    #[test]
    fn logarithmic_scale_multiplies_and_a_long_swipe_covers_the_range() {
        let mut s = Stepper::new("3000", spec(20.0, 20000.0, true, false), 1.0).unwrap();
        let up: f64 = s.nudge(1.0).parse().unwrap();
        let ratio = up / 3000.0;
        assert!((ratio - 1000f64.powf(1.0 / 60.0)).abs() < 0.01, "{up}");
        s.nudge(-1.0);
        assert_eq!(s.text(), "3000");
        // Abajo del todo y arriba del todo en un deslizamiento largo.
        let mut s = Stepper::new("3000", spec(20.0, 20000.0, true, false), 1.0).unwrap();
        s.nudge(-FULL_RANGE_TICKS);
        assert_eq!(s.value(), 20.0);
        s.nudge(FULL_RANGE_TICKS);
        assert_eq!(s.value(), 20000.0);
        assert_eq!(s.fraction(), Some(1.0));
        // Cerca de 20 Hz el paso es de 2,4 Hz: sigue en enteros.
        let mut s = Stepper::new("20", spec(20.0, 20000.0, true, false), 1.0).unwrap();
        assert_eq!(s.nudge(1.0), "22");
    }

    #[test]
    fn integers_of_the_registry_and_sensitivity() {
        let mut s = Stepper::new("8", spec(1.0, 16.0, false, true), 1.0).unwrap();
        assert_eq!(s.nudge(1.0), "9");
        // Media sensibilidad: un paso cada dos.
        let mut s = Stepper::new("8", spec(1.0, 16.0, false, true), 0.5).unwrap();
        assert_eq!(s.nudge(1.0), "8");
        assert_eq!(s.nudge(1.0), "9");
        // Escrito con decimales, un entero se queda sin ellos.
        let mut s = Stepper::new("4.0", spec(1.0, 16.0, false, true), 1.0).unwrap();
        assert_eq!(s.nudge(1.0), "5");
    }

    #[test]
    fn writing_keeps_the_format() {
        assert_eq!(written(1.5, 2, 2, false), "1.50");
        assert_eq!(written(1.5, 3, 0, false), "1.5");
        assert_eq!(written(2.0, 3, 0, false), "2");
        assert_eq!(written(-0.0001, 2, 2, false), "0.00");
        assert_eq!(written(-0.25, 2, 2, true), "-.25");
        assert_eq!(written(123456789.0, 0, 0, false), "123456789");
        assert_eq!(shown("3200", spec(20.0, 2e4, true, false)), "3200");
    }
}
