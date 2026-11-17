//! Qué parámetro del motor mueve un número escrito en el código, **solo si
//! se sabe seguro**, para que suene mientras se ajusta con el ratón (ver
//! `tui/src/app/adjust.rs`).
//!
//! Al evaluar el script, cada llamada que deja un valor que se mueve en vivo
//! (`:fx(nombre, { … })`, `:gain`, `:pan`, `:send` y las tablas de
//! `synth`, `sh101` y `subharm`) anota un **sitio** ([`LiveSite`]): la fila
//! que da Lua para la llamada, la función, su primera cadena, la pista (o
//! pistas) y los números que recibió. Solo en el trozo principal: ni dentro
//! de `every` ni de una sección ni de una variante (y una canción con
//! secciones o variantes no anota nada: sus pistas son copias).
//!
//! [`resolve`] busca el sitio de un literal con lo que el linter sabe del
//! texto ([`crate::lint::Place`]) y exige: un solo sitio con esa fila,
//! función y primera cadena (un bucle, una función llamada dos veces o dos
//! llamadas iguales en la misma expresión dan varios), una sola pista, que
//! el valor que recibió sea el del literal, que sea el último que escribió
//! ese volumen, pan o envío, y que el parámetro se mueva de forma continua
//! y sin curva (`learn`). Si algo no cuadra, no hay destino: el número
//! cambia en el texto y suena al compás siguiente.

use engine::{KnobCurve, KnobMap, MAX_TRACKS};
use mlua::{Lua, Table, Value};

use crate::Song;
use crate::learn::{self, Learned};
use crate::lint::Place;

/// Una llamada que dejó valores que se mueven en vivo.
#[derive(Debug, Clone, PartialEq)]
pub struct LiveSite {
    /// La fila (desde 0) que da Lua para la llamada.
    pub row: usize,
    /// `fx`, `gain`, `pan`, `send`, `synth`, `sh101` o `subharm`.
    pub call: &'static str,
    /// Su primera cadena: el efecto, el bus o el tipo de sintetizador.
    pub first: Option<String>,
    /// Las pistas a las que fue (un sintetizador puede ir a varias).
    pub tracks: Vec<String>,
    pub kind: LiveKind,
    /// Los números que recibió: por clave en una tabla, o `""` si es el
    /// argumento.
    pub values: Vec<(String, f64)>,
}

/// A qué va un sitio.
#[derive(Debug, Clone, PartialEq)]
pub enum LiveKind {
    /// El efecto de esa ranura de la cadena de la pista.
    Fx { slot: usize },
    /// `gain`, `pan` o `send.<bus>`.
    Mix(String),
    /// Los parámetros del sintetizador.
    Synth,
}

/// El destino en el motor de un número, con el rango de su mando.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LiveTarget {
    pub map: KnobMap,
}

impl LiveTarget {
    /// Dónde está `value` en el rango del mando (de 0 a 1).
    pub fn position(&self, value: f64) -> f32 {
        let (min, max) = (self.map.min as f64, self.map.max as f64);
        let t = if self.map.curve == KnobCurve::Log && min > 0.0 && max > 0.0 && value > 0.0 {
            (value / min).ln() / (max / min).ln()
        } else {
            (value - min) / (max - min)
        };
        if t.is_finite() {
            t.clamp(0.0, 1.0) as f32
        } else {
            0.0
        }
    }
}

/// Los números de una tabla, por clave.
pub(crate) fn numbers(table: Option<&Table>) -> Vec<(String, f64)> {
    let Some(table) = table else {
        return Vec::new();
    };
    table
        .pairs::<Value, Value>()
        .flatten()
        .filter_map(|(k, v)| {
            let key = match k {
                Value::String(s) => s.to_str().ok()?.to_string(),
                _ => return None,
            };
            let n = match v {
                Value::Integer(i) => i as f64,
                Value::Number(n) => n,
                _ => return None,
            };
            Some((key, n))
        })
        .collect()
}

/// Un número suelto (el argumento de `:gain`...).
pub(crate) fn number(value: &Value) -> Vec<(String, f64)> {
    match value {
        Value::Integer(i) => vec![(String::new(), *i as f64)],
        Value::Number(n) => vec![(String::new(), *n)],
        _ => Vec::new(),
    }
}

/// La fila de la llamada en curso si se anota (en el trozo principal, ni
/// en `every` ni en una sección ni después).
pub(crate) fn row(lua: &Lua, recording: bool) -> Option<usize> {
    if !recording {
        return None;
    }
    crate::written::current_row(lua)
}

/// El destino del número `value` escrito en `place` (con su clave), en la
/// canción `song` con las pistas del motor `slots`; o por qué no se sabe.
/// `range` (mínimo, máximo, logarítmica) es el del registro, si se sabe:
/// manda sobre el de los mandos MIDI (que para `gain` es de 0 a 1).
pub fn resolve(
    song: &Song,
    slots: &[Option<String>; MAX_TRACKS],
    place: &Place,
    value: f64,
    range: Option<(f64, f64, bool)>,
) -> Result<LiveTarget, &'static str> {
    let call = place.call.as_deref().ok_or("no está en una llamada")?;
    let key = place.key.clone().unwrap_or_default();
    if !place.bare {
        return Err("es parte de una expresión");
    }
    let mut candidates = song.live.iter().enumerate().filter(|(_, s)| {
        s.row == place.call_row && s.call == call && s.first.as_deref() == place.first.as_deref()
    });
    let (index, site) = candidates.next().ok_or("no se sabe de qué parámetro es")?;
    if candidates.next().is_some() {
        return Err("esa llamada se hace más de una vez");
    }
    let [track] = &site.tracks[..] else {
        return Err("va a más de una pista");
    };
    let written = site
        .values
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, v)| *v)
        .ok_or("no se sabe de qué parámetro es")?;
    if (written as f32) != (value as f32) {
        return Err("lo que llegó al parámetro no es este número");
    }
    let param = match &site.kind {
        LiveKind::Fx { slot } => {
            let def = song
                .tracks
                .iter()
                .find(|t| &t.name == track)
                .ok_or("la pista no suena")?;
            if *slot >= def.fx.len() {
                return Err("el efecto ya no está");
            }
            format!("{}.{key}", learn::fx_name(&def.fx, *slot))
        }
        LiveKind::Mix(name) => {
            // Si otra llamada lo escribe después, la que suena es esa.
            let later = song.live[index + 1..]
                .iter()
                .any(|s| s.kind == site.kind && s.tracks.contains(track));
            if later {
                return Err("otra línea lo cambia después");
            }
            name.clone()
        }
        LiveKind::Synth => format!("synth.{key}"),
    };
    let learnable = learn::learnable(song)
        .into_iter()
        .find(|l| &l.track == track && l.param == param)
        .ok_or("no se mueve en vivo")?;
    let (min, max, log) = range
        .map(|(a, b, log)| (a as f32, b as f32, log))
        .unwrap_or((learnable.min, learnable.max, learnable.log));
    let learned = Learned {
        channel: None,
        cc: 0,
        track: track.clone(),
        param,
        min,
        max,
        curve: if log && min > 0.0 {
            KnobCurve::Log
        } else {
            KnobCurve::Linear
        },
    };
    let (target, fixed) = learn::target(&learned, song, slots).ok_or("no se mueve en vivo")?;
    if !fixed {
        return Err("tiene una curva (ramp, lfo o lane)");
    }
    Ok(LiveTarget {
        map: KnobMap {
            target,
            min: learned.min,
            max: learned.max,
            curve: learned.curve,
            latch: true,
        },
    })
}
