//! `mod(nombre, cadenas)`: la modulación por golpe desde Lua (ver
//! [`engine::modulation`]). Lee las tablas con el registro
//! ([`api::MOD_DOMAINS`], [`api::MOD_COMMON`], [`api::MOD_DEVICES`]) y
//! monta el `ModSet` del motor.

use engine::modulation::{
    Ahdsr, Chain, Device, DeviceKind, EnvLength, EnvLoop, Fader, FaderScale, KeyTrack,
    MAX_POINT_ENVS, MAX_STEPPER_VALUES, MAX_STEPPERS, ModLfo, ModSet, ModValue, ModWave, Op,
    PointEnv, Stepper, VelocityTrack,
};
use mlua::{Table, UserData, Value};

use crate::api;

/// Un mod set de la canción, con su nombre (su índice en la lista es el de
/// la tabla del motor).
#[derive(Debug, Clone, PartialEq)]
pub struct ModSetDef {
    pub name: String,
    pub set: ModSet,
}

/// Lo que devuelve `mod()`: el set (su índice y su nombre).
#[derive(Debug, Clone)]
pub struct ModHandle {
    pub index: u8,
    pub name: String,
}

impl UserData for ModHandle {}

/// Los nombres de los sets definidos hasta ahora en la carga (para
/// `3{mod=nombre}`), en los datos del estado de Lua.
#[derive(Debug, Clone, Default)]
pub struct ModNames(pub Vec<String>);

/// El índice del set que se pasa a `:mod()`: lo que devolvió `mod()` o su
/// nombre.
pub fn set_index(names: &[String], who: &str, value: &Value) -> Result<u8, String> {
    match value {
        Value::UserData(ud) if ud.is::<ModHandle>() => ud
            .borrow::<ModHandle>()
            .map(|h| h.index)
            .map_err(|e| e.to_string()),
        Value::String(s) => {
            let name = s.to_string_lossy().to_string();
            index_of(names, &name).ok_or_else(|| unknown(who, &name, names))
        }
        _ => Err(format!(
            "{who}: el set es lo que devuelve mod() o su nombre"
        )),
    }
}

/// El índice del set `name`.
pub fn index_of(names: &[String], name: &str) -> Option<u8> {
    names.iter().position(|n| n == name).map(|i| i as u8)
}

fn unknown(who: &str, name: &str, names: &[String]) -> String {
    if names.is_empty() {
        format!(
            "{who}: no hay ningún mod set «{name}» (defínelo antes con mod(\"{name}\", {{ ... }}))"
        )
    } else {
        format!(
            "{who}: no hay ningún mod set «{name}» (hay: {})",
            names.join(", ")
        )
    }
}

/// Un número de una tabla: `None` si no está.
fn number(t: &Table, key: &str, who: &str) -> Result<Option<f32>, String> {
    match t.get::<Value>(key).map_err(|e| e.to_string())? {
        Value::Nil => Ok(None),
        Value::Integer(i) => Ok(Some(i as f32)),
        Value::Number(n) => Ok(Some(n as f32)),
        _ => Err(format!("{who}: {key} tiene que ser un número")),
    }
}

/// Un número o una curva de la canción (`ramp`, `lfo`, `macro`, `lane`).
fn value(t: &Table, key: &str, who: &str, default: f32) -> Result<ModValue, String> {
    match t.get::<Value>(key).map_err(|e| e.to_string())? {
        Value::Nil => Ok(ModValue::fixed(default)),
        Value::Integer(i) => Ok(ModValue::fixed(i as f32)),
        Value::Number(n) => Ok(ModValue::fixed(n as f32)),
        Value::UserData(ud) => match crate::curve_of(&ud) {
            Some(curve) => Ok(ModValue {
                value: curve.bounds().0,
                curve: Some(curve),
            }),
            None => Err(format!(
                "{who}: {key} tiene que ser un número, ramp(), lfo() o macro()"
            )),
        },
        _ => Err(format!(
            "{who}: {key} tiene que ser un número, ramp(), lfo() o macro()"
        )),
    }
}

fn flag(t: &Table, key: &str, who: &str) -> Result<bool, String> {
    match t.get::<Value>(key).map_err(|e| e.to_string())? {
        Value::Nil => Ok(false),
        Value::Boolean(b) => Ok(b),
        _ => Err(format!("{who}: {key} es true o false")),
    }
}

fn text(t: &Table, key: &str, who: &str) -> Result<Option<String>, String> {
    match t.get::<Value>(key).map_err(|e| e.to_string())? {
        Value::Nil => Ok(None),
        Value::String(s) => Ok(Some(s.to_string_lossy().to_string())),
        _ => Err(format!("{who}: {key} tiene que ser un texto")),
    }
}

/// Un valor con nombre de una lista cerrada.
fn choice<T: Copy>(
    t: &Table,
    key: &str,
    who: &str,
    options: &[(T, &str)],
) -> Result<Option<T>, String> {
    let Some(name) = text(t, key, who)? else {
        return Ok(None);
    };
    options
        .iter()
        .find(|(_, n)| *n == name)
        .map(|(v, _)| Some(*v))
        .ok_or_else(|| {
            let all: Vec<&str> = options.iter().map(|(_, n)| *n).collect();
            format!("{who}: {key} «{name}» no existe ({})", all.join(", "))
        })
}

/// Lo que se va llenando al leer un set: sus envolventes de puntos y los
/// valores de sus steppers.
struct Pools {
    envs: [Option<engine::Lane>; MAX_POINT_ENVS],
    env_count: usize,
    steps: [f32; MAX_STEPPER_VALUES],
    step_count: usize,
    steppers: usize,
}

/// `mod(nombre, cadenas)`: el set.
pub fn parse_set(name: &str, chains: &Table) -> Result<ModSet, String> {
    let who = format!("mod(\"{name}\")");
    let mut set = ModSet::default();
    let mut pools = Pools {
        envs: [None; MAX_POINT_ENVS],
        env_count: 0,
        steps: [0.0; MAX_STEPPER_VALUES],
        step_count: 0,
        steppers: 0,
    };
    for pair in chains.pairs::<Value, Value>() {
        let (key, chain) = pair.map_err(|e| e.to_string())?;
        let Value::String(key) = key else {
            return Err(format!(
                "{who}: las claves son los dominios (vol, pan, pitch, cutoff) y range"
            ));
        };
        let key = key.to_string_lossy().to_string();
        if key == "range" {
            set.pitch_range = match chain {
                Value::Integer(i) => i as f32,
                Value::Number(n) => n as f32,
                _ => return Err(format!("{who}: range son semitonos")),
            };
            continue;
        }
        let Some(domain) = engine::modulation::DOMAIN_NAMES
            .iter()
            .position(|d| *d == key)
        else {
            let known: Vec<&str> = api::MOD_DOMAINS.iter().map(|p| p.name).collect();
            return Err(format!(
                "{who}: el dominio «{key}» no existe ({})",
                known.join(", ")
            ));
        };
        let Value::Table(chain) = chain else {
            return Err(format!(
                "{who}: {key} es una lista de dispositivos ({{ \"ahdsr\", ... }})"
            ));
        };
        set.chains[domain] = Some(parse_chain(&who, &key, domain, &chain, &mut pools)?);
    }
    set.envs = pools.envs;
    set.steps = pools.steps;
    set.validate().map_err(|e| format!("{who}: {e}"))?;
    Ok(set)
}

/// Una cadena: una lista de dispositivos (o uno solo) y `input`.
fn parse_chain(
    who: &str,
    key: &str,
    domain: usize,
    t: &Table,
    pools: &mut Pools,
) -> Result<Chain, String> {
    let who = format!("{who}: {key}");
    let mut chain = Chain::new(domain);
    if let Some(input) = number(t, "input", &who)? {
        // El corte también en Hz.
        chain.input = if domain == engine::modulation::CUTOFF && input > 1.0 {
            engine::modulation::cutoff_norm(input)
        } else {
            input
        };
    }
    // Un solo dispositivo sin lista: `{ "ahdsr", d = 100 }`.
    let single = matches!(t.raw_get::<Value>(1), Ok(Value::String(_)));
    if !single {
        for pair in t.pairs::<Value, Value>() {
            let (k, _) = pair.map_err(|e| e.to_string())?;
            if let Value::String(k) = k
                && k.to_string_lossy() != "input"
            {
                return Err(format!(
                    "{who}: una cadena lleva sus dispositivos en lista e `input` (no «{}»)",
                    k.to_string_lossy()
                ));
            }
        }
    }
    let devices: Vec<Table> = if single {
        vec![t.clone()]
    } else {
        t.clone()
            .sequence_values::<Value>()
            .map(|v| match v {
                Ok(Value::Table(d)) => Ok(d),
                _ => Err(format!(
                    "{who}: cada dispositivo es una tabla con su tipo primero ({{ \"ahdsr\", ... }})"
                )),
            })
            .collect::<Result<_, _>>()?
    };
    for d in devices {
        let device = parse_device(&who, &d, pools)?;
        chain = chain
            .with_device(device)
            .map_err(|e| format!("{who}: {e}"))?;
    }
    Ok(chain)
}

/// Un dispositivo: su tipo (el primer elemento) y sus parámetros.
fn parse_device(who: &str, t: &Table, pools: &mut Pools) -> Result<Device, String> {
    let kind: String = match t.raw_get::<Value>(1).map_err(|e| e.to_string())? {
        Value::String(s) => s.to_string_lossy().to_string(),
        _ => {
            return Err(format!(
                "{who}: el dispositivo lleva su tipo primero ({})",
                device_names()
            ));
        }
    };
    let Some(def) = api::mod_device(&kind) else {
        return Err(format!(
            "{who}: el dispositivo «{kind}» no existe ({})",
            device_names()
        ));
    };
    let who = format!("{who} {{\"{kind}\"}}");
    // Claves que no son suyas ni comunes, y curvas donde no van.
    for pair in t.pairs::<Value, Value>() {
        let (k, v) = pair.map_err(|e| e.to_string())?;
        match k {
            Value::Integer(1) => {}
            Value::String(k) => {
                let k = k.to_string_lossy();
                let param = def
                    .param(&k)
                    .or_else(|| api::MOD_COMMON.iter().find(|p| p.name == k));
                if let (Some(p), Value::UserData(ud)) = (param, &v)
                    && crate::curve_of(ud).is_some()
                    && let api::Auto::No(why) = p.automate
                {
                    return Err(format!("{who}: {k} no admite ramp() ni lfo(): {why}"));
                }
                if param.is_none() {
                    let mut known: Vec<&str> = def.params.iter().map(|p| p.name).collect();
                    known.extend(api::MOD_COMMON.iter().map(|p| p.name));
                    return Err(format!(
                        "{who}: parámetro desconocido «{k}» (admite: {})",
                        known.join(", ")
                    ));
                }
            }
            _ => {
                return Err(format!(
                    "{who}: después del tipo, solo parámetros con nombre"
                ));
            }
        }
    }
    let kind = match def.name {
        "ahdsr" => {
            let d = Ahdsr::DEFAULT;
            let curve = |k: &str, default: f32| -> Result<f32, String> {
                Ok(number(t, k, &who)?.unwrap_or(default))
            };
            DeviceKind::Ahdsr(Ahdsr {
                attack: value(t, "a", &who, d.attack.value)?,
                hold: value(t, "h", &who, d.hold.value)?,
                decay: value(t, "d", &who, d.decay.value)?,
                sustain: value(t, "s", &who, d.sustain.value)?,
                release: value(t, "r", &who, d.release.value)?,
                curves: [
                    curve("curve_a", d.curves[0])?,
                    curve("curve_d", d.curves[1])?,
                    curve("curve_r", d.curves[2])?,
                ],
            })
        }
        "env" => DeviceKind::Points(parse_points(&who, t, pools)?),
        "lfo" => {
            let sync = match t.get::<Value>("sync").map_err(|e| e.to_string())? {
                Value::Nil => None,
                Value::Integer(i) => {
                    Some(crate::lfo_sync_value(&who, crate::FxArg::Number(i as f32))?)
                }
                Value::Number(n) => {
                    Some(crate::lfo_sync_value(&who, crate::FxArg::Number(n as f32))?)
                }
                Value::String(s) => Some(crate::lfo_sync_value(
                    &who,
                    crate::FxArg::Text(s.to_string_lossy().to_string()),
                )?),
                _ => {
                    return Err(format!(
                        "{who}: sync es una figura o un número de ciclos por compás"
                    ));
                }
            };
            DeviceKind::Lfo(ModLfo {
                wave: choice(t, "wave", &who, &ModWave::ALL)?.unwrap_or(ModWave::Sine),
                rate: value(t, "rate", &who, ModLfo::DEFAULT.rate.value)?,
                sync,
                phase: number(t, "phase", &who)?.unwrap_or(0.0),
                delay: number(t, "delay", &who)?.unwrap_or(0.0),
                free: flag(t, "free", &who)?,
            })
        }
        "stepper" => DeviceKind::Stepper(parse_stepper(&who, t, pools)?),
        "fader" => DeviceKind::Fader(Fader {
            from: number(t, "from", &who)?.unwrap_or(1.0),
            to: number(t, "to", &who)?.unwrap_or(0.0),
            time: number(t, "time", &who)?.unwrap_or(500.0),
            delay: number(t, "delay", &who)?.unwrap_or(0.0),
            scale: choice(t, "scale", &who, &FaderScale::ALL)?.unwrap_or(FaderScale::Linear),
        }),
        "value" => DeviceKind::Operand(value(t, "value", &who, 1.0)?),
        "velocity" => DeviceKind::Velocity(VelocityTrack {
            min: number(t, "min", &who)?.unwrap_or(0.0),
            max: number(t, "max", &who)?.unwrap_or(1.0),
            clamp: flag(t, "clamp", &who)?,
        }),
        "keytrack" => DeviceKind::Key(KeyTrack {
            min: number(t, "min", &who)?.unwrap_or(0.0),
            max: number(t, "max", &who)?.unwrap_or(1.0),
            low: number(t, "low", &who)?.unwrap_or(0.0),
            high: number(t, "high", &who)?.unwrap_or(127.0),
        }),
        other => unreachable!("dispositivo del registro sin lectura: {other}"),
    };
    // El LFO es bipolar si no se dice otra cosa.
    let bipolar = match t.get::<Value>("bipolar").map_err(|e| e.to_string())? {
        Value::Nil => matches!(kind, DeviceKind::Lfo(_)),
        _ => flag(t, "bipolar", &who)?,
    };
    let device = Device {
        kind,
        op: choice(t, "op", &who, &Op::ALL)?,
        bipolar,
        bypass: flag(t, "bypass", &who)?,
        amount: value(t, "amount", &who, 1.0)?,
    };
    device.validate().map_err(|e| format!("{who}: {e}"))?;
    Ok(device)
}

fn device_names() -> String {
    api::MOD_DEVICES
        .iter()
        .map(|d| d.name)
        .collect::<Vec<_>>()
        .join(", ")
}

/// `{ "env", points = {...}, len, loop, sustain }`.
fn parse_points(who: &str, t: &Table, pools: &mut Pools) -> Result<PointEnv, String> {
    let Value::Table(points) = t.get::<Value>("points").map_err(|e| e.to_string())? else {
        return Err(format!(
            "{who}: points es una lista de {{posición, valor [, curva]}}"
        ));
    };
    let mut list = Vec::new();
    for p in points.sequence_values::<Value>() {
        let Value::Table(p) = p.map_err(|e| e.to_string())? else {
            return Err(format!(
                "{who}: cada punto es {{posición, valor [, curva]}}"
            ));
        };
        let at: f64 = p
            .raw_get::<Option<f64>>(1)
            .ok()
            .flatten()
            .ok_or_else(|| format!("{who}: cada punto es {{posición, valor [, curva]}}"))?;
        let v: f32 = p
            .raw_get::<Option<f32>>(2)
            .ok()
            .flatten()
            .ok_or_else(|| format!("{who}: cada punto es {{posición, valor [, curva]}}"))?;
        let curve = match p.raw_get::<Option<String>>(3).ok().flatten().as_deref() {
            None | Some("linear") => engine::LaneCurve::Linear,
            Some("exp") => engine::LaneCurve::Exp,
            Some("smooth") => engine::LaneCurve::Smooth,
            Some("step") => engine::LaneCurve::Step,
            Some(other) => {
                return Err(format!(
                    "{who}: la curva «{other}» no existe (linear, exp, smooth, step)"
                ));
            }
        };
        list.push(engine::LanePoint {
            at,
            value: v,
            curve,
        });
    }
    let lane = engine::Lane::new(&list, 1.0, false).map_err(|e| {
        format!(
            "{who}: {}",
            e.replace("dentro de su duración (len)", "de 0 a 1")
        )
    })?;
    if pools.env_count == MAX_POINT_ENVS {
        return Err(format!(
            "{who}: demasiadas envolventes de puntos (máximo {MAX_POINT_ENVS} por set)"
        ));
    }
    let index = pools.env_count;
    pools.envs[index] = Some(lane);
    pools.env_count += 1;
    let length = match t.get::<Value>("len").map_err(|e| e.to_string())? {
        Value::Nil => EnvLength::Ms(500.0),
        Value::Integer(i) => EnvLength::Ms(i as f32),
        Value::Number(n) => EnvLength::Ms(n as f32),
        Value::String(s) => {
            let text = s.to_string_lossy().to_string();
            let sync = crate::lfo_sync_value(who, crate::FxArg::Text(text))
                .map_err(|_| format!("{who}: len son ms o una figura (\"1/4\", \"2 compases\")"))?;
            EnvLength::Steps(sync.steps())
        }
        _ => return Err(format!("{who}: len son ms o una figura")),
    };
    let sustain = match number(t, "sustain", who)? {
        None => None,
        Some(n) if n.fract() == 0.0 && n >= 1.0 && (n as usize) <= list.len() => {
            Some((n as usize - 1) as u8)
        }
        Some(n) => {
            return Err(format!(
                "{who}: sustain es el número de un punto (de 1 a {}), no {n}",
                list.len()
            ));
        }
    };
    Ok(PointEnv {
        lane: index as u8,
        length,
        looping: choice(t, "loop", who, &EnvLoop::ALL)?.unwrap_or(EnvLoop::Off),
        sustain,
    })
}

/// `{ "stepper", values = {...}, step, random, seed, reset }`.
fn parse_stepper(who: &str, t: &Table, pools: &mut Pools) -> Result<Stepper, String> {
    let Value::Table(values) = t.get::<Value>("values").map_err(|e| e.to_string())? else {
        return Err(format!("{who}: values es una lista de números"));
    };
    let values: Vec<f32> = values
        .sequence_values::<f32>()
        .collect::<Result<_, _>>()
        .map_err(|_| format!("{who}: values es una lista de números"))?;
    if values.is_empty() {
        return Err(format!("{who}: values lleva al menos un número"));
    }
    if pools.step_count + values.len() > MAX_STEPPER_VALUES {
        return Err(format!(
            "{who}: los steppers de un set llevan como mucho {MAX_STEPPER_VALUES} valores entre todos"
        ));
    }
    if pools.steppers == MAX_STEPPERS {
        return Err(format!(
            "{who}: demasiados steppers (máximo {MAX_STEPPERS} por set)"
        ));
    }
    let first = pools.step_count;
    pools.steps[first..first + values.len()].copy_from_slice(&values);
    pools.step_count += values.len();
    let id = pools.steppers;
    pools.steppers += 1;
    let whole = |key: &str, default: f32, max: f32| -> Result<f32, String> {
        let v = number(t, key, who)?.unwrap_or(default);
        if v.fract() != 0.0 || !(0.0..=max).contains(&v) {
            return Err(format!("{who}: {key} es un entero de 0 a {max}"));
        }
        Ok(v)
    };
    Ok(Stepper {
        id: id as u8,
        first: first as u16,
        len: values.len() as u16,
        advance: whole("step", 1.0, engine::modulation::MAX_STEPPER_ADVANCE as f32)? as u8,
        random: flag(t, "random", who)?,
        seed: whole("seed", 1.0, u32::MAX as f32)? as u64,
        reset: whole("reset", 0.0, 64.0)? as u16,
    })
}
