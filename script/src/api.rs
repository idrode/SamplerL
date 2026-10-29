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
    /// Instrumento: `sample("..."):instrument({ ... })`.
    Instrument,
    /// Grupo de pistas: `group("bateria", { ... })`.
    Group,
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
            Receiver::Instrument => "instrumento",
            Receiver::Group => "grupo",
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
    /// Si admite `ramp()` y `lfo()` en vez de un número, y cómo se mueve.
    /// Es la lista única de lo automatizable: de aquí salen los errores, el
    /// autocompletado de `ramp`/`lfo` y la tabla de la guía (un test
    /// comprueba que el motor lo acepta).
    pub automate: Auto,
}

/// Si un parámetro admite una curva (`ramp()`, `lfo()`) y cómo la sigue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Auto {
    /// Se mueve también dentro de un golpe: el motor evalúa la curva cada
    /// 32 frames y la suaviza (~5 ms), sin clics.
    Smooth,
    /// Cada golpe toma el valor de la curva en el momento en que suena (y
    /// lo mantiene mientras dura).
    PerHit,
    /// Cada grano toma el valor de la curva al nacer.
    PerGrain,
    /// No admite curvas, por esto.
    No(&'static str),
}

impl Auto {
    /// Admite `ramp()` y `lfo()`.
    pub fn curves(self) -> bool {
        !matches!(self, Auto::No(_))
    }

    /// Cómo se mueve, para la guía.
    pub fn text(self) -> &'static str {
        match self {
            Auto::Smooth => "continuo",
            Auto::PerHit => "por golpe",
            Auto::PerGrain => "por grano",
            Auto::No(_) => "no",
        }
    }
}

/// Por qué un parámetro no admite curvas (ver [`Auto::No`]).
const DISCRETE: &str =
    "elige entre valores con nombre (o sí/no): no hay nada entre uno y otro que recorrer";
const TRACK_REF: &str = "es una pista, no un número";
const AT_LOAD: &str = "se aplica al cargar, en el hilo de control (el sample o el bucle se preparan una vez): no suena en el tiempo";
const STRUCTURE: &str =
    "cambia cuántas cosas suenan, no cómo suenan: entra con el patrón al compás";
const SEED: &str = "es la semilla del azar: entre dos semillas no hay nada continuo";
const INSTRUMENT: &str = "cada nota lo lee al nacer y el instrumento se valida entero al cargar; para cambiar la nota, la del paso (`^`)";
const SUB_DIVIDER: &str =
    "es un divisor entero: entre dividir por 2 y por 3 no hay nada que recorrer";
const LAYER: &str = "es una capa del subharm: sus divisores son enteros y su nivel y su pan entran con el patrón al compás";
const ENVELOPE: &str = "la envolvente se fija con el sintetizador al cargar; para mover el volumen en el tiempo, el de la pista (`:gain()` con `ramp()` o `lfo()`)";
const DELAY_TIME: &str = "es una figura (\"1/8\"), no un número; además, mover el retardo de una línea con repeticiones dentro suena a cambio de pitch";

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
    /// Su parámetro `name`.
    pub fn param(&self, name: &str) -> Option<&'static FxParam> {
        self.params.iter().find(|p| p.name == name)
    }

    /// Su parámetro principal: el primero que es un número (el que va tras
    /// `=` en los efectos de un paso, `{lp=800}`).
    pub fn main_param(&self) -> Option<&'static FxParam> {
        self.params
            .iter()
            .find(|p| p.values.is_empty() && p.automate.curves())
    }

    /// Si su parámetro `name` admite `ramp()` y `lfo()` (ver [`Auto`]).
    pub fn automatable(&self, name: &str) -> bool {
        self.param(name).is_some_and(|p| p.automate.curves())
    }

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
        signature: "sample(ruta [, opciones])",
        returns: Some(Receiver::Sample),
        summary: "Carga un WAV (y, con opciones, lo prepara: recorte, normalizado, fundidos).",
        doc: "La ruta es relativa a la carpeta del script o, en un proyecto, a su \
              carpeta `samples`. Recargar el script no vuelve a leer el archivo si \
              no ha cambiado. Las opciones (tabla de abajo) preparan el sample al \
              cargarlo, en este orden: recortar el silencio de los extremos, \
              normalizar el pico y los fundidos; se guarda para la carga siguiente. \
              Los cortes, los ajustes en ms y `stretch` van sobre el sample ya \
              preparado (recortar desplaza los ms).",
        params: &[
            Param {
                name: "ruta",
                description: "archivo WAV (cualquier frecuencia; se resamplea al cargar)",
            },
            Param {
                name: "opciones",
                description: "tabla con las de la tabla de abajo",
            },
        ],
        template: "sample(\"${amen.wav}\")",
        example: Some(
            "local amen = sample(\"amen.wav\")\n\
             -- Sin el silencio de los extremos, con el pico a -1 dB y sin chasquido al final.\n\
             local limpio = sample(\"amen.wav\", { trim = true, normalize = -1, fade_out = 10 })\n",
        ),
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
        name: "bpm_ramp",
        receiver: Receiver::Global,
        signature: "bpm_ramp(desde, hasta, duración)",
        returns: None,
        summary: "Rampa de tempo: de desde a hasta BPM en esos compases, y se queda en hasta.",
        doc: "Manda sobre `bpm()`. Cada compás suena a su tempo (cuantizado al \
              compás: el compás `k` desde que entra, a `desde + (hasta − desde)·k/n`), \
              así que los pasos siguen cayendo al frame exacto; el delay desliza \
              su tiempo para no hacer clic. Entra al empezar el compás, como todo \
              cambio del script, y recargar el mismo script no la vuelve a \
              empezar. Quitarla (con un `bpm()`) la para.",
        params: &[
            Param {
                name: "desde",
                description: "tempo del primer compás (20 a 400)",
            },
            Param {
                name: "hasta",
                description: "tempo al acabar (20 a 400)",
            },
            Param {
                name: "duración",
                description: "`\"N compases\"` (hasta 64)",
            },
        ],
        template: "bpm_ramp(${140}, ${170}, \"${8 compases}\")",
        example: Some(
            "-- De 140 a 174 en 16 compases.\n\
             bpm_ramp(140, 174, \"16 compases\")\n\
             local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n",
        ),
    },
    Function {
        name: "meter",
        receiver: Receiver::Global,
        signature: "meter(n, d)",
        returns: None,
        summary: "Compás n/d (3/4, 7/8...): cuántas semicorcheas tiene cada compás.",
        doc: "Por defecto 4/4 (16 pasos). 3/4 son 12 pasos, 7/8 son 14, 5/4 son 20. \
              Los cambios entran al empezar el compás. Las pistas siguen contando \
              semicorcheas: una de 16 pasos en un 3/4 se desfasa; dale \
              `:length(12)`. El metrónomo marca un pulso cada negra (cada corchea \
              en x/8, cada semicorchea en x/16) con acento en el primero.",
        params: &[
            Param {
                name: "n",
                description: "pulsos del compás",
            },
            Param {
                name: "d",
                description: "figura del pulso: 4, 8 o 16",
            },
        ],
        template: "meter(${3}, ${4})",
        example: Some(
            "meter(3, 4)\n\
             local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"vals\", amen):length(12):steps(\"1 . . .  2 . 3 .  2 . 3 .\")\n",
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
        name: "priority",
        receiver: Receiver::Global,
        signature: "priority(pistas)",
        returns: None,
        summary: "Qué pista manda cuando dos chocan en frecuencia: la primera de la lista, la que más.",
        doc: "Solo la usa el análisis de choques de frecuencia («Choques de \
              frecuencia» en la paleta, `--analyze-masking`): en cada choque, la \
              sugerencia de recorte va a la pista que cede. Una pista de la lista \
              manda sobre las que van detrás y sobre las que no están; entre dos \
              que no están, manda la que suena más fuerte en esa banda (y, si \
              suenan igual, la que se definió antes). Sin llamarla, siempre manda \
              la más fuerte. Un nombre que no es de ninguna pista da un aviso. No \
              cambia cómo suena nada.",
        params: &[Param {
            name: "pistas",
            description: "lista de nombres, de la que más manda a la que menos",
        }],
        template: "priority({ \"${kick}\", \"${bass}\" })",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"kick\", amen:slice_at(1)):steps(\"x . . .  x . . .  x . . .  x . . .\")\n\
             track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n\
             priority({ \"kick\", \"drums\" })\n",
        ),
    },
    Function {
        name: "section",
        receiver: Receiver::Global,
        signature: "section(nombre, función)",
        returns: None,
        summary: "Una sección con nombre (intro, drop, break...): lo que cambian las pistas mientras suena.",
        doc: "La función se llama al acabar el script, sobre una copia de las pistas \
              definidas fuera de las secciones: dentro, `track(\"drums\")` cambia esa \
              pista solo en esta sección (patrón, efectos, `mute`, `solo`, `every`...) \
              y `track(\"nueva\", cortes)` añade una que solo suena aquí. Entre todas \
              las secciones caben 8 pistas. Lo que es de toda la canción (`bpm`, \
              `bpm_ramp`, `meter`, `bus`, `metronome`) va fuera. Dentro de una \
              sección todo cuenta desde su primer compás: las vueltas de `every`, \
              `fill`, `chance`, `humanize` y `:length()`, y las curvas de `ramp` y \
              `lfo`. Las secciones se encadenan con `arrange()`; sin él suena solo la \
              primera, como una canción sin secciones.",
        params: &[
            Param {
                name: "nombre",
                description: "el de la sección (único)",
            },
            Param {
                name: "función",
                description: "`function() ... end` con lo que cambia",
            },
        ],
        template: "section(\"${drop}\", function()\n  ${-- lo que cambia en la sección}\nend)",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n\
             section(\"intro\", function()\n  \
             track(\"drums\"):fx(\"lowpass\", { cutoff = ramp(300, 8000, \"4 compases\") })\n\
             end)\n\
             section(\"drop\", function() end)\n\
             arrange({ {\"intro\", 4}, {\"drop\", 8} })\n",
        ),
    },
    Function {
        name: "arrange",
        receiver: Receiver::Global,
        signature: "arrange({ {sección, compases}, ... })",
        returns: None,
        summary: "La línea de tiempo: las secciones en orden, cada una con sus compases.",
        doc: "El motor toca la canción entera: al empezar cada compás mira en qué \
              sección está y, si cambia, pone sus patrones (siempre al empezar el \
              compás y todo junto). Al acabar vuelve a empezar. Recargar el script \
              conserva la sección y el compás por los que iba; si esa sección ya no \
              está, vuelve al principio con un aviso. Desde el modo tocar y la paleta \
              se puede saltar a una sección, repetir la actual y volver a la línea de \
              tiempo. Exportar sigue la línea de tiempo entera (hasta 256 compases). \
              Una misma sección puede salir varias veces.",
        params: &[Param {
            name: "línea",
            description: "lista de `{\"sección\", compases}` (de 1 a 256 compases cada una)",
        }],
        template: "arrange({ {\"${drop}\", ${8}} })",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n\
             section(\"intro\", function() track(\"drums\"):steps(\"1 . . .  . . . .  1 . . .  . . . .\") end)\n\
             section(\"drop\", function() end)\n\
             section(\"break\", function() track(\"drums\"):mute() end)\n\
             arrange({ {\"intro\", 4}, {\"drop\", 8}, {\"break\", 2}, {\"drop\", 8} })\n",
        ),
    },
    Function {
        name: "variant",
        receiver: Receiver::Global,
        signature: "variant(nombre, letra, función)",
        returns: None,
        summary: "Variante A o B de una sección o de una pista, para alternarlas en vivo.",
        doc: "Con `nombre` de una sección, la función cambia sus pistas como dentro \
              de `section()` (sin argumentos: `track(\"drums\"):mute()`); con el de \
              una pista de fuera de las secciones, la recibe (`function(t) \
              t:steps(\"...\") end`) y el cambio vale en todas las secciones donde \
              suene. La letra es `\"A\"` o `\"B\"`; la que no se define es lo que está \
              escrito. Al cargar se preparan las dos versiones de la canción y suena \
              la A (al recargar, la que sonaba). «Alternar variante A/B» en la paleta \
              o `v` en el modo tocar pasa a la otra: entra entera al empezar el \
              compás siguiente y, con secciones, sigue por la misma sección y el \
              mismo compás. Exportar graba la que suena.",
        params: &[
            Param {
                name: "nombre",
                description: "una sección o una pista de fuera de las secciones",
            },
            Param {
                name: "letra",
                description: "`\"A\"` o `\"B\"`",
            },
            Param {
                name: "función",
                description: "lo que cambia (con una pista, la recibe)",
            },
        ],
        template: "variant(\"${t}\", \"${B}\", function(t)\n  ${t:gain(0.5)}\nend)",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n\
             -- B: otro patrón para la misma pista; v en el modo tocar alterna.\n\
             variant(\"drums\", \"B\", function(t)\n  \
             t:steps(\"1 . . 1  . . 3 .  1 . 5 .  . 3 . 6\")\n\
             end)\n",
        ),
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
        name: "group",
        receiver: Receiver::Global,
        signature: "group(nombre, pistas)",
        returns: Some(Receiver::Group),
        summary: "Junta varias pistas: se suman y pasan juntas por los efectos, el volumen, el pan y los envíos del grupo.",
        doc: "Cada pista del grupo manda su salida (después de su volumen, pan, mute y \
              solo) al grupo en vez de al master; el grupo pasa la suma por su cadena \
              (`grupo:fx`, como la de una pista) y la manda al master con su volumen y \
              su pan. Así un compresor o una saturación reaccionan a todas a la vez, \
              como en una mesa de mezclas. Los envíos de cada pista (`pista:send`) no \
              pasan por el grupo; los del grupo (`grupo:send`) mandan la suma. Cada \
              pista va a un grupo como mucho, y hay 4 grupos; un grupo no va a otro. \
              Es de toda la canción (va fuera de `section()`) y entra al empezar el \
              compás, como los buses. Para cambiar uno ya creado, `group(nombre)` sin \
              la lista. Las pistas pueden definirse después; si alguna no existe, \
              error.",
        params: &[
            Param {
                name: "nombre",
                description: "nombre del grupo",
            },
            Param {
                name: "pistas",
                description: "lista de nombres de pista: `{ \"kick\", \"snare\" }` (sin ella, el grupo ya creado)",
            },
        ],
        template: "group(\"${bateria}\", { \"${kick}\", \"${snare}\" })",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"kick\", amen:slice_at(1)):steps(\"x . . .  x . . .  x . . .  x . . .\")\n\
             track(\"caja\", amen:slice_at(3)):steps(\". . . .  x . . .  . . . .  x . . .\")\n\
             -- El compresor oye las dos a la vez y las pega.\n\
             group(\"bateria\", { \"kick\", \"caja\" })\n  \
             :fx(\"compressor\", { threshold = -20, ratio = 4, makeup = 3 })\n  \
             :gain(0.9)\n  \
             :send(\"reverb\", 0.1)\n",
        ),
    },
    Function {
        name: "fx",
        receiver: Receiver::Group,
        signature: "grupo:fx(nombre, parámetros)",
        returns: Some(Receiver::Group),
        summary: "Añade un efecto a la cadena del grupo (sobre la suma de sus pistas).",
        doc: "Los mismos efectos y parámetros que `pista:fx` (hasta 4, en orden), con \
              `ramp()`, `lfo()` y `macro()` en los que se mueven. El `sidechain` del \
              EQ dinámico puede ser cualquier pista, también una del grupo.",
        params: &[
            Param {
                name: "nombre",
                description: "efecto (los de `pista:fx`)",
            },
            Param {
                name: "parámetros",
                description: "tabla con sus parámetros",
            },
        ],
        template: "fx(\"${compressor}\", { threshold = ${-20}, ratio = ${4} })",
        example: None,
    },
    Function {
        name: "gain",
        receiver: Receiver::Group,
        signature: "grupo:gain(g)",
        returns: Some(Receiver::Group),
        summary: "Volumen del grupo (lineal, 0 a 4), después de sus efectos.",
        doc: "Se suma al de cada pista. Admite `ramp()`, `lfo()` y `macro()`.",
        params: &[Param {
            name: "g",
            description: "volumen (1 = tal cual), `ramp()`, `lfo()` o `macro()`",
        }],
        template: "gain(${0.8})",
        example: None,
    },
    Function {
        name: "pan",
        receiver: Receiver::Group,
        signature: "grupo:pan(p)",
        returns: Some(Receiver::Group),
        summary: "Pan del grupo (-1 izquierda, 1 derecha), después de su volumen.",
        doc: "Admite `ramp()`, `lfo()` y `macro()`.",
        params: &[Param {
            name: "p",
            description: "de -1 a 1, `ramp()`, `lfo()` o `macro()`",
        }],
        template: "pan(${-0.3})",
        example: None,
    },
    Function {
        name: "send",
        receiver: Receiver::Group,
        signature: "grupo:send(bus, nivel)",
        returns: Some(Receiver::Group),
        summary: "Manda la suma del grupo (tras su volumen y su pan) al delay o a la reverb.",
        doc: "Aparte de los envíos de cada pista, que no pasan por el grupo. Admite \
              `ramp()`, `lfo()` y `macro()`.",
        params: &[
            Param {
                name: "bus",
                description: "`\"delay\"` o `\"reverb\"`",
            },
            Param {
                name: "nivel",
                description: "cuánto se manda (0 a 1)",
            },
        ],
        template: "send(\"${reverb}\", ${0.3})",
        example: None,
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
        summary: "Valor que va de desde a hasta en esa duración y vuelve a empezar (en lugar de un número).",
        doc: "Se pone en lugar de un número: en la tabla de `:fx`, de `bus()` o de \
              `:grains`, o en `:gain()`, `:pan()` y `:send()` (lo que lo admite y \
              lo que no, con el porqué, en la tabla de «Qué se puede automatizar», \
              justo después de `lfo`). Es una rampa en diente de \
              sierra alineada con la canción: con `\"4 compases\"` sube en los \
              compases 0-3, otra vez en los 4-7... (como `every`, cuenta desde que \
              arranca el secuenciador, así que recargar no la desplaza). Las \
              frecuencias suben en escala logarítmica (de 200 a 4000 Hz, la mitad \
              del camino es 894 Hz). Entra con el patrón al empezar el compás y se \
              suaviza: nunca hace clic. También vale en la tabla de `:grains` y \
              `:freeze` (y en el `pos` de `freeze`): ahí se mira al nacer cada \
              grano (`size` y `density`, en escala logarítmica).",
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
        summary: "Valor que oscila (seno) entre mín y máx (en lugar de un número).",
        doc: "Como `ramp`, va en lugar de un número (ver la tabla que sigue). La \
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
        name: "macro",
        receiver: Receiver::Global,
        signature: "macro(nombre, desde, hasta)",
        returns: None,
        summary: "Valor que sigue a una macro: un control de 0 a 1 que se mueve en vivo (en lugar de un número).",
        doc: "Como `ramp` y `lfo`, va en lugar de un número, en los mismos sitios (la \
              tabla de «Qué se puede automatizar», justo antes): con la macro a 0 vale \
              `desde`, a 1 `hasta` (las frecuencias, en escala logarítmica). Varios \
              parámetros pueden seguir la misma macro, cada uno con su rango, en una o \
              en varias pistas y grupos: al moverla se mueven todos a la vez, ya, sin \
              esperar al compás y sin clics. Se mueve desde «Macros» en la paleta o \
              con un control MIDI (`[midi] macros` en la configuración). Empieza en 0, \
              o en lo que diga `macro_init`; recargar el script la deja donde se haya \
              puesto. Hay 8 macros como mucho; su número es el orden en que aparecen.",
        params: &[
            Param {
                name: "nombre",
                description: "nombre de la macro",
            },
            Param {
                name: "desde",
                description: "valor con la macro a 0",
            },
            Param {
                name: "hasta",
                description: "valor con la macro a 1",
            },
        ],
        template: "macro(\"${brillo}\", ${300}, ${8000})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             macro_init(\"brillo\", 0.5)\n\
             track(\"drums\", amen)\n  \
             :steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n  \
             -- Una macro abre el filtro y sube la reverb a la vez.\n  \
             :fx(\"lowpass\", { cutoff = macro(\"brillo\", 300, 9000) })\n  \
             :send(\"reverb\", macro(\"brillo\", 0, 0.4))\n",
        ),
    },
    Function {
        name: "macro_init",
        receiver: Receiver::Global,
        signature: "macro_init(nombre, valor)",
        returns: None,
        summary: "Valor con el que empieza una macro (0 a 1).",
        doc: "Lo toma al cargar el script por primera vez, y al exportar. Después la \
              macro se queda donde se mueva en vivo, salvo que se cambie este valor en \
              el script. Va fuera de `section()`.",
        params: &[
            Param {
                name: "nombre",
                description: "nombre de la macro",
            },
            Param {
                name: "valor",
                description: "de 0 a 1",
            },
        ],
        template: "macro_init(\"${brillo}\", ${0.5})",
        example: None,
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
              proyecto, el número de cortes se escribe `rejilla16`). Admite las \
              mismas opciones que `slice` (`zero_cross`); en rejilla es donde más se \
              nota, porque las divisiones caen en cualquier punto de la onda.",
        params: &[
            Param {
                name: "n",
                description: "partes (1 a 64)",
            },
            Param {
                name: "opciones",
                description: "tabla con `zero_cross` (la de `slice`)",
            },
        ],
        template: "slice_grid(${16})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice_grid(16)\n\
             track(\"drums\", amen):steps(\"1 2 3 4  5 6 7 8  9 10 11 12  13 14 15 16\")\n\
             -- La misma rejilla, con cada inicio en el cruce por cero más cercano\n\
             -- (a no más de 3 ms): sin clics al empezar cada parte.\n\
             local limpio = sample(\"amen.wav\"):slice_grid(16, { zero_cross = 3 })\n",
        ),
    },
    Function {
        name: "instrument",
        receiver: Receiver::Sample,
        signature: "sample:instrument([opciones])",
        returns: Some(Receiver::Instrument),
        summary: "Toca el sample como un instrumento: notas con polifonía, ADSR y bucle sostenido.",
        doc: "Una pista con un instrumento (`track(nombre, instrumento)`) toca notas en \
              vez de cortes: cada golpe abre una voz sin cortar las que suenan (si no \
              queda ninguna libre, se roba la más antigua), sube y baja con su \
              envolvente ADSR y dura su paso más sus `_`, y luego el release. El sample \
              suena tal cual en la nota `root`: `pista:melody(\"c e g e\")` toca \
              cromáticamente desde ahí, y en `steps` el pitch (`^`) son semitonos \
              sobre la raíz (varios `^` hacen un acorde: `1^0^4^7`). Con `loop`, la \
              nota sigue en bucle mientras dure (también en su release), con un \
              fundido cruzado en el salto para que no haga clic: los últimos ms antes \
              del fin del bucle se funden con los de antes de su inicio (si no hay \
              tanto audio antes del inicio, el fundido se acorta y se avisa). Sin \
              `loop`, la nota acaba con el sample. Al bajar mucho el tono el sample \
              dura más (y al subir, menos): el pitch cambia la velocidad. Con \
              `keep = true` no: cada nota conserva la duración del sample (granos \
              alineados, como `%` en los pasos; bien hasta ±12 semitonos, más allá \
              se avisa).",
        params: &[Param {
            name: "opciones",
            description: "tabla con `root`, `loop`, `voices`, `adsr` y `crossfade` (ver abajo)",
        }],
        template: "instrument({ root = \"${C3}\", voices = ${4} })",
        example: Some(
            "bpm(100)\n\
             -- El sample suena tal cual en C3; el bucle va de 200 a 600 ms.\n\
             local piano = sample(\"amen.wav\"):instrument({\n  \
             root = \"C3\", loop = { 200, 600 }, voices = 4, adsr = { 5, 80, 0.7, 300 },\n\
             })\n\
             track(\"lead\", piano):melody(\"c _ e _  g _ e _  c+e+g _ _ _  . . b2 .\")\n",
        ),
    },
    Function {
        name: "synth",
        receiver: Receiver::Global,
        signature: "synth(tipo, [opciones])",
        returns: Some(Receiver::Instrument),
        summary: "Un instrumento sintetizado, sin sample: osciladores, filtro y envolventes.",
        doc: "Se toca como un instrumento de sample (`track(nombre, synth(...))`, \
              `:melody()`, `scale`, los pads del modo tocar y el MIDI de entrada): \
              el pitch de cada paso son semitonos sobre su `root`. Los parámetros \
              continuos admiten `ramp()` y `lfo()` (se mueven también dentro de una \
              nota, sin clics) y los efectos de la pista van detrás. En sus pasos, \
              dos sufijos más: `!` accent (la nota suena más fuerte y más abierta) \
              y `/` slide (la nota llega ligada desde la anterior: se desliza hasta \
              su altura en `glide` ms sin volver a disparar las envolventes; la \
              anterior sigue sonando hasta que llega). En `melody` van detrás de la \
              nota: `\"c!\"`, `\"eb/\"`, `\"g!/\"`. Hay dos tipos, los dos \
              monofónicos: `\"acid\"`, el bajo del 303 (un oscilador de sierra o \
              cuadrada, paso bajo de 4 polos con mucha resonancia; en él, el accent \
              también abre el filtro), y `\"sub\"` (o `\"subbass\"`), un subgrave \
              limpio para dar profundidad (un seno por un paso bajo suave que \
              empieza cada nota en el cruce por cero y sigue su fase de una nota a \
              la siguiente: sin golpes al arrancar ni clics entre notas; en él, el \
              accent es un 40 % más fuerte). Y `\"subharm\"`, dos osciladores con dos \
              subs cada uno que dividen la frecuencia y el compás (ver `subharm`), \
              y `\"sh101\"`, un monosynth de bajo y lead (ver `sh101`).",
        params: &[
            Param {
                name: "tipo",
                description: "`\"acid\"`, `\"sub\"`, `\"subharm\"` o `\"sh101\"`",
            },
            Param {
                name: "opciones",
                description: "tabla con sus parámetros (ver abajo)",
            },
        ],
        template: "synth(\"${acid}\", { cutoff = ${400}, res = ${0.7} })",
        example: Some(
            "bpm(130)\n\
             local bass = synth(\"acid\", { cutoff = 350, res = 0.8, envmod = 0.7, decay = 250 })\n\
             track(\"acid\", bass)\n  \
             :melody(\"c c! eb c/  . c bb2 c/  g2! . c c/  eb! f/ eb c\")\n\
             -- Debajo, un subgrave en C1 que sigue la tónica.\n\
             track(\"sub\", synth(\"sub\", { tone = 200, release = 120 }))\n  \
             :melody(\"c _ _ _  _ _ _ _  ab0 _ _ _  bb0 _ g0/ _\")\n",
        ),
    },
    Function {
        name: "subharm",
        receiver: Receiver::Global,
        signature: "subharm{ opciones }",
        returns: Some(Receiver::Instrument),
        summary: "Inspirado en el Subharmonicon: seis capas que dividen la frecuencia (f/n) y el compás (1/n).",
        doc: "Lo mismo que `synth(\"subharm\", { ... })`. Dos osciladores de sierra \
              (`o1`, `o2`), cada uno a la nota dividida por su `div`, y dos subs de \
              cada uno (`s1`, `s2` de `o1`; `s3`, `s4` de `o2`), a la frecuencia de su \
              oscilador dividida por el suyo y sacados de su fase (siempre en fase). \
              Cada capa tiene su `pan` (de potencia constante) y su `level`, y \
              vuelve a sonar (su envolvente, con `attack` y `decay`) cada 1/`rdiv` \
              del compás: todas siguen el mismo reloj, así que los ritmos (3 contra \
              4, 5 contra 7...) vuelven a coincidir al empezar cada compás; con el \
              transporte parado (pads, MIDI), el compás empieza con la nota. Todo \
              pasa por un paso bajo de \
              4 polos. Se toca como cualquier instrumento (`melody`, pads, MIDI, `!` \
              y `/`); sus capas cambian con el patrón, al compás.",
        params: &[Param {
            name: "opciones",
            description: "tabla con las capas (`o1`...`s4`, cada una `{ div, rdiv, pan, level }`) y el resto de parámetros (ver abajo)",
        }],
        template: "subharm{ o1 = { div = ${1}, rdiv = ${4}, pan = ${-0.6} }, s1 = { div = ${3}, rdiv = ${3}, pan = ${0.5} } }",
        example: Some(
            "bpm(110)\n\
             local sh = subharm{\n  \
             o1 = { div = 1, rdiv = 4, pan = -0.6 },\n  \
             s1 = { div = 3, rdiv = 3, pan = 0.5, level = 0.7 },\n  \
             cutoff = 900, res = 0.4,\n\
             }\n\
             track(\"sh\", sh):melody(\"c _ _ _  _ _ _ _  eb _ _ _  g _ _ _\")\n",
        ),
    },
    Function {
        name: "sh101",
        receiver: Receiver::Global,
        signature: "sh101{ opciones }",
        returns: Some(Receiver::Instrument),
        summary: "Monosynth de bajo y lead como el SH-101: oscilador de sierra y pulso, sub, ruido, ladder, una envolvente, LFO y portamento.",
        doc: "Lo mismo que `synth(\"sh101\", { ... })`. Un oscilador con la sierra \
              (`saw`) y el pulso (`pulse`, de ancho `pw`, que mueve el LFO o la \
              envolvente con `pwm_src` y `pwm_amt`) de la misma fase, el bloque sub \
              (los tres del original: `sub_div = 2` y `4` con `sub_wave = \"square\"`, \
              y `sub_div = 4` con `\"pulse25\"`) y ruido, por un paso bajo de 4 \
              polos con saturación en cada etapa que autooscila afinado cerca de \
              `res = 1` (`quality = 2`, por defecto, lo pasa al doble: pliega \
              menos), y un paso alto fijo (`hpf`). Una sola envolvente (`a`, `d`, \
              `s`, `r`, en ms) mueve el filtro (`env_amt`, negativo la invierte) y \
              el VCA (o `vca = \"gate\"`). Mono con prioridad a la última nota: \
              una tecla pulsada con otra aún abajo, o un paso con `/`, se liga (no \
              vuelve a disparar la envolvente, solo cambia la altura); al soltarla, \
              vuelve a la que siga pulsada. `glide` desliza según `glide_mode`. El \
              LFO (`lfo_wave`, `lfo_rate` o `lfo_sync` al compás) va a la altura, \
              al corte y al ancho de pulso. Por paso, `!`, `/` y el sub de la nota \
              (`{sub=...}`).",
        params: &[Param {
            name: "opciones",
            description: "tabla con sus parámetros (ver abajo)",
        }],
        template: "sh101{ saw = ${0.8}, pulse = ${0.3}, sub = ${0.6}, cutoff = ${600}, res = ${0.7} }",
        example: Some(
            "bpm(124)\n\
             local bass = sh101{\n  \
             saw = 0.8, pulse = 0.3, pw = 0.4, sub = 0.6, sub_div = 2,\n  \
             cutoff = 600, res = 0.7, env_amt = 0.5,\n  \
             a = 2, d = 250, s = 0, r = 100,\n  \
             glide = 80, glide_mode = \"auto\",\n\
             }\n\
             track(\"bass\", bass):melody(\"c . c c/  eb . c . g1 . bb1 c/  . c eb f\")\n",
        ),
    },
    Function {
        name: "slice",
        receiver: Receiver::Sample,
        signature: "sample:slice(n [, opciones])",
        returns: Some(Receiver::Slices),
        summary: "Corta el sample en sus n golpes más fuertes.",
        doc: "Si hay menos golpes, devuelve menos cortes. Usa el detector por flujo \
              espectral con 40 ms de separación mínima. Con `zero_cross` (tabla de abajo), \
              cada inicio de corte se lleva al cruce por cero más cercano, a no más de esos \
              ms, para que el golpe no empiece en un salto (un clic); vale también para los \
              cortes de `add`, `move` y los ajustes del proyecto. Sin la opción, los cortes \
              son exactamente los de siempre.",
        params: &[
            Param {
                name: "n",
                description: "cortes a buscar (1 a 64)",
            },
            Param {
                name: "opciones",
                description: "tabla con `zero_cross` (ver abajo)",
            },
        ],
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
        name: "note",
        receiver: Receiver::Slices,
        signature: "cortes:note(i)",
        returns: None,
        summary: "Nota del corte i (desde 1) en cifrado americano, como \"C#4\".",
        doc: "Devuelve dos valores, la nota más cercana (`\"C#4\"`, con sostenidos y \
              C4 = 60, como en `melody`) y cuántos cents está por encima o por debajo \
              (de −50 a 50), o `nil` si el corte no tiene un tono claro (un golpe de \
              caja, ruido, silencio). Busca el periodo de 41 Hz (E1) a 2,1 kHz (C7) \
              con YIN en ventanas a lo largo del corte, sin el ataque; si el corte \
              tiene dos notas, da la que más suena. La vista del sample escribe la \
              nota en cada marca y «Notas de los cortes» de la paleta las lista.",
        params: &[Param {
            name: "i",
            description: "número del corte (desde 1)",
        }],
        template: "note(${1})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             -- El bajo, en la nota del bombo (o en C1 si no tiene).\n\
             local nota = amen:note(1) or \"C1\"\n\
             track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\")\n\
             track(\"sub\", synth(\"sub\", {}))\n  \
             :melody(nota .. \" _ _ _  _ _ _ _  . . . .  . . . .\")\n",
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
              estaban. Los números admiten `ramp()` y `lfo()`: cada golpe toma el \
              valor de la curva al sonar (la voz los lee al nacer; en los pads, el \
              primer extremo).",
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
        name: "fx",
        receiver: Receiver::Slices,
        signature: "cortes:fx(cuál, nombre, parámetros)",
        returns: Some(Receiver::Slices),
        summary: "Efecto que solo suena en un corte (o en varios): por número, etiqueta o lista.",
        doc: "Los efectos son los de `pista:fx` (la misma tabla, con `ramp()` y `lfo()`), \
              hasta 4 por corte, en orden. Cada golpe de ese corte pasa por ellos con su \
              propio estado (el filtro de un golpe no arrastra el anterior) y luego por los \
              de la pista: voz → efectos del corte → efectos de la pista → volumen de la \
              pista → mezcla. Los demás cortes suenan exactamente igual que sin ellos. \
              Vale para todas las pistas que usan esta variable, también las definidas \
              antes (como `:set`), y para sus pads. `cuál` es el número del corte (desde \
              1), su etiqueta (ponla antes con `:name`) o una lista: `{\"snare\", 7}`. \
              Una etiqueta o un corte que no existe, o un parámetro fuera de rango, da \
              error con la línea y sigue sonando lo anterior. El EQ dinámico de un corte \
              no admite `sidechain`. Con `:grains()` no suenan (los granos no son voces: \
              se avisa) y un instrumento no tiene cortes que elegir (usa `pista:fx`). \
              Con `~` en `steps` se sigue moviendo el primer filtro de la pista, no el del \
              corte. Los envíos (`send`) son de la pista entera.",
        params: &[
            Param {
                name: "cuál",
                description: "número de corte (desde 1), etiqueta o lista de ellos",
            },
            Param {
                name: "nombre",
                description: "el efecto (los de pista:fx)",
            },
            Param {
                name: "parámetros",
                description: "tabla con los parámetros del efecto",
            },
        ],
        template: "fx(${3}, \"${bitcrush}\", { bits = ${6} })",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             amen:name(3, \"snare\"):name(1, \"kick\")\n\
             -- Solo la caja, sucia y con un filtro que se abre cada dos compases.\n\
             amen:fx(\"snare\", \"bitcrush\", { bits = 6 })\n\
             amen:fx(\"snare\", \"lowpass\", { cutoff = ramp(800, 8000, \"2 compases\") })\n\
             -- El bombo y el corte 7, más bajos.\n\
             amen:fx({ \"kick\", 7 }, \"gain\", { level = 0.7 })\n\
             track(\"drums\", amen):steps(\"kick . snare .  3 . snare 7  kick kick snare .  7 . snare 3\")\n",
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
              existen dan error en esa línea. Un paso puede llevar efectos solo para \
              su golpe entre llaves: `3{lp=800,q=4}{crush=6}` (los de `:fx`, con \
              rampas del golpe: `{lp=200>4000:4}`).",
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
        name: "melody",
        receiver: Receiver::Track,
        signature: "pista:melody(notas)",
        returns: Some(Receiver::Track),
        summary: "Patrón de notas por su nombre (\"c e g e\"), cromático respecto a la raíz del instrumento.",
        doc: "Como `steps`, con notas en vez de cortes: `c`, `e`, `f#`, `bb3`... (la \
              octava es opcional; sin ella, la de la raíz; C4 es el do central), \
              acordes con `+` (`c+e+g`: las notas sin octava van justo por encima de \
              la anterior), `.` silencio y `_` para alargar la nota de antes un paso. \
              Cada nota admite los sufijos de los pasos salvo `^` (`e@0.5`, `gr2`). \
              En una pista con instrumento la raíz es su `root`; en una de cortes, la \
              nota C4 es el corte 1 tal cual (como en `scale`), y los acordes no valen \
              (no hay polifonía).",
        params: &[Param {
            name: "notas",
            description: "notas, acordes (`c+e+g`), `.` o `_`: tantas como pasos tiene la pista",
        }],
        template: "melody(\"${c . e .  g . e .  c . e .  g . c4 .}\")",
        example: Some(
            "local piano = sample(\"amen.wav\"):instrument({ root = \"C3\", adsr = { 5, 0, 1, 200 } })\n\
             track(\"bajo\", piano)\n  \
             :melody(\"c2 _ . c2  eb2 _ . f2  g2 _ _ _  bb1 _ c2 .\")\n  \
             :gain(0.8)\n",
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
        doc: "Se aplica después de los efectos. Admite `ramp()` y `lfo()`: el \
              volumen se mueve suavizado, también dentro de un golpe.",
        params: &[Param {
            name: "g",
            description: "volumen lineal (0 a 4; 1 lo deja igual), `ramp()` o `lfo()`",
        }],
        template: "gain(${0.8})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"drums\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\"):gain(0.7)\n\
             track(\"sube\", amen):steps(\"x . x .  x . x .  x . x .  x . x .\"):gain(ramp(0, 1, \"8 compases\"))\n",
        ),
    },
    Function {
        name: "pan",
        receiver: Receiver::Track,
        signature: "pista:pan(p)",
        returns: Some(Receiver::Track),
        summary: "Pan de la pista: -1 izquierda, 0 centro, 1 derecha.",
        doc: "Va después del volumen y antes de los envíos, con la ley de balance \
              de los cortes (el lado contrario baja, el propio no sube). Admite \
              `ramp()` y `lfo()`. El pan de cada corte, con `cortes:set(i, { pan = ... })`.",
        params: &[Param {
            name: "p",
            description: "de -1 a 1, `ramp()` o `lfo()`",
        }],
        template: "pan(${-0.3})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             track(\"hats\", amen):steps(\"3 3 3 3  3 3 3 3  3 3 3 3  3 3 3 3\"):pan(lfo(\"2 compases\", -0.6, 0.6))\n",
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
              propósito). El EQ tiene hasta 4 bandas en serie, cada una con su \
              `freqN`, `gainN`, `qN` y `typeN` (`\"bell\"`, campana; \
              `\"lowshelf\"` y `\"highshelf\"`, estantes que mueven todo lo que \
              queda por debajo o por encima); una banda con `gainN = 0` no hace nada, \
              y un EQ sin ganancias suena idéntico a no tenerlo. Al recargar con \
              otras ganancias, el EQ va hacia ellas en unos 5 ms, sin clic. El EQ \
              dinámico (`dyneq`) tiene una banda que escucha (un paso de banda en \
              `freq` y `q`) y baja solo mientras lo que oye pasa de `threshold`: \
              `1 − 1/ratio` dB por dB de más, como mucho `range`, con `attack` y \
              `release`. Sin `sidechain` escucha la propia pista; con \
              `sidechain = \"kick\"`, la salida de esa pista (tras sus efectos, \
              antes de su volumen, aunque esté en mute o fuera del solo). Por \
              debajo del umbral suena idéntico a no tenerlo. Todos los parámetros \
              numéricos admiten `ramp()` y `lfo()` en lugar de un número (los \
              tipos y el `sidechain`, no: ver «Qué se puede automatizar», tras \
              `lfo`); el corte del primer filtro, además, puede cambiar en cada \
              golpe con `~` en `steps` (`3~800`).",
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
             :fx(\"bitcrush\", { bits = 6, downsample = 2 })\n\
             track(\"drums\")\n  \
             :fx(\"eq\", { freq1 = 300, gain1 = -3, q1 = 2, \
             type2 = \"highshelf\", freq2 = 8000, gain2 = 2 })\n",
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
        name: "midi",
        receiver: Receiver::Track,
        signature: "pista:midi(canal [, nota_base])",
        returns: Some(Receiver::Track),
        summary: "Cada golpe sale también como una nota MIDI (corte 1 = nota_base, 36 por defecto).",
        doc: "Con `--allow-midi`, las notas salen por el puerto virtual «sampler \
              salida» (ALSA): conéctalo a Surge XT u otro synth (ver «MIDI» en la \
              sección 2). La nota es `nota_base` más el número de corte menos 1, la \
              velocidad sale del volumen del paso (`@`) y cada nota dura lo que su \
              paso (o su parte, con retrigger). Una pista en mute, o fuera del \
              solo, no manda nada. El sample sigue sonando: para oír solo el synth, \
              `:gain(0)`. Los pads del modo tocar no mandan notas. En una pista con \
              instrumento va sin `nota_base`: cada nota (también las de un acorde) \
              sale a su altura, la raíz (`root`) más sus semitonos, y dura su paso y \
              sus `_` (el release es solo del sample); una nota robada por falta de \
              voces se apaga en ese momento.",
        params: &[
            Param {
                name: "canal",
                description: "canal MIDI, de 1 a 16",
            },
            Param {
                name: "nota_base",
                description: "la nota del corte 1 (0 a 127; 36 por defecto); en un instrumento, no va",
            },
        ],
        template: "midi(${1}, ${48})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             -- Una línea de bajo para el synth: el corte 1 es do (48), el 3, re.\n\
             track(\"bajo\", amen)\n  \
             :steps(\"1 . . 1  . . 3 .  1 . . .  5 . 3 .\")\n  \
             :midi(2, 48)\n  \
             -- Bajo, para que mande el synth (con 0, solo el synth).\n  \
             :gain(0.3)\n",
        ),
    },
    Function {
        name: "mpe",
        receiver: Receiver::Track,
        signature: "pista:mpe{ eje = destino, ... }",
        returns: Some(Receiver::Track),
        summary: "Expresión por nota de un sintetizador: el bend, la presión y el slide de cada nota MIDI en vivo mueven su altura, su volumen o un parámetro.",
        doc: "Para tocar con un controlador expresivo (MPE, como un Roli Seaboard; \
              ver «MIDI» en la sección 2) o con un teclado con aftertouch. Tres ejes: \
              `bend` (el pitch bend de la nota, en semitonos: el rango lo dice `bend` \
              en `[midi]`), `pressure` (la presión, de 0 a 1: aftertouch del canal o \
              por nota) y `slide` (el CC74, deslizar en vertical, de 0 a 1). Cada uno \
              va a `\"pitch\"` (semitonos), `\"gain\"` (el volumen de la nota: con \
              `range = 1`, sin presión calla; con 0,5, suena a la mitad) o a un \
              parámetro del sintetizador que la voz lea muestra a muestra (el corte, \
              la resonancia, la saturación, el ancho de pulso...; no el glide, ni los \
              decays, ni las voces). Como texto (`pressure = \"cutoff\"`) o como \
              tabla con `range` (lo que mueve el eje entero: semitonos, octavas en el \
              corte, unidades del parámetro en los demás) y `curve` (exponente del \
              eje: 2 responde poco al principio y mucho al final). En el bend, el eje \
              entero es una octava de bend (12 semitonos): `bend = \"pitch\"` sigue \
              al dedo tal cual. Por defecto `range` es 12 en el bend al pitch, 2 en \
              los demás al pitch, 1 en el volumen, 2 octavas en el corte y medio \
              rango en los demás parámetros; `curve`, 1. Cada nota mueve solo su voz \
              (en un sintetizador polifónico, como el `subharm` con `voices`, cada \
              dedo la suya; en uno mono, la nota que suena), suavizado unos 8 ms. Las \
              notas del patrón y los pads del teclado no tienen expresión: suenan \
              como sin `:mpe`. Sin tabla (`:mpe()`), ninguno.",
        params: &[Param {
            name: "ejes",
            description: "tabla con `bend`, `pressure` y `slide`: un destino o `{ destino, range = n, curve = n }`",
        }],
        template: "mpe{ bend = \"${pitch}\", pressure = \"${cutoff}\" }",
        example: Some(
            "-- Un subharm de 6 voces para el Seaboard: cada dedo dobla su nota,\n\
             -- la presión abre su filtro y el slide le sube la resonancia.\n\
             track(\"tecla\", subharm{ voices = 6, cutoff = 300 })\n  \
             :mpe{\n    \
             bend = \"pitch\",\n    \
             pressure = { \"cutoff\", range = 3, curve = 1.5 },\n    \
             slide = { \"res\", range = 0.4 },\n  \
             }\n  \
             -- Las notas del patrón suenan sin expresión.\n  \
             :melody(\"c . . .  . . . .  eb . . .  . . . .\")\n",
        ),
    },
    Function {
        name: "grains",
        receiver: Receiver::Track,
        signature: "pista:grains([parámetros])",
        returns: Some(Receiver::Track),
        summary: "Toca el sample en granos: cada golpe abre una nube sobre su corte mientras dura su paso.",
        doc: "Cada golpe de la pista, en vez de tocar su corte, hace nacer granos de \
              él durante su paso (y sus `_`): trozos de `size` ms con ventana de \
              Hann, `density` por segundo, leídos desde `pos` (fracción del corte) \
              más lo que avanza la lectura (`speed`: 1 recorre el corte a su \
              velocidad, 0 lo congela) más hasta `jitter` ms al azar, con el pitch \
              del paso más `pitch` más hasta `spread` semitonos al azar. Al acabar el \
              paso no nacen más y los que suenan acaban su ventana. Todos los \
              números admiten `ramp()` y `lfo()` (se miran al nacer cada grano). El \
              azar sale de `seed` y de la posición de la canción: la misma canción \
              suena igual en cada render; sin `seed`, una por pista sacada de su \
              nombre. Hasta 32 granos a la vez por pista (`density × size`). Una \
              pista con granos no puede ser instrumento. En el modo tocar (y con el MIDI \
              de entrada) la tecla abre una nube mientras se mantiene: con varios \
              cortes elige el corte de donde salen los granos, y con uno solo (lo \
              normal en una textura) es el pitch, un semitono por tecla; el resto \
              son los parámetros de la pista.",
        params: &[Param {
            name: "parámetros",
            description: "tabla con `size`, `density`, `pos`, `speed`, `jitter`, `pitch`, `spread` y `seed` (ver abajo)",
        }],
        template: "grains({ size = ${80}, density = ${20} })",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             -- Una nube larga sobre la caja, cada vez más densa.\n\
             track(\"nube\", amen)\n  \
             :steps(\"3 _ _ _  _ _ _ _  _ _ _ _  _ _ _ _\")\n  \
             :grains({ size = 60, density = ramp(5, 60, \"1 compás\"), jitter = 20, spread = 3 })\n",
        ),
    },
    Function {
        name: "freeze",
        receiver: Receiver::Track,
        signature: "pista:freeze([pos [, parámetros]])",
        returns: Some(Receiver::Track),
        summary: "Congela un instante del corte y lo sostiene en granos (`grains` con `speed = 0`).",
        doc: "Como `grains`, pero la lectura no avanza: todos los granos salen de \
              `pos` (fracción del corte, 0 por defecto; admite `ramp()` y `lfo()` \
              para pasearse por él), más su `jitter`. Con granos largos y \
              solapados, un instante se convierte en un sonido sostenido mientras \
              dure el paso y sus `_`.",
        params: &[
            Param {
                name: "pos",
                description: "dónde se congela, en fracción del corte (0 a 1)",
            },
            Param {
                name: "parámetros",
                description: "la tabla de `grains`, sin `speed`",
            },
        ],
        template: "freeze(${0.5})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             -- La cola de la caja, sostenida dos compases y una octava abajo.\n\
             track(\"hielo\", amen)\n  \
             :length(32)\n  \
             :steps(\"3 _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _  _ _ _ _\")\n  \
             :freeze(0.4, { size = 120, density = 25, jitter = 15, pitch = -12 })\n",
        ),
    },
    Function {
        name: "pitch",
        receiver: Receiver::Track,
        signature: "pista:pitch(semitonos)",
        returns: Some(Receiver::Track),
        summary: "Transpone todos los golpes de la pista (se suma al pitch de cada paso).",
        doc: "Como `^` en cada paso: cambia la velocidad de lectura (y la duración); \
              con `%` en el paso, sin cambiar la duración. Admite `ramp()` y \
              `lfo()`: cada golpe toma el valor de la curva en el momento en que \
              suena y lo mantiene mientras dura (una voz no cambia de velocidad a \
              media lectura). El total, con el del paso, como mucho ±36 semitonos. \
              Vale también en los instrumentos y en los granos.",
        params: &[Param {
            name: "semitonos",
            description: "-36 a 36, `ramp()` o `lfo()`",
        }],
        template: "pitch(${-12})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             -- Cada golpe, un poco más agudo que el anterior durante 4 compases.\n\
             track(\"sube\", amen):steps(\"1 . 2 .  3 . 4 .  5 6 2 .  7 . 8 .\"):pitch(ramp(0, 12, \"4 compases\"))\n",
        ),
    },
    Function {
        name: "formant",
        receiver: Receiver::Track,
        signature: "pista:formant(semitonos)",
        returns: Some(Receiver::Track),
        summary: "Formantes de los pasos con % (pitch sin cambiar la duración): 0 los deja donde estaban, n los desplaza n semitonos.",
        doc: "Al transponer, la envolvente del sonido sube o baja con el tono: una \
              voz subida suena a ardilla y una caja bajada, a bombo de juguete. Con \
              `formant(0)`, los pasos con `%` cambian de tono pero su envolvente se \
              queda donde estaba (cada grano se analiza con LPC y se blanquea y se \
              vuelve a colorear con la del original); con `formant(n)` se desplaza \
              `n` semitonos por su cuenta, también con `%0` (cambiar el timbre sin \
              cambiar la nota). Solo actúa en los pasos con `%` (si no hay \
              ninguno, avisa); con `^` el sonido se lee más deprisa o más despacio \
              entero, como un sampler clásico. Cuesta otro 2 % de un núcleo por \
              voz. En un instrumento va en su tabla: `sample:instrument({ keep = \
              true, formant = 0 })`. Admite `ramp()` y `lfo()`, por golpe.",
        params: &[Param {
            name: "semitonos",
            description: "-24 a 24 (0 deja los formantes donde estaban), `ramp()` o `lfo()`",
        }],
        template: "formant(${0})",
        example: Some(
            "local amen = sample(\"amen.wav\"):slice(8)\n\
             -- La caja una quinta arriba sin que encoja su cuerpo.\n\
             track(\"drums\", amen)\n  \
             :steps(\"1 . 3 .  . . 3%7 .  1 1 3%-5 .  . . 3%7 .\")\n  \
             :formant(0)\n",
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
              para que no haga clic. El nivel admite `ramp()` y `lfo()`. Los \
              parámetros de los buses, con `bus()`.",
        params: &[
            Param {
                name: "bus",
                description: "`\"delay\"` o `\"reverb\"`",
            },
            Param {
                name: "nivel",
                description: "cuánto se manda (0 a 1; 0 quita el envío), `ramp()` o `lfo()`",
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
        name: "melody",
        receiver: Receiver::Bar,
        signature: "c:melody(notas)",
        returns: Some(Receiver::Bar),
        summary: "Sustituye los pasos del compás por notas, como pista:melody.",
        doc: "",
        params: &[Param {
            name: "notas",
            description: "notas, acordes (`c+e+g`), `.` o `_` (tantas como la longitud de la pista)",
        }],
        template: "melody(\"${c . e .  g . e .  c . e .  g . c4 .}\")",
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
        automate: Auto::Smooth,
    },
    FxParam {
        name: "q",
        description: "resonancia",
        default: Some("0.707"),
        valid: "0.1–20",
        values: &[],
        template: "q = ${0.707}",
        automate: Auto::Smooth,
    },
];

/// Parámetros de las bandas del EQ: frecuencia, ganancia, q y forma de cada
/// una, con su frecuencia por defecto (la de `engine::EQ_DEFAULT_FREQS`).
macro_rules! eq_params {
    ($($n:literal $freq:literal),*) => {
        &[$(
            FxParam {
                name: concat!("freq", $n),
                description: concat!("frecuencia de la banda ", $n, " en Hz"),
                default: Some($freq),
                valid: "20–20000",
                values: &[],
                template: concat!("freq", $n, " = ${", $freq, "}"),
                automate: Auto::Smooth,
            },
            FxParam {
                name: concat!("gain", $n),
                description: concat!("dB que sube (o baja, en negativo) la banda ", $n, "; 0 la apaga"),
                default: Some("0"),
                valid: "-24–24",
                values: &[],
                template: concat!("gain", $n, " = ${-3}"),
                automate: Auto::Smooth,
            },
            FxParam {
                name: concat!("q", $n),
                description: concat!("ancho de la banda ", $n, " (más q, más estrecha)"),
                default: Some("1"),
                valid: "0.1–18",
                values: &[],
                template: concat!("q", $n, " = ${1}"),
                automate: Auto::Smooth,
            },
            FxParam {
                name: concat!("type", $n),
                description: concat!("forma de la banda ", $n),
                default: Some("\"bell\""),
                valid: "`\"bell\"`, `\"lowshelf\"` o `\"highshelf\"`",
                values: &["bell", "lowshelf", "highshelf"],
                template: concat!("type", $n, " = \"${bell}\""),
                automate: Auto::No(DISCRETE),
            },
        )*]
    };
}

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
                automate: Auto::No(DISCRETE),
            },
            FILTER_PARAMS[0],
            FILTER_PARAMS[1],
        ],
        template: "{ type = \"${lowpass}\", cutoff = ${3000} }",
    },
    Effect {
        name: "lowpass",
        aliases: &["lp"],
        summary: "Filtro paso bajo (como filter con type = \"lowpass\").",
        params: FILTER_PARAMS,
        template: "{ cutoff = ${3000} }",
    },
    Effect {
        name: "highpass",
        aliases: &["hp"],
        summary: "Filtro paso alto (como filter con type = \"highpass\").",
        params: FILTER_PARAMS,
        template: "{ cutoff = ${200} }",
    },
    Effect {
        name: "bitcrush",
        aliases: &["bitcrusher", "crush"],
        summary: "Menos bits y menos frecuencia de muestreo: suena sucio.",
        params: &[
            FxParam {
                name: "bits",
                description: "resolución",
                default: Some("8"),
                valid: "1–16",
                values: &[],
                template: "bits = ${8}",
                automate: Auto::Smooth,
            },
            FxParam {
                name: "downsample",
                description: "repite cada muestra n veces",
                default: Some("1"),
                valid: "1–64",
                values: &[],
                template: "downsample = ${2}",
                automate: Auto::Smooth,
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
                automate: Auto::Smooth,
            },
            FxParam {
                name: "ratio",
                description: "cuántos dB de entrada por cada dB de salida sobre el umbral",
                default: Some("4"),
                valid: "1–20",
                values: &[],
                template: "ratio = ${4}",
                automate: Auto::Smooth,
            },
            FxParam {
                name: "attack",
                description: "ms que tarda en comprimir (corto: aplasta los ataques)",
                default: Some("5"),
                valid: "0.1–200",
                values: &[],
                template: "attack = ${5}",
                automate: Auto::Smooth,
            },
            FxParam {
                name: "release",
                description: "ms que tarda en soltar",
                default: Some("100"),
                valid: "1–2000",
                values: &[],
                template: "release = ${100}",
                automate: Auto::Smooth,
            },
            FxParam {
                name: "makeup",
                description: "dB que se suben después, para compensar",
                default: Some("0"),
                valid: "0–24",
                values: &[],
                template: "makeup = ${6}",
                automate: Auto::Smooth,
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
                automate: Auto::Smooth,
            },
            FxParam {
                name: "mix",
                description: "parte saturada (1 = solo la saturada)",
                default: Some("1"),
                valid: "0–1",
                values: &[],
                template: "mix = ${1}",
                automate: Auto::Smooth,
            },
        ],
        template: "{ drive = ${4}, mix = ${0.7} }",
    },
    Effect {
        name: "gain",
        aliases: &[],
        summary: "Volumen (lineal): sobre todo para un corte (cortes:fx) o para moverlo con ramp() o lfo().",
        params: &[FxParam {
            name: "level",
            description: "volumen (1 = igual, 0 = silencio)",
            default: Some("1"),
            valid: "0–4",
            values: &[],
            template: "level = ${0.5}",
            automate: Auto::Smooth,
        }],
        template: "{ level = ${0.5} }",
    },
    Effect {
        name: "eq",
        aliases: &[],
        summary: "EQ paramétrico de hasta 4 bandas (campana o estante grave/agudo); una banda con gain 0 no hace nada.",
        params: eq_params!(1 "100", 2 "500", 3 "2000", 4 "8000"),
        template: "{ freq1 = ${300}, gain1 = ${-3}, q1 = ${1} }",
    },
    Effect {
        name: "dyneq",
        aliases: &[],
        summary: "EQ dinámico de una banda: la baja solo mientras suena fuerte en ella la propia pista o, con sidechain, otra.",
        params: &[
            FxParam {
                name: "freq",
                description: "centro de la banda en Hz (la que se escucha y la que baja)",
                default: None,
                valid: "20–20000",
                values: &[],
                template: "freq = ${100}",
                automate: Auto::Smooth,
            },
            FxParam {
                name: "q",
                description: "ancho de la banda (más q, más estrecha)",
                default: Some("1"),
                valid: "0.1–18",
                values: &[],
                template: "q = ${1}",
                automate: Auto::Smooth,
            },
            FxParam {
                name: "type",
                description: "forma de la banda que baja",
                default: Some("\"bell\""),
                valid: "`\"bell\"`, `\"lowshelf\"` o `\"highshelf\"`",
                values: &["bell", "lowshelf", "highshelf"],
                template: "type = \"${bell}\"",
                automate: Auto::No(DISCRETE),
            },
            FxParam {
                name: "threshold",
                description: "nivel en dB de la banda (en lo que se escucha) desde el que baja",
                default: Some("-30"),
                valid: "-60–0",
                values: &[],
                template: "threshold = ${-30}",
                automate: Auto::Smooth,
            },
            FxParam {
                name: "ratio",
                description: "por cada dB de más, baja 1 − 1/ratio dB",
                default: Some("4"),
                valid: "1–20",
                values: &[],
                template: "ratio = ${4}",
                automate: Auto::Smooth,
            },
            FxParam {
                name: "range",
                description: "lo más que baja, en dB",
                default: Some("6"),
                valid: "0–24",
                values: &[],
                template: "range = ${6}",
                automate: Auto::Smooth,
            },
            FxParam {
                name: "attack",
                description: "ms que tarda en bajar",
                default: Some("5"),
                valid: "0.1–200",
                values: &[],
                template: "attack = ${5}",
                automate: Auto::Smooth,
            },
            FxParam {
                name: "release",
                description: "ms que tarda en volver",
                default: Some("120"),
                valid: "1–2000",
                values: &[],
                template: "release = ${120}",
                automate: Auto::Smooth,
            },
            FxParam {
                name: "sidechain",
                description: "pista que se escucha en vez de la propia (aunque esté en mute)",
                default: Some("la propia"),
                valid: "nombre de otra pista",
                values: &[],
                template: "sidechain = \"${kick}\"",
                automate: Auto::No(TRACK_REF),
            },
        ],
        template: "{ freq = ${80}, range = ${6}, sidechain = \"${kick}\" }",
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
        automate: Auto::PerHit,
    },
    FxParam {
        name: "release",
        description: "cola al final del corte (o al cerrarse la puerta en el modo gate), en ms",
        default: Some("0"),
        valid: "0–5000",
        values: &[],
        template: "release = ${40}",
        automate: Auto::PerHit,
    },
    FxParam {
        name: "pan",
        description: "panorama: -1 izquierda, 0 centro, 1 derecha (el lado contrario baja)",
        default: Some("0"),
        valid: "-1–1",
        values: &[],
        template: "pan = ${-0.5}",
        automate: Auto::PerHit,
    },
    FxParam {
        name: "length",
        description: "parte del corte que suena, desde su inicio (0.5: la primera mitad)",
        default: Some("1"),
        valid: "0.01–1",
        values: &[],
        template: "length = ${0.5}",
        automate: Auto::PerHit,
    },
    FxParam {
        name: "play",
        description: "cómo se lee: una vez, en bucle o de ida y vuelta (los bucles suenan hasta el siguiente golpe, la puerta o el choke)",
        default: Some("\"normal\""),
        valid: "`\"normal\"`, `\"loop\"` o `\"pingpong\"`",
        values: &["normal", "loop", "pingpong"],
        template: "play = \"${loop}\"",
        automate: Auto::No(DISCRETE),
    },
    FxParam {
        name: "crossfade",
        description: "con play = \"loop\", fundido en el salto del bucle en ms (0: salto seco); funde con lo que suena justo antes del corte o, si no hay, con lo que sigue tras él, y el bucle dura lo mismo",
        default: Some("10"),
        valid: "0–1000",
        values: &[],
        template: "crossfade = ${10}",
        automate: Auto::PerHit,
    },
];

/// Opciones de `sample(ruta, { ... })`: cómo se prepara el sample al
/// importarlo.
pub const SAMPLE_PARAMS: &[FxParam] = &[
    FxParam {
        name: "trim",
        description: "recorta el silencio del principio y del final: `true` (por debajo de -60 dB) o el nivel en dB",
        default: Some("false"),
        valid: "`true`, `false` o -120–0",
        values: &[],
        template: "trim = ${true}",
        automate: Auto::No(AT_LOAD),
    },
    FxParam {
        name: "normalize",
        description: "lleva el pico a 0 dB (`true`) o al nivel en dB que se diga",
        default: Some("false"),
        valid: "`true`, `false` o -60–0",
        values: &[],
        template: "normalize = ${true}",
        automate: Auto::No(AT_LOAD),
    },
    FxParam {
        name: "fade_in",
        description: "fundido de entrada, en ms",
        default: Some("0"),
        valid: "0–10000",
        values: &[],
        template: "fade_in = ${5}",
        automate: Auto::No(AT_LOAD),
    },
    FxParam {
        name: "fade_out",
        description: "fundido de salida, en ms",
        default: Some("0"),
        valid: "0–10000",
        values: &[],
        template: "fade_out = ${20}",
        automate: Auto::No(AT_LOAD),
    },
];

/// Opciones de `sample:slice(n, { ... })` y `sample:slice_grid(n, { ... })`.
pub const CUT_PARAMS: &[FxParam] = &[FxParam {
    name: "zero_cross",
    description: "lleva cada inicio de corte al cruce por cero más cercano, a no más de estos ms (`true`: 2 ms); también los de `add`, `move` y los ajustes del proyecto",
    default: Some("false"),
    valid: "`true`, `false` o 0–50",
    values: &[],
    template: "zero_cross = ${2}",
    automate: Auto::No(AT_LOAD),
}];

/// Opciones de `sample:instrument({ ... })`.
pub const INSTRUMENT_PARAMS: &[FxParam] = &[
    FxParam {
        name: "root",
        description: "la nota en la que suena el sample tal cual (`melody` toca desde ahí); C4 es el do central",
        default: Some("\"C4\""),
        valid: "una nota (`\"C3\"`, `\"F#2\"`, `\"Bb4\"`) o un número MIDI (0–127)",
        values: &[],
        template: "root = \"${C3}\"",
        automate: Auto::No(INSTRUMENT),
    },
    FxParam {
        name: "loop",
        description: "bucle sostenido `{inicio, fin}` en ms: la nota sigue en bucle mientras dure (sin él, acaba con el sample)",
        default: Some("nil"),
        valid: "dentro del sample, con el fin después del inicio",
        values: &[],
        template: "loop = { ${200}, ${600} }",
        automate: Auto::No(AT_LOAD),
    },
    FxParam {
        name: "voices",
        description: "notas que pueden sonar a la vez; con todas ocupadas, una nota nueva roba la más antigua",
        default: Some("4"),
        valid: "1–16",
        values: &[],
        template: "voices = ${4}",
        automate: Auto::No(STRUCTURE),
    },
    FxParam {
        name: "adsr",
        description: "envolvente de cada nota `{ataque, decay, sustain, release}`: ms, ms, nivel (0–1) y ms (0 ms: el fundido mínimo de 1,5 ms)",
        default: Some("{ 0, 0, 1, 100 }"),
        valid: "ms de 0 a 10000; sustain de 0 a 1",
        values: &[],
        template: "adsr = { ${5}, ${80}, ${0.7}, ${300} }",
        automate: Auto::No(INSTRUMENT),
    },
    FxParam {
        name: "crossfade",
        description: "fundido cruzado del salto del bucle, en ms (usa el audio de antes del inicio del bucle; si no cabe, se acorta y se avisa)",
        default: Some("10"),
        valid: "0–1000",
        values: &[],
        template: "crossfade = ${20}",
        automate: Auto::No(AT_LOAD),
    },
    FxParam {
        name: "keep",
        description: "transponer las notas sin cambiar su duración (granos alineados, como `%` en los pasos): el sample dura lo mismo en cualquier nota; va bien hasta ±12 semitonos",
        default: Some("false"),
        valid: "`true` o `false`",
        values: &[],
        template: "keep = ${true}",
        automate: Auto::No(DISCRETE),
    },
    FxParam {
        name: "formant",
        description: "con `keep`, dónde quedan los formantes de las notas: 0 donde estaban (la voz no suena a ardilla al subir), n desplazados n semitonos; sin él, se mueven con la nota",
        default: Some("nil"),
        valid: "-24–24 (necesita `keep = true`)",
        values: &[],
        template: "formant = ${0}",
        automate: Auto::No(INSTRUMENT),
    },
];

/// El bloque sub, en cualquier bajo sintetizado (la tabla de `synth()` y
/// las llaves de un paso: `{sub=0.5,sub_div=3}`). Los rangos y los valores
/// por defecto de los numéricos son los de
/// `engine::synth::preset::SUB_BLOCK` (un test lo comprueba).
pub const SUB_BLOCK_PARAMS: &[FxParam] = &SUB_BLOCK;
const SUB_BLOCK: [FxParam; 5] = [
    FxParam {
        name: "sub",
        description: "nivel del sub: una onda a la frecuencia de la nota dividida por `sub_div`, sacada de la fase del oscilador (siempre en fase, también con glide); 0 lo apaga",
        default: Some("0"),
        valid: "0–1",
        values: &[],
        template: "sub = ${0.5}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "sub_div",
        description: "divisor de frecuencia del sub: 2 una octava abajo, 3 una octava y una quinta, 4 dos octavas...",
        default: Some("2"),
        valid: "1–16",
        values: &[],
        template: "sub_div = ${2}",
        automate: Auto::No(SUB_DIVIDER),
    },
    FxParam {
        name: "sub_wave",
        description: "forma de onda del sub: cuadrada (con PolyBLEP en sus saltos), triangular, seno o el pulso estrecho al 25 % del SH-101 (con `sub_div = 4`, dos octavas abajo)",
        default: Some("\"square\""),
        valid: "`\"square\"`, `\"tri\"`, `\"sine\"` o `\"pulse25\"`",
        values: &["square", "tri", "sine", "pulse25"],
        template: "sub_wave = \"${tri}\"",
        automate: Auto::No(DISCRETE),
    },
    FxParam {
        name: "sub_pan",
        description: "pan del sub, de potencia constante (en el centro suena igual en los dos lados)",
        default: Some("0"),
        valid: "-1–1",
        values: &[],
        template: "sub_pan = ${-0.5}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "sub_drive",
        description: "saturación suave de la suma del oscilador y el sub, antes del filtro (solo con el sub sonando)",
        default: Some("0"),
        valid: "0–1",
        values: &[],
        template: "sub_drive = ${0.3}",
        automate: Auto::Smooth,
    },
];

/// Los parámetros propios de un sintetizador seguidos de los del bloque
/// sub (`C` = `A` + 5).
const fn with_sub_block<const A: usize, const C: usize>(own: [FxParam; A]) -> [FxParam; C] {
    assert!(C == A + SUB_BLOCK.len());
    let mut out = [SUB_BLOCK[0]; C];
    let mut i = 0;
    while i < A {
        out[i] = own[i];
        i += 1;
    }
    while i < C {
        out[i] = SUB_BLOCK[i - A];
        i += 1;
    }
    out
}

/// Parámetros de `synth("acid", { ... })` (los rangos y los valores por
/// defecto de los continuos son los de `engine::synth::acid::PARAMS`; un
/// test lo comprueba).
pub const ACID_PARAMS: &[FxParam] = &ACID_ALL;
const ACID_ALL: [FxParam; 10 + 5] = with_sub_block(ACID_OWN);
const ACID_OWN: [FxParam; 10] = [
    FxParam {
        name: "wave",
        description: "forma de onda del oscilador",
        default: Some("\"saw\""),
        valid: "`\"saw\"` o `\"square\"`",
        values: &["saw", "square"],
        template: "wave = \"${square}\"",
        automate: Auto::No(DISCRETE),
    },
    FxParam {
        name: "root",
        description: "la nota que suena con 0 semitonos (`melody` sin octava toca en la suya)",
        default: Some("\"C2\""),
        valid: "una nota (`\"C2\"`, `\"F#1\"`) o un número MIDI (0–127)",
        values: &[],
        template: "root = \"${C2}\"",
        automate: Auto::No(INSTRUMENT),
    },
    FxParam {
        name: "cutoff",
        description: "corte del filtro de 4 polos, en Hz",
        default: Some("400"),
        valid: "20–20000",
        values: &[],
        template: "cutoff = ${400}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "res",
        description: "resonancia: el pico en el corte (con 1, el filtro silba solo)",
        default: Some("0.7"),
        valid: "0–1",
        values: &[],
        template: "res = ${0.7}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "envmod",
        description: "cuánto abre la envolvente del filtro el corte en cada nota (1: cuatro octavas)",
        default: Some("0.6"),
        valid: "0–1",
        values: &[],
        template: "envmod = ${0.6}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "decay",
        description: "lo que tarda en cerrarse la envolvente del filtro, en ms",
        default: Some("300"),
        valid: "10–5000",
        values: &[],
        template: "decay = ${300}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "accent",
        description: "cuánto pesa el `!` de un paso: más volumen (el doble con 1), dos octavas más de filtro con un decay corto, y varios seguidos se acumulan",
        default: Some("0.6"),
        valid: "0–1",
        values: &[],
        template: "accent = ${0.6}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "dist",
        description: "saturación a la salida (0, limpio)",
        default: Some("0"),
        valid: "0–1",
        values: &[],
        template: "dist = ${0.3}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "glide",
        description: "lo que tarda el `/` de un paso en llegar a su nota, en ms",
        default: Some("60"),
        valid: "0–1000",
        values: &[],
        template: "glide = ${60}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "seed",
        description: "semilla de la fase del oscilador al empezar desde el silencio: con la misma, el mismo audio; sin ella, una sacada del nombre de la pista",
        default: Some("del nombre"),
        valid: "entero de 0 en adelante",
        values: &[],
        template: "seed = ${1}",
        automate: Auto::No(SEED),
    },
];

/// Parámetros de `synth("sub", { ... })` (los rangos y los valores por
/// defecto de los numéricos son los de `engine::synth::sub::PARAMS`; un
/// test lo comprueba).
pub const SUB_PARAMS: &[FxParam] = &SUB_ALL;
const SUB_ALL: [FxParam; 7 + 5] = with_sub_block(SUB_OWN);
const SUB_OWN: [FxParam; 7] = [
    FxParam {
        name: "wave",
        description: "forma de onda: el seno es el subgrave limpio; la triangular añade algún armónico impar (se oye algo más en altavoces pequeños)",
        default: Some("\"sine\""),
        valid: "`\"sine\"` o `\"triangle\"`",
        values: &["sine", "triangle"],
        template: "wave = \"${triangle}\"",
        automate: Auto::No(DISCRETE),
    },
    FxParam {
        name: "root",
        description: "la nota que suena con 0 semitonos (`melody` sin octava toca en la suya); C1 son 32,7 Hz",
        default: Some("\"C1\""),
        valid: "una nota (`\"C1\"`, `\"F#1\"`) o un número MIDI (0–127)",
        values: &[],
        template: "root = \"${C1}\"",
        automate: Auto::No(INSTRUMENT),
    },
    FxParam {
        name: "tone",
        description: "corte del paso bajo suave (12 dB/oct, sin resonancia), en Hz: más bajo, más redondo; más alto, deja pasar los armónicos de `drive`",
        default: Some("250"),
        valid: "40–2000",
        values: &[],
        template: "tone = ${250}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "drive",
        description: "saturación antes del filtro: le da armónicos graves para que se oiga en altavoces pequeños (0, un seno limpio)",
        default: Some("0"),
        valid: "0–1",
        values: &[],
        template: "drive = ${0.3}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "glide",
        description: "lo que tarda el `/` de un paso en llegar a su nota, en ms",
        default: Some("40"),
        valid: "0–1000",
        values: &[],
        template: "glide = ${40}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "attack",
        description: "lo que tarda en subir cada nota que empieza desde el silencio, en ms (las pegadas no vuelven a subir)",
        default: Some("5"),
        valid: "0–2000",
        values: &[],
        template: "attack = ${5}",
        automate: Auto::No(ENVELOPE),
    },
    FxParam {
        name: "release",
        description: "lo que tarda en apagarse al acabar la nota, en ms",
        default: Some("80"),
        valid: "0–5000",
        values: &[],
        template: "release = ${80}",
        automate: Auto::No(ENVELOPE),
    },
];

/// Las claves de la tabla de cada capa de `subharm` (`o1 = { div = 1,
/// rdiv = 4, pan = -0.6 }`).
pub const LAYER_PARAMS: &[FxParam] = &[
    FxParam {
        name: "div",
        description: "divisor de frecuencia: en `o1` y `o2`, de la nota (1 la nota, 2 una octava abajo, 3 una octava y una quinta...); en los subs, de su oscilador",
        default: Some("o1, o2: 1; s1: 2; s2: 3; s3: 2; s4: 4"),
        valid: "entero de 1 a 16",
        values: &[],
        template: "div = ${2}",
        automate: Auto::No(LAYER),
    },
    FxParam {
        name: "rdiv",
        description: "divisor rítmico: la capa vuelve a sonar (su envolvente, con `attack` y `decay`) cada 1/rdiv del compás; todas a la vez al empezar cada compás",
        default: Some("1"),
        valid: "entero de 1 a 16",
        values: &[],
        template: "rdiv = ${4}",
        automate: Auto::No(LAYER),
    },
    FxParam {
        name: "pan",
        description: "pan de la capa, de potencia constante",
        default: Some("0"),
        valid: "-1–1",
        values: &[],
        template: "pan = ${-0.5}",
        automate: Auto::No(LAYER),
    },
    FxParam {
        name: "level",
        description: "nivel de la capa (0, callada)",
        default: Some("o1: 1; s1: 0.6; las demás: 0"),
        valid: "0–1",
        values: &[],
        template: "level = ${0.8}",
        automate: Auto::No(LAYER),
    },
];

/// La tabla de una capa de `subharm`, como un efecto (para el
/// autocompletado y la ayuda de sus claves).
pub const LAYER_TABLE: Effect = Effect {
    name: "capa",
    aliases: &[],
    summary: "Una capa del subharm: divisor de frecuencia, divisor rítmico, pan y nivel.",
    params: LAYER_PARAMS,
    template: "{ div = ${1}, rdiv = ${4}, pan = ${0} }",
};

/// Parámetros de `subharm{ ... }` (los numéricos, con los rangos y valores
/// por defecto de `engine::synth::subharm::PARAMS`; un test lo comprueba).
pub const SUBHARM_PARAMS: &[FxParam] = &[
    FxParam {
        name: "o1",
        description: "el primer oscilador de sierra: tabla con `div`, `rdiv`, `pan` y `level` (ver la tabla de capa)",
        default: Some("{ div = 1, level = 1 }"),
        valid: "tabla de capa",
        values: &[],
        template: "o1 = { div = ${1}, rdiv = ${4}, pan = ${-0.6} }",
        automate: Auto::No(LAYER),
    },
    FxParam {
        name: "o2",
        description: "el segundo oscilador de sierra",
        default: Some("{ div = 1, level = 0 }"),
        valid: "tabla de capa",
        values: &[],
        template: "o2 = { div = ${2}, rdiv = ${3}, pan = ${0.6} }",
        automate: Auto::No(LAYER),
    },
    FxParam {
        name: "s1",
        description: "primer sub de `o1` (su frecuencia entre `div`)",
        default: Some("{ div = 2, level = 0.6 }"),
        valid: "tabla de capa",
        values: &[],
        template: "s1 = { div = ${3}, rdiv = ${3}, pan = ${0.5} }",
        automate: Auto::No(LAYER),
    },
    FxParam {
        name: "s2",
        description: "segundo sub de `o1`",
        default: Some("{ div = 3, level = 0 }"),
        valid: "tabla de capa",
        values: &[],
        template: "s2 = { div = ${4}, rdiv = ${5}, level = ${0.5} }",
        automate: Auto::No(LAYER),
    },
    FxParam {
        name: "s3",
        description: "primer sub de `o2`",
        default: Some("{ div = 2, level = 0 }"),
        valid: "tabla de capa",
        values: &[],
        template: "s3 = { div = ${2}, rdiv = ${7}, level = ${0.5} }",
        automate: Auto::No(LAYER),
    },
    FxParam {
        name: "s4",
        description: "segundo sub de `o2`",
        default: Some("{ div = 4, level = 0 }"),
        valid: "tabla de capa",
        values: &[],
        template: "s4 = { div = ${5}, rdiv = ${2}, level = ${0.5} }",
        automate: Auto::No(LAYER),
    },
    FxParam {
        name: "wave",
        description: "forma de onda de todas las capas",
        default: Some("\"saw\""),
        valid: "`\"saw\"` o `\"square\"`",
        values: &["saw", "square"],
        template: "wave = \"${square}\"",
        automate: Auto::No(DISCRETE),
    },
    FxParam {
        name: "root",
        description: "la nota que suena con 0 semitonos (`melody` sin octava toca en la suya)",
        default: Some("\"C2\""),
        valid: "una nota (`\"C2\"`) o un número MIDI (0–127)",
        values: &[],
        template: "root = \"${C2}\"",
        automate: Auto::No(INSTRUMENT),
    },
    FxParam {
        name: "cutoff",
        description: "corte del paso bajo de 4 polos, en Hz",
        default: Some("1200"),
        valid: "20–20000",
        values: &[],
        template: "cutoff = ${1200}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "res",
        description: "resonancia del filtro",
        default: Some("0.3"),
        valid: "0–1",
        values: &[],
        template: "res = ${0.3}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "envmod",
        description: "cuánto abre la envolvente del filtro el corte en cada nota (1: cuatro octavas)",
        default: Some("0.3"),
        valid: "0–1",
        values: &[],
        template: "envmod = ${0.3}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "fdecay",
        description: "lo que tarda en cerrarse la envolvente del filtro de cada nota, en ms",
        default: Some("400"),
        valid: "10–5000",
        values: &[],
        template: "fdecay = ${400}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "glide",
        description: "lo que tarda el `/` de un paso en llegar a su nota, en ms",
        default: Some("0"),
        valid: "0–1000",
        values: &[],
        template: "glide = ${60}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "drive",
        description: "saturación a la salida (0, limpio)",
        default: Some("0"),
        valid: "0–1",
        values: &[],
        template: "drive = ${0.3}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "attack",
        description: "ataque de la envolvente de cada capa (la que se redispara con su `rdiv`), en ms",
        default: Some("2"),
        valid: "0.5–2000",
        values: &[],
        template: "attack = ${2}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "decay",
        description: "lo que tarda en apagarse cada capa tras sonar, en ms: corto, golpes; largo, notas que se solapan",
        default: Some("800"),
        valid: "10–10000",
        values: &[],
        template: "decay = ${300}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "voices",
        description: "notas a la vez (1: mono, las notas se ligan; más: polifónico, cada voz con sus seis capas)",
        default: Some("1"),
        valid: "1–8",
        values: &[],
        template: "voices = ${1}",
        automate: Auto::No(STRUCTURE),
    },
    FxParam {
        name: "seed",
        description: "semilla de la fase de los osciladores al empezar desde el silencio; sin ella, una sacada del nombre de la pista",
        default: Some("del nombre"),
        valid: "entero de 0 en adelante",
        values: &[],
        template: "seed = ${1}",
        automate: Auto::No(SEED),
    },
];

/// Parámetros de `sh101{ ... }` (los rangos y los valores por defecto de
/// los numéricos son los de `engine::synth::sh101::PARAMS`; un test lo
/// comprueba), con el bloque sub detrás.
pub const SH101_PARAMS: &[FxParam] = &SH101_ALL;
const SH101_ALL: [FxParam; 28 + 5] = with_sub_block(SH101_OWN);
const SH101_OWN: [FxParam; 28] = [
    FxParam {
        name: "root",
        description: "la nota que suena con 0 semitonos (`melody` sin octava toca en la suya)",
        default: Some("\"C2\""),
        valid: "una nota (`\"C2\"`) o un número MIDI (0–127)",
        values: &[],
        template: "root = \"${C2}\"",
        automate: Auto::No(INSTRUMENT),
    },
    FxParam {
        name: "saw",
        description: "nivel de la sierra del oscilador",
        default: Some("1"),
        valid: "0–1",
        values: &[],
        template: "saw = ${0.8}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "pulse",
        description: "nivel del pulso del oscilador (la misma fase que la sierra; su ancho, `pw`)",
        default: Some("0"),
        valid: "0–1",
        values: &[],
        template: "pulse = ${0.5}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "pw",
        description: "ancho del pulso (0,5 es cuadrada; más estrecho, más nasal)",
        default: Some("0.5"),
        valid: "0.05–0.95",
        values: &[],
        template: "pw = ${0.4}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "pwm_src",
        description: "qué mueve el ancho del pulso: nada (`manual`, el de `pw`), el LFO o la envolvente",
        default: Some("\"manual\""),
        valid: "`\"manual\"`, `\"lfo\"` o `\"env\"`",
        values: &["manual", "lfo", "env"],
        template: "pwm_src = \"${lfo}\"",
        automate: Auto::No(DISCRETE),
    },
    FxParam {
        name: "pwm_amt",
        description: "cuánto mueve su fuente el ancho del pulso (con 1, ±0,45)",
        default: Some("0"),
        valid: "0–1",
        values: &[],
        template: "pwm_amt = ${0.5}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "noise",
        description: "nivel del ruido blanco",
        default: Some("0"),
        valid: "0–1",
        values: &[],
        template: "noise = ${0.2}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "cutoff",
        description: "corte del paso bajo de 4 polos (24 dB/oct, saturación en cada etapa), en Hz",
        default: Some("1500"),
        valid: "20–20000",
        values: &[],
        template: "cutoff = ${600}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "res",
        description: "resonancia: cerca de 1 el filtro autooscila (un seno en el corte, afinado); como en el original, los graves caen al subirla",
        default: Some("0.3"),
        valid: "0–1",
        values: &[],
        template: "res = ${0.7}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "env_amt",
        description: "cuánto abre la envolvente el corte (1: seis octavas; negativo, lo cierra)",
        default: Some("0.4"),
        valid: "-1–1",
        values: &[],
        template: "env_amt = ${0.5}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "kbd",
        description: "seguimiento de teclado: con 1, el corte sube una octava por cada octava de la nota sobre `root`",
        default: Some("0"),
        valid: "0–1",
        values: &[],
        template: "kbd = ${0.5}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "res_comp",
        description: "devuelve los graves que quita la resonancia (0, como el original; 1, todos)",
        default: Some("0"),
        valid: "0–1",
        values: &[],
        template: "res_comp = ${0.5}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "quality",
        description: "sobremuestreo del filtro: 2 lo pasa al doble de la frecuencia (pliega menos al saturar con resonancia alta; un 25 % más de CPU), 1 a la del motor",
        default: Some("2"),
        valid: "1–2",
        values: &[],
        template: "quality = ${1}",
        automate: Auto::No(STRUCTURE),
    },
    FxParam {
        name: "hpf",
        description: "paso alto de un polo tras el filtro: 0 apagado; 1, 2 y 3 cortan a 80, 180 y 400 Hz",
        default: Some("0"),
        valid: "0–3",
        values: &[],
        template: "hpf = ${1}",
        automate: Auto::No(SUB_DIVIDER),
    },
    FxParam {
        name: "a",
        description: "ataque de la envolvente (la del filtro y el VCA), en ms",
        default: Some("2"),
        valid: "0–5000",
        values: &[],
        template: "a = ${2}",
        automate: Auto::No(ENVELOPE),
    },
    FxParam {
        name: "d",
        description: "decay de la envolvente, en ms",
        default: Some("300"),
        valid: "1–10000",
        values: &[],
        template: "d = ${250}",
        automate: Auto::No(ENVELOPE),
    },
    FxParam {
        name: "s",
        description: "sustain de la envolvente (0: la nota cae hasta el silencio aunque siga)",
        default: Some("0.6"),
        valid: "0–1",
        values: &[],
        template: "s = ${0}",
        automate: Auto::No(ENVELOPE),
    },
    FxParam {
        name: "r",
        description: "release de la envolvente, en ms",
        default: Some("100"),
        valid: "0–10000",
        values: &[],
        template: "r = ${100}",
        automate: Auto::No(ENVELOPE),
    },
    FxParam {
        name: "vca",
        description: "qué abre el volumen: la envolvente (`env`) o la puerta de la nota (`gate`: suena igual mientras dura; la envolvente solo mueve el filtro)",
        default: Some("\"env\""),
        valid: "`\"env\"` o `\"gate\"`",
        values: &["env", "gate"],
        template: "vca = \"${gate}\"",
        automate: Auto::No(DISCRETE),
    },
    FxParam {
        name: "glide",
        description: "portamento: lo que tarda en llegar a la nota, en ms (curva exponencial); según `glide_mode`",
        default: Some("0"),
        valid: "0–2000",
        values: &[],
        template: "glide = ${80}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "glide_mode",
        description: "cuándo desliza: nunca (`off`), siempre (`on`, también entre notas separadas) o solo en las ligadas (`auto`: una tecla con otra pulsada o un paso con `/`)",
        default: Some("\"off\""),
        valid: "`\"off\"`, `\"on\"` o `\"auto\"`",
        values: &["off", "on", "auto"],
        template: "glide_mode = \"${auto}\"",
        automate: Auto::No(DISCRETE),
    },
    FxParam {
        name: "lfo_wave",
        description: "forma del LFO: triangular, cuadrada o un valor al azar en cada ciclo (sample & hold)",
        default: Some("\"tri\""),
        valid: "`\"tri\"`, `\"square\"` o `\"random\"`",
        values: &["tri", "square", "random"],
        template: "lfo_wave = \"${random}\"",
        automate: Auto::No(DISCRETE),
    },
    FxParam {
        name: "lfo_rate",
        description: "velocidad del LFO, en Hz (sin `lfo_sync`)",
        default: Some("5"),
        valid: "0.05–30",
        values: &[],
        template: "lfo_rate = ${5}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "lfo_sync",
        description: "fija el LFO al compás: n ciclos por compás, en fase con él (0, libre a `lfo_rate`)",
        default: Some("0"),
        valid: "0–16",
        values: &[],
        template: "lfo_sync = ${4}",
        automate: Auto::No(SUB_DIVIDER),
    },
    FxParam {
        name: "lfo_pitch",
        description: "cuánto mueve el LFO la altura, en semitonos (vibrato; con `square`, trino)",
        default: Some("0"),
        valid: "0–12",
        values: &[],
        template: "lfo_pitch = ${0.3}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "lfo_filter",
        description: "cuánto mueve el LFO el corte, en octavas",
        default: Some("0"),
        valid: "0–4",
        values: &[],
        template: "lfo_filter = ${1}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "drift",
        description: "deriva de afinación, lenta y mínima (con 1, hasta ±10 cents), reproducible con la semilla",
        default: Some("0"),
        valid: "0–1",
        values: &[],
        template: "drift = ${0.3}",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "seed",
        description: "semilla de la fase del oscilador, el ruido, el LFO al azar y la deriva; sin ella, una sacada del nombre de la pista",
        default: Some("del nombre"),
        valid: "entero de 0 en adelante",
        values: &[],
        template: "seed = ${1}",
        automate: Auto::No(SEED),
    },
];

/// Los tipos de `synth(tipo, { ... })`, como efectos (para el
/// autocompletado, la ayuda y la guía).
pub const SYNTHS: &[Effect] = &[
    Effect {
        name: "acid",
        aliases: &[],
        summary: "Bajo del 303: un oscilador por un paso bajo de 4 polos resonante, mono, con accent y slide.",
        params: ACID_PARAMS,
        template: "{ cutoff = ${400}, res = ${0.7}, envmod = ${0.6}, decay = ${300} }",
    },
    Effect {
        name: "sub",
        aliases: &["subbass"],
        summary: "Subgrave limpio: un seno por un paso bajo suave, mono, sin clics al empezar ni entre notas.",
        params: SUB_PARAMS,
        template: "{ tone = ${250}, release = ${80} }",
    },
    Effect {
        name: "subharm",
        aliases: &[],
        summary: "Inspirado en el Subharmonicon: dos osciladores y dos subs de cada uno, seis capas que dividen la frecuencia y el compás.",
        params: SUBHARM_PARAMS,
        template: "{ o1 = { div = ${1}, rdiv = ${4} }, s1 = { div = ${3}, rdiv = ${3} } }",
    },
    Effect {
        name: "sh101",
        aliases: &[],
        summary: "Monosynth de bajo y lead como el SH-101: sierra y pulso, sub, ruido, ladder que autooscila, una envolvente, LFO y portamento.",
        params: SH101_PARAMS,
        template: "{ saw = ${0.8}, pulse = ${0.3}, cutoff = ${600}, res = ${0.7} }",
    },
];

/// El tipo de sintetizador `name` (o uno de sus alias).
pub fn synth(name: &str) -> Option<&'static Effect> {
    SYNTHS
        .iter()
        .find(|s| s.name == name || s.aliases.contains(&name))
}

/// La tabla de `sample:instrument()`, como un efecto (para el
/// autocompletado y la ayuda de sus claves).
pub const INSTRUMENT_TABLE: Effect = Effect {
    name: "instrument",
    aliases: &[],
    summary: "Cómo se toca el sample como instrumento.",
    params: INSTRUMENT_PARAMS,
    template: "{ root = \"${C3}\", voices = ${4} }",
};

/// Parámetros de `pista:grains()` y `pista:freeze()` (los rangos y los
/// valores por defecto son los de `engine::GRAIN_PARAMS`).
pub const GRAIN_PARAMS: &[FxParam] = &[
    FxParam {
        name: "size",
        description: "duración de cada grano, en ms (admite `ramp()` y `lfo()`)",
        default: Some("80"),
        valid: "5–1000",
        values: &[],
        template: "size = ${80}",
        automate: Auto::PerGrain,
    },
    FxParam {
        name: "density",
        description: "granos que nacen por segundo; con `size`, como mucho 32 a la vez (`density × size / 1000`)",
        default: Some("20"),
        valid: "0.5–400",
        values: &[],
        template: "density = ${20}",
        automate: Auto::PerGrain,
    },
    FxParam {
        name: "pos",
        description: "de dónde se leen, en fracción del corte (0 su inicio, 1 su final)",
        default: Some("0"),
        valid: "0–1",
        values: &[],
        template: "pos = ${0.5}",
        automate: Auto::PerGrain,
    },
    FxParam {
        name: "speed",
        description: "cuánto avanza la lectura: 1 recorre el corte a su velocidad, 0 lo congela, negativo va hacia atrás",
        default: Some("1"),
        valid: "-4–4",
        values: &[],
        template: "speed = ${0.5}",
        automate: Auto::PerGrain,
    },
    FxParam {
        name: "jitter",
        description: "desplazamiento al azar de la posición de cada grano, en ms (±)",
        default: Some("0"),
        valid: "0–2000",
        values: &[],
        template: "jitter = ${20}",
        automate: Auto::PerGrain,
    },
    FxParam {
        name: "pitch",
        description: "pitch de los granos en semitonos (se suma al del paso)",
        default: Some("0"),
        valid: "-24–24",
        values: &[],
        template: "pitch = ${-12}",
        automate: Auto::PerGrain,
    },
    FxParam {
        name: "spread",
        description: "desplazamiento al azar del pitch de cada grano, en semitonos (±)",
        default: Some("0"),
        valid: "0–24",
        values: &[],
        template: "spread = ${3}",
        automate: Auto::PerGrain,
    },
    FxParam {
        name: "seed",
        description: "semilla del azar (`jitter`, `spread`): con la misma, la misma nube; sin ella, una sacada del nombre de la pista",
        default: Some("del nombre"),
        valid: "entero de 0 en adelante",
        values: &[],
        template: "seed = ${1}",
        automate: Auto::No(SEED),
    },
];

/// La tabla de `pista:grains()` y `pista:freeze()`, como un efecto (para
/// el autocompletado y la ayuda de sus claves).
pub const GRAINS_TABLE: Effect = Effect {
    name: "grains",
    aliases: &[],
    summary: "Cómo nacen y suenan los granos.",
    params: GRAIN_PARAMS,
    template: "{ size = ${80}, density = ${20} }",
};

/// La tabla de opciones de `sample()`, como un efecto (para el
/// autocompletado y la ayuda de sus claves).
pub const SAMPLE_TABLE: Effect = Effect {
    name: "sample",
    aliases: &[],
    summary: "Cómo se prepara el sample al importarlo.",
    params: SAMPLE_PARAMS,
    template: "{ trim = ${true}, normalize = ${true} }",
};

/// La tabla de opciones de `slice()` y `slice_grid()`, como un efecto
/// (para el autocompletado y la ayuda de sus claves).
pub const CUT_TABLE: Effect = Effect {
    name: "slice",
    aliases: &[],
    summary: "Cómo se corta el sample.",
    params: CUT_PARAMS,
    template: "{ zero_cross = ${2} }",
};

/// La tabla de `cortes:set()`, como un efecto (para el autocompletado y la
/// ayuda de sus claves).
pub const SLICE_TABLE: Effect = Effect {
    name: "set",
    aliases: &[],
    summary: "Forma del corte.",
    params: SLICE_PARAMS,
    template: "{ attack = ${5}, release = ${40} }",
};

/// Parámetros de mezcla de la pista (`:gain()`, `:pan()`, `:send()`) y
/// los demás valores de pista que se preguntan por una curva: si admiten
/// `ramp()`/`lfo()` en lugar del número.
pub const TRACK_PARAMS: &[FxParam] = &[
    FxParam {
        name: "gain",
        description: "volumen de la pista (`:gain(g)`), después de los efectos",
        default: Some("1"),
        valid: "0–4",
        values: &[],
        template: "gain(${0.8})",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "pan",
        description: "pan de la pista (`:pan(p)`), después del volumen",
        default: Some("0"),
        valid: "-1–1",
        values: &[],
        template: "pan(${-0.3})",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "send",
        description: "nivel de un envío (`:send(bus, nivel)`)",
        default: Some("0"),
        valid: "0–1",
        values: &[],
        template: "send(\"${reverb}\", ${0.3})",
        automate: Auto::Smooth,
    },
    FxParam {
        name: "pitch",
        description: "pitch de la pista (`:pitch(semitonos)`), sumado al de cada golpe",
        default: Some("0"),
        valid: "-36–36",
        values: &[],
        template: "pitch(${-12})",
        automate: Auto::PerHit,
    },
    FxParam {
        name: "formant",
        description: "formantes de los pasos con `%` (`:formant(semitonos)`)",
        default: Some("nil"),
        valid: "-24–24",
        values: &[],
        template: "formant(${0})",
        automate: Auto::PerHit,
    },
    FxParam {
        name: "swing",
        description: "swing de la pista (`:swing(porcentaje)`)",
        default: Some("50"),
        valid: "50–75",
        values: &[],
        template: "swing(${56})",
        automate: Auto::No(
            "se convierte al cargar en el retraso de cada contratiempo (como `humanize`): es parte del patrón",
        ),
    },
];

/// Parámetro de pista por nombre (ver [`TRACK_PARAMS`]).
pub fn track_param(name: &str) -> Option<&'static FxParam> {
    TRACK_PARAMS.iter().find(|p| p.name == name)
}

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
                automate: Auto::No(DELAY_TIME),
            },
            FxParam {
                name: "feedback",
                description: "cuánto de cada repetición vuelve a entrar",
                default: Some("0.4"),
                valid: "0–0.95",
                values: &[],
                template: "feedback = ${0.4}",
                automate: Auto::Smooth,
            },
            FxParam {
                name: "level",
                description: "volumen de lo que devuelve el bus",
                default: Some("0.8"),
                valid: "0–1",
                values: &[],
                template: "level = ${0.8}",
                automate: Auto::Smooth,
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
                automate: Auto::Smooth,
            },
            FxParam {
                name: "damp",
                description: "cuánto se apagan los agudos de la cola",
                default: Some("0.5"),
                valid: "0–1",
                values: &[],
                template: "damp = ${0.5}",
                automate: Auto::Smooth,
            },
            FxParam {
                name: "level",
                description: "volumen de lo que devuelve el bus",
                default: Some("0.8"),
                valid: "0–1",
                values: &[],
                template: "level = ${0.8}",
                automate: Auto::Smooth,
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
    ChanceEffect {
        name: "accent",
        aliases: &[],
        value: None,
        summary: "Le pone accent (`!`; solo en un sintetizador)",
    },
    ChanceEffect {
        name: "slide",
        aliases: &[],
        value: None,
        summary: "Lo liga a la nota anterior (`/`; solo en un sintetizador)",
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
    Snippet {
        name: "instrumento",
        summary: "Un sample tocado como instrumento (raíz, bucle, voces y ADSR) con una melodía.",
        template: "local piano = sample(\"${amen.wav}\"):instrument({\n  \
                   root = \"${C3}\", loop = { ${200}, ${600} }, voices = ${4}, adsr = { ${5}, ${80}, ${0.7}, ${300} },\n\
                   })\n\
                   track(\"${lead}\", piano)\n  \
                   :melody(\"${c _ e _  g _ e _  c+e+g _ _ _  . . . .}\")\n",
        requires: "",
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
/// registro) admite `ramp()` y `lfo()`: lo dice su [`FxParam::automate`].
pub fn automatable(effect: &str, param: &str) -> bool {
    self::effect(effect).is_some_and(|e| e.automatable(param))
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

/// Todas las tablas de parámetros de sonido del registro, con su nombre
/// para la guía (lo que se puede automatizar sale de aquí).
pub fn sound_params() -> Vec<(String, &'static FxParam)> {
    let mut out = Vec::new();
    let mut add = |what: String, params: &'static [FxParam]| {
        out.extend(params.iter().map(|p| (what.clone(), p)));
    };
    add("pista".into(), TRACK_PARAMS);
    for e in EFFECTS {
        // `lowpass` y `highpass` son el mismo filtro que `filter`.
        if matches!(e.name, "lowpass" | "highpass") {
            continue;
        }
        add(format!("fx(\"{}\")", e.name), e.params);
    }
    for b in BUSES {
        add(format!("bus(\"{}\")", b.name), b.params);
    }
    add("slice".into(), CUT_PARAMS);
    add("cortes:set".into(), SLICE_PARAMS);
    add(":grains".into(), GRAIN_PARAMS);
    add("instrument".into(), INSTRUMENT_PARAMS);
    for synth in SYNTHS {
        add(format!("synth(\"{}\")", synth.name), synth.params);
    }
    add("sample".into(), SAMPLE_PARAMS);
    out
}

/// «Qué se puede automatizar»: lo que admite `ramp()`/`lfo()` y cómo, y lo
/// que no, con el porqué (todo de [`FxParam::automate`]).
fn automation_table() -> String {
    let params = sound_params();
    let mut out = String::from(
        "#### Qué se puede automatizar\n\n\
         *Continuo*: la curva se evalúa cada 32 frames y se suaviza (~5 ms), \
         también dentro de un golpe. *Por golpe*: cada golpe toma el valor de la \
         curva al sonar y lo mantiene. *Por grano*: al nacer cada grano. Una curva \
         en un parámetro que no la admite da error con la línea.\n\n\
         | Dónde | Admiten curva |\n| --- | --- |\n",
    );
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for (what, p) in &params {
        if !p.automate.curves() {
            continue;
        }
        let item = format!("{} ({})", code(p.name), p.automate.text());
        match groups.iter_mut().find(|(w, _)| w == what) {
            Some((_, items)) => items.push(item),
            None => groups.push((what.clone(), vec![item])),
        }
    }
    for (what, items) in groups {
        out.push_str(&format!("| {} | {} |\n", code(&what), items.join(", ")));
    }
    out.push_str("\n| Dónde | No admiten curva | Por qué |\n| --- | --- | --- |\n");
    // Los del mismo sitio y por lo mismo, en una fila.
    let mut rows: Vec<(String, &str, Vec<String>)> = Vec::new();
    for (what, p) in &params {
        let Auto::No(why) = p.automate else { continue };
        match rows.iter_mut().find(|(w, y, _)| w == what && *y == why) {
            Some((_, _, names)) => names.push(code(p.name)),
            None => rows.push((what.clone(), why, vec![code(p.name)])),
        }
    }
    for (what, why, names) in rows {
        out.push_str(&format!(
            "| {} | {} | {why} |\n",
            code(&what),
            names.join(", ")
        ));
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
            (Receiver::Global, "synth") => out.push_str(&params_table("Tipo", SYNTHS)),
            (Receiver::Global, "subharm") => {
                out.push_str(&params_table("Tabla", std::slice::from_ref(&LAYER_TABLE)))
            }
            (Receiver::Slices, "set")
            | (Receiver::Sample, "slice")
            | (Receiver::Global, "sample")
            | (Receiver::Sample, "instrument")
            | (Receiver::Track, "grains") => {
                out.push_str(
                    "| Parámetro | Qué es | Por defecto | Valores |\n| --- | --- | --- | --- |\n",
                );
                let params = match f.name {
                    "set" => SLICE_PARAMS,
                    "instrument" => INSTRUMENT_PARAMS,
                    "grains" => GRAIN_PARAMS,
                    "slice" => CUT_PARAMS,
                    _ => SAMPLE_PARAMS,
                };
                for p in params {
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
        if (f.receiver, f.name) == (Receiver::Global, "lfo") {
            out.push_str(&automation_table());
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
            // `nombre(...)` o, con una tabla, `nombre{...}`.
            let call = [format!("{}(", f.name), format!("{}{{", f.name)];
            assert!(call.iter().any(|c| text.starts_with(c)), "{}", f.name);
        }
        for e in EFFECTS {
            for p in e.params {
                assert!(p.template.starts_with(p.name), "{}", p.name);
            }
        }
    }
}
