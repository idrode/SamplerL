//! El registro de la API (`script::api`) coincide con el entorno de Lua:
//! cada función registrada existe, no hay funciones globales sin registrar,
//! y las plantillas del autocompletado y los valores válidos funcionan.

#[path = "../../engine/tests/support/synth_break.rs"]
mod synth;

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use engine::offline;
use script::ScriptHost;
use script::api::{
    self, CHANCE_EFFECTS, EFFECTS, FUNCTIONS, Receiver, SNIPPETS, expand_template, functions,
};

const RATE: u32 = 48_000;

/// Carpeta con el break sintético como `amen.wav`. Se escribe una sola vez:
/// los tests van en paralelo y no deben leerlo a medio escribir.
fn samples() -> PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("registry_samples");
        std::fs::create_dir_all(&dir).unwrap();
        let (audio, _) = synth::synth_break();
        offline::write_wav(dir.join("amen.wav"), &audio, 1, synth::RATE).unwrap();
        dir
    })
    .clone()
}

fn eval(source: &str) {
    let dir = samples();
    let mut host = ScriptHost::new(RATE);
    host.set_seed(Some(1));
    if let Err(e) = host.eval(source, "registro.lua", &dir) {
        panic!("{e}\n{source}");
    }
}

/// Variable de Lua que tiene un objeto de cada receptor (el compás solo
/// existe dentro de `every`).
fn variable(receiver: Receiver) -> &'static str {
    match receiver {
        Receiver::Global => "_G",
        Receiver::Sample => "s",
        Receiver::Slices => "amen",
        Receiver::Track => "t",
        Receiver::Bar => "c",
        Receiver::Scale => "escala",
        Receiver::Instrument => "inst",
    }
}

const PRELUDE: &str = r#"
local s = sample("amen.wav")
local amen = s:slice(8)
local t = track("t", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
-- La pista que escuchan las plantillas con sidechain.
track("kick", amen:slice_at(1))
local escala = scale("minor", "C")
local inst = s:instrument({ root = "C3" })
"#;

#[test]
fn every_registered_function_exists_in_lua() {
    let mut source = String::from(PRELUDE);
    let check = |f: &api::Function| {
        format!(
            "assert(type({}.{}) == \"function\", \"{} no existe\")\n",
            variable(f.receiver),
            f.name,
            f.signature
        )
    };
    let mut bar = String::new();
    for f in FUNCTIONS {
        match f.receiver {
            Receiver::Bar => bar.push_str(&check(f)),
            _ => source.push_str(&check(f)),
        }
    }
    source.push_str(&format!("t:every(1, function(c)\n{bar}end)\n"));
    eval(&source);
}

#[test]
fn no_global_function_is_missing_from_the_registry() {
    // Las funciones de la librería base de Lua; el resto son de la API.
    let base = [
        "assert",
        "collectgarbage",
        "error",
        "getmetatable",
        "ipairs",
        "load",
        "next",
        "pairs",
        "pcall",
        "print",
        "rawequal",
        "rawget",
        "rawlen",
        "rawset",
        "select",
        "setmetatable",
        "tonumber",
        "tostring",
        "type",
        "xpcall",
        "warn",
        "dofile",
        "loadfile",
        "require",
    ];
    let registered: Vec<String> = functions(Receiver::Global)
        .map(|f| format!("\"{}\"", f.name))
        .collect();
    let base: Vec<String> = base.iter().map(|n| format!("\"{n}\"")).collect();
    let source = format!(
        r#"
local known = {{}}
for _, n in ipairs({{ {} }}) do known[n] = true end
for _, n in ipairs({{ {} }}) do known[n] = true end
for name, value in pairs(_G) do
  if type(value) == "function" and not known[name] then
    error("función global sin registrar: " .. name)
  end
end
"#,
        base.join(", "),
        registered.join(", ")
    );
    eval(&source);
}

#[test]
fn every_template_is_valid_code() {
    let mut source = String::from(PRELUDE);
    let mut bar = String::new();
    for f in FUNCTIONS {
        let (text, marks) = expand_template(f.template);
        assert_eq!(
            marks.len(),
            f.template.matches("${").count(),
            "{}",
            f.signature
        );
        match f.receiver {
            Receiver::Global if f.name == "track" => source.push_str(&format!(
                "{text}:steps(\"1 . . .  . . . .  . . . .  . . . .\")\n"
            )),
            Receiver::Global => source.push_str(&format!("local _ = {text}\n")),
            Receiver::Bar => bar.push_str(&format!("c:{text}\n")),
            r => source.push_str(&format!("local _ = {}:{text}\n", variable(r))),
        }
    }
    source.push_str(&format!("t:every(1, function(c)\n{bar}end)\n"));
    eval(&source);
}

#[test]
fn every_effect_accepts_its_registered_parameters_and_values() {
    // Un script por efecto (cada nombre, en su pista: no más de 8 pistas).
    for e in EFFECTS {
        let mut source = String::from(PRELUDE);
        let (table, _) = expand_template(e.template);
        let all: Vec<String> = e
            .params
            .iter()
            .map(|p| expand_template(p.template).0)
            .collect();
        let all = format!("{{ {} }}", all.join(", "));
        for name in std::iter::once(&e.name).chain(e.aliases) {
            // Cada efecto en su pista, para no pasar de 4 por pista.
            source.push_str(&format!(
                "track(\"{name}\", amen):fx(\"{name}\", {table})\n\
                 track(\"{name}\"):fx(\"{name}\", {all})\n"
            ));
            // Cada valor de las listas cerradas, en su pista: la vuelta k da
            // a cada parámetro de lista su valor k (y los obligatorios, el
            // de su plantilla).
            let listed: Vec<_> = e.params.iter().filter(|p| !p.values.is_empty()).collect();
            let required: Vec<String> = e
                .params
                .iter()
                .filter(|p| p.default.is_none())
                .map(|p| expand_template(p.template).0)
                .collect();
            let rounds = listed.iter().map(|p| p.values.len()).max().unwrap_or(0);
            for k in 0..rounds {
                let mut table: Vec<String> = listed
                    .iter()
                    .map(|p| format!("{} = \"{}\"", p.name, p.values[k % p.values.len()]))
                    .collect();
                table.extend(required.iter().cloned());
                source.push_str(&format!(
                    "track(\"{name}{k}\", amen):fx(\"{name}\", {{ {} }})\n",
                    table.join(", ")
                ));
            }
        }
        assert!(api::effect(e.name).is_some());
        eval(&source);
    }
}

#[test]
fn every_bus_accepts_its_registered_parameters_and_values() {
    let mut source = String::from(PRELUDE);
    for b in api::BUSES {
        let (table, _) = expand_template(b.template);
        let all: Vec<String> = b
            .params
            .iter()
            .map(|p| expand_template(p.template).0)
            .collect();
        source.push_str(&format!(
            "bus(\"{0}\", {table})\nbus(\"{0}\", {{ {1} }})\nt:send(\"{0}\", 0.5)\n",
            b.name,
            all.join(", ")
        ));
        for p in b.params {
            for value in p.values {
                source.push_str(&format!(
                    "bus(\"{}\", {{ {} = \"{value}\" }})\n",
                    b.name, p.name
                ));
            }
        }
        assert!(api::bus(b.name).is_some());
    }
    eval(&source);
}

#[test]
fn every_chance_effect_exists() {
    let mut source = String::from(PRELUDE);
    source.push_str("local acid = track(\"acid\", synth(\"acid\")):melody(\"c c c c  c c c c  c c c c  c c c c\")\n");
    for e in CHANCE_EFFECTS {
        // Los del sintetizador, en uno.
        let on = if matches!(e.name, "accent" | "slide") {
            "acid"
        } else {
            "t"
        };
        for name in std::iter::once(&e.name).chain(e.aliases) {
            let value = if e.value.is_some() { ", 2" } else { "" };
            source.push_str(&format!("{on}:chance(0.5, \"{name}\"{value})\n"));
        }
    }
    eval(&source);
}

#[test]
fn unknown_effect_errors_list_the_registry() {
    let dir = samples();
    let mut host = ScriptHost::new(RATE);
    let Err(err) = host.eval(
        &format!("{PRELUDE}t:fx(\"chorus\", {{}})\n"),
        "registro.lua",
        &dir,
    ) else {
        panic!("debería fallar");
    };
    assert!(
        err.message.contains(
            "no existe (filter, lowpass, highpass, bitcrush, compressor, distortion, gain, eq, dyneq)"
        ),
        "{err}"
    );
}

#[test]
fn every_snippet_runs_after_what_it_requires() {
    for snippet in SNIPPETS {
        let (text, marks) = expand_template(snippet.template);
        assert!(!marks.is_empty(), "{}: sin campos", snippet.name);
        let source = format!("{}{text}", snippet.requires);
        eval(&source);
        assert!(api::snippet(snippet.name).is_some());
    }
    let names: Vec<&str> = SNIPPETS.iter().map(|s| s.name).collect();
    assert_eq!(
        names,
        [
            "amen",
            "fill",
            "ghost",
            "intro",
            "euclid",
            "melodia",
            "polirritmo",
            "barrido",
            "groove",
            "instrumento"
        ]
    );
}

#[test]
fn every_fill_effect_exists() {
    let mut source = String::from(PRELUDE);
    for e in api::FILL_EFFECTS {
        let value = if e.value.is_some() { ", 2" } else { "" };
        source.push_str(&format!("t:fill(\"{}\", 2, 4{value})\n", e.name));
    }
    eval(&source);
}

#[test]
fn every_instrument_parameter_is_accepted() {
    let mut source = String::from(PRELUDE);
    let (table, _) = expand_template(api::INSTRUMENT_TABLE.template);
    let all: Vec<String> = api::INSTRUMENT_PARAMS
        .iter()
        .map(|p| expand_template(p.template).0)
        .collect();
    source.push_str(&format!(
        "track(\"a\", s:instrument({table})):melody(\"c . e .  g . c+e+g .  . . . .  . . . .\")\n\
         track(\"b\", s:instrument({{ {} }})):steps(\"1^0^4 _ . .  . . . .  . . . .  . . . .\")\n",
        all.join(", ")
    ));
    eval(&source);
}

#[test]
fn every_synth_parameter_is_accepted() {
    for synth in api::SYNTHS {
        let (table, _) = expand_template(synth.template);
        let all: Vec<String> = synth
            .params
            .iter()
            .map(|p| expand_template(p.template).0)
            .collect();
        let mut source = String::from(PRELUDE);
        source.push_str(&format!(
            "track(\"a\", synth(\"{name}\", {table})):melody(\"c c! eb/ .  g . c . c . c . c . c .\")\n\
             track(\"b\", synth(\"{name}\", {{ {} }})):steps(\"1! 1/ . .  . . . .  . . . .  . . . .\")\n",
            all.join(", "),
            name = synth.name,
        ));
        eval(&source);
    }
}

/// Los parámetros numéricos de cada sintetizador del registro tienen el
/// rango y el valor por defecto de su preset del motor; admiten curvas los
/// que el motor mueve, y los demás no.
#[test]
fn every_synth_registry_matches_the_engine() {
    for synth in api::SYNTHS {
        let preset = engine::synth::preset::preset(synth.name)
            .unwrap_or_else(|| panic!("{} no está en el motor", synth.name));
        for p in synth.params {
            let what = format!("{}.{}", synth.name, p.name);
            match preset.param(p.name) {
                Some(e) => {
                    assert_eq!(p.valid, format!("{}–{}", e.min, e.max), "{what}");
                    assert_eq!(p.default, Some(e.default.to_string().as_str()), "{what}");
                    assert_eq!(p.automate.curves(), e.moves(), "{what}");
                }
                None => assert!(!p.automate.curves(), "{what}"),
            }
        }
        let numeric = synth
            .params
            .iter()
            .filter(|p| preset.param(p.name).is_some())
            .count();
        assert_eq!(numeric, preset.params.len(), "{}", synth.name);
    }
    assert_eq!(
        api::SYNTHS.len(),
        engine::synth::preset::PRESETS.len(),
        "cada preset del motor, en el registro"
    );
}

#[test]
fn every_grain_parameter_is_accepted() {
    let mut source = String::from(PRELUDE);
    let (table, _) = expand_template(api::GRAINS_TABLE.template);
    let all: Vec<String> = api::GRAIN_PARAMS
        .iter()
        .filter(|p| p.name != "speed")
        .map(|p| expand_template(p.template).0)
        .collect();
    source.push_str(&format!(
        "track(\"a\", s:slice(2)):steps(\"1 _ . .  . . . .  . . . .  . . . .\"):grains({table})\n\
         track(\"b\", s:slice(2)):steps(\"1 _ . .  . . . .  . . . .  . . . .\"):grains({{ {}, speed = 0.5 }})\n\
         track(\"c\", s:slice(2)):steps(\"1 _ . .  . . . .  . . . .  . . . .\"):freeze(0.5, {{ {} }})\n",
        all.join(", "),
        all.join(", ")
    ));
    eval(&source);
}

/// El efecto del motor que corresponde a un nombre del registro.
fn engine_fx(name: &str) -> engine::Fx {
    match api::effect(name).map(|e| e.name) {
        Some("filter" | "lowpass" | "highpass") => engine::Fx::lowpass(1000.0),
        Some("compressor") => engine::Fx::compressor(-18.0, 4.0),
        Some("distortion") => engine::Fx::distortion(4.0, 1.0),
        Some("gain") => engine::Fx::gain(1.0),
        Some("bitcrush") => engine::Fx::bitcrush(8, 1),
        Some("eq") => engine::Fx::eq(&[]),
        Some("dyneq") => engine::Fx::DynEq(engine::DynEqParams::new(100.0)),
        other => panic!("efecto sin motor: {other:?}"),
    }
}

/// Los extremos de `valid` (`"20–20000"`); para lo que no es un rango,
/// 0 y 1.
fn bounds(valid: &str) -> (String, String) {
    match valid.split_once('–') {
        Some((a, b)) if a.parse::<f32>().is_ok() && b.parse::<f32>().is_ok() => {
            (a.to_string(), b.to_string())
        }
        _ => ("0".into(), "1".into()),
    }
}

#[test]
fn the_engine_moves_exactly_what_the_registry_says() {
    for e in EFFECTS {
        let fx = engine_fx(e.name);
        for p in e.params {
            assert_eq!(
                fx.param_index(p.name).is_some(),
                p.automate.curves(),
                "fx(\"{}\").{}: el registro y el motor no coinciden",
                e.name,
                p.name
            );
        }
    }
}

#[test]
fn every_sound_parameter_takes_a_curve_or_says_why_not() {
    let dir = samples();
    let try_eval = |source: &str| {
        let mut host = ScriptHost::new(RATE);
        host.set_seed(Some(1));
        host.eval(source, "registro.lua", &dir)
            .map(|_| ())
            .map_err(|e| e.message)
    };
    let mut checked = 0;
    for (what, p) in api::sound_params() {
        let (lo, hi) = bounds(p.valid);
        let curve = format!("ramp({lo}, {hi}, \"1 compás\")");
        let call = match what.as_str() {
            "pista" => match p.name {
                "send" => format!("t:send(\"delay\", {curve})"),
                name => format!("t:{name}({curve})"),
            },
            "cortes:set" => format!("amen:set(1, {{ {} = {curve} }})", p.name),
            ":grains" => format!("t:grains({{ {} = {curve} }})", p.name),
            "instrument" => format!("local i = s:instrument({{ {} = {curve} }})", p.name),
            "sample" => format!("local x = sample(\"amen.wav\", {{ {} = {curve} }})", p.name),
            synth if synth.starts_with("synth(") => {
                format!(
                    "local x = {}, {{ {} = {curve} }})",
                    &synth[..synth.len() - 1],
                    p.name
                )
            }
            fx if fx.starts_with("fx(") => {
                let name = &fx[4..fx.len() - 2];
                let effect = api::effect(name).unwrap();
                // Los obligatorios (menos este), con su plantilla.
                let mut items: Vec<String> = effect
                    .params
                    .iter()
                    .filter(|q| q.default.is_none() && q.name != p.name)
                    .map(|q| expand_template(q.template).0)
                    .collect();
                items.push(format!("{} = {curve}", p.name));
                format!("t:fx(\"{name}\", {{ {} }})", items.join(", "))
            }
            bus if bus.starts_with("bus(") => {
                format!("{}, {{ {} = {curve} }})", &bus[..bus.len() - 1], p.name)
            }
            other => panic!("tabla sin probar: {other}"),
        };
        let source = format!("{PRELUDE}{call}\n");
        let result = try_eval(&source);
        match p.automate {
            api::Auto::No(why) => {
                let message = result
                    .err()
                    .unwrap_or_else(|| panic!("{call}: debería fallar"));
                assert!(
                    message.contains("no admite ramp() ni lfo()") && message.contains(why),
                    "{call}: {message}"
                );
            }
            _ => {
                if let Err(e) = result {
                    panic!("{call}: {e}");
                }
            }
        }
        checked += 1;
    }
    assert!(checked > 50, "{checked}");
}
