//! Registro único de la API de Lua: nombre, firma, descripción, parámetros
//! y valores válidos de cada función, efecto de `:fx` y efecto de
//! `:chance`.
//!
//! Lo usan el autocompletado del editor y la sección 4 de `docs/GUIA.md`
//! (que se genera con [`guide_markdown`]; `cargo run -p script --example
//! guia` la reescribe). Los tests comprueban que cada función registrada
//! existe de verdad en el entorno de Lua y que la guía está al día.
//!
//! Plantillas: el texto que inserta el autocompletado. Cada `${...}` es un
//! parámetro que el editor deja marcado para sustituirlo; se insertan sin
//! los `${` `}`.

/// Sobre qué se llama una función.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Receiver {
    /// Función global: `bpm(170)`.
    Global,
    /// `sample("...")` sin cortar.
    Sample,
    /// Sample con sus cortes: `sample("..."):slice(n)`.
    Slices,
    /// Pista: `track(...)`.
    Track,
    /// Compás que recibe la función de `every`.
    Bar,
    /// Escala: `scale("minor", "C")`.
    Scale,
}

impl Receiver {
    /// Nombre corto (el de la variable en las firmas).
    pub fn label(self) -> &'static str {
        match self {
            Receiver::Global => "función",
            Receiver::Sample => "sample",
            Receiver::Slices => "cortes",
            Receiver::Track => "pista",
            Receiver::Bar => "compás",
            Receiver::Scale => "escala",
        }
    }
}

/// Parámetro de una función.
#[derive(Debug, Clone, Copy)]
pub struct Param {
    pub name: &'static str,
    pub description: &'static str,
}

/// Función o método de la API.
#[derive(Debug, Clone, Copy)]
pub struct Function {
    pub name: &'static str,
    pub receiver: Receiver,
    /// Firma tal como se escribe: `pista:fx(nombre, parámetros)`.
    pub signature: &'static str,
    /// Lo que devuelve, si es un objeto de la API.
    pub returns: Option<Receiver>,
    /// Una línea (el autocompletado la muestra).
    pub summary: &'static str,
    /// Explicación para la guía (Markdown), después del resumen.
    pub doc: &'static str,
    pub params: &'static [Param],
    /// Texto que inserta el autocompletado (ver la cabecera del módulo).
    pub template: &'static str,
    /// Ejemplo para la guía (se comprueba en los tests).
    pub example: Option<&'static str>,
}

impl Function {
    /// Firma con lo que devuelve: `sample:slice(n) → cortes`.
    pub fn full_signature(&self) -> String {
        match self.returns {
            Some(r) => format!("{} → {}", self.signature, r.label()),
            None => self.signature.to_string(),
        }
    }
}

/// Parámetro de un efecto de `:fx`.
#[derive(Debug, Clone, Copy)]
pub struct FxParam {
    pub name: &'static str,
    pub description: &'static str,
    /// Valor por defecto (como se escribe en Lua); `None`: obligatorio.
    pub default: Option<&'static str>,
    /// Valores válidos en texto: `20–20000`, `"lowpass"` o `"highpass"`.
    pub valid: &'static str,
    /// Valores posibles, si es una lista cerrada de cadenas.
    pub values: &'static [&'static str],
    /// Texto que inserta el autocompletado.
    pub template: &'static str,
}

/// Efecto de pista para `:fx(nombre, parámetros)`.
#[derive(Debug, Clone, Copy)]
pub struct Effect {
    pub name: &'static str,
    /// Otros nombres que admite.
    pub aliases: &'static [&'static str],
    pub summary: &'static str,
    pub params: &'static [FxParam],
    /// Tabla de parámetros que inserta el autocompletado tras el nombre.
    pub template: &'static str,
}

impl Effect {
    /// Plantilla de la tabla de parámetros (con sus `${...}`) para cambiar
    /// a este efecto desde una tabla escrita: conserva el valor de cada
    /// parámetro de `old` (clave y valor tal como está escrito) que este
    /// efecto también tenga, y pone los de su plantilla que falten con sus
    /// valores por defecto. En el orden de sus parámetros.
    pub fn table_with(&self, old: &[(String, String)]) -> String {
        let defaults = table_items(
            self.template
                .trim()
                .trim_start_matches('{')
                .trim_end_matches('}'),
        );
        let items: Vec<String> = self
            .params
            .iter()
            .filter_map(|p| {
                let kept = old.iter().find(|(k, _)| k == p.name).map(|(_, v)| v.trim());
                let default = defaults
                    .iter()
                    .find(|(k, _)| k == p.name)
                    .map(|(_, v)| v.as_str());
                match (kept, default) {
                    // El valor anterior, marcado (entre comillas, el texto de dentro).
                    (Some(value), _) => Some(
                        match value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
                            Some(inner) => format!("{} = \"${{{inner}}}\"", p.name),
                            None => format!("{} = ${{{value}}}", p.name),
                        },
                    ),
                    (None, Some(default)) => Some(format!("{} = {default}", p.name)),
                    (None, None) => None,
                }
            })
            .collect();
        format!("{{ {} }}", items.join(", "))
    }
}

/// Lee los elementos `clave = valor` de una tabla de Lua escrita en una
/// línea, sin las llaves (`type = "lowpass", cutoff = 3000`). Los que no
/// tienen esa forma se ignoran.
pub fn table_items(inner: &str) -> Vec<(String, String)> {
    let mut items = Vec::new();
    let mut quote = None;
    let mut start = 0;
    let mut push = |item: &str| {
        if let Some((key, value)) = item.split_once('=') {
            let key = key.trim();
            if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
                items.push((key.to_string(), value.trim().to_string()));
            }
        }
    };
    for (i, c) in inner.char_indices() {
        match (c, quote) {
            ('"' | '\'', None) => quote = Some(c),
            (c, Some(q)) if c == q => quote = None,
            (',', None) => {
                push(&inner[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    push(&inner[start..]);
    items
}

/// Efecto de `:chance(p, efecto[, valor])`.
#[derive(Debug, Clone, Copy)]
pub struct ChanceEffect {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    /// Qué es el valor y su rango; `None`: no lleva.
    pub value: Option<&'static str>,
    pub summary: &'static str,
}

/// Snippet: un trozo de script con campos (`${...}`) que se rellenan
/// saltando con Tab.
#[derive(Debug, Clone, Copy)]
pub struct Snippet {
    pub name: &'static str,
    pub summary: &'static str,
    /// Plantilla (varias líneas; las siguientes llevan la sangría de la
    /// primera al insertarse).
    pub template: &'static str,
    /// Lo que tiene que haber antes en el script para que funcione (se
    /// muestra en la guía y se usa en los tests).
    pub requires: &'static str,
}

/// Efecto de `:fill(efecto, n [, pasos [, valor]])`.
#[derive(Debug, Clone, Copy)]
pub struct FillEffect {
    pub name: &'static str,
    /// Qué es el valor, su rango y su valor por defecto; `None`: no lleva.
    pub value: Option<&'static str>,
    pub summary: &'static str,
}

/// Efectos de `:fill`.
pub const FILL_EFFECTS: &[FillEffect] = &[
    FillEffect {
        name: "stutter",
        value: Some("retrigger, 1 a 16 (4)"),
        summary: "Repite el último golpe en cada paso del relleno, con retrigger",
    },
    FillEffect {
        name: "roll",
        value: None,
        summary: "Como stutter, cada vez más rápido: retrigger 2, 4, 8, 16",
    },
    FillEffect {
        name: "reverse",
        value: None,
        summary: "Invierte el sentido de los golpes del relleno",
    },
    FillEffect {
        name: "pitch",
        value: Some("semitonos, -36 a 36 (12)"),
        summary: "Sube (o baja) los golpes del relleno",
    },
    FillEffect {
        name: "mute",
        value: None,
        summary: "Silencio: un corte antes del compás siguiente",
    },
];

/// Escala para `scale(nombre, tónica)`: intervalos en semitonos desde la
/// tónica.
#[derive(Debug, Clone, Copy)]
pub struct ScaleDef {
    pub name: &'static str,
    pub intervals: &'static [i32],
    pub summary: &'static str,
}

/// Escalas de `scale`.
pub const SCALES: &[ScaleDef] = &[
    ScaleDef {
        name: "major",
        intervals: &[0, 2, 4, 5, 7, 9, 11],
        summary: "mayor (jónica)",
    },
    ScaleDef {
        name: "minor",
        intervals: &[0, 2, 3, 5, 7, 8, 10],
        summary: "menor natural (eólica)",
    },
    ScaleDef {
        name: "harmonic_minor",
        intervals: &[0, 2, 3, 5, 7, 8, 11],
        summary: "menor armónica",
    },
    ScaleDef {
        name: "melodic_minor",
        intervals: &[0, 2, 3, 5, 7, 9, 11],
        summary: "menor melódica (ascendente)",
    },
    ScaleDef {
        name: "dorian",
        intervals: &[0, 2, 3, 5, 7, 9, 10],
        summary: "dórica",
    },
    ScaleDef {
        name: "phrygian",
        intervals: &[0, 1, 3, 5, 7, 8, 10],
        summary: "frigia",
    },
    ScaleDef {
        name: "lydian",
        intervals: &[0, 2, 4, 6, 7, 9, 11],
        summary: "lidia",
    },
    ScaleDef {
        name: "mixolydian",
        intervals: &[0, 2, 4, 5, 7, 9, 10],
        summary: "mixolidia",
    },
    ScaleDef {
        name: "locrian",
        intervals: &[0, 1, 3, 5, 6, 8, 10],
        summary: "locria",
    },
    ScaleDef {
        name: "major_pentatonic",
        intervals: &[0, 2, 4, 7, 9],
        summary: "pentatónica mayor",
    },
    ScaleDef {
        name: "minor_pentatonic",
        intervals: &[0, 3, 5, 7, 10],
        summary: "pentatónica menor",
    },
    ScaleDef {
        name: "blues",
        intervals: &[0, 3, 5, 6, 7, 10],
        summary: "blues",
    },
    ScaleDef {
        name: "chromatic",
        intervals: &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11],
        summary: "cromática (cada grado, un semitono)",
    },
];

/// Tónicas de `scale`, con su distancia en semitonos a do (C).
pub const NOTES: &[(&str, i32)] = &[
    ("C", 0),
    ("C#", 1),
    ("Db", 1),
    ("D", 2),
    ("D#", 3),
    ("Eb", 3),
    ("E", 4),
    ("F", 5),
    ("F#", 6),
    ("Gb", 6),
    ("G", 7),
    ("G#", 8),
    ("Ab", 8),
    ("A", 9),
    ("A#", 10),
    ("Bb", 10),
    ("B", 11),
];

/// Escala por nombre.
pub fn scale(name: &str) -> Option<&'static ScaleDef> {
    SCALES.iter().find(|s| s.name == name)
}

/// Semitonos de una tónica sobre do (`"Eb"` → 3). La letra puede ir en
/// minúscula.
pub fn note(name: &str) -> Option<i32> {
    let mut chars = name.chars();
    let first = chars.next()?.to_ascii_uppercase();
    let name = format!("{first}{}", chars.as_str());
    NOTES.iter().find(|(n, _)| *n == name).map(|(_, s)| *s)
}

/// Funciones globales y métodos, en el orden de la guía.
pub const FUNCTIONS: &[Function] = &[
    Function {
        name: "bpm",
        receiver: Receiver::Global,
        signature: "bpm(n)",
        returns: None,
        summary: "Tempo en pulsos por minuto, de 20 a 400.",
        doc: "Si no se llama, se mantiene el que haya (o el del proyecto).",
        params: &[Param {
            name: "n",
            description: "pulsos por minuto (20 a 400, admite decimales)",
        }],
        template: "bpm(${170})",
        example: Some("bpm(174)\n"),
    },
    Function {
        name: "sample",
        receiver: Receiver::Global,
        signature: "sample(ruta)",
        returns: Some(Receiver::Sample),
        summary: "Carga un WAV.",
        doc: "La ruta es relativa a la carpeta del script o, en un proyecto, a su \
              carpeta `samples`. Recargar el script no vuelve a leer el archivo si \
              no ha cambiado.",
        params: &[Param {
            name: "ruta",
            description: "archivo WAV (cualquier frecuencia; se resamplea al cargar)",
        }],
        template: "sample(\"${amen.wav}\")",
        example: Some("local amen = sample(\"amen.wav\")\n"),
    },
    Function {
        name: "track",
        receiver: Receiver::Global,
        signature: "track(nombre, cortes) / track(nombre)",
        returns: Some(Receiver::Track),
        summary: "Define una pista que toca esos cortes, o recupera una ya definida.",
        doc: "Con dos argumentos define (o redefine) la pista; con uno, recupera \
              una pista ya definida para seguir configurándola. Hay como mucho 8 \
              pistas; suenan a la vez y se suman. Todos los métodos de la pista \
              devuelven la pista, así que se pueden encadenar.",
        params: &[
            Param {
                name: "nombre",
                description: "nombre de la pista; conserva su pista del motor entre recargas",
            },
            Param {
                name: "cortes",
                description: "lo que devuelve `sample(...):slice(n)` o `:slice_at(i)`",
            },
        ],
        template: "track(\"${drums}\", ${amen})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen)\n\
             track(\"drums\"):steps(\"1 . . .  3 . . .  1 . . .  3 . . .\")\n",
        ),
    },
    Function {
        name: "metronome",
        receiver: Receiver::Global,
        signature: "metronome([sí])",
        returns: None,
        summary: "Enciende (o, con false, apaga) el metrónomo: un clic en cada pulso.",
        doc: "El primer pulso del compás lleva acento (más agudo y más fuerte). Es \
              para tocar y escuchar: el render exportado no lo lleva salvo que se \
              pida (`exportar ... metronomo`, o `--metronome` en la línea de \
              comandos). También se enciende y apaga desde la paleta. Sin llamarlo, \
              se queda como esté.",
        params: &[Param {
            name: "sí",
            description: "`true` (por defecto) lo enciende; `false` lo apaga",
        }],
        template: "metronome(${true})",
        example: Some("bpm(170)\nmetronome(true)\n"),
    },
    Function {
        name: "bus",
        receiver: Receiver::Global,
        signature: "bus(nombre, parámetros)",
        returns: None,
        summary: "Parámetros de un bus de envío (delay o reverb), para toda la canción.",
        doc: "Hay dos buses que comparten todas las pistas: un delay sincronizado al \
              tempo y una reverb. Cada pista les manda una parte de su salida con \
              `pista:send(nombre, nivel)` (después de su volumen, mute y solo) y lo \
              que devuelven se suma al master antes del limitador. Sin `bus()`, los \
              valores por defecto. Los cambios entran al empezar el compás, como \
              el tempo; la cola del delay y de la reverb sigue sonando aunque se \
              pare el secuenciador. Una clave desconocida da error.",
        params: &[
            Param {
                name: "nombre",
                description: "`\"delay\"` o `\"reverb\"`",
            },
            Param {
                name: "parámetros",
                description: "tabla con los de la tabla de abajo (los que falten, por defecto)",
            },
        ],
        template: "bus(\"${delay}\", { time = \"${1/8}\", feedback = ${0.4} })",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             bus(\"delay\", { time = \"3/16\", feedback = 0.5, level = 0.7 })\n\
             bus(\"reverb\", { size = 0.8, damp = 0.4 })\n\
             track(\"drums\", amen)\n  \
             :steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n  \
             :send(\"reverb\", 0.2)\n\
             -- Solo la caja, al delay.\n\
             track(\"caja\", amen:slice_at(2))\n  \
             :steps(\". . . .  x . . .  . . . .  x . . .\")\n  \
             :send(\"delay\", 0.6)\n",
        ),
    },
    Function {
        name: "euclid",
        receiver: Receiver::Global,
        signature: "euclid(k, n [, golpe [, pasos]])",
        returns: None,
        summary: "Patrón euclídeo: k golpes repartidos por igual en n pasos, repetido hasta llenar el patrón.",
        doc: "Devuelve texto en mini-notación para `steps`. Empieza por un golpe (el paso \
              `i` suena si `i·k mod n < k`); para desplazarlo, `rotate`. El ritmo se \
              repite hasta llenar `pasos` (16 por defecto; para una pista con \
              `:length(n)`, pasa su longitud) y se corta ahí si `n` no lo divide. \
              El golpe puede ser cualquier paso que suene, con sufijos o una \
              etiqueta.",
        params: &[
            Param {
                name: "k",
                description: "golpes (0 a n)",
            },
            Param {
                name: "n",
                description: "pasos del ritmo (1 a 64)",
            },
            Param {
                name: "golpe",
                description: "el paso que suena (por defecto \"x\", el corte 1)",
            },
            Param {
                name: "pasos",
                description: "longitud del patrón que devuelve (1 a 64; por defecto 16)",
            },
        ],
        template: "euclid(${3}, ${8}, \"${x}\")",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             -- \"1 . . 1  . . 1 .  1 . . 1  . . 1 .\"\n\
             track(\"kick\", amen):steps(euclid(3, 8, \"1\"))\n\
             track(\"hats\", amen):steps(euclid(5, 16, \"2@0.6\"))\n\
             -- Tres golpes en 12 pasos, contra el compás de 16.\n\
             track(\"tres\", amen):length(12):steps(euclid(3, 12, \"3\", 12))\n",
        ),
    },
    Function {
        name: "rotate",
        receiver: Receiver::Global,
        signature: "rotate(patrón, n)",
        returns: None,
        summary: "Desplaza un patrón n pasos: positivo, más tarde; negativo, antes.",
        doc: "Los pasos que se salen por un lado entran por el otro. Devuelve texto en \
              mini-notación; se puede combinar con las demás funciones de patrones.",
        params: &[
            Param {
                name: "patrón",
                description: "patrón en mini-notación (de 1 a 64 pasos)",
            },
            Param {
                name: "n",
                description: "pasos que se desplaza (negativo: hacia antes)",
            },
        ],
        template: "rotate(\"${1 . . .  . . . .  . . . .  . . . .}\", ${2})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             -- El break empezando en su segundo tiempo.\n\
             track(\"drums\", amen):steps(rotate(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\", -4))\n\
             -- Un euclídeo que empieza más tarde.\n\
             track(\"kick\", amen):steps(rotate(euclid(3, 8, \"1\"), 2))\n",
        ),
    },
    Function {
        name: "reverse_pattern",
        receiver: Receiver::Global,
        signature: "reverse_pattern(patrón)",
        returns: None,
        summary: "El patrón al revés: el último paso pasa a ser el primero.",
        doc: "Invierte el orden de los pasos, no el sonido de cada golpe: para que un \
              golpe suene al revés está el sufijo `'` (y `c:reverse()` en `every`).",
        params: &[Param {
            name: "patrón",
            description: "patrón en mini-notación (de 1 a 64 pasos)",
        }],
        template: "reverse_pattern(\"${1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .}\")",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             local base = \"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\"\n\
             track(\"drums\", amen)\n  \
             :steps(base)\n  \
             :every(4, function(c) c:steps(reverse_pattern(base)) end)\n",
        ),
    },
    Function {
        name: "mutate",
        receiver: Receiver::Global,
        signature: "mutate(patrón, p [, semilla])",
        returns: None,
        summary: "Cambia cada paso, con probabilidad p, por otro del mismo patrón al azar.",
        doc: "Se queda con los golpes y silencios del patrón, solo los cambia de \
              sitio aquí y allá. Con `semilla`, el resultado es siempre el mismo. \
              Sin ella, el azar sigue la regla de `chance`: se tira al cargar (otro \
              en cada carga, salvo con la semilla fija de los tests), y dentro de \
              `every` cada compás sale distinto y el ciclo se repite.",
        params: &[
            Param {
                name: "patrón",
                description: "patrón en mini-notación (de 1 a 64 pasos)",
            },
            Param {
                name: "p",
                description: "probabilidad de cambiar cada paso (0 a 1)",
            },
            Param {
                name: "semilla",
                description: "número entero; con la misma, el mismo resultado",
            },
        ],
        template: "mutate(\"${1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .}\", ${0.25}, ${7})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             local base = \"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\"\n\
             -- Una variación fija (semilla 7) y, cada 2 compases, otra nueva.\n\
             track(\"drums\", amen)\n  \
             :steps(mutate(base, 0.25, 7))\n  \
             :every(2, function(c) c:steps(mutate(base, 0.3)) end)\n",
        ),
    },
    Function {
        name: "shuffle",
        receiver: Receiver::Global,
        signature: "shuffle(patrón [, semilla])",
        returns: None,
        summary: "Los mismos pasos en otro orden al azar.",
        doc: "Conserva cada golpe y cada silencio, solo los cambia de sitio. La semilla \
              funciona como en `mutate`: con ella, siempre el mismo orden; sin ella, \
              el azar de la carga (como `chance`).",
        params: &[
            Param {
                name: "patrón",
                description: "patrón en mini-notación (de 1 a 64 pasos)",
            },
            Param {
                name: "semilla",
                description: "número entero; con la misma, el mismo orden",
            },
        ],
        template: "shuffle(\"${1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .}\", ${3})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             -- Los golpes del break en un orden nuevo, siempre el mismo (semilla 3).\n\
             track(\"drums\", amen):steps(shuffle(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\", 3))\n",
        ),
    },
    Function {
        name: "ramp",
        receiver: Receiver::Global,
        signature: "ramp(desde, hasta, duración)",
        returns: None,
        summary: "Valor que va de desde a hasta en esa duración y vuelve a empezar (para un parámetro de :fx).",
        doc: "Se pone en lugar de un número en la tabla de `:fx` (`cutoff`, `q`, \
              `threshold`, `makeup`, `drive` o `mix`). Es una rampa en diente de \
              sierra alineada con la canción: con `\"4 compases\"` sube en los \
              compases 0-3, otra vez en los 4-7... (como `every`, cuenta desde que \
              arranca el secuenciador, así que recargar no la desplaza). Las \
              frecuencias suben en escala logarítmica (de 200 a 4000 Hz, la mitad \
              del camino es 894 Hz). Entra con el patrón al empezar el compás y se \
              suaviza: nunca hace clic.",
        params: &[
            Param {
                name: "desde",
                description: "valor al empezar",
            },
            Param {
                name: "hasta",
                description: "valor al acabar",
            },
            Param {
                name: "duración",
                description: "`\"N compases\"` o `\"N pasos\"` (hasta 64 compases)",
            },
        ],
        template: "ramp(${200}, ${4000}, \"${4 compases}\")",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen)\n  \
             :steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n  \
             -- El filtro se abre durante 4 compases y vuelve a cerrarse.\n  \
             :fx(\"lowpass\", { cutoff = ramp(200, 8000, \"4 compases\") })\n",
        ),
    },
    Function {
        name: "lfo",
        receiver: Receiver::Global,
        signature: "lfo(frecuencia, mín, máx)",
        returns: None,
        summary: "Valor que oscila (seno) entre mín y máx (para un parámetro de :fx).",
        doc: "Como `ramp`, va en lugar de un número en la tabla de `:fx`. La \
              frecuencia es un número en Hz (de 0.01 a 50) o lo que dura un ciclo \
              en el tempo de la canción (`\"1 compás\"`, `\"8 pasos\"`). Empieza en \
              `mín` cuando arranca el secuenciador; las frecuencias oscilan en \
              escala logarítmica.",
        params: &[
            Param {
                name: "frecuencia",
                description: "Hz, o la duración de un ciclo: `\"1 compás\"`, `\"8 pasos\"`",
            },
            Param {
                name: "mín",
                description: "valor más bajo",
            },
            Param {
                name: "máx",
                description: "valor más alto",
            },
        ],
        template: "lfo(\"${1 compás}\", ${400}, ${3000})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen)\n  \
             :steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n  \
             :fx(\"lowpass\", { cutoff = lfo(\"2 compases\", 400, 6000), q = 2 })\n  \
             :fx(\"distortion\", { drive = 6, mix = lfo(0.5, 0, 0.8) })\n",
        ),
    },
    Function {
        name: "scale",
        receiver: Receiver::Global,
        signature: "scale(nombre [, tónica])",
        returns: Some(Receiver::Scale),
        summary: "Una escala, para tocar melodías con un corte: sus grados se convierten en pitch.",
        doc: "El pitch se cuenta desde el tono original del corte, que hace de do \
              (C): con tónica `\"D\"` todo sube 2 semitonos. Las escalas están en la \
              tabla de abajo; las tónicas, de `C` a `B` con `#` o `b` (`F#`, `Bb`).",
        params: &[
            Param {
                name: "nombre",
                description: "la escala (ver la tabla)",
            },
            Param {
                name: "tónica",
                description: "nota de partida (por defecto \"C\", el tono del corte)",
            },
        ],
        template: "scale(\"${minor}\", \"${C}\")",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             local menor = scale(\"minor\", \"C\")\n\
             -- Grados 1 3 5 3... de la escala sobre el corte 3:\n\
             -- \"3^0 . 3^3 .  3^7 . 3^3 .  ...\"\n\
             track(\"melodia\", amen)\n  \
             :steps(menor:melody(\"1 . 3 .  5 . 3 .  8 . 7 .  5 . 3 -2\", \"3\"))\n  \
             :gain(0.6)\n",
        ),
    },
    Function {
        name: "pitch",
        receiver: Receiver::Scale,
        signature: "escala:pitch(grado)",
        returns: None,
        summary: "Semitonos de un grado de la escala (1 es la tónica, 8 la octava).",
        doc: "Los grados siguen hacia arriba por octavas (en una escala de 7 notas, \
              8 es la tónica una octava arriba) y hacia abajo con negativos (-1 es el \
              último grado una octava abajo). El 0 no existe. Sirve para escribir \
              pitch a mano: `\"3^\" .. menor:pitch(5)`.",
        params: &[Param {
            name: "grado",
            description: "número de grado (1, 2, 3...; negativos, por debajo)",
        }],
        template: "pitch(${5})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             local quinta = scale(\"major\"):pitch(5) -- 7 semitonos\n\
             track(\"d\", amen):steps(\"3 . . .  3^\" .. quinta .. \" . . .  3 . . .  3^12 . . .\")\n",
        ),
    },
    Function {
        name: "melody",
        receiver: Receiver::Scale,
        signature: "escala:melody(grados [, golpe])",
        returns: None,
        summary: "Convierte un patrón de grados en uno de pasos con pitch: \"1 3 5 3\" → \"1^0 1^3 1^7 1^3\".",
        doc: "Cada paso de `grados` es un grado (o `.`), con los sufijos que quieras \
              salvo `^`: `5@0.5` da el golpe con el pitch del grado 5 a medio volumen. \
              Devuelve texto en mini-notación para `steps`.",
        params: &[
            Param {
                name: "grados",
                description: "pasos: grados de la escala o `.`",
            },
            Param {
                name: "golpe",
                description: "el corte que suena (número o etiqueta; por defecto \"1\")",
            },
        ],
        template: "melody(\"${1 . 3 .  5 . 3 .  1 . 3 .  5 . 8 .}\", \"${1}\")",
        example: None,
    },
    Function {
        name: "detect_bpm",
        receiver: Receiver::Sample,
        signature: "sample:detect_bpm()",
        returns: None,
        summary: "Tempo y compases del sample, suponiendo que es un loop de compases enteros.",
        doc: "Devuelve dos valores, el tempo (entero si sale muy cerca de uno) y los \
              compases, o `nil` si no se ve (sin golpes, o con un tempo fuera de 60 \
              a 200). Mira cómo encajan los golpes en la rejilla de semicorcheas \
              para 1, 2, 4 y 8 compases; si dos encajan igual (un break en corcheas \
              vale a 86 y a 172), elige el más cercano a 130. Lo mismo hacen \
              `--detect-bpm` y «Detectar el BPM del sample» de la paleta.",
        params: &[],
        template: "detect_bpm()",
        example: Some(
            "local amen = sample(\"amen.wav\")\n\
             local tempo, compases = amen:detect_bpm()\n\
             bpm(tempo)\n\
             track(\"drums\", amen:slice(8)):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n",
        ),
    },
    Function {
        name: "stretch",
        receiver: Receiver::Sample,
        signature: "sample:stretch([bpm_origen])",
        returns: Some(Receiver::Sample),
        summary: "Estira el sample del tempo de origen al de la canción sin cambiar el tono.",
        doc: "Al cargar, el sample pasa de `bpm_origen` al tempo de la canción (el de \
              `bpm()`, el del proyecto o 120), sin cambiar el tono: un break de 140 \
              suena a 170 y sus golpes caen en la rejilla. Sin `bpm_origen`, usa el \
              que detecta `detect_bpm()` (si no lo ve, da error: pásalo). Los cortes \
              se buscan en el sample original y se escalan, así que `slice(n)`, las \
              etiquetas y los ajustes en ms (`move`, proyecto) son los de siempre. \
              Va bien entre ×0,7 y ×1,4 (de ×0,25 a ×4 se admite). Es WSOLA por \
              tramos de golpe a golpe: cada golpe empieza justo en su sitio y el \
              resto se estira copiando la forma de onda; estirar mucho repite o \
              emborrona un poco las colas largas.",
        params: &[Param {
            name: "bpm_origen",
            description: "tempo original del sample (20 a 400); sin él, el detectado",
        }],
        template: "stretch(${140})",
        example: Some(
            "bpm(170)\n\
             -- El break está a 140: suena a 170 con el mismo tono.\n\
             local amen = sample(\"amen.wav\"):stretch(140):slice(8)\n\
             track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n\
             -- O con el tempo que detecta.\n\
             local otro = sample(\"amen.wav\"):stretch():slice(8)\n",
        ),
    },
    Function {
        name: "slice_grid",
        receiver: Receiver::Sample,
        signature: "sample:slice_grid(n)",
        returns: Some(Receiver::Slices),
        summary: "Corta el sample en n partes iguales (rejilla), sin mirar los golpes.",
        doc: "Para un break bien cuadrado (un compás en 16 o en 8) o para material \
              sin golpes claros. Las etiquetas y los ajustes del proyecto de un \
              sample cortado en rejilla son distintos de los de `slice(n)` (en el \
              proyecto, el número de cortes se escribe `rejilla16`).",
        params: &[Param {
            name: "n",
            description: "partes (1 a 64)",
        }],
        template: "slice_grid(${16})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice_grid(16)\n\
             track(\"drums\", amen):steps(\"1 2 3 4  5 6 7 8  9 10 11 12  13 14 15 16\")\n",
        ),
    },
    Function {
        name: "slice",
        receiver: Receiver::Sample,
        signature: "sample:slice(n)",
        returns: Some(Receiver::Slices),
        summary: "Corta el sample en sus n golpes más fuertes.",
        doc: "Si hay menos golpes, devuelve menos cortes. Usa el detector por flujo \
              espectral con 40 ms de separación mínima.",
        params: &[Param {
            name: "n",
            description: "cortes a buscar (1 a 64)",
        }],
        template: "slice(${8})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             local cuantos = amen:count()\n\
             track(\"drums\", amen):steps(\"1 . . .  . . . .  1 . . .  . . . .\")\n",
        ),
    },
    Function {
        name: "count",
        receiver: Receiver::Slices,
        signature: "cortes:count()",
        returns: None,
        summary: "Número de cortes.",
        doc: "Puede ser menor que el pedido en `slice(n)` si el detector encontró \
              menos golpes.",
        params: &[],
        template: "count()",
        example: None,
    },
    Function {
        name: "slice_at",
        receiver: Receiver::Slices,
        signature: "cortes:slice_at(i)",
        returns: Some(Receiver::Slices),
        summary: "Solo el corte i (desde 1), como fuente de un único corte.",
        doc: "En el patrón de la pista es `1` o `x`.",
        params: &[Param {
            name: "i",
            description: "número del corte (desde 1)",
        }],
        template: "slice_at(${1})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             local caja = amen:slice_at(3)\n\
             track(\"caja\", caja):steps(\". . . .  x . . .  . . . .  x . . .\")\n",
        ),
    },
    Function {
        name: "set",
        receiver: Receiver::Slices,
        signature: "cortes:set([i,] parámetros)",
        returns: Some(Receiver::Slices),
        summary: "Forma de un corte (o de todos, sin i): ataque, cola, pan, parte que suena y bucle.",
        doc: "Vale para todas las pistas que usan esta variable, también las \
              definidas antes (como `:name`). Una clave desconocida da error. Los \
              parámetros están en la tabla de abajo; los que no se dan quedan como \
              estaban.",
        params: &[
            Param {
                name: "i",
                description: "el corte (desde 1); sin él, todos",
            },
            Param {
                name: "parámetros",
                description: "tabla con los de la tabla de abajo",
            },
        ],
        template: "set(${1}, { attack = ${5}, release = ${40} })",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             -- El charles (corte 3), corto y sin chasquido; todos, con algo de cola.\n\
             amen:set({ release = 20 }):set(3, { attack = 2, release = 5, pan = 0.4 })\n\
             -- El corte 8, la mitad y en bucle de ida y vuelta.\n\
             amen:set(8, { length = 0.5, play = \"pingpong\" })\n\
             track(\"drums\", amen):steps(\"1 . 3 .  2 . 3 .  1 1 3 .  2 . 3 3\"):mode(\"gate\")\n",
        ),
    },
    Function {
        name: "add",
        receiver: Receiver::Slices,
        signature: "cortes:add(ms)",
        returns: Some(Receiver::Slices),
        summary: "Añade un corte que empieza en ms (parte en dos el que lo contiene).",
        doc: "Los cortes siguientes se corren un número. Como `move`, cambia la \
              variable (y devuelve una copia: para seguir cambiándola, otra línea \
              con la variable): las pistas definidas antes con ella no lo ven. Tiene que \
              quedar a más de 1 ms de los cortes que ya hay. Desde la interfaz, \
              «Partir el corte seleccionado» lo hace en el proyecto.",
        params: &[Param {
            name: "ms",
            description: "dónde empieza el corte nuevo, en ms desde el principio",
        }],
        template: "add(${250})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             -- Un corte más dentro del tercero, y sin la marca del sexto (que,\n\
             -- con el nuevo, es el séptimo).\n\
             amen:add(250)\n\
             amen:remove(7)\n\
             track(\"drums\", amen):steps(\"1 . 3 4  5 . 6 .  7 . 8 .  1 . 3 4\")\n",
        ),
    },
    Function {
        name: "remove",
        receiver: Receiver::Slices,
        signature: "cortes:remove(i)",
        returns: Some(Receiver::Slices),
        summary: "Quita el corte i (desde 2): el anterior se alarga hasta donde acababa este.",
        doc: "Los cortes siguientes se corren un número. Como `move`, cambia la \
              variable. Desde la interfaz, «Unir el corte seleccionado con el \
              anterior» lo hace en el proyecto.",
        params: &[Param {
            name: "i",
            description: "el corte que se quita (desde 2)",
        }],
        template: "remove(${2})",
        example: None,
    },
    Function {
        name: "name",
        receiver: Receiver::Slices,
        signature: "cortes:name(i, nombre)",
        returns: Some(Receiver::Slices),
        summary: "Pone una etiqueta al corte i para usarla en steps en vez del número.",
        doc: "Con la etiqueta, `steps(\"kick . snare .\")` es lo mismo que \
              `steps(\"1 . 3 .\")`, y admite los mismos sufijos (`kick^7@0.5`). Vale \
              para las pistas de esa variable, también las definidas antes, y \
              `slice_at` conserva la etiqueta de su corte. `nil` la quita. El nombre \
              lleva letras sin tilde, números y `_`, y no puede ser un número, \
              `x` ni acabar en `r` y un número (se confundiría con el \
              retrigger); dos cortes no pueden llevar la misma. Las etiquetas \
              también se pueden guardar en el proyecto (sección 1); las de \
              `name` mandan.",
        params: &[
            Param {
                name: "i",
                description: "número del corte (desde 1)",
            },
            Param {
                name: "nombre",
                description: "la etiqueta (`\"kick\"`, `\"snare\"`...), o `nil` para quitarla",
            },
        ],
        template: "name(${1}, \"${kick}\")",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             amen:name(1, \"kick\"):name(2, \"hat\"):name(3, \"snare\")\n\
             track(\"drums\", amen):steps(\"kick . hat .  snare . hat .  kick kick hat .  snare . hat^7@0.5 .\")\n",
        ),
    },
    Function {
        name: "move",
        receiver: Receiver::Slices,
        signature: "cortes:move(i, ms)",
        returns: Some(Receiver::Slices),
        summary: "Ajuste manual: el corte i empieza en ms (desde el principio del sample).",
        doc: "Manda sobre el detector; el corte anterior acaba donde empieza este. \
              Tiene que quedar a más de 1 ms de sus vecinos. Cambia la variable, \
              así que hazlo antes de usarla en `track()`. Desde la interfaz, el \
              mismo ajuste se hace con teclas en el modo tocar y se guarda en el \
              proyecto (`corte = amen.wav 8 3 412.5`).",
        params: &[
            Param {
                name: "i",
                description: "número del corte (desde 1)",
            },
            Param {
                name: "ms",
                description: "dónde empieza, en ms desde el principio del sample",
            },
        ],
        template: "move(${2}, ${200})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             -- El charles del corte 2, un poco antes para que entre con su ataque.\n\
             amen:move(2, 205.5)\n\
             track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n",
        ),
    },
    Function {
        name: "steps",
        receiver: Receiver::Track,
        signature: "pista:steps(texto)",
        returns: Some(Receiver::Track),
        summary: "Patrón de la pista en mini-notación (16 pasos, o los de :length).",
        doc: "La notación está en la sección 3. Lleva tantos pasos como la longitud \
              de la pista (16, o la de `:length(n)`, que va antes). Los cortes que no \
              existen dan error en esa línea.",
        params: &[Param {
            name: "texto",
            description: "pasos separados por espacios: 16, o los de :length(n)",
        }],
        template: "steps(\"${1 . . .  . . . .  . . . .  . . . .}\")",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n",
        ),
    },
    Function {
        name: "length",
        receiver: Receiver::Track,
        signature: "pista:length(n)",
        returns: Some(Receiver::Track),
        summary: "Pasos de una vuelta del patrón (16 por defecto, de 1 a 64): polirritmia entre pistas.",
        doc: "Va antes de `:steps()`, que entonces lleva `n` pasos. Los pasos siguen \
              siendo semicorcheas: una pista de 12 pasos da la vuelta cada 12 y se \
              desfasa contra el compás de 16 (cada 3 compases vuelven a coincidir); \
              una de 32 ocupa dos compases. Las vueltas van alineadas con la canción \
              desde que arranca, aunque se recargue. En una pista así, `every`, \
              `fill`, `chance` y `humanize` cuentan vueltas del patrón en vez de \
              compases, y los cambios del script siguen entrando al empezar un \
              compás.",
        params: &[Param {
            name: "n",
            description: "pasos de una vuelta (1 a 64)",
        }],
        template: "length(${16})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n\
             -- 3 contra 4: un charles de 12 pasos sobre el compás de 16.\n\
             track(\"hats\", amen):length(12):steps(\"3 . 3@0.5 .  3 . . 3  . 3@0.5 . .\")\n\
             -- Dos compases de bombo.\n\
             track(\"kick\", amen:slice_at(1))\n  \
             :length(32)\n  \
             :steps(\"x . . .  . . . .  x . . .  . . . .  x . . .  . . x .  . . . .  . . . .\")\n",
        ),
    },
    Function {
        name: "gain",
        receiver: Receiver::Track,
        signature: "pista:gain(g)",
        returns: Some(Receiver::Track),
        summary: "Volumen de la pista, lineal de 0 a 4.",
        doc: "Se aplica después de los efectos.",
        params: &[Param {
            name: "g",
            description: "volumen lineal (0 a 4; 1 lo deja igual)",
        }],
        template: "gain(${0.8})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\"):gain(0.7)\n",
        ),
    },
    Function {
        name: "swing",
        receiver: Receiver::Track,
        signature: "pista:swing(porcentaje)",
        returns: Some(Receiver::Track),
        summary: "Swing: retrasa los contratiempos de semicorchea (pasos 2, 4, 6...).",
        doc: "El porcentaje es el de las cajas de ritmos (MPC): dónde cae el \
              contratiempo dentro de cada par de semicorcheas. 50 es recto, 54-58 \
              un swing suave, 66 casi tresillo y 75 el máximo (el contratiempo a \
              medio paso). Se aplica después de `every`, `chance` y `fill`, al \
              frame exacto y igual en cada pasada.",
        params: &[Param {
            name: "porcentaje",
            description: "de 50 (recto) a 75",
        }],
        template: "swing(${58})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen)\n  \
             :steps(\"1 2 2 2  3 2 4 2  5 6 2 2  7 2 8 2\")\n  \
             :swing(62)\n",
        ),
    },
    Function {
        name: "humanize",
        receiver: Receiver::Track,
        signature: "pista:humanize(ms [, volumen [, semilla]])",
        returns: Some(Receiver::Track),
        summary: "Imperfección humana: cada golpe se retrasa y se suaviza un poco al azar.",
        doc: "Cada golpe se retrasa entre 0 y `ms` milisegundos (nunca se adelanta; \
              como mucho, 0,9 pasos) y su volumen se multiplica por un factor entre \
              `1 - volumen` y 1 (nunca sube). Cada compás sale distinto: la pista \
              pasa a un ciclo de 16 compases, como con `chance`. Con `semilla`, \
              siempre igual; sin ella, el azar de la carga (como `chance`), distinto \
              para cada pista. Se suma al swing.",
        params: &[
            Param {
                name: "ms",
                description: "retraso máximo en milisegundos (0 a 100)",
            },
            Param {
                name: "volumen",
                description: "cuánto puede bajar el volumen (0 a 1; por defecto 0)",
            },
            Param {
                name: "semilla",
                description: "número entero; con la misma, el mismo resultado",
            },
        ],
        template: "humanize(${8}, ${0.15})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen)\n  \
             :steps(\"1 2 2 2  3 2 4 2  5 6 2 2  7 2 8 2\")\n  \
             :swing(56)\n  \
             :humanize(6, 0.2, 1)\n",
        ),
    },
    Function {
        name: "fill",
        receiver: Receiver::Track,
        signature: "pista:fill(efecto, n [, pasos [, valor]])",
        returns: Some(Receiver::Track),
        summary: "Relleno: los últimos pasos del último compás de cada n, con un efecto.",
        doc: "En los compases n, 2n, 3n... (contando desde 1: el último de cada grupo \
              de n) los últimos `pasos` pasos (4 por defecto: el último tiempo) \
              sufren el efecto. `stutter` y `roll` los rellenan con el golpe del \
              principio del relleno (o el último antes, si ahí hay silencio). Se \
              aplica en orden con `every` y `chance`, y alarga el ciclo como \
              `every(n)`. Los efectos están en la tabla de abajo.",
        params: &[
            Param {
                name: "efecto",
                description: "el efecto (ver la tabla)",
            },
            Param {
                name: "n",
                description: "cada cuántos compases (1 a 64)",
            },
            Param {
                name: "pasos",
                description: "cuántos pasos del final (1 a 64, como mucho la longitud; por defecto 4)",
            },
            Param {
                name: "valor",
                description: "el valor del efecto, si lo lleva",
            },
        ],
        template: "fill(\"${stutter}\", ${4}, ${4})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen)\n  \
             :steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n  \
             :fill(\"roll\", 4)\n  \
             :fill(\"pitch\", 8, 2, 7)\n",
        ),
    },
    Function {
        name: "mute",
        receiver: Receiver::Track,
        signature: "pista:mute([sí])",
        returns: Some(Receiver::Track),
        summary: "Silencia la pista (sigue avanzando, no se oye). `mute(false)` la deja sonar.",
        doc: "Entra al empezar el compás, como cualquier cambio del script. Desde \
              la interfaz hay además un mute en vivo, inmediato (paleta o `m` en el \
              modo tocar), que se suma a este: la pista calla si cualquiera de los \
              dos lo pide.",
        params: &[Param {
            name: "sí",
            description: "`true` (por defecto) silencia; `false` no",
        }],
        template: "mute()",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n\
             track(\"kick\", amen:slice_at(1)):steps(\"x . . .  x . . .  x . . .  x . . .\"):mute()\n",
        ),
    },
    Function {
        name: "solo",
        receiver: Receiver::Track,
        signature: "pista:solo([sí])",
        returns: Some(Receiver::Track),
        summary: "Pone la pista en solo: mientras alguna lo esté, solo suenan las que están en solo.",
        doc: "Como `mute`: entra al compás y se suma al solo en vivo de la interfaz \
              (paleta o `s` en el modo tocar). Un mute gana a un solo.",
        params: &[Param {
            name: "sí",
            description: "`true` (por defecto) la pone en solo; `false` no",
        }],
        template: "solo()",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\"):solo()\n\
             track(\"kick\", amen:slice_at(1)):steps(\"x . . .  x . . .  x . . .  x . . .\")\n",
        ),
    },
    Function {
        name: "fx",
        receiver: Receiver::Track,
        signature: "pista:fx(nombre, parámetros)",
        returns: Some(Receiver::Track),
        summary: "Añade un efecto a la cadena de la pista (hasta 4, en orden).",
        doc: "Un parámetro desconocido da error, para pillar las erratas. Los \
              efectos y sus parámetros están en la tabla de abajo.\n\n\
              El filtro es un biquad de 12 dB/octava; `q = 0.707` es la respuesta \
              más plana y valores altos resuenan en el corte. El bitcrusher reduce \
              la resolución a `bits` y repite cada muestra `downsample` veces \
              (reducción de la frecuencia de muestreo, sin filtro: suena sucio a \
              propósito). Los parámetros `cutoff`, `q`, `threshold`, `makeup`, `drive` y `mix` \
              admiten `ramp()` y `lfo()` en lugar de un número; el corte del primer \
              filtro, además, puede cambiar en cada golpe con `~` en `steps` \
              (`3~800`).",
        params: &[
            Param {
                name: "nombre",
                description: "el efecto (ver la tabla)",
            },
            Param {
                name: "parámetros",
                description: "tabla con los parámetros del efecto",
            },
        ],
        template: "fx(\"${filter}\", { type = \"${lowpass}\", cutoff = ${3000} })",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen)\n  \
             :steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n  \
             :fx(\"filter\", { type = \"lowpass\", cutoff = 3000 })\n  \
             :fx(\"highpass\", { cutoff = 80, q = 0.9 })\n  \
             :fx(\"bitcrush\", { bits = 6, downsample = 2 })\n",
        ),
    },
    Function {
        name: "mode",
        receiver: Receiver::Track,
        signature: "pista:mode(modo)",
        returns: Some(Receiver::Track),
        summary: "\"gate\": cada golpe suena lo que dura su paso; \"oneshot\" (por defecto): el corte entero.",
        doc: "En el modo gate la voz se cierra al acabar su paso (o su parte, con \
              retrigger) y hace la cola del corte (`cortes:set(i, { release = ms })`, \
              o el fundido mínimo de 1,5 ms). En one-shot el corte suena hasta su \
              final o hasta el siguiente golpe de la pista.",
        params: &[Param {
            name: "modo",
            description: "`\"gate\"` u `\"oneshot\"`",
        }],
        template: "mode(\"${gate}\")",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen):steps(\"1 1 2 2  3 3 4 4  5 5 6 6  7 7 8 8\"):mode(\"gate\")\n",
        ),
    },
    Function {
        name: "choke",
        receiver: Receiver::Track,
        signature: "pista:choke(grupo)",
        returns: Some(Receiver::Track),
        summary: "Grupo de choke (1 a 16): un golpe de la pista calla a las demás del grupo.",
        doc: "Como en las cajas de ritmos, el charles cerrado calla al abierto: \
              ponlos en el mismo grupo. El corte callado hace el fundido mínimo \
              (1,5 ms). Vale también para los pads del modo tocar. `choke(0)` o \
              `choke()` lo quita.",
        params: &[Param {
            name: "grupo",
            description: "de 1 a 16 (0, ninguno)",
        }],
        template: "choke(${1})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"abierto\", amen:slice_at(3)):steps(\"x . . .  . . x .  x . . .  . . x .\"):choke(1)\n\
             track(\"cerrado\", amen:slice_at(5)):steps(\". . x .  . . . x  . . x .  . . . x\"):choke(1)\n",
        ),
    },
    Function {
        name: "send",
        receiver: Receiver::Track,
        signature: "pista:send(bus, nivel)",
        returns: Some(Receiver::Track),
        summary: "Manda una parte de la pista al bus de delay o de reverb (0 a 1).",
        doc: "El envío va después del volumen, el mute y el solo de la pista: una \
              pista en mute no suena tampoco en el delay (pero lo que ya estaba en \
              la cola sigue). Entra con el patrón, al empezar el compás, suavizado \
              para que no haga clic. Los parámetros de los buses, con `bus()`.",
        params: &[
            Param {
                name: "bus",
                description: "`\"delay\"` o `\"reverb\"`",
            },
            Param {
                name: "nivel",
                description: "cuánto se manda (0 a 1; 0 quita el envío)",
            },
        ],
        template: "send(\"${reverb}\", ${0.3})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen)\n  \
             :steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n  \
             :send(\"delay\", 0.3)\n  \
             :send(\"reverb\", 0.15)\n",
        ),
    },
    Function {
        name: "every",
        receiver: Receiver::Track,
        signature: "pista:every(n, función)",
        returns: Some(Receiver::Track),
        summary: "En los compases 0, n, 2n... llama a función(compás).",
        doc: "Como en Tidal, cuenta los compases de la canción (en una pista con \
              `:length(n)`, las vueltas de su patrón). La función puede \
              cambiar ese compás con los métodos de la tabla de abajo, que se \
              pueden encadenar. Dentro de la función no se puede llamar a `track`, \
              `bpm` ni `sample`.",
        params: &[
            Param {
                name: "n",
                description: "cada cuántos compases (1 a 64)",
            },
            Param {
                name: "función",
                description: "recibe el compás (`function(c) ... end`)",
            },
        ],
        template: "every(${4}, function(c) ${c:reverse()} end)",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen)\n  \
             :steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n  \
             :every(4, function(c) c:reverse() end)\n",
        ),
    },
    Function {
        name: "chance",
        receiver: Receiver::Track,
        signature: "pista:chance(p, efecto [, valor])",
        returns: Some(Receiver::Track),
        summary: "Cada golpe sufre el efecto con probabilidad p.",
        doc: "El azar se tira al cargar, para 16 compases que se repiten. Cada carga \
              tira otra vez los dados. Los efectos están en la tabla de abajo.\n\n\
              `every` y `chance` se aplican en el orden en que se escriben. La \
              pista queda como un ciclo de compases: el mínimo común múltiplo de \
              los `every`, con 16 si hay `chance`, hasta 64 (más da error).",
        params: &[
            Param {
                name: "p",
                description: "probabilidad (0 a 1)",
            },
            Param {
                name: "efecto",
                description: "el efecto (ver la tabla)",
            },
            Param {
                name: "valor",
                description: "el valor del efecto, si lo lleva",
            },
        ],
        template: "chance(${0.2}, \"${retrig}\", ${4})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen)\n  \
             :steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n  \
             :chance(0.2, \"retrig\", 4)\n  \
             :chance(0.1, \"mute\")\n",
        ),
    },
    Function {
        name: "steps",
        receiver: Receiver::Bar,
        signature: "c:steps(texto)",
        returns: Some(Receiver::Bar),
        summary: "Sustituye los pasos del compás.",
        doc: "",
        params: &[Param {
            name: "texto",
            description: "pasos en mini-notación (tantos como la longitud de la pista)",
        }],
        template: "steps(\"${1 . . .  . . . .  . . . .  . . . .}\")",
        example: None,
    },
    Function {
        name: "reverse",
        receiver: Receiver::Bar,
        signature: "c:reverse()",
        returns: Some(Receiver::Bar),
        summary: "Invierte el sentido de cada golpe (lo que iba al revés vuelve al derecho).",
        doc: "",
        params: &[],
        template: "reverse()",
        example: None,
    },
    Function {
        name: "pitch",
        receiver: Receiver::Bar,
        signature: "c:pitch(st)",
        returns: Some(Receiver::Bar),
        summary: "Pone el pitch de cada golpe (-36 a 36).",
        doc: "",
        params: &[Param {
            name: "st",
            description: "semitonos (-36 a 36)",
        }],
        template: "pitch(${7})",
        example: None,
    },
    Function {
        name: "retrig",
        receiver: Receiver::Bar,
        signature: "c:retrig(n)",
        returns: Some(Receiver::Bar),
        summary: "Pone el retrigger de cada golpe (1 a 16).",
        doc: "",
        params: &[Param {
            name: "n",
            description: "repeticiones por paso (1 a 16)",
        }],
        template: "retrig(${4})",
        example: None,
    },
    Function {
        name: "gain",
        receiver: Receiver::Bar,
        signature: "c:gain(g)",
        returns: Some(Receiver::Bar),
        summary: "Pone el volumen de cada golpe (0 a 4); no el de la pista.",
        doc: "",
        params: &[Param {
            name: "g",
            description: "volumen lineal (0 a 4)",
        }],
        template: "gain(${0.5})",
        example: None,
    },
];

const FILTER_PARAMS: &[FxParam] = &[
    FxParam {
        name: "cutoff",
        description: "frecuencia de corte en Hz",
        default: None,
        valid: "20–20000",
        values: &[],
        template: "cutoff = ${3000}",
    },
    FxParam {
        name: "q",
        description: "resonancia",
        default: Some("0.707"),
        valid: "0.1–20",
        values: &[],
        template: "q = ${0.707}",
    },
];

/// Efectos de `:fx`.
pub const EFFECTS: &[Effect] = &[
    Effect {
        name: "filter",
        aliases: &[],
        summary: "Filtro paso bajo o paso alto (biquad de 12 dB/octava).",
        params: &[
            FxParam {
                name: "type",
                description: "tipo de filtro",
                default: Some("\"lowpass\""),
                valid: "`\"lowpass\"` o `\"highpass\"`",
                values: &["lowpass", "highpass"],
                template: "type = \"${lowpass}\"",
            },
            FILTER_PARAMS[0],
            FILTER_PARAMS[1],
        ],
        template: "{ type = \"${lowpass}\", cutoff = ${3000} }",
    },
    Effect {
        name: "lowpass",
        aliases: &[],
        summary: "Filtro paso bajo (como filter con type = \"lowpass\").",
        params: FILTER_PARAMS,
        template: "{ cutoff = ${3000} }",
    },
    Effect {
        name: "highpass",
        aliases: &[],
        summary: "Filtro paso alto (como filter con type = \"highpass\").",
        params: FILTER_PARAMS,
        template: "{ cutoff = ${200} }",
    },
    Effect {
        name: "bitcrush",
        aliases: &["bitcrusher"],
        summary: "Menos bits y menos frecuencia de muestreo: suena sucio.",
        params: &[
            FxParam {
                name: "bits",
                description: "resolución",
                default: Some("8"),
                valid: "1–16",
                values: &[],
                template: "bits = ${8}",
            },
            FxParam {
                name: "downsample",
                description: "repite cada muestra n veces",
                default: Some("1"),
                valid: "1–64",
                values: &[],
                template: "downsample = ${2}",
            },
        ],
        template: "{ bits = ${6}, downsample = ${2} }",
    },
    Effect {
        name: "compressor",
        aliases: &["comp"],
        summary: "Compresor: baja lo que pasa del umbral para pegar la pista (estéreo enlazado).",
        params: &[
            FxParam {
                name: "threshold",
                description: "umbral en dB: por encima empieza a comprimir",
                default: Some("-18"),
                valid: "-60–0",
                values: &[],
                template: "threshold = ${-18}",
            },
            FxParam {
                name: "ratio",
                description: "cuántos dB de entrada por cada dB de salida sobre el umbral",
                default: Some("4"),
                valid: "1–20",
                values: &[],
                template: "ratio = ${4}",
            },
            FxParam {
                name: "attack",
                description: "ms que tarda en comprimir (corto: aplasta los ataques)",
                default: Some("5"),
                valid: "0.1–200",
                values: &[],
                template: "attack = ${5}",
            },
            FxParam {
                name: "release",
                description: "ms que tarda en soltar",
                default: Some("100"),
                valid: "1–2000",
                values: &[],
                template: "release = ${100}",
            },
            FxParam {
                name: "makeup",
                description: "dB que se suben después, para compensar",
                default: Some("0"),
                valid: "0–24",
                values: &[],
                template: "makeup = ${6}",
            },
        ],
        template: "{ threshold = ${-18}, ratio = ${4}, makeup = ${6} }",
    },
    Effect {
        name: "distortion",
        aliases: &["drive"],
        summary: "Distorsión suave (tanh): más drive, más saturada; mix la mezcla con la limpia.",
        params: &[
            FxParam {
                name: "drive",
                description: "ganancia antes de saturar",
                default: Some("4"),
                valid: "1–50",
                values: &[],
                template: "drive = ${4}",
            },
            FxParam {
                name: "mix",
                description: "parte saturada (1 = solo la saturada)",
                default: Some("1"),
                valid: "0–1",
                values: &[],
                template: "mix = ${1}",
            },
        ],
        template: "{ drive = ${4}, mix = ${0.7} }",
    },
];

/// Parámetros de `cortes:set([i,] { ... })`: la forma de un corte.
pub const SLICE_PARAMS: &[FxParam] = &[
    FxParam {
        name: "attack",
        description: "rampa de entrada, en ms (0: el fundido mínimo de 1,5 ms)",
        default: Some("0"),
        valid: "0–2000",
        values: &[],
        template: "attack = ${5}",
    },
    FxParam {
        name: "release",
        description: "cola al final del corte (o al cerrarse la puerta en el modo gate), en ms",
        default: Some("0"),
        valid: "0–5000",
        values: &[],
        template: "release = ${40}",
    },
    FxParam {
        name: "pan",
        description: "panorama: -1 izquierda, 0 centro, 1 derecha (el lado contrario baja)",
        default: Some("0"),
        valid: "-1–1",
        values: &[],
        template: "pan = ${-0.5}",
    },
    FxParam {
        name: "length",
        description: "parte del corte que suena, desde su inicio (0.5: la primera mitad)",
        default: Some("1"),
        valid: "0.01–1",
        values: &[],
        template: "length = ${0.5}",
    },
    FxParam {
        name: "play",
        description: "cómo se lee: una vez, en bucle o de ida y vuelta (los bucles suenan hasta el siguiente golpe, la puerta o el choke)",
        default: Some("\"normal\""),
        valid: "`\"normal\"`, `\"loop\"` o `\"pingpong\"`",
        values: &["normal", "loop", "pingpong"],
        template: "play = \"${loop}\"",
    },
];

/// La tabla de `cortes:set()`, como un efecto (para el autocompletado y la
/// ayuda de sus claves).
pub const SLICE_TABLE: Effect = Effect {
    name: "set",
    aliases: &[],
    summary: "Forma del corte.",
    params: SLICE_PARAMS,
    template: "{ attack = ${5}, release = ${40} }",
};

/// Buses de envío para `bus(nombre, parámetros)` y `pista:send(nombre,
/// nivel)`.
pub const BUSES: &[Effect] = &[
    Effect {
        name: "delay",
        aliases: &[],
        summary: "Delay sincronizado al tempo: repeticiones cada time, cada una feedback veces la anterior.",
        params: &[
            FxParam {
                name: "time",
                description: "tiempo entre repeticiones (negra, corchea con puntillo, corchea, semicorchea)",
                default: Some("\"1/8\""),
                valid: "`\"1/4\"`, `\"3/16\"`, `\"1/8\"` o `\"1/16\"`",
                values: &["1/4", "3/16", "1/8", "1/16"],
                template: "time = \"${1/8}\"",
            },
            FxParam {
                name: "feedback",
                description: "cuánto de cada repetición vuelve a entrar",
                default: Some("0.4"),
                valid: "0–0.95",
                values: &[],
                template: "feedback = ${0.4}",
            },
            FxParam {
                name: "level",
                description: "volumen de lo que devuelve el bus",
                default: Some("0.8"),
                valid: "0–1",
                values: &[],
                template: "level = ${0.8}",
            },
        ],
        template: "{ time = \"${1/8}\", feedback = ${0.4} }",
    },
    Effect {
        name: "reverb",
        aliases: &[],
        summary: "Reverb (Freeverb): una sala para todas las pistas.",
        params: &[
            FxParam {
                name: "size",
                description: "tamaño de la sala (cola más larga)",
                default: Some("0.7"),
                valid: "0–1",
                values: &[],
                template: "size = ${0.7}",
            },
            FxParam {
                name: "damp",
                description: "cuánto se apagan los agudos de la cola",
                default: Some("0.5"),
                valid: "0–1",
                values: &[],
                template: "damp = ${0.5}",
            },
            FxParam {
                name: "level",
                description: "volumen de lo que devuelve el bus",
                default: Some("0.8"),
                valid: "0–1",
                values: &[],
                template: "level = ${0.8}",
            },
        ],
        template: "{ size = ${0.7}, damp = ${0.5} }",
    },
];

/// Bus de envío por nombre.
pub fn bus(name: &str) -> Option<&'static Effect> {
    BUSES.iter().find(|b| b.name == name)
}

/// Efectos de `:chance`.
pub const CHANCE_EFFECTS: &[ChanceEffect] = &[
    ChanceEffect {
        name: "retrig",
        aliases: &["stutter"],
        value: Some("n (1 a 16)"),
        summary: "Retrigger del golpe",
    },
    ChanceEffect {
        name: "reverse",
        aliases: &[],
        value: None,
        summary: "Invierte su sentido",
    },
    ChanceEffect {
        name: "pitch",
        aliases: &[],
        value: Some("semitonos (-36 a 36)"),
        summary: "Pone su pitch",
    },
    ChanceEffect {
        name: "gain",
        aliases: &[],
        value: Some("volumen (0 a 4)"),
        summary: "Pone su volumen",
    },
    ChanceEffect {
        name: "mute",
        aliases: &[],
        value: None,
        summary: "Lo quita",
    },
];

/// Snippets, por nombre.
pub const SNIPPETS: &[Snippet] = &[
    Snippet {
        name: "amen",
        summary: "Tempo, el break cortado y una pista que lo toca en orden.",
        template: "bpm(${170})\n\
                   local amen = sample(\"${amen.wav}\"):slice(${8})\n\
                   \n\
                   track(\"${drums}\", amen)\n  \
                   :steps(\"${1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .}\")\n",
        requires: "",
    },
    Snippet {
        name: "fill",
        summary: "Relleno al final de cada N compases en una pista existente (roll, stutter...).",
        template: "track(\"${drums}\")\n  \
                   :fill(\"${roll}\", ${4})\n",
        requires: "local amen = sample(\"amen.wav\"):slice(8)\n\
                   track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n",
    },
    Snippet {
        name: "ghost",
        summary: "Pista de ghost notes: un golpe muy suave en contratiempos que aparece al azar.",
        template: "track(\"${ghost}\", amen:slice_at(${3}))\n  \
                   :steps(\"${. x@0.2 . x@0.15  . x@0.2 x@0.1 .  . x@0.2 . x@0.15  . . x@0.2 x@0.1}\")\n  \
                   :chance(${0.4}, \"mute\")\n  \
                   :fx(\"lowpass\", { cutoff = ${4000} })\n",
        requires: "local amen = sample(\"amen.wav\"):slice(8)\n",
    },
    Snippet {
        name: "intro",
        summary: "Pista de intro: pocos golpes, filtrada y más baja.",
        template: "track(\"${intro}\", amen)\n  \
                   :steps(\"${1 . . .  . . . .  1 . . .  . . . .}\")\n  \
                   :fx(\"lowpass\", { cutoff = ${800} })\n  \
                   :gain(${0.8})\n",
        requires: "local amen = sample(\"amen.wav\"):slice(8)\n",
    },
    Snippet {
        name: "euclid",
        summary: "Pista con un ritmo euclídeo (k golpes repartidos en n pasos).",
        template: "track(\"${hats}\", amen)\n  \
                   :steps(euclid(${5}, ${16}, \"${2@0.6}\"))\n",
        requires: "local amen = sample(\"amen.wav\"):slice(8)\n",
    },
    Snippet {
        name: "melodia",
        summary: "Melodía con un corte sobre una escala: grados que se convierten en pitch.",
        template: "local escala = scale(\"${minor}\", \"${C}\")\n\
                   track(\"${melodia}\", amen)\n  \
                   :steps(escala:melody(\"${1 . 3 .  5 . 3 .  8 . 7 .  5 . 3 .}\", \"${3}\"))\n  \
                   :gain(${0.6})\n",
        requires: "local amen = sample(\"amen.wav\"):slice(8)\n",
    },
    Snippet {
        name: "polirritmo",
        summary: "Pista de otra longitud (12 pasos contra el compás de 16): polirritmia.",
        template: "track(\"${tres}\", amen)\n  \
                   :length(${12})\n  \
                   :steps(\"${3 . . 3@0.5  . . 3 .  . 3@0.5 . .}\")\n",
        requires: "local amen = sample(\"amen.wav\"):slice(8)\n",
    },
    Snippet {
        name: "barrido",
        summary: "Filtro que se abre poco a poco (ramp) y un envío a la reverb en una pista existente.",
        template: "track(\"${drums}\")\n  \
                   :fx(\"lowpass\", { cutoff = ramp(${200}, ${8000}, \"${4 compases}\") })\n  \
                   :send(\"${reverb}\", ${0.2})\n",
        requires: "local amen = sample(\"amen.wav\"):slice(8)\n\
                   track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n",
    },
    Snippet {
        name: "groove",
        summary: "Swing y humanize en una pista existente, para que no suene a máquina.",
        template: "track(\"${drums}\")\n  \
                   :swing(${58})\n  \
                   :humanize(${6}, ${0.15})\n",
        requires: "local amen = sample(\"amen.wav\"):slice(8)\n\
                   track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n",
    },
];

/// Snippet por nombre.
pub fn snippet(name: &str) -> Option<&'static Snippet> {
    SNIPPETS.iter().find(|s| s.name == name)
}

/// Funciones de un receptor.
pub fn functions(receiver: Receiver) -> impl Iterator<Item = &'static Function> {
    FUNCTIONS.iter().filter(move |f| f.receiver == receiver)
}

/// Método o función por receptor y nombre.
pub fn function(receiver: Receiver, name: &str) -> Option<&'static Function> {
    functions(receiver).find(|f| f.name == name)
}

/// Si el parámetro `param` del efecto `effect` (nombre o alias del
/// registro) admite `ramp()` y `lfo()`: lo dice el propio motor
/// ([`engine::Fx::params`]).
pub fn automatable(effect: &str, param: &str) -> bool {
    let fx = match self::effect(effect).map(|e| e.name) {
        Some("filter" | "lowpass" | "highpass") => engine::Fx::lowpass(1000.0),
        Some("compressor") => engine::Fx::compressor(-18.0, 4.0),
        Some("distortion") => engine::Fx::distortion(4.0, 1.0),
        Some("bitcrush") => engine::Fx::bitcrush(8, 1),
        _ => return false,
    };
    fx.param_index(param).is_some()
}

/// Efecto de `:fx` por nombre o alias.
pub fn effect(name: &str) -> Option<&'static Effect> {
    EFFECTS
        .iter()
        .find(|e| e.name == name || e.aliases.contains(&name))
}

/// Efecto de `:chance` por nombre o alias.
pub fn chance_effect(name: &str) -> Option<&'static ChanceEffect> {
    CHANCE_EFFECTS
        .iter()
        .find(|e| e.name == name || e.aliases.contains(&name))
}

/// Quita las marcas `${` `}` de una plantilla y devuelve el texto y los
/// parámetros marcados como rangos de caracteres (no bytes). Un `${` sin
/// cerrar se queda como texto (los bloques de la biblioteca los escribe el
/// usuario).
pub fn expand_template(template: &str) -> (String, Vec<std::ops::Range<usize>>) {
    let mut text = String::new();
    let mut marks = Vec::new();
    let mut chars = 0;
    let mut rest = template;
    while let Some(open) = rest.find("${") {
        text.push_str(&rest[..open]);
        chars += rest[..open].chars().count();
        let after = &rest[open + 2..];
        let Some(close) = after.find('}') else {
            text.push_str(&rest[open..]);
            return (text, marks);
        };
        let inner = &after[..close];
        let len = inner.chars().count();
        text.push_str(inner);
        marks.push(chars..chars + len);
        chars += len;
        rest = &after[close + 1..];
    }
    text.push_str(rest);
    (text, marks)
}

/// Comillas invertidas de Markdown.
fn code(text: &str) -> String {
    format!("`{text}`")
}

/// Alternativas: `"a"` (o `"b"`).
fn with_aliases(name: &str, aliases: &[&str]) -> String {
    let mut text = code(&format!("\"{name}\""));
    for alias in aliases {
        text.push_str(&format!(" (o {})", code(&format!("\"{alias}\""))));
    }
    text
}

/// Tabla de parámetros de unos efectos (o buses) para la guía.
fn params_table(what: &str, effects: &[Effect]) -> String {
    let mut out = format!(
        "| {what} | Parámetro | Qué es | Por defecto | Valores |\n| --- | --- | --- | --- | --- |\n"
    );
    for e in effects {
        for (i, p) in e.params.iter().enumerate() {
            let name = if i == 0 {
                with_aliases(e.name, e.aliases)
            } else {
                String::new()
            };
            let default = p.default.map_or("obligatorio".into(), code);
            out.push_str(&format!(
                "| {name} | {} | {} | {default} | {} |\n",
                code(p.name),
                p.description,
                p.valid
            ));
        }
    }
    out.push('\n');
    out
}

/// Sección de la API de `docs/GUIA.md` (entre las marcas `api:inicio` y
/// `api:fin`), generada a partir del registro.
pub fn guide_markdown() -> String {
    let mut out = String::new();
    for f in FUNCTIONS.iter().filter(|f| f.receiver != Receiver::Bar) {
        out.push_str(&format!("### {}\n\n", code(f.signature)));
        if let Some(r) = f.returns {
            out.push_str(&format!("Devuelve: {}.\n\n", r.label()));
        }
        out.push_str(f.summary);
        if !f.doc.is_empty() {
            out.push(' ');
            out.push_str(f.doc);
        }
        out.push_str("\n\n");
        if !f.params.is_empty() {
            for p in f.params {
                out.push_str(&format!("- {}: {}\n", code(p.name), p.description));
            }
            out.push('\n');
        }
        match (f.receiver, f.name) {
            (Receiver::Track, "fx") => out.push_str(&params_table("Efecto", EFFECTS)),
            (Receiver::Global, "bus") => out.push_str(&params_table("Bus", BUSES)),
            (Receiver::Slices, "set") => {
                out.push_str(
                    "| Parámetro | Qué es | Por defecto | Valores |\n| --- | --- | --- | --- |\n",
                );
                for p in SLICE_PARAMS {
                    let default = p.default.map_or("obligatorio".into(), code);
                    out.push_str(&format!(
                        "| {} | {} | {default} | {} |\n",
                        code(p.name),
                        p.description,
                        p.valid
                    ));
                }
                out.push('\n');
            }
            (Receiver::Global, "scale") => {
                out.push_str("| Escala | Qué es | Semitonos |\n| --- | --- | --- |\n");
                for s in SCALES {
                    let semis: Vec<String> = s.intervals.iter().map(|i| i.to_string()).collect();
                    out.push_str(&format!(
                        "| {} | {} | {} |\n",
                        code(&format!("\"{}\"", s.name)),
                        s.summary,
                        semis.join(" ")
                    ));
                }
                out.push('\n');
            }
            (Receiver::Track, "every") => {
                out.push_str("| Método | Qué hace |\n| --- | --- |\n");
                for b in functions(Receiver::Bar) {
                    out.push_str(&format!("| {} | {} |\n", code(b.signature), b.summary));
                }
                out.push('\n');
            }
            (Receiver::Track, "fill") => {
                out.push_str("| Efecto | Valor | Qué hace |\n| --- | --- | --- |\n");
                for e in FILL_EFFECTS {
                    out.push_str(&format!(
                        "| {} | {} | {} |\n",
                        code(&format!("\"{}\"", e.name)),
                        e.value.unwrap_or("—"),
                        e.summary
                    ));
                }
                out.push('\n');
            }
            (Receiver::Track, "chance") => {
                out.push_str("| Efecto | Valor | Qué hace |\n| --- | --- | --- |\n");
                for e in CHANCE_EFFECTS {
                    out.push_str(&format!(
                        "| {} | {} | {} |\n",
                        with_aliases(e.name, e.aliases),
                        e.value.unwrap_or("—"),
                        e.summary
                    ));
                }
                out.push('\n');
            }
            _ => {}
        }
        if let Some(example) = f.example {
            out.push_str(&format!("```lua\n{example}```\n\n"));
        }
    }
    out.push_str(
        "### Snippets\n\n\
         Trozos de script con campos para rellenar. Se insertan desde el \
         autocompletado (escribiendo su nombre al inicio de una línea) o desde \
         la paleta de comandos; Tab salta de un campo al siguiente.\n\n\
         | Snippet | Qué inserta |\n| --- | --- |\n",
    );
    for snippet in SNIPPETS {
        out.push_str(&format!(
            "| {} | {} |\n",
            code(snippet.name),
            snippet.summary
        ));
    }
    out.push('\n');
    for snippet in SNIPPETS {
        let (text, _) = expand_template(snippet.template);
        out.push_str(&format!("`{}`", snippet.name));
        if snippet.requires.is_empty() {
            out.push_str(":\n\n");
        } else {
            out.push_str(" (las primeras líneas son lo que necesita tener antes):\n\n");
        }
        out.push_str(&format!("```lua\n{}{text}```\n\n", snippet.requires));
    }
    out.truncate(out.trim_end().len());
    out.push('\n');
    out
}

/// Marcas de la sección generada en la guía.
pub const GUIDE_BEGIN: &str = "<!-- api:inicio (generado desde script/src/api.rs) -->";
pub const GUIDE_END: &str = "<!-- api:fin -->";

/// Sustituye la sección generada de la guía. `None` si faltan las marcas.
pub fn update_guide(guide: &str) -> Option<String> {
    let begin = guide.find(GUIDE_BEGIN)? + GUIDE_BEGIN.len();
    let end = guide[begin..].find(GUIDE_END)? + begin;
    Some(format!(
        "{}\n\n{}\n{}",
        &guide[..begin],
        guide_markdown(),
        &guide[end..]
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_expand_to_text_and_marks() {
        let (text, marks) = expand_template("fx(\"${filter}\", { cutoff = ${3000} })");
        assert_eq!(text, "fx(\"filter\", { cutoff = 3000 })");
        assert_eq!(marks, vec![4..10, 24..28]);
        assert_eq!(expand_template("count()"), ("count()".into(), vec![]));
        // Un campo sin cerrar se queda como texto.
        let (text, marks) = expand_template("a(${1}, \"${b\")");
        assert_eq!(text, "a(1, \"${b\")");
        assert_eq!(marks.first(), Some(&(2..3)));
        assert_eq!(marks.len(), 1);
    }

    #[test]
    fn changing_effect_keeps_the_shared_parameters() {
        let old = table_items("type = \"highpass\", cutoff = 800, q = 2");
        assert_eq!(old[0], ("type".into(), "\"highpass\"".into()));
        // De filter a lowpass: cutoff y q se conservan (q no está en su plantilla).
        assert_eq!(
            effect("lowpass").unwrap().table_with(&old),
            "{ cutoff = ${800}, q = ${2} }"
        );
        // De lowpass a filter: type sale por defecto.
        let old = table_items("cutoff = 800");
        assert_eq!(
            effect("filter").unwrap().table_with(&old),
            "{ type = \"${lowpass}\", cutoff = ${800} }"
        );
        // Sin nada en común: la plantilla del efecto.
        assert_eq!(
            effect("bitcrush").unwrap().table_with(&old),
            "{ bits = ${6}, downsample = ${2} }"
        );
        assert_eq!(
            effect("highpass").unwrap().table_with(&[]),
            "{ cutoff = ${200} }"
        );
        // Comas dentro de cadenas no parten.
        assert_eq!(table_items("a = \"x, y\", b = 1").len(), 2);
        // El valor entre comillas conserva sus comillas al marcarse.
        let old = table_items("type = \"highpass\"");
        assert!(
            effect("filter")
                .unwrap()
                .table_with(&old)
                .starts_with("{ type = \"${highpass}\"")
        );
    }

    #[test]
    fn every_template_starts_with_its_name() {
        for f in FUNCTIONS {
            let (text, _) = expand_template(f.template);
            assert!(text.starts_with(&format!("{}(", f.name)), "{}", f.name);
        }
        for e in EFFECTS {
            for p in e.params {
                assert!(p.template.starts_with(p.name), "{}", p.name);
            }
        }
    }
}
