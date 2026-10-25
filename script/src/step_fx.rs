//! Efectos de un paso (`3{lp=800,q=4}{crush=6}` en `steps`): de lo escrito
//! ([`engine::notation::WrittenFx`]) a la cadena del motor, con los
//! efectos, alias, parámetros y valores del registro de la API (los mismos
//! que `:fx`). Un valor puede ser una rampa del golpe (`200>4000` dura el
//! paso, `200>4000:4` cuatro pasos) si el parámetro admite curvas.

use engine::notation::{FxValue, StepBraces, WrittenFx};
use engine::synth::SubNote;
use engine::{Automation, Curve, SliceFx};

use crate::api;
use crate::{FxArg, build_fx};

/// Lo de las llaves de un paso, o el error (sin línea: la pone quien lee
/// el patrón): si todas las claves son del bloque sub (`sub`, `sub_div`,
/// `sub_wave`, `sub_pan`, `sub_drive`), lo que cambia del sub de su nota
/// (de un sintetizador; [`sub_note`]); si no, una cadena de efectos
/// ([`effects`]). Qué vale en cada pista lo dice la carga.
pub fn resolve(written: &[WrittenFx]) -> Result<StepBraces, String> {
    let keys = || {
        written.iter().flat_map(|w| {
            std::iter::once(w.name.as_str()).chain(w.params.iter().map(|(k, _)| k.as_str()))
        })
    };
    let is_sub = |k: &str| api::SUB_BLOCK_PARAMS.iter().any(|p| p.name == k);
    if keys().all(is_sub) {
        return sub_note(written).map(StepBraces::Sub);
    }
    if let Some(k) = keys().find(|k| is_sub(k)) {
        return Err(format!(
            "{{{k}}}: el sub de la nota va en sus propias llaves, sin efectos ({{sub=0.5,sub_div=3}})"
        ));
    }
    effects(written).map(StepBraces::Fx)
}

/// Lo que cambia del sub de una nota (`{sub=0.5,sub_div=3}`), con los
/// rangos del motor.
pub fn sub_note(written: &[WrittenFx]) -> Result<SubNote, String> {
    let mut sub = SubNote::default();
    let mut seen: Vec<String> = Vec::new();
    for w in written {
        let values = w
            .main
            .iter()
            .map(|v| (w.name.as_str(), v))
            .chain(w.params.iter().map(|(k, v)| (k.as_str(), v)));
        for (key, value) in values {
            let who = format!("{{{key}}}");
            if seen.iter().any(|k| k == key) {
                return Err(format!("{who}: {key} repetido"));
            }
            seen.push(key.to_string());
            let number = || match value {
                FxValue::Number(n) => Ok(*n),
                FxValue::Ramp { .. } => Err(format!(
                    "{who}: {key} no admite rampas: cada nota lo lee al nacer"
                )),
                FxValue::Word(w) => Err(format!("{who}: {key} es un número, no \"{w}\"")),
            };
            match key {
                "sub" => sub.level = Some(number()?),
                "sub_pan" => sub.pan = Some(number()?),
                "sub_drive" => sub.drive = Some(number()?),
                "sub_div" => {
                    let n = number()?;
                    if n.fract() != 0.0 || !(1.0..=engine::synth::MAX_SUB_DIV as f32).contains(&n) {
                        return Err(format!("{who}: sub_div es un entero de 1 a 16"));
                    }
                    sub.div = Some(n as u8);
                }
                "sub_wave" => {
                    let FxValue::Word(name) = value else {
                        return Err(format!("{who}: sub_wave es square, tri, sine o pulse25"));
                    };
                    sub.wave = Some(sub_wave(name).ok_or_else(|| {
                        format!("{who}: sub_wave es square, tri, sine o pulse25")
                    })?);
                }
                _ => unreachable!("solo claves del sub"),
            }
            if key == "sub" && !w.params.is_empty() && w.name != "sub" {
                unreachable!();
            }
        }
    }
    sub.validate().map_err(|e| format!("{{sub}}: {e}"))?;
    Ok(sub)
}

/// La forma de onda del sub por su nombre (`tri` o `triangle`).
pub fn sub_wave(name: &str) -> Option<engine::synth::Wave> {
    use engine::synth::Wave;
    match name {
        "square" => Some(Wave::Square),
        "tri" | "triangle" => Some(Wave::Triangle),
        "sine" => Some(Wave::Sine),
        "pulse25" => Some(Wave::Pulse25),
        _ => None,
    }
}

/// La cadena de efectos de un paso, o el error.
pub fn effects(written: &[WrittenFx]) -> Result<SliceFx, String> {
    let mut chain = Vec::new();
    let mut automation = Vec::new();
    for w in written {
        let effect = api::effect(&w.name).ok_or_else(|| {
            let known: Vec<String> = api::EFFECTS
                .iter()
                .map(|e| match e.aliases {
                    [] => e.name.to_string(),
                    aliases => format!("{} ({})", e.name, aliases.join(", ")),
                })
                .collect();
            format!(
                "{{{}}}: el efecto no existe (hay: {})",
                w.name,
                known.join(", ")
            )
        })?;
        let who = format!("{{{}}}", w.name);
        let mut values: Vec<(&str, &FxValue)> = Vec::new();
        if let Some(main) = &w.main {
            let param = effect
                .main_param()
                .ok_or_else(|| format!("{who}: este efecto no tiene un valor principal"))?;
            values.push((param.name, main));
        }
        for (key, value) in &w.params {
            values.push((key.as_str(), value));
        }
        for (i, (key, value)) in values.iter().enumerate() {
            let Some(param) = effect.param(key) else {
                let known: Vec<&str> = effect.params.iter().map(|p| p.name).collect();
                return Err(format!(
                    "{who}: parámetro desconocido \"{key}\" (admite: {})",
                    known.join(", ")
                ));
            };
            if values[..i].iter().any(|(k, _)| k == key) {
                return Err(format!("{who}: {key} repetido"));
            }
            if param.name == "sidechain" {
                return Err(format!(
                    "{who}: el EQ dinámico de un paso no puede escuchar otra pista (sidechain)"
                ));
            }
            if let (FxValue::Ramp { .. }, api::Auto::No(why)) = (value, param.automate) {
                return Err(format!("{who}: {key} no admite rampas: {why}"));
            }
        }
        let arg = |key: &str| -> Result<Option<FxArg>, String> {
            Ok(values
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| match v {
                    FxValue::Number(n) => FxArg::Number(*n),
                    FxValue::Word(w) => FxArg::Text(w.clone()),
                    FxValue::Ramp { from, to, steps } => FxArg::Curve(Curve::Hit {
                        from: *from,
                        to: *to,
                        steps: steps.unwrap_or(1.0),
                    }),
                }))
        };
        let (fx, curves) = build_fx(effect.name, &arg).map_err(|e| step_message(&who, e))?;
        if chain.len() == engine::MAX_FX {
            return Err(format!(
                "un paso tiene como mucho {} efectos",
                engine::MAX_FX
            ));
        }
        for (key, curve) in curves {
            let param = fx
                .param_index(&key)
                .ok_or_else(|| format!("{who}: {key} no admite rampas"))?;
            automation.push(Automation {
                fx: chain.len(),
                param,
                curve,
            });
        }
        chain.push(fx);
    }
    SliceFx::new(&chain, &automation).map_err(|e| format!("efectos del paso: {e}"))
}

/// Los mensajes de `build_fx` hablan de `fx("lowpass")`: en un paso, del
/// efecto tal como se escribió.
fn step_message(who: &str, message: String) -> String {
    match message.split_once("): ") {
        Some((head, rest)) if head.starts_with("fx(") => format!("{who}: {rest}"),
        _ => message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use engine::notation::parse_step_with;

    fn step(token: &str) -> Result<engine::Step, String> {
        parse_step_with(token, &[], Some(&resolve)).map(|s| s.expect("un golpe"))
    }

    #[test]
    fn names_aliases_main_values_and_keys() {
        let s = step("3{lp=800,q=4}{crush=6}").unwrap();
        let fx = s.fx.unwrap();
        let chain: Vec<engine::Fx> = fx.effects().copied().collect();
        assert_eq!(
            chain,
            [
                engine::Fx::Filter {
                    kind: engine::FilterKind::LowPass,
                    cutoff_hz: 800.0,
                    q: 4.0
                },
                engine::Fx::bitcrush(6, 1),
            ]
        );
        assert_eq!(s.slice, 2);
        // Con sufijos antes y después, y espacios dentro de las llaves.
        let s = step("2'{ hp = 300 }^7").unwrap();
        assert!(s.reverse && s.semitones == 7.0);
        assert_eq!(
            s.fx.unwrap().effects().next(),
            Some(&engine::Fx::highpass(300.0))
        );
        let s = step("1{filter,type=highpass,cutoff=500}").unwrap();
        assert_eq!(
            s.fx.unwrap().effects().next(),
            Some(&engine::Fx::highpass(500.0))
        );
    }

    #[test]
    fn ramps_of_the_hit() {
        let s = step("1{lp=200>4000:4}").unwrap();
        let fx = s.fx.unwrap();
        let a: Vec<&Automation> = fx.automations().collect();
        assert_eq!(
            a[0].curve,
            Curve::Hit {
                from: 200.0,
                to: 4000.0,
                steps: 4.0
            }
        );
        // El valor fijo es su principio.
        assert_eq!(fx.effects().next(), Some(&engine::Fx::lowpass(200.0)));
        let s = step("1{drive=2>20}").unwrap();
        let fx = s.fx.unwrap();
        assert_eq!(
            fx.automations().next().unwrap().curve,
            Curve::Hit {
                from: 2.0,
                to: 20.0,
                steps: 1.0
            }
        );
    }

    #[test]
    fn errors_say_what_is_wrong() {
        let error = |t: &str| step(t).unwrap_err();
        assert!(
            error("1{zap=3}").contains("{zap}: el efecto no existe (hay: filter, lowpass (lp)")
        );
        assert!(error("1{lp=800,zz=1}").contains("{lp}: parámetro desconocido \"zz\""));
        assert!(error("1{lp=5}").contains("{lp}: frecuencia de corte fuera de rango"));
        assert!(error("1{filter,type=200>300,cutoff=4}").contains("type no admite rampas"));
        assert!(error("1{dyneq=100,sidechain=kick}").contains("no puede escuchar otra pista"));
        assert!(error("1{lp=800").contains("falta }"));
        assert!(error("1{}").contains("efecto vacío"));
        assert!(error("1{lp=}").contains("falta el valor de lp"));
        assert!(error("1{lp=1>}").contains("una rampa es desde>hasta"));
        assert!(error("1{lp=1>2:0}").contains("tras : van los pasos"));
        assert!(error("1{lp=200,cutoff=300}").contains("cutoff repetido"));
        assert!(error("1{lp=800}{hp=90}{crush}{comp}{gain}").contains("como mucho 4 efectos"));
        // Sin quien los resuelva (fuera del script), un error claro.
        assert!(
            engine::notation::parse_step("1{lp=800}")
                .unwrap_err()
                .contains("solo valen en steps(\"...\") de una pista de cortes")
        );
    }
}
