//! MIDI learn: asignaciones de un mando (un CC de un canal) a un parámetro
//! que se mueve de forma continua (`api::Auto::Smooth` en el registro): de
//! un efecto de pista, su volumen, pan y envíos, de un sintetizador o de los
//! buses. Se escriben por nombre (`drums lowpass.cutoff`), así que siguen
//! valiendo aunque el script cambie el orden de las pistas o de los
//! efectos; al cargar se traducen a la tabla del motor ([`resolve`]).
//!
//! En texto (proyecto y `config.toml`), una por línea:
//! `canal cc pista parámetro mínimo máximo curva`, con `*` para cualquier
//! canal, `bus` como pista de los buses y la curva `lin`, `log` o `cuad`:
//! `1 21 drums lowpass.cutoff 200 8000 log`.

use engine::{Fx, KnobCurve, KnobMap, KnobTarget, Knobs, MAX_KNOBS, MAX_TRACKS};

use crate::api;
use crate::{Song, TrackDef};

/// Un mando asignado.
#[derive(Debug, Clone, PartialEq)]
pub struct Learned {
    /// Canal (1-16); `None`, cualquiera.
    pub channel: Option<u8>,
    pub cc: u8,
    /// La pista (o `bus`).
    pub track: String,
    /// El parámetro: `lowpass.cutoff` (el segundo del mismo efecto,
    /// `lowpass#2.cutoff`), `gain`, `pan`, `send.reverb`, `synth.cutoff` o,
    /// en `bus`, `reverb.size`.
    pub param: String,
    pub min: f32,
    pub max: f32,
    pub curve: KnobCurve,
}

/// La pista de los buses en una asignación.
pub const BUS: &str = "bus";

impl Learned {
    /// `drums · lowpass.cutoff`.
    pub fn target(&self) -> String {
        format!("{} · {}", self.track, self.param)
    }

    /// `CC 21 · canal 1` (o `todos los canales`).
    pub fn source(&self) -> String {
        match self.channel {
            Some(c) => format!("CC {} · canal {c}", self.cc),
            None => format!("CC {} · todos los canales", self.cc),
        }
    }

    /// Recibe este mando el CC `cc` del canal `channel` (1-16).
    pub fn listens(&self, channel: u8, cc: u8) -> bool {
        self.cc == cc && self.channel.is_none_or(|c| c == channel)
    }

    /// Una línea de texto (ver el formato arriba).
    pub fn to_line(&self) -> String {
        let channel = self.channel.map_or("*".to_string(), |c| c.to_string());
        format!(
            "{channel} {} {} {} {} {} {}",
            self.cc,
            self.track,
            self.param,
            number(self.min),
            number(self.max),
            curve_name(self.curve)
        )
    }

    /// Lee una línea; el error dice qué falta.
    pub fn parse(line: &str) -> Result<Learned, String> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        let &[channel, cc, track, param, min, max, ref rest @ ..] = &parts[..] else {
            return Err(format!(
                "«{line}»: canal cc pista parámetro mínimo máximo [curva] \
                 (1 21 drums lowpass.cutoff 200 8000 log)"
            ));
        };
        let channel = match channel {
            "*" => None,
            c => Some(
                c.parse::<u8>()
                    .ok()
                    .filter(|c| (1..=16).contains(c))
                    .ok_or_else(|| format!("«{line}»: el canal es de 1 a 16, o *"))?,
            ),
        };
        let cc = cc
            .parse::<u8>()
            .ok()
            .filter(|c| *c < 128)
            .ok_or_else(|| format!("«{line}»: el CC es de 0 a 127"))?;
        let number = |text: &str| {
            text.parse::<f32>()
                .ok()
                .filter(|v| v.is_finite())
                .ok_or_else(|| format!("«{line}»: «{text}» no es un número"))
        };
        let curve = match *rest {
            [] => KnobCurve::Linear,
            [name] => {
                curve_named(name).ok_or_else(|| format!("«{line}»: la curva es lin, log o cuad"))?
            }
            _ => return Err(format!("«{line}»: sobra algo al final")),
        };
        Ok(Learned {
            channel,
            cc,
            track: track.to_string(),
            param: param.to_string(),
            min: number(min)?,
            max: number(max)?,
            curve,
        })
    }
}

/// `lin`, `log` o `cuad`.
pub fn curve_name(curve: KnobCurve) -> &'static str {
    match curve {
        KnobCurve::Linear => "lin",
        KnobCurve::Log => "log",
        KnobCurve::Square => "cuad",
    }
}

pub fn curve_named(name: &str) -> Option<KnobCurve> {
    match name {
        "lin" | "lineal" => Some(KnobCurve::Linear),
        "log" => Some(KnobCurve::Log),
        "cuad" => Some(KnobCurve::Square),
        _ => None,
    }
}

/// Un número sin ceros de más (`200`, `0.5`).
fn number(v: f32) -> String {
    let text = format!("{v}");
    text.strip_suffix(".0").unwrap_or(&text).to_string()
}

/// Un parámetro que se puede asignar a un mando.
#[derive(Debug, Clone, PartialEq)]
pub struct Learnable {
    pub track: String,
    pub param: String,
    /// Su rango entero (el de partida de una asignación nueva).
    pub min: f32,
    pub max: f32,
    /// Se mueve en escala logarítmica (frecuencias, tiempos).
    pub log: bool,
}

impl Learnable {
    /// Una asignación nueva de este parámetro al CC `cc` de `channel`, con
    /// su rango entero y la curva que le va.
    pub fn assign(&self, channel: Option<u8>, cc: u8) -> Learned {
        Learned {
            channel,
            cc,
            track: self.track.clone(),
            param: self.param.clone(),
            min: self.min,
            max: self.max,
            curve: if self.log {
                KnobCurve::Log
            } else {
                KnobCurve::Linear
            },
        }
    }
}

/// Lo que se puede asignar en `song`, por pista y en el orden de sus
/// efectos; los buses al final.
pub fn learnable(song: &Song) -> Vec<Learnable> {
    let mut out = Vec::new();
    for track in &song.tracks {
        if track.frozen.is_none() {
            for (slot, fx) in track.fx.iter().enumerate() {
                let name = fx_name(&track.fx, slot);
                for (p, param) in fx.params().iter().enumerate() {
                    if !smooth_fx(fx, param) {
                        continue;
                    }
                    let (min, max, log) = fx.param_range(p).unwrap_or((0.0, 1.0, false));
                    out.push(Learnable {
                        track: track.name.clone(),
                        param: format!("{name}.{param}"),
                        min,
                        max,
                        log,
                    });
                }
            }
            if track.instrument.and_then(|i| i.synth).is_some() {
                for p in engine::synth::PARAMS.iter() {
                    if smooth_synth(p.name) {
                        out.push(Learnable {
                            track: track.name.clone(),
                            param: format!("synth.{}", p.name),
                            min: p.min,
                            max: p.max,
                            log: p.log,
                        });
                    }
                }
            }
        }
        let mut mix = vec![
            ("gain".to_string(), 0.0, 1.0),
            ("pan".to_string(), -1.0, 1.0),
        ];
        mix.extend(
            api::BUSES
                .iter()
                .map(|b| (format!("send.{}", b.name), 0.0, 1.0)),
        );
        for (param, min, max) in mix {
            out.push(Learnable {
                track: track.name.clone(),
                param,
                min,
                max,
                log: false,
            });
        }
    }
    for (param, index) in BUS_PARAMS {
        let (min, max) = engine::bus::param_range(*index);
        out.push(Learnable {
            track: BUS.into(),
            param: param.to_string(),
            min,
            max,
            log: false,
        });
    }
    out
}

/// Los parámetros de los buses que se mueven, con su índice en el motor.
const BUS_PARAMS: &[(&str, usize)] = &[
    ("delay.feedback", engine::bus::DELAY_FEEDBACK),
    ("delay.level", engine::bus::DELAY_LEVEL),
    ("reverb.size", engine::bus::REVERB_SIZE),
    ("reverb.damp", engine::bus::REVERB_DAMP),
    ("reverb.level", engine::bus::REVERB_LEVEL),
];

/// Nombre del efecto de la ranura `slot` en una asignación: el del
/// registro y, si hay otro igual antes, `#2`, `#3`...
fn fx_name(chain: &[Fx], slot: usize) -> String {
    let name = api::effect_name(&chain[slot]);
    let before = chain[..slot]
        .iter()
        .filter(|f| api::effect_name(f) == name)
        .count();
    if before == 0 {
        name.to_string()
    } else {
        format!("{name}#{}", before + 1)
    }
}

/// El registro dice que este parámetro del efecto se mueve de forma
/// continua.
fn smooth_fx(fx: &Fx, param: &str) -> bool {
    api::effect(api::effect_name(fx))
        .and_then(|e| e.params.iter().find(|p| p.name == param))
        .is_some_and(|p| matches!(p.automate, api::Auto::Smooth))
}

/// Lo mismo para un parámetro del sintetizador (en cualquiera de ellos).
fn smooth_synth(param: &str) -> bool {
    api::SYNTHS.iter().any(|s| {
        s.params
            .iter()
            .any(|p| p.name == param && matches!(p.automate, api::Auto::Smooth))
    })
}

/// La tabla del motor para `learned` en `song` (`slots`: la pista del motor
/// de cada nombre). Las que no encuentran su parámetro se quedan sin mando
/// y se avisa; las que pasan de [`MAX_KNOBS`], también.
pub fn resolve(
    learned: &[Learned],
    song: &Song,
    slots: &[Option<String>; MAX_TRACKS],
) -> (Knobs, Vec<String>) {
    let mut knobs = [None; MAX_KNOBS];
    let mut warnings = Vec::new();
    for (i, l) in learned.iter().enumerate() {
        if i >= MAX_KNOBS {
            warnings.push(format!(
                "mando {}: hay {MAX_KNOBS} mandos como mucho",
                l.target()
            ));
            continue;
        }
        match target(l, song, slots) {
            Some((target, latch)) => {
                knobs[i] = Some(KnobMap {
                    target,
                    min: l.min,
                    max: l.max,
                    curve: l.curve,
                    latch,
                })
            }
            None => warnings.push(format!(
                "mando {} ({}): ese parámetro no está en el script",
                l.target(),
                l.source()
            )),
        }
    }
    (knobs, warnings)
}

/// El destino en el motor de una asignación y si es un valor fijo (el
/// mando se queda) o con curva (la devuelve).
fn target(
    l: &Learned,
    song: &Song,
    slots: &[Option<String>; MAX_TRACKS],
) -> Option<(KnobTarget, bool)> {
    if l.track == BUS {
        let index = BUS_PARAMS.iter().find(|(n, _)| *n == l.param)?.1;
        return Some((
            KnobTarget::Bus { param: index as u8 },
            song.buses.curves[index].is_none(),
        ));
    }
    let track: &TrackDef = song.tracks.iter().find(|t| t.name == l.track)?;
    let slot = slots.iter().position(|s| s.as_deref() == Some(&l.track))? as u8;
    let mix = |index: usize| {
        Some((
            KnobTarget::Mix {
                track: slot,
                index: index as u8,
            },
            track.mix_curves[index].is_none(),
        ))
    };
    match l.param.as_str() {
        "gain" => return mix(engine::MIX_GAIN),
        "pan" => return mix(engine::MIX_PAN),
        _ => {}
    }
    if let Some(bus) = l.param.strip_prefix("send.") {
        let b = api::BUSES.iter().position(|e| e.name == bus)?;
        return mix(engine::MIX_SEND + b);
    }
    if track.frozen.is_some() {
        return None;
    }
    if let Some(name) = l.param.strip_prefix("synth.") {
        let synth = track.instrument.and_then(|i| i.synth)?;
        let p = engine::synth::PARAMS.iter().position(|p| p.name == name)?;
        return Some((
            KnobTarget::Synth {
                track: slot,
                param: p as u8,
            },
            synth.curves[p].is_none(),
        ));
    }
    let (effect, param) = l.param.split_once('.')?;
    let fx = (0..track.fx.len()).find(|&s| fx_name(&track.fx, s) == effect)?;
    let p = track.fx[fx].param_index(param)?;
    if !smooth_fx(&track.fx[fx], param) {
        return None;
    }
    let curved = track.automation.iter().any(|a| a.fx == fx && a.param == p);
    Some((
        KnobTarget::Fx {
            track: slot,
            slot: fx as u8,
            param: p as u8,
        },
        !curved,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_goes_and_comes_back() {
        for line in [
            "1 21 drums lowpass.cutoff 200 8000 log",
            "* 74 bus reverb.size 0 1 lin",
            "16 0 bajo synth.cutoff 60.5 4000 cuad",
        ] {
            assert_eq!(Learned::parse(line).unwrap().to_line(), line);
        }
        let l = Learned::parse("2 7 drums gain 0 1").unwrap();
        assert_eq!(l.curve, KnobCurve::Linear);
        assert!(l.listens(2, 7) && !l.listens(1, 7) && !l.listens(2, 8));
        assert!(Learned::parse("* 7 drums gain 0 1").unwrap().listens(9, 7));
        for (bad, why) in [
            ("1 21 drums", "canal cc pista"),
            ("17 21 drums gain 0 1", "el canal es de 1 a 16"),
            ("1 200 drums gain 0 1", "el CC es de 0 a 127"),
            ("1 2 drums gain x 1", "no es un número"),
            ("1 2 drums gain 0 1 exp", "la curva es lin, log o cuad"),
        ] {
            let e = Learned::parse(bad).unwrap_err();
            assert!(e.contains(why), "{bad}: {e}");
        }
    }
}
