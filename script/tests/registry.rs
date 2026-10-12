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
    }
}

const PRELUDE: &str = r#"
local s = sample("amen.wav")
local amen = s:slice(8)
local t = track("t", amen):steps("1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .")
local escala = scale("minor", "C")
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
    let mut source = String::from(PRELUDE);
    for e in EFFECTS {
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
            for p in e.params {
                for value in p.values {
                    source.push_str(&format!(
                        "track(\"{name}\"):fx(\"{name}\", {{ {} = \"{value}\", cutoff = 1000 }})\n",
                        p.name
                    ));
                }
            }
        }
        assert!(api::effect(e.name).is_some());
    }
    eval(&source);
}

#[test]
fn every_chance_effect_exists() {
    let mut source = String::from(PRELUDE);
    for e in CHANCE_EFFECTS {
        for name in std::iter::once(&e.name).chain(e.aliases) {
            let value = if e.value.is_some() { ", 2" } else { "" };
            source.push_str(&format!("t:chance(0.5, \"{name}\"{value})\n"));
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
        err.message
            .contains("no existe (filter, lowpass, highpass, bitcrush)"),
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
    assert_eq!(names, ["amen", "fill", "ghost", "intro"]);
}
