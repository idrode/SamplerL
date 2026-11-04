//! Lanes de automatización desde Lua: `pista:auto(parámetro, puntos,
//! opciones)`, `pista:auto_clear([parámetro])`, `pista:auto_list()` y
//! `lane(puntos, opciones)` (una curva más, como `ramp`).
//!
//! Una lane es una curva del motor (`Curve::Lane`, ver `engine::lane`) que
//! apunta a un hueco de la tabla de lanes de la canción (`Song::lanes`, como
//! mucho `engine::MAX_LANES`): `auto` la pone en el sitio de las curvas del
//! parámetro (la automatización de su efecto, la mezcla, el pitch, el
//! sintetizador o la forma de los cortes), así que se suaviza y se combina
//! como cualquier curva. Las posiciones de los puntos van en compases (los
//! del compás de la canción) y la duración, en compases, figuras, pasos o
//! ms.

use mlua::{Lua, Table, Value};

use super::{Builder, CurveHandle, TrackBuild, TrackDef, api, fail, location};
use engine::{Curve, LaneCurve, LaneMode};

/// Una lane de la canción: a qué pista y parámetro va (si es de `auto`),
/// la del motor y sus puntos tal como se escribieron (para la vista y la
/// edición).
#[derive(Debug, Clone, PartialEq)]
pub struct LaneDef {
    /// La pista de `auto` (`None`: una `lane()` suelta).
    pub track: Option<String>,
    /// El parámetro, con su nombre completo (`"lowpass.cutoff"`); en una
    /// `lane()` suelta, vacío.
    pub param: String,
    pub lane: engine::Lane,
    /// Los puntos como se escribieron: posición en compases, valor (antes
    /// de `range`) y curva.
    pub points: Vec<(f64, f32, LaneCurve)>,
    /// Pasos de un compás al hacerla.
    pub bar_steps: usize,
    /// `range = {mín, máx}` si los valores van de 0 a 1.
    pub range: Option<(f32, f32)>,
    /// Lo que mide el parámetro (para la vista): su rango.
    pub bounds: (f32, f32),
    /// La escala en que se dibuja y se edita: el rango de `range` o el del
    /// parámetro, y si es logarítmica (las frecuencias).
    pub scale: (f32, f32, bool),
    /// Con `range`, los valores escritos van de 0 a 1 en escala log.
    pub range_log: bool,
    /// Dónde está escrita la tabla de puntos (para editarla desde la
    /// interfaz), o por qué no se puede.
    pub written: Result<super::TableSpan, String>,
    /// A qué va (`None`: una `lane()` suelta).
    pub target: Option<LaneTarget>,
    /// Lo que la lane lleva multiplicado respecto a lo escrito (los
    /// parámetros del sintetizador van en las unidades de su núcleo): el
    /// valor de verdad es el de la lane dividido por esto.
    pub unit_scale: f32,
    /// Dónde se llamó (`song.lua:12: `).
    pub location: String,
}

impl LaneDef {
    /// Lo que dura, en compases.
    pub fn bars(&self) -> f64 {
        self.lane.length() / self.bar_steps as f64
    }
}

/// A qué va una lane de `auto()`, para quien la enseña: el efecto (su
/// índice en la cadena de la pista) y el nombre de su parámetro, la
/// mezcla, el pitch, el formante, un parámetro del sintetizador o uno de
/// la forma de los cortes.
#[derive(Debug, Clone, PartialEq)]
pub enum LaneTarget {
    Fx {
        slot: usize,
        param: &'static str,
    },
    /// Volumen, pan o un envío (`engine::MIX_*`).
    Mix(usize),
    Pitch,
    Formant,
    Synth(&'static str),
    Shape(&'static str),
}

/// Lo que puede seguir una lane en una pista.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum AutoTarget {
    /// El parámetro `param` del efecto `slot` de la cadena.
    Fx {
        slot: usize,
        param: usize,
    },
    /// Volumen, pan o un envío (`engine::MIX_*`).
    Mix(usize),
    Pitch,
    Formant,
    /// El parámetro `name` del sintetizador.
    Synth(&'static str),
    /// Un parámetro de la forma de todos los cortes (`engine::SHAPE_*`): se
    /// lee al nacer cada golpe.
    Shape(usize),
}

/// Un destino posible, con los nombres por los que se le llama.
struct Candidate {
    /// El nombre completo (`"lowpass.cutoff"`, `"send.reverb"`).
    full: String,
    /// Otros nombres que valen (`"2.cutoff"`, `"lp.cutoff"`).
    aliases: Vec<String>,
    /// El nombre suelto (`"cutoff"`): si lo tienen varios, el primero, con
    /// aviso.
    bare: String,
    target: AutoTarget,
    /// Rango del parámetro (y si es logarítmico), su valor fijo y cuánto se
    /// escala (el sintetizador, a las unidades de su núcleo).
    range: (f32, f32),
    log: bool,
    rest: f32,
    scale: f32,
}

/// Los parámetros de la forma de los cortes que admiten curvas, con su
/// índice del motor.
const SHAPE_TARGETS: &[(&str, usize)] = &[
    ("attack", engine::SHAPE_ATTACK),
    ("release", engine::SHAPE_RELEASE),
    ("pan", engine::SHAPE_PAN),
    ("length", engine::SHAPE_LENGTH),
    ("crossfade", engine::SHAPE_CROSSFADE),
    ("bend", engine::SHAPE_BEND),
    ("bend_time", engine::SHAPE_BEND_TIME),
    ("formant", engine::SHAPE_FORMANT),
];

/// El nombre del registro de un efecto de la cadena y sus alias.
fn effect_names(fx: &engine::Fx) -> Vec<&'static str> {
    let name = match fx {
        engine::Fx::Filter { kind, .. } => match kind {
            engine::FilterKind::LowPass => "lowpass",
            engine::FilterKind::HighPass => "highpass",
        },
        engine::Fx::Bitcrush { .. } => "bitcrush",
        engine::Fx::Compressor { .. } => "compressor",
        engine::Fx::Distortion { .. } => "distortion",
        engine::Fx::Gain { .. } => "gain",
        engine::Fx::Eq { .. } => "eq",
        engine::Fx::DynEq(_) => "dyneq",
    };
    let mut names = vec![name];
    if let Some(effect) = api::effect(name) {
        names.extend(effect.aliases.iter().copied());
    }
    if matches!(fx, engine::Fx::Filter { .. }) {
        names.push("filter");
    }
    names
}

/// Todo lo que puede seguir una lane en la pista, en el orden en que se
/// resuelve un nombre suelto: efectos (en su orden), mezcla, pitch y
/// formante de la pista, sintetizador y forma de los cortes.
fn candidates(def: &TrackDef) -> Vec<Candidate> {
    let mut out = Vec::new();
    for (slot, fx) in def.fx.iter().enumerate() {
        let names = effect_names(fx);
        for (p, param) in fx.params().iter().enumerate() {
            let Some((lo, hi, log)) = fx.param_range(p) else {
                continue;
            };
            // Lo que el registro dice que no se mueve, fuera.
            if api::effect(names[0])
                .and_then(|e| e.param(param))
                .is_some_and(|p| !p.automate.curves())
            {
                continue;
            }
            let mut aliases: Vec<String> =
                names[1..].iter().map(|n| format!("{n}.{param}")).collect();
            aliases.push(format!("{}.{param}", slot + 1));
            out.push(Candidate {
                full: format!("{}.{param}", names[0]),
                aliases,
                bare: param.to_string(),
                target: AutoTarget::Fx { slot, param: p },
                range: (lo, hi),
                log,
                rest: fx.param_value(p).unwrap_or(lo),
                scale: 1.0,
            });
        }
    }
    let mix = |full: &str, bare: &str, index, range, rest| Candidate {
        full: full.to_string(),
        aliases: Vec::new(),
        bare: bare.to_string(),
        target: AutoTarget::Mix(index),
        range,
        log: false,
        rest,
        scale: 1.0,
    };
    out.push(mix(
        "gain",
        "gain",
        engine::MIX_GAIN,
        (0.0, engine::MAX_GAIN),
        def.gain,
    ));
    out.push(mix("pan", "pan", engine::MIX_PAN, (-1.0, 1.0), def.pan));
    for bus in api::BUSES {
        let index = match bus.name {
            "delay" => engine::bus::DELAY,
            _ => engine::bus::REVERB,
        };
        out.push(mix(
            &format!("send.{}", bus.name),
            &format!("send.{}", bus.name),
            engine::MIX_SEND + index,
            (0.0, engine::bus::MAX_SEND),
            def.sends[index],
        ));
    }
    let semitones = engine::MAX_SEMITONES;
    out.push(Candidate {
        full: "pitch".into(),
        aliases: Vec::new(),
        bare: "pitch".into(),
        target: AutoTarget::Pitch,
        range: (-semitones, semitones),
        log: false,
        rest: def.pitch,
        scale: 1.0,
    });
    let formant = engine::MAX_FORMANT;
    out.push(Candidate {
        full: "formant".into(),
        aliases: Vec::new(),
        bare: "formant".into(),
        target: AutoTarget::Formant,
        range: (-formant, formant),
        log: false,
        rest: def.formant.unwrap_or(0.0),
        scale: 1.0,
    });
    if let Some(synth) = def.instrument.and_then(|i| i.synth)
        && let (Some(registered), Some(preset)) = (
            api::SYNTHS.iter().find(|s| s.name == synth.kind),
            engine::synth::preset::preset(synth.kind),
        )
    {
        for param in registered.params {
            let Some(p) = preset.param(param.name).filter(|p| p.moves()) else {
                continue;
            };
            if !param.automate.curves() {
                continue;
            }
            out.push(Candidate {
                full: format!("synth.{}", param.name),
                aliases: Vec::new(),
                bare: param.name.to_string(),
                target: AutoTarget::Synth(param.name),
                range: (p.min, p.max),
                log: false,
                rest: p.get(&synth.patch),
                scale: p.scale,
            });
        }
    }
    if def.instrument.is_none() {
        let first = def.shapes.first().copied().unwrap_or(super::DEFAULT_SHAPE);
        for &(name, index) in SHAPE_TARGETS {
            let (lo, hi) = engine::SliceShape::curve_range(index);
            let rest = match index {
                engine::SHAPE_ATTACK => first.attack_ms,
                engine::SHAPE_RELEASE => first.release_ms,
                engine::SHAPE_PAN => first.pan,
                engine::SHAPE_LENGTH => first.length,
                engine::SHAPE_CROSSFADE => first.crossfade_ms,
                engine::SHAPE_BEND => first.bend,
                engine::SHAPE_BEND_TIME => first.bend_ms,
                _ => first.formant.unwrap_or(0.0),
            };
            out.push(Candidate {
                full: format!("slice.{name}"),
                aliases: Vec::new(),
                bare: name.to_string(),
                target: AutoTarget::Shape(index),
                range: (lo, hi),
                log: false,
                rest,
                scale: 1.0,
            });
        }
    }
    out
}

/// El destino de `name` en la pista y, si el nombre suelto era de varios,
/// el aviso (se queda con el primero).
fn resolve(def: &TrackDef, name: &str) -> Result<(Candidate, Option<String>), String> {
    let all = candidates(def);
    let exact = |c: &Candidate| c.full == name || c.aliases.iter().any(|a| a == name);
    // Los que valen con ese nombre: primero el que lo tiene como nombre
    // completo o alias (`"pan"` es el de la pista), luego los sueltos.
    let mut matching: Vec<Candidate> = Vec::new();
    let mut rest: Vec<Candidate> = Vec::new();
    for c in all {
        if exact(&c) {
            matching.insert(0, c);
        } else if c.bare == name {
            rest.push(c);
        }
    }
    let qualified = matching
        .first()
        .is_some_and(|c| c.full != name && c.bare != name);
    matching.extend(rest);
    if matching.is_empty() {
        return Err(format!(
            "la pista no tiene el parámetro \"{name}\" (o no admite curvas); los que sí: {}",
            candidates(def)
                .iter()
                .map(|c| c.full.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    // Un nombre completo o un alias (`"lowpass.cutoff"`, `"2.q"`) no es
    // ambiguo; uno suelto de varios sitios, sí.
    let warning = (!qualified && matching.len() > 1).then(|| {
        let others: Vec<&str> = matching[1..].iter().map(|c| c.full.as_str()).collect();
        format!(
            "\"{name}\" es de varios sitios: va a \"{}\" (para los otros: {})",
            matching[0].full,
            others.join(", ")
        )
    });
    Ok((matching.swap_remove(0), warning))
}

/// Una duración o posición escrita en una lane, en pasos: un número son
/// compases; `"N compases"`, `"N pasos"`, una figura `"n/d"` (fracción de
/// redonda) o `"N ms"` (al tempo de la canción al cargar).
fn steps_of(value: &Value, bar_steps: usize, bpm: f64) -> Option<f64> {
    let bar = bar_steps as f64;
    match value {
        Value::Integer(n) => Some(*n as f64 * bar),
        Value::Number(n) => n.is_finite().then_some(*n * bar),
        Value::String(text) => {
            let text = text.to_str().ok()?.trim().to_string();
            if let Some(ms) = text
                .strip_suffix("ms")
                .and_then(|n| n.trim().replace(',', ".").parse::<f64>().ok())
            {
                return Some(ms / 1000.0 * bpm * 4.0 / 60.0);
            }
            if let Some((n, d)) = text.split_once('/') {
                let (n, d): (f64, f64) = (n.trim().parse().ok()?, d.trim().parse().ok()?);
                return (d > 0.0).then_some(engine::STEPS as f64 * n / d);
            }
            let mut words = text.split_whitespace();
            let n: f64 = words.next()?.replace(',', ".").parse().ok()?;
            let unit = words.next()?;
            if words.next().is_some() {
                return None;
            }
            match unit {
                "compás" | "compas" | "compases" | "bar" | "bars" => Some(n * bar),
                "paso" | "pasos" | "step" | "steps" => Some(n),
                _ => None,
            }
        }
        _ => None,
    }
}

/// La curva de un punto por su nombre.
fn curve_name(name: &str) -> Option<LaneCurve> {
    match name {
        "linear" | "lin" => Some(LaneCurve::Linear),
        "exp" => Some(LaneCurve::Exp),
        "smooth" => Some(LaneCurve::Smooth),
        "step" => Some(LaneCurve::Step),
        _ => None,
    }
}

/// Lo que dice la tabla de opciones de una lane.
struct Options {
    length: Option<f64>,
    mode: LaneMode,
    offset: f64,
    range: Option<(f32, f32)>,
    log: bool,
    rest: Option<f32>,
}

fn options(
    lua: &Lua,
    who: &str,
    table: Option<&Table>,
    bar_steps: usize,
    bpm: f64,
) -> mlua::Result<Options> {
    let mut out = Options {
        length: None,
        mode: LaneMode::Loop,
        offset: 0.0,
        range: None,
        log: false,
        rest: None,
    };
    let Some(table) = table else {
        return Ok(out);
    };
    let known: Vec<&str> = api::LANE_PARAMS.iter().map(|p| p.name).collect();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let key = match &key {
            Value::String(k) => k.to_str()?.to_string(),
            other => format!("{other:?}"),
        };
        let time = |what: &str| {
            steps_of(&value, bar_steps, bpm).ok_or_else(|| {
                fail(
                    lua,
                    format!(
                        "{who}: {what} es un número de compases, \"N compases\", \"N pasos\", una figura (\"3/4\") o \"N ms\""
                    ),
                )
            })
        };
        match key.as_str() {
            "len" => {
                let steps = time("len")?;
                if !(steps > 0.0 && steps <= engine::MAX_LANE_STEPS) {
                    return Err(fail(
                        lua,
                        format!("{who}: len va de más de 0 a 64 compases"),
                    ));
                }
                out.length = Some(steps);
            }
            "offset" => out.offset = time("offset")?,
            "mode" => {
                let text = match &value {
                    Value::String(s) => s.to_str()?.to_string(),
                    _ => String::new(),
                };
                out.mode = match text.as_str() {
                    "loop" => LaneMode::Loop,
                    "once" => LaneMode::Once,
                    "hold" => LaneMode::Hold,
                    _ => {
                        return Err(fail(
                            lua,
                            format!(
                                "{who}: mode = \"{text}\" no existe (\"loop\", \"once\" o \"hold\")"
                            ),
                        ));
                    }
                };
            }
            "range" => {
                let pair = match &value {
                    Value::Table(t) => (t.get::<f32>(1).ok(), t.get::<f32>(2).ok()),
                    _ => (None, None),
                };
                let (Some(lo), Some(hi)) = pair else {
                    return Err(fail(lua, format!("{who}: range es {{mín, máx}}")));
                };
                out.range = Some((lo, hi));
            }
            "scale" => {
                out.log = match &value {
                    Value::String(s) if s.to_str()?.as_ref() as &str == "log" => true,
                    Value::String(s) if s.to_str()?.as_ref() as &str == "lin" => false,
                    _ => {
                        return Err(fail(lua, format!("{who}: scale es \"lin\" o \"log\"")));
                    }
                };
            }
            "rest" => match value {
                Value::Integer(n) => out.rest = Some(n as f32),
                Value::Number(n) => out.rest = Some(n as f32),
                _ => return Err(fail(lua, format!("{who}: rest es un número"))),
            },
            _ => {
                return Err(fail(
                    lua,
                    format!(
                        "{who}: opción desconocida \"{key}\" (admite: {})",
                        known.join(", ")
                    ),
                ));
            }
        }
    }
    Ok(out)
}

/// Lo que da [`build`]: la lane, sus puntos como se escribieron y sus
/// opciones.
type Built = (engine::Lane, Vec<(f64, f32, LaneCurve)>, Options);

/// Hace la lane de `points` con `table` (opciones): los valores, en el
/// rango del parámetro (`bounds`, si se sabe) y escalados por `scale`; su
/// valor fijo para `once`, `rest` (si las opciones no dicen otro).
#[allow(clippy::too_many_arguments)]
fn build(
    lua: &Lua,
    who: &str,
    points: &Table,
    table: Option<&Table>,
    builder: &Builder,
    bounds: Option<(f32, f32)>,
    rest: Option<f32>,
    scale: f32,
) -> mlua::Result<Built> {
    let bar_steps = builder.meter.map_or(engine::STEPS, |m| m.steps);
    let bpm = builder.bpm.unwrap_or(engine::DEFAULT_BPM);
    let opts = options(lua, who, table, bar_steps, bpm)?;
    let mut written = Vec::new();
    for (i, point) in points.clone().sequence_values::<Value>().enumerate() {
        let wrong = || {
            fail(
                lua,
                format!(
                    "{who}: el punto {} es {{posición en compases, valor [, \"linear\"|\"exp\"|\"smooth\"|\"step\"]}}",
                    i + 1
                ),
            )
        };
        let Value::Table(point) = point? else {
            return Err(wrong());
        };
        let at: f64 = point.get(1).map_err(|_| wrong())?;
        let value: f32 = point.get(2).map_err(|_| wrong())?;
        let curve = match point.get::<Value>(3)? {
            Value::Nil => LaneCurve::Linear,
            Value::String(s) => {
                let name = s.to_str()?.to_string();
                curve_name(&name).ok_or_else(|| {
                    fail(
                        lua,
                        format!(
                            "{who}: el punto {}: la curva \"{name}\" no existe (\"linear\", \"exp\", \"smooth\" o \"step\")",
                            i + 1
                        ),
                    )
                })?
            }
            _ => return Err(wrong()),
        };
        written.push((at, value, curve));
    }
    if written.is_empty() {
        return Err(fail(
            lua,
            format!("{who}: faltan los puntos {{ {{0, valor}}, ... }}"),
        ));
    }
    // Los valores de verdad: con `range`, de 0 a 1 al rango.
    let real = |v: f32| match opts.range {
        None => v,
        Some((lo, hi)) if opts.log && lo > 0.0 && hi > 0.0 => lo * (hi / lo).powf(v),
        Some((lo, hi)) => lo + v * (hi - lo),
    };
    if let Some((lo, hi)) = bounds {
        for (i, &(_, v, _)) in written.iter().enumerate() {
            let value = real(v);
            if !(lo..=hi).contains(&value) {
                return Err(fail(
                    lua,
                    format!(
                        "{who}: el punto {} vale {value}, fuera del rango del parámetro ({lo} a {hi})",
                        i + 1
                    ),
                ));
            }
        }
    }
    let last = written.iter().map(|p| p.0).fold(0.0, f64::max);
    let length = opts
        .length
        .unwrap_or_else(|| last.ceil().max(1.0) * bar_steps as f64);
    let points: Vec<engine::LanePoint> = written
        .iter()
        .map(|&(at, v, curve)| engine::LanePoint {
            at: at * bar_steps as f64,
            value: real(v) * scale,
            curve,
        })
        .collect();
    let rest = opts.rest.or(rest).unwrap_or(points[0].value / scale) * scale;
    let lane = engine::Lane::new(&points, length, opts.log)
        .and_then(|l| l.with_timing(opts.mode, opts.offset, rest))
        .map_err(|e| fail(lua, format!("{who}: {e}")))?;
    Ok((lane, written, opts))
}

/// Mete la lane en la tabla de la canción (en `slot`, si ya tenía uno) y
/// devuelve su índice; si no caben más, error sin tocar nada.
fn place(builder: &mut Builder, slot: Option<usize>, def: LaneDef) -> Result<usize, String> {
    if let Some(i) = slot {
        builder.lanes[i] = Some(def);
        return Ok(i);
    }
    if let Some(i) = builder.lanes.iter().position(Option::is_none) {
        builder.lanes[i] = Some(def);
        return Ok(i);
    }
    if builder.lanes.len() >= engine::MAX_LANES {
        return Err(format!(
            "no caben más lanes: hay {} como mucho (engine::MAX_LANES); quita alguna con auto_clear o usa la misma para varios parámetros con lane()",
            engine::MAX_LANES
        ));
    }
    builder.lanes.push(Some(def));
    Ok(builder.lanes.len() - 1)
}

/// Pone (o quita, con `None`) la curva de `target` en la pista.
fn assign(track: &mut TrackBuild, target: AutoTarget, curve: Option<Curve>) {
    let def = &mut track.def;
    match target {
        AutoTarget::Fx { slot, param } => {
            def.automation
                .retain(|a| !(a.fx == slot && a.param == param));
            if let Some(curve) = curve {
                def.automation.push(engine::Automation {
                    fx: slot,
                    param,
                    curve,
                });
            }
        }
        AutoTarget::Mix(index) => def.mix_curves[index] = curve,
        AutoTarget::Pitch => def.pitch_curve = curve,
        AutoTarget::Formant => def.formant_curve = curve,
        AutoTarget::Synth(name) => {
            if let Some(synth) = def.instrument.as_mut().and_then(|i| i.synth.as_mut())
                && let Some(p) =
                    engine::synth::preset::preset(synth.kind).and_then(|p| p.param(name))
            {
                // La curva ya va en las unidades del núcleo (los puntos, por
                // `scale`; sus extremos, también).
                if let engine::synth::preset::Target::Param(index) = p.target {
                    synth.curves[index] = curve;
                }
            }
        }
        AutoTarget::Shape(index) => {
            track.shape_lanes.retain(|(i, _)| *i != index);
            if let Some(curve) = curve {
                track.shape_lanes.push((index, curve));
            }
        }
    }
}

/// `pista:auto(parámetro, puntos [, opciones])`: devuelve la pista y, si el
/// nombre era de varios sitios, el aviso.
pub(crate) fn auto(
    lua: &Lua,
    builder: &mut Builder,
    name: &str,
    param: &str,
    points: &Table,
    table: Option<&Table>,
) -> mlua::Result<Option<String>> {
    let who = format!("auto(\"{param}\")");
    let track = builder
        .tracks
        .iter()
        .find(|t| t.def.name == name)
        .expect("la pista existe desde track()");
    let (target, warning) =
        resolve(&track.def, param).map_err(|e| fail(lua, format!("{who}: {e}")))?;
    let slot = track
        .lanes
        .iter()
        .find(|(p, _)| *p == target.full)
        .map(|&(_, i)| i);
    let (lane, written, opts) = build(
        lua,
        &who,
        points,
        table,
        builder,
        Some(target.range),
        Some(target.rest),
        target.scale,
    )?;
    let (lo, hi) = lane.bounds();
    let source = written_table(lua, param, &written);
    let scale = match opts.range {
        Some((a, b)) => (a, b, opts.log),
        None => (target.range.0, target.range.1, target.log),
    };
    let def = LaneDef {
        track: Some(name.to_string()),
        param: target.full.clone(),
        lane,
        points: written,
        bar_steps: builder.meter.map_or(engine::STEPS, |m| m.steps),
        range: opts.range,
        bounds: target.range,
        scale,
        range_log: opts.log,
        written: source,
        target: Some(public_target(&track_def_fx(builder, name), target.target)),
        unit_scale: target.scale,
        location: location(lua),
    };
    let index = place(builder, slot, def).map_err(|e| fail(lua, format!("{who}: {e}")))?;
    let curve = Curve::Lane {
        index: index as u8,
        min: lo,
        max: hi,
    };
    let track = builder
        .tracks
        .iter_mut()
        .find(|t| t.def.name == name)
        .expect("la pista existe desde track()");
    assign(track, target.target, Some(curve));
    if slot.is_none() {
        track.lanes.push((target.full.clone(), index));
    }
    if let Some(warning) = &warning {
        let warning = format!("{}{who}: {warning}", location(lua));
        let mut warnings = builder.warnings.borrow_mut();
        if !warnings.contains(&warning) {
            warnings.push(warning);
        }
    }
    Ok(warning)
}

/// `pista:auto_clear([parámetro])`: quita la lane de ese parámetro (o
/// todas las de la pista); el parámetro vuelve a su valor fijo.
pub(crate) fn auto_clear(
    lua: &Lua,
    builder: &mut Builder,
    name: &str,
    param: Option<&str>,
) -> mlua::Result<()> {
    let track = builder
        .tracks
        .iter()
        .find(|t| t.def.name == name)
        .expect("la pista existe desde track()");
    let full = match param {
        None => None,
        Some(param) => Some(
            resolve(&track.def, param)
                .map_err(|e| fail(lua, format!("auto_clear(\"{param}\"): {e}")))?
                .0,
        ),
    };
    let gone: Vec<(String, usize)> = track
        .lanes
        .iter()
        .filter(|(p, _)| full.as_ref().is_none_or(|c| &c.full == p))
        .cloned()
        .collect();
    let all = candidates(&track.def);
    for (p, index) in &gone {
        builder.lanes[*index] = None;
        let track = builder
            .tracks
            .iter_mut()
            .find(|t| t.def.name == name)
            .expect("la pista existe");
        if let Some(c) = all.iter().find(|c| &c.full == p) {
            assign(track, c.target, None);
        }
        track.lanes.retain(|(q, _)| q != p);
    }
    Ok(())
}

/// `pista:auto_list()`: las lanes de la pista, como tablas de Lua.
pub(crate) fn auto_list(lua: &Lua, builder: &Builder, name: &str) -> mlua::Result<Table> {
    let track = builder
        .tracks
        .iter()
        .find(|t| t.def.name == name)
        .expect("la pista existe desde track()");
    let list = lua.create_table()?;
    for (param, index) in &track.lanes {
        let Some(Some(def)) = builder.lanes.get(*index) else {
            continue;
        };
        let item = lua.create_table()?;
        item.set("param", param.as_str())?;
        item.set("len", def.bars())?;
        item.set(
            "mode",
            match def.lane.mode() {
                LaneMode::Loop => "loop",
                LaneMode::Once => "once",
                LaneMode::Hold => "hold",
            },
        )?;
        let points = lua.create_table()?;
        for &(at, value, curve) in &def.points {
            let point = lua.create_table()?;
            point.push(at)?;
            point.push(value)?;
            point.push(match curve {
                LaneCurve::Linear => "linear",
                LaneCurve::Exp => "exp",
                LaneCurve::Smooth => "smooth",
                LaneCurve::Step => "step",
            })?;
            points.push(point)?;
        }
        item.set("points", points)?;
        list.push(item)?;
    }
    Ok(list)
}

/// `lane(puntos [, opciones])`: una curva más, para ponerla donde va un
/// `ramp()`; su rango lo comprueba quien la usa.
pub(crate) fn lane(
    lua: &Lua,
    builder: &mut Builder,
    points: &Table,
    table: Option<&Table>,
) -> mlua::Result<CurveHandle> {
    let who = "lane".to_string();
    let (lane, written, opts) = build(lua, &who, points, table, builder, None, None, 1.0)?;
    let (lo, hi) = lane.bounds();
    let def = LaneDef {
        track: None,
        param: String::new(),
        lane,
        points: written,
        bar_steps: builder.meter.map_or(engine::STEPS, |m| m.steps),
        range: opts.range,
        bounds: (lo, hi),
        scale: opts
            .range
            .map_or((lo, hi, opts.log), |(a, b)| (a, b, opts.log)),
        range_log: opts.log,
        written: Err("una lane() suelta no tiene pista: edítala en el código".into()),
        target: None,
        unit_scale: 1.0,
        location: location(lua),
    };
    let index = place(builder, None, def).map_err(|e| fail(lua, format!("{who}: {e}")))?;
    Ok(CurveHandle(Curve::Lane {
        index: index as u8,
        min: lo,
        max: hi,
    }))
}

/// Dónde está escrita la tabla de puntos de `auto("param", ...)` en la
/// llamada en curso, si está tal cual (solo números, cadenas de curva y
/// llaves) y dice lo mismo que se leyó; si no, por qué y cómo arreglarlo.
fn written_table(
    lua: &Lua,
    param: &str,
    points: &[(f64, f32, LaneCurve)],
) -> Result<super::TableSpan, String> {
    let how = format!(
        "para editarla desde el panel, escríbela tal cual dentro de la llamada: \
         auto(\"{param}\", {{ {{0, 200}}, {{1, 2000, \"exp\"}} }})"
    );
    let span = super::written::call_table(lua, "auto", param)
        .map_err(|why| format!("auto(\"{param}\"): {why}; {how}"))?;
    let text = super::written::span_text(lua, span).unwrap_or_default();
    match parse_points(&text) {
        Some(read)
            if read.len() == points.len()
                && read
                    .iter()
                    .zip(points)
                    .all(|(a, b)| a.0 == b.0 && a.1 == b.1 && a.2 == b.2) =>
        {
            Ok(span)
        }
        Some(_) => Err(format!(
            "auto(\"{param}\"): la tabla escrita no es la que llega (¿otra llamada igual en la misma línea?); {how}"
        )),
        None => Err(format!(
            "auto(\"{param}\"): la tabla de puntos no es literal (lleva variables, cuentas o comentarios); {how}"
        )),
    }
}

/// Lee una tabla de puntos escrita tal cual: `{ {0, 200}, {1, 2000, "exp"} }`
/// (con espacios, saltos de línea y una coma final), o nada si lleva otra
/// cosa.
pub fn parse_points(text: &str) -> Option<Vec<(f64, f32, LaneCurve)>> {
    let mut chars = text.trim().chars().peekable();
    let skip = |c: &mut std::iter::Peekable<std::str::Chars>| {
        while c.peek().is_some_and(|ch| ch.is_whitespace()) {
            c.next();
        }
    };
    let number = |c: &mut std::iter::Peekable<std::str::Chars>| -> Option<f64> {
        skip(c);
        let mut text = String::new();
        while let Some(&ch) = c.peek() {
            if ch.is_ascii_digit() || matches!(ch, '-' | '+' | '.' | 'e' | 'E') {
                text.push(ch);
                c.next();
            } else {
                break;
            }
        }
        text.parse().ok()
    };
    if chars.next()? != '{' {
        return None;
    }
    let mut out = Vec::new();
    loop {
        skip(&mut chars);
        match chars.next()? {
            '}' => break,
            '{' => {}
            _ => return None,
        }
        let at = number(&mut chars)?;
        skip(&mut chars);
        if chars.next()? != ',' {
            return None;
        }
        let value = number(&mut chars)? as f32;
        skip(&mut chars);
        let curve = match chars.next()? {
            '}' => LaneCurve::Linear,
            ',' => {
                skip(&mut chars);
                let quote = chars.next()?;
                if quote != '"' && quote != '\'' {
                    return None;
                }
                let name: String = chars.by_ref().take_while(|&ch| ch != quote).collect();
                skip(&mut chars);
                if chars.next()? != '}' {
                    return None;
                }
                curve_name(&name)?
            }
            _ => return None,
        };
        out.push((at, value, curve));
        skip(&mut chars);
        if chars.peek() == Some(&',') {
            chars.next();
        }
    }
    skip(&mut chars);
    (chars.next().is_none() && !out.is_empty()).then_some(out)
}

/// Un número como se escribe en una tabla de puntos: lo más corto que lo
/// dice (hasta 4 decimales).
pub fn number_text(value: f64) -> String {
    let text = format!("{value:.4}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".into()
    } else {
        text.to_string()
    }
}

/// La tabla de puntos escrita otra vez, con la forma de `original`: en una
/// línea si estaba en una, o un punto por línea con la sangría que tenía.
pub fn points_text(points: &[(f64, f32, LaneCurve)], original: &str) -> String {
    let point = |&(at, value, curve): &(f64, f32, LaneCurve)| {
        let curve = match curve {
            LaneCurve::Linear => String::new(),
            LaneCurve::Exp => ", \"exp\"".into(),
            LaneCurve::Smooth => ", \"smooth\"".into(),
            LaneCurve::Step => ", \"step\"".into(),
        };
        format!(
            "{{{}, {}{curve}}}",
            number_text(at),
            number_text(value as f64)
        )
    };
    let items: Vec<String> = points.iter().map(point).collect();
    let lines: Vec<&str> = original.lines().collect();
    if lines.len() <= 1 {
        return format!("{{ {} }}", items.join(", "));
    }
    // La sangría de la segunda línea (la del primer punto) y la de la
    // última (la de la llave que cierra).
    let indent = |line: &str| {
        line.chars()
            .take_while(|c| c.is_whitespace())
            .collect::<String>()
    };
    let inner = indent(lines[1]);
    let outer = indent(lines[lines.len() - 1]);
    let mut out = String::from("{\n");
    for item in &items {
        out.push_str(&format!("{inner}{item},\n"));
    }
    out.push_str(&format!("{outer}}}"));
    out
}

/// Los efectos de la pista `name` (para nombrar el parámetro de un destino).
fn track_def_fx(builder: &Builder, name: &str) -> Vec<engine::Fx> {
    builder
        .tracks
        .iter()
        .find(|t| t.def.name == name)
        .map_or_else(Vec::new, |t| t.def.fx.clone())
}

/// El destino como lo ve quien lo enseña.
fn public_target(fx: &[engine::Fx], target: AutoTarget) -> LaneTarget {
    match target {
        AutoTarget::Fx { slot, param } => LaneTarget::Fx {
            slot,
            param: fx
                .get(slot)
                .and_then(|f| f.params().get(param).copied())
                .unwrap_or(""),
        },
        AutoTarget::Mix(i) => LaneTarget::Mix(i),
        AutoTarget::Pitch => LaneTarget::Pitch,
        AutoTarget::Formant => LaneTarget::Formant,
        AutoTarget::Synth(name) => LaneTarget::Synth(name),
        AutoTarget::Shape(index) => LaneTarget::Shape(
            SHAPE_TARGETS
                .iter()
                .find(|(_, i)| *i == index)
                .map_or("", |(n, _)| n),
        ),
    }
}
