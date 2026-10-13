//! Autocompletado del editor, sin terminal: a partir del texto y la
//! posición del cursor decide qué se puede escribir ahí ([`context`]) y lo
//! filtra por el prefijo ([`Context::matches`]). Las funciones, efectos y
//! parámetros salen del registro de la API ([`script::api`]).
//!
//! Contextos:
//! - tras `:`, los métodos del objeto (sample, cortes, pista o compás de
//!   `every`; si no se sabe cuál es, todos);
//! - dentro de `fx("...")`, los efectos; dentro de su tabla, los parámetros
//!   del efecto y, en `type = "..."`, sus valores;
//! - dentro de `sample("...")`, los WAV de la carpeta de muestras;
//! - dentro de `track("...")`, las pistas ya definidas; dentro de
//!   `chance(p, "...")`, sus efectos;
//! - al inicio de línea, funciones globales, snippets, bloques de la
//!   biblioteca, variables locales y palabras clave (en mitad de una
//!   expresión, lo mismo sin snippets ni bloques);
//! - dentro de `steps("...")`, las etiquetas de corte conocidas para el
//!   paso que se escribe (si empieza por letra);
//! - dentro de cualquier otra cadena o comentario, nada.

use script::api::{self, Receiver};
use script::blocks::Block;

/// Una opción del autocompletado.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    /// Lo que se muestra y se compara con el prefijo.
    pub label: String,
    /// Qué es: «función», «método de pista», «efecto»...
    pub kind: String,
    /// Firma o forma de uso (primera línea del detalle).
    pub signature: String,
    /// Descripción (segunda parte del detalle).
    pub description: String,
    /// Texto que se inserta en lugar del prefijo; los `${...}` son
    /// parámetros que quedan marcados (ver [`api::expand_template`]).
    pub insert: String,
}

/// Lo que se puede escribir en la posición del cursor.
#[derive(Debug, Clone, PartialEq)]
pub struct Context {
    /// Lo ya escrito que se sustituye al aceptar (caracteres antes del
    /// cursor).
    pub prefix: String,
    /// Todas las opciones del contexto, sin filtrar.
    pub candidates: Vec<Candidate>,
}

impl Context {
    /// Opciones que empiezan por el prefijo (sin distinguir mayúsculas).
    pub fn matches(&self) -> Vec<Candidate> {
        let prefix = self.prefix.to_lowercase();
        self.candidates
            .iter()
            .filter(|c| c.label.to_lowercase().starts_with(&prefix))
            .cloned()
            .collect()
    }
}

/// Palabras clave de Lua.
pub const KEYWORDS: &[&str] = &[
    "local", "function", "end", "if", "then", "else", "elseif", "for", "in", "do", "while",
    "repeat", "until", "return", "break", "and", "or", "not", "true", "false", "nil",
];

/// Profundidad máxima al deducir el tipo de una expresión.
const MAX_DEPTH: usize = 8;

/// Contexto del cursor en (`row`, `col`), con `col` en caracteres. `wavs`
/// da los archivos de la carpeta de muestras (solo se llama dentro de
/// `sample("...")`); `labels`, las etiquetas de corte conocidas; `blocks`,
/// los bloques de la biblioteca (al inicio de línea). `None`:
/// ahí no se completa nada (un número de corte o un sufijo en
/// `steps("...")`, otra cadena, un comentario o un número).
pub fn context(
    lines: &[String],
    row: usize,
    col: usize,
    wavs: impl FnOnce() -> Vec<String>,
    labels: &[String],
    blocks: &[Block],
) -> Option<Context> {
    let mut before = String::new();
    for line in lines.iter().take(row) {
        before.push_str(line);
        before.push('\n');
    }
    let current = lines.get(row).map_or("", String::as_str);
    before.extend(current.chars().take(col));
    let scan = Scan::of(&before);
    let all = lines.join("\n");

    if let Some(quote) = scan.string {
        let prefix: String = before[quote + 1..].to_string();
        return string_context(&scan, &before[..quote], prefix, &all, wavs, labels);
    }
    if scan.comment {
        return None;
    }

    let word_start = before
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_ident(c))
        .last()
        .map_or(before.len(), |(i, _)| i);
    let prefix = before[word_start..].to_string();
    if prefix.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    let head = &scan.code[..word_start];
    match head.chars().last() {
        Some(':') => {
            let receiver = type_of(&scan.code, &head[..head.len() - 1], 0);
            return Some(Context {
                prefix,
                candidates: methods(receiver),
            });
        }
        Some('.') => return None,
        _ => {}
    }

    // Clave de la tabla de parámetros de `fx("efecto", { ... })`.
    if let Some(Frame::Brace { item_start }) = scan.frames.last() {
        if let Some(effect) = scan.fx_effect()
            && head[*item_start..].trim().is_empty()
        {
            return Some(Context {
                prefix,
                candidates: effect.params.iter().map(fx_param).collect(),
            });
        }
        return None;
    }

    let line_start = head.rsplit('\n').next().unwrap_or("").trim().is_empty();
    let mut candidates: Vec<Candidate> = api::functions(Receiver::Global).map(function).collect();
    if line_start {
        candidates.extend(api::SNIPPETS.iter().map(|s| {
            let (text, _) = api::expand_template(s.template);
            Candidate {
                label: s.name.into(),
                kind: "snippet".into(),
                signature: format!("snippet {}", s.name),
                description: format!("{}\n{}", s.summary, text.lines().next().unwrap_or("")),
                insert: s.template.into(),
            }
        }));
        candidates.extend(blocks.iter().map(block));
    }
    candidates.extend(locals(&all).into_iter().map(|name| Candidate {
        label: name.clone(),
        kind: "variable".into(),
        signature: format!("local {name}"),
        description: "variable local del script".into(),
        insert: name,
    }));
    if line_start || !prefix.is_empty() {
        candidates.extend(KEYWORDS.iter().map(|k| Candidate {
            label: k.to_string(),
            kind: "palabra clave".into(),
            signature: k.to_string(),
            description: "palabra clave de Lua".into(),
            insert: k.to_string(),
        }));
    }
    Some(Context { prefix, candidates })
}

/// Opción de un bloque de la biblioteca: se inserta como un snippet.
fn block(b: &Block) -> Candidate {
    let tags: String = b.tags.iter().map(|t| format!(" #{t}")).collect();
    let preview = b.preview();
    let mut description = b.description.clone();
    if !description.is_empty() {
        description.push('\n');
    }
    description.push_str(preview.lines().next().unwrap_or(""));
    Candidate {
        label: b.name.clone(),
        kind: "bloque".into(),
        signature: format!("bloque {} ({}){tags}", b.name, b.scope.label()),
        description,
        insert: b.body.clone(),
    }
}

/// Contexto dentro de una cadena: depende de la llamada en la que está.
fn string_context(
    scan: &Scan,
    head: &str,
    prefix: String,
    all: &str,
    wavs: impl FnOnce() -> Vec<String>,
    labels: &[String],
) -> Option<Context> {
    // En `steps("...")`: las etiquetas, para el paso que se está escribiendo.
    if let Some(Frame::Paren { name, arg: 0, .. }) = scan.frames.last()
        && name == "steps"
    {
        let token = prefix.rsplit(char::is_whitespace).next().unwrap_or("");
        let is_word = token.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !token.starts_with(|c: char| c.is_ascii_digit());
        if labels.is_empty() || !is_word {
            return None;
        }
        return Some(Context {
            prefix: token.to_string(),
            candidates: labels
                .iter()
                .map(|l| Candidate {
                    label: l.clone(),
                    kind: "etiqueta".into(),
                    signature: format!("{l} (con sufijos: {l}^7@0.5, {l}'r2)"),
                    description: "etiqueta de corte".into(),
                    insert: l.clone(),
                })
                .collect(),
        });
    }
    let candidates = match scan.frames.last()? {
        Frame::Paren { name, arg, .. } => match (name.as_str(), *arg) {
            ("sample", 0) => wavs()
                .into_iter()
                .map(|w| Candidate {
                    label: w.clone(),
                    kind: "WAV".into(),
                    signature: format!("sample(\"{w}\")"),
                    description: "archivo de la carpeta de muestras".into(),
                    insert: w,
                })
                .collect(),
            ("fx", 0) => api::EFFECTS
                .iter()
                .map(|e| Candidate {
                    label: e.name.into(),
                    kind: "efecto".into(),
                    signature: format!(
                        "fx(\"{}\", {{ {} }})",
                        e.name,
                        e.params
                            .iter()
                            .map(|p| p.name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    description: e.summary.into(),
                    insert: e.name.into(),
                })
                .collect(),
            ("track", 0) => track_names(all)
                .into_iter()
                .map(|n| Candidate {
                    label: n.clone(),
                    kind: "pista".into(),
                    signature: format!("track(\"{n}\")"),
                    description: "pista definida en el script".into(),
                    insert: n,
                })
                .collect(),
            ("fill", 0) => api::FILL_EFFECTS
                .iter()
                .map(|e| Candidate {
                    label: e.name.into(),
                    kind: "efecto de fill".into(),
                    signature: match e.value {
                        Some(v) => format!("fill(\"{}\", n, pasos, {v})", e.name),
                        None => format!("fill(\"{}\", n, pasos)", e.name),
                    },
                    description: e.summary.into(),
                    insert: e.name.into(),
                })
                .collect(),
            ("scale", 0) => api::SCALES
                .iter()
                .map(|s| Candidate {
                    label: s.name.into(),
                    kind: "escala".into(),
                    signature: format!("scale(\"{}\")", s.name),
                    description: s.summary.into(),
                    insert: s.name.into(),
                })
                .collect(),
            ("scale", 1) => api::NOTES
                .iter()
                .map(|(n, semis)| Candidate {
                    label: n.to_string(),
                    kind: "tónica".into(),
                    signature: format!("scale(nombre, \"{n}\")"),
                    description: format!("{semis} semitonos sobre el tono del corte"),
                    insert: n.to_string(),
                })
                .collect(),
            ("chance", 1) => api::CHANCE_EFFECTS
                .iter()
                .map(|e| Candidate {
                    label: e.name.into(),
                    kind: "efecto de chance".into(),
                    signature: match e.value {
                        Some(v) => format!("chance(p, \"{}\", {v})", e.name),
                        None => format!("chance(p, \"{}\")", e.name),
                    },
                    description: e.summary.into(),
                    insert: e.name.into(),
                })
                .collect(),
            _ => return None,
        },
        // Valor de un parámetro con valores cerrados: `type = "..."`.
        Frame::Brace { item_start } => {
            let effect = scan.fx_effect()?;
            let item = head[*item_start..].trim();
            let key = item.strip_suffix('=')?.trim();
            let param = effect.params.iter().find(|p| p.name == key)?;
            if param.values.is_empty() {
                return None;
            }
            param
                .values
                .iter()
                .map(|v| Candidate {
                    label: v.to_string(),
                    kind: format!("valor de {key}"),
                    signature: format!("{key} = \"{v}\""),
                    description: param.description.into(),
                    insert: v.to_string(),
                })
                .collect()
        }
    };
    Some(Context { prefix, candidates })
}

fn function(f: &api::Function) -> Candidate {
    let kind = match f.receiver {
        Receiver::Global => "función".to_string(),
        r => format!("método de {}", r.label()),
    };
    let mut description = f.summary.to_string();
    for p in f.params {
        description.push_str(&format!("\n{}: {}", p.name, p.description));
    }
    Candidate {
        label: f.name.into(),
        kind,
        signature: f.full_signature(),
        description,
        insert: f.template.into(),
    }
}

fn fx_param(p: &api::FxParam) -> Candidate {
    let default = p
        .default
        .map_or("obligatorio".to_string(), |d| format!("por defecto {d}"));
    Candidate {
        label: p.name.into(),
        kind: "parámetro".into(),
        signature: format!("{} = {} ({default})", p.name, p.valid.replace('`', "")),
        description: p.description.into(),
        insert: p.template.into(),
    }
}

/// Métodos de un receptor; si no se sabe cuál, los de todos (sin repetir
/// nombres).
fn methods(receiver: Option<Receiver>) -> Vec<Candidate> {
    let order = match receiver {
        Some(r) => vec![r],
        None => vec![
            Receiver::Track,
            Receiver::Slices,
            Receiver::Sample,
            Receiver::Bar,
            Receiver::Scale,
        ],
    };
    let mut out: Vec<Candidate> = Vec::new();
    for r in order {
        for f in api::functions(r) {
            if !out.iter().any(|c| c.label == f.name) {
                out.push(function(f));
            }
        }
    }
    out
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Llamada o tabla abierta en el cursor.
#[derive(Debug, Clone)]
enum Frame {
    Paren {
        /// Nombre de la función (`fx`, `sample`...).
        name: String,
        /// Argumento en el que está el cursor (desde 0).
        arg: usize,
        /// Primera cadena de cada argumento, si la hay.
        strings: Vec<Option<String>>,
        /// Si es un método, byte de `code` donde está su `:`.
        colon: Option<usize>,
    },
    Brace {
        /// Byte donde empieza el elemento actual (tras `{` o `,`).
        item_start: usize,
    },
}

/// Recorrido del texto hasta el cursor.
struct Scan {
    /// El texto con los comentarios cambiados por espacios.
    code: String,
    /// Llamadas y tablas abiertas.
    frames: Vec<Frame>,
    /// Byte de la comilla que abre la cadena en la que está el cursor.
    string: Option<usize>,
    /// El cursor está en un comentario.
    comment: bool,
}

impl Scan {
    fn of(text: &str) -> Scan {
        let mut code = String::with_capacity(text.len());
        let mut frames: Vec<Frame> = Vec::new();
        let mut string: Option<(char, usize)> = None;
        let mut comment = false;
        let mut chars = text.char_indices().peekable();
        while let Some((i, c)) = chars.next() {
            if c == '\n' {
                // Las cadenas y los comentarios de línea no pasan de línea.
                string = None;
                comment = false;
                code.push(c);
                continue;
            }
            if comment {
                // Mismos bytes que el original: las posiciones coinciden.
                code.extend(std::iter::repeat_n(' ', c.len_utf8()));
                continue;
            }
            if let Some((quote, start)) = string {
                code.push(c);
                if c == '\\' {
                    if let Some((_, next)) = chars.next() {
                        code.push(next);
                    }
                } else if c == quote {
                    string = None;
                    if let Some(Frame::Paren { arg, strings, .. }) = frames.last_mut() {
                        if strings.len() <= *arg {
                            strings.resize(*arg + 1, None);
                        }
                        strings[*arg].get_or_insert_with(|| text[start + 1..i].to_string());
                    }
                }
                continue;
            }
            match c {
                '-' if chars.peek().is_some_and(|&(_, n)| n == '-') => {
                    comment = true;
                    code.push(' ');
                    continue;
                }
                '"' | '\'' => string = Some((c, i)),
                '(' => frames.push({
                    let name = last_ident(&code);
                    let callee = code.trim_end();
                    let before = &callee[..callee.len() - name.len()];
                    Frame::Paren {
                        colon: before.ends_with(':').then(|| before.len() - 1),
                        name,
                        arg: 0,
                        strings: Vec::new(),
                    }
                }),
                '{' => frames.push(Frame::Brace { item_start: i + 1 }),
                ')' | '}' => {
                    frames.pop();
                }
                ',' => match frames.last_mut() {
                    Some(Frame::Paren { arg, .. }) => *arg += 1,
                    Some(Frame::Brace { item_start }) => *item_start = i + 1,
                    None => {}
                },
                _ => {}
            }
            code.push(c);
        }
        Scan {
            code,
            frames,
            string: string.map(|(_, start)| start),
            comment,
        }
    }

    /// Efecto de la tabla abierta, si es la de `fx("efecto", { ... })`.
    fn fx_effect(&self) -> Option<&'static api::Effect> {
        let [
            ..,
            Frame::Paren {
                name, arg, strings, ..
            },
            Frame::Brace { .. },
        ] = &self.frames[..]
        else {
            return None;
        };
        if name != "fx" || *arg != 1 {
            return None;
        }
        api::effect(strings.first()?.as_deref()?)
    }
}

/// Ayuda contextual: qué es lo que hay bajo el cursor.
#[derive(Debug, Clone, PartialEq)]
pub struct Help {
    /// Firma de la función o forma de uso del efecto.
    pub signature: String,
    /// Resumen, o el parámetro en el que está el cursor.
    pub description: String,
}

/// Ayuda de la función (o el efecto de `fx`) bajo el cursor: la palabra
/// sobre la que está o, si no, la llamada dentro de la que está, con el
/// parámetro que toca. `None` si no hay nada de la API.
pub fn help_at(lines: &[String], row: usize, col: usize) -> Option<Help> {
    let current = lines.get(row)?;
    // La palabra bajo el cursor: hasta su final, para que cuente entera.
    let chars: Vec<char> = current.chars().collect();
    let mut end = col.min(chars.len());
    while end < chars.len() && is_ident(chars[end]) {
        end += 1;
    }
    let mut before = String::new();
    for line in lines.iter().take(row) {
        before.push_str(line);
        before.push('\n');
    }
    before.extend(chars[..end].iter());
    let scan = Scan::of(&before);
    if scan.comment {
        return None;
    }

    if scan.string.is_none() {
        let name = last_ident(&scan.code);
        let head = &scan.code[..scan.code.len() - name.len()];
        // Solo si la palabra llega al cursor (no una anterior separada por espacios).
        if !name.is_empty() && head.len() + name.len() == scan.code.len() {
            let found = match head.chars().last() {
                Some(':') => method(&scan.code, &head[..head.len() - 1], &name),
                Some('.') => None,
                _ => api::function(Receiver::Global, &name),
            };
            if let Some(f) = found {
                return Some(Help {
                    signature: f.full_signature(),
                    description: f.summary.into(),
                });
            }
        }
    }

    // Dentro de la tabla de `fx("efecto", { ... })`: el efecto y sus parámetros.
    if let (Some(effect), Some(Frame::Brace { item_start })) =
        (scan.fx_effect(), scan.frames.last())
    {
        let item = &scan.code[*item_start..];
        let key = item.split('=').next().unwrap_or("").trim();
        let description = match effect
            .params
            .iter()
            .find(|p| p.name == key && item.contains('='))
        {
            Some(p) => format!(
                "{}: {} ({}{})",
                p.name,
                p.description,
                p.valid.replace('`', ""),
                p.default
                    .map_or(", obligatorio".into(), |d| format!(", por defecto {d}"))
            ),
            None => format!(
                "{} Parámetros: {}",
                effect.summary,
                effect
                    .params
                    .iter()
                    .map(|p| p.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        return Some(Help {
            signature: format!("fx(\"{}\", {{ ... }})", effect.name),
            description,
        });
    }

    // La llamada más interior de la API.
    for frame in scan.frames.iter().rev() {
        let Frame::Paren {
            name,
            arg,
            strings,
            colon,
        } = frame
        else {
            continue;
        };
        let found = match colon {
            Some(at) => method(&scan.code, &scan.code[..*at], name),
            None => api::function(Receiver::Global, name),
        };
        let Some(f) = found else { continue };
        let mut description = match f.params.get(*arg) {
            Some(p) => format!("{}: {}", p.name, p.description),
            None => f.summary.into(),
        };
        // El efecto elegido en `fx("...")` o `chance(p, "...")`.
        let chosen = |i: usize| strings.get(i).cloned().flatten();
        match (f.receiver, f.name) {
            (Receiver::Track, "fx") => {
                if let Some(e) = chosen(0).as_deref().and_then(api::effect) {
                    description = format!("{} · {}: {}", description, e.name, e.summary);
                }
            }
            (Receiver::Track, "chance") => {
                if let Some(e) = chosen(1).as_deref().and_then(api::chance_effect) {
                    description = format!("{} · {}: {}", description, e.name, e.summary);
                }
            }
            _ => {}
        }
        return Some(Help {
            signature: f.full_signature(),
            description,
        });
    }
    None
}

/// Método `name` del objeto que acaba al final de `receiver`; si no se
/// sabe qué es, el primero con ese nombre.
fn method(code: &str, receiver: &str, name: &str) -> Option<&'static api::Function> {
    match type_of(code, receiver, 0) {
        Some(r) => api::function(r, name),
        None => [
            Receiver::Track,
            Receiver::Slices,
            Receiver::Sample,
            Receiver::Bar,
            Receiver::Scale,
        ]
        .into_iter()
        .find_map(|r| api::function(r, name)),
    }
}

/// Identificador al final del texto (sin contar espacios).
fn last_ident(text: &str) -> String {
    let trimmed = text.trim_end();
    let start = trimmed
        .char_indices()
        .rev()
        .take_while(|&(_, c)| is_ident(c))
        .last()
        .map_or(trimmed.len(), |(i, _)| i);
    trimmed[start..].to_string()
}

/// Tipo de la expresión que acaba al final de `expr` (`code` es todo el
/// texto, para buscar las variables). `None` si no se sabe.
fn type_of(code: &str, expr: &str, depth: usize) -> Option<Receiver> {
    if depth > MAX_DEPTH {
        return None;
    }
    let expr = expr.trim_end();
    if let Some(open_end) = expr.strip_suffix(')') {
        // Llamada: buscar el paréntesis que abre.
        let mut level = 0usize;
        let mut open = None;
        for (i, c) in open_end.char_indices().rev() {
            match c {
                ')' => level += 1,
                '(' if level == 0 => {
                    open = Some(i);
                    break;
                }
                '(' => level -= 1,
                _ => {}
            }
        }
        let callee = expr[..open?].trim_end();
        let name = last_ident(callee);
        let before = &callee[..callee.len() - name.len()];
        return match before.chars().last() {
            Some(':') => {
                let receiver = type_of(code, &before[..before.len() - 1], depth + 1);
                match receiver {
                    Some(r) => api::function(r, &name)?.returns,
                    // Receptor desconocido: lo que devuelva un método con ese
                    // nombre, si todos coinciden.
                    None => {
                        let mut kinds = api::FUNCTIONS
                            .iter()
                            .filter(|f| f.receiver != Receiver::Global && f.name == name)
                            .map(|f| f.returns);
                        let first = kinds.next()??;
                        kinds.all(|k| k == Some(first)).then_some(first)
                    }
                }
            }
            Some('.') => None,
            _ => api::function(Receiver::Global, &name)?.returns,
        };
    }
    let name = last_ident(expr);
    if name.is_empty() {
        return None;
    }
    variable_type(code, &name, depth)
}

/// Tipo de una variable: parámetro de la función de `every` (un compás) o
/// lo último que se le asignó con `local nombre = ...` o `nombre = ...`.
fn variable_type(code: &str, name: &str, depth: usize) -> Option<Receiver> {
    let mut found = None;
    for line in code.lines() {
        if function_params(line).is_some_and(|args| args.iter().any(|a| a == name)) {
            found = Some(Receiver::Bar);
            continue;
        }
        let line = line.trim();
        let line = line.strip_prefix("local ").unwrap_or(line).trim_start();
        let Some(rest) = line.strip_prefix(name) else {
            continue;
        };
        let Some(value) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        if value.starts_with('=') {
            continue;
        }
        found = type_of(code, value, depth + 1);
    }
    found
}

/// Parámetros de una `function(...)` anónima de la línea, si la hay.
fn function_params(line: &str) -> Option<Vec<String>> {
    let start = line.find("function")? + "function".len();
    let rest = line[start..].trim_start().strip_prefix('(')?;
    let args = &rest[..rest.find(')')?];
    Some(args.split(',').map(|a| a.trim().to_string()).collect())
}

/// Variables declaradas con `local` (en orden, sin repetir).
fn locals(text: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (i, _) in text.match_indices("local ") {
        if text[..i].chars().last().is_some_and(is_ident) {
            continue;
        }
        let rest = text[i + "local ".len()..].trim_start();
        let rest = rest.strip_prefix("function ").unwrap_or(rest).trim_start();
        let names_part = rest.split(['=', '(', '\n']).next().unwrap_or("");
        for name in names_part.split(',') {
            let name = name.trim();
            if !name.is_empty() && name.chars().all(is_ident) && !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// Etiquetas puestas en el texto con `:name(i, "etiqueta")` (en orden,
/// sin repetir).
pub fn text_labels(text: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (i, _) in text.match_indices(":name(") {
        let rest = &text[i + ":name(".len()..];
        let Some((_, after)) = rest.split_once(',') else {
            continue;
        };
        let after = after.trim_start();
        let Some(quote) = after.chars().next().filter(|&c| c == '"' || c == '\'') else {
            continue;
        };
        let Some(end) = after[1..].find(quote) else {
            continue;
        };
        let name = &after[1..1 + end];
        if engine::notation::check_label(name).is_ok() && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    names
}

/// Nombres de pista definidos con `track("nombre", ...)` (en orden, sin
/// repetir).
pub fn track_names(text: &str) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    for (i, _) in text.match_indices("track(") {
        if text[..i].chars().last().is_some_and(is_ident) {
            continue;
        }
        let rest = text[i + "track(".len()..].trim_start();
        let Some(quote) = rest.chars().next().filter(|&c| c == '"' || c == '\'') else {
            continue;
        };
        let Some(end) = rest[1..].find(quote) else {
            continue;
        };
        let name = &rest[1..1 + end];
        if !name.is_empty() && !name.contains('\n') && !names.iter().any(|n| n == name) {
            names.push(name.to_string());
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Contexto con el cursor donde está `|`.
    fn at(text: &str) -> Option<Context> {
        at_with(text, &[])
    }

    fn at_with(text: &str, wavs: &[&str]) -> Option<Context> {
        let lines: Vec<String> = text.lines().map(String::from).collect();
        let (row, line) = lines
            .iter()
            .enumerate()
            .find(|(_, l)| l.contains('|'))
            .expect("falta el cursor |");
        let col = line[..line.find('|').unwrap()].chars().count();
        let lines: Vec<String> = lines.iter().map(|l| l.replace('|', "")).collect();
        let wavs: Vec<String> = wavs.iter().map(|w| w.to_string()).collect();
        let labels = ["kick".to_string(), "snare".to_string(), "hat".to_string()];
        context(&lines, row, col, || wavs, &labels, &[])
    }

    fn labels(context: &Context) -> Vec<String> {
        context.matches().into_iter().map(|c| c.label).collect()
    }

    const SONG: &str = "bpm(170)\nlocal amen = sample(\"amen.wav\"):slice(8)\n";

    #[test]
    fn line_start_offers_globals_locals_and_keywords() {
        let c = at(&format!("{SONG}|")).unwrap();
        let l = labels(&c);
        assert_eq!(&l[..3], ["bpm", "sample", "track"]);
        assert!(l.contains(&"amen".into()), "{l:?}");
        assert!(l.contains(&"local".into()), "{l:?}");
        assert!(l.contains(&"ghost".into()), "snippets: {l:?}");
        let c = at(&format!("{SONG}tr|")).unwrap();
        assert_eq!(c.prefix, "tr");
        assert_eq!(labels(&c), ["track", "true"]);
        // En mitad de una expresión no hay snippets.
        let c = at("local x = gh|").unwrap();
        assert!(labels(&c).is_empty());
    }

    #[test]
    fn line_start_offers_library_blocks_like_snippets() {
        let block = Block {
            name: "fantasma".into(),
            description: "Ghost notes".into(),
            tags: vec!["ghost".into()],
            body: "track(\"${g}\", amen)\n".into(),
            scope: script::blocks::Scope::Global,
            path: "fantasma.lua".into(),
        };
        let at_blocks = |text: &str| {
            let (before, after) = text.split_once('|').unwrap();
            let lines: Vec<String> = format!("{before}{after}")
                .lines()
                .map(String::from)
                .collect();
            let row = before.lines().count().max(1) - 1;
            let col = before.lines().last().unwrap_or("").chars().count();
            context(
                &lines,
                row,
                col,
                Vec::new,
                &[],
                std::slice::from_ref(&block),
            )
        };
        let c = at_blocks("  fan|").unwrap();
        let m = c.matches();
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].kind, "bloque");
        assert_eq!(m[0].signature, "bloque fantasma (global) #ghost");
        assert_eq!(m[0].description, "Ghost notes\ntrack(\"g\", amen)");
        assert_eq!(m[0].insert, "track(\"${g}\", amen)\n");
        // En mitad de una expresión, no.
        let c = at_blocks("local x = fan|").unwrap();
        assert!(c.matches().is_empty());
    }

    #[test]
    fn methods_follow_the_type_of_the_receiver() {
        let c = at(&format!("{SONG}track(\"drums\", amen):|")).unwrap();
        let track: Vec<String> = api::functions(Receiver::Track)
            .map(|f| f.name.to_string())
            .collect();
        assert_eq!(
            labels(&c),
            track,
            "métodos de pista, en el orden del registro"
        );
        assert!(track.starts_with(&["steps".into(), "gain".into()]));
        assert_eq!(c.matches()[0].kind, "método de pista");
        let c = at("local s = sample(\"amen.wav\")\ns:|").unwrap();
        assert_eq!(labels(&c), ["detect_bpm", "slice"]);
        let c = at(&format!("{SONG}amen:s|")).unwrap();
        assert_eq!(labels(&c), ["slice_at"], "cortes");
        let c = at("sample(\"amen.wav\"):slice(8):|").unwrap();
        let slices: Vec<String> = api::functions(Receiver::Slices)
            .map(|f| f.name.to_string())
            .collect();
        assert_eq!(labels(&c), slices, "métodos de cortes, del registro");
        assert!(slices.starts_with(&["count".into(), "slice_at".into()]));
    }

    #[test]
    fn chains_on_following_lines_and_every_bars() {
        let text = format!(
            "{SONG}track(\"drums\", amen)\n  :steps(\"1 . . .  . . . .  . . . .  . . . .\")\n  -- comentario\n  :f|"
        );
        let c = at(&text).unwrap();
        assert_eq!(labels(&c), ["fill", "fx"]);
        let text = format!("{SONG}track(\"drums\", amen):every(4, function(c) c:|) end)");
        let c = at(&text).unwrap();
        assert_eq!(labels(&c), ["steps", "reverse", "pitch", "retrig", "gain"]);
        assert_eq!(c.matches()[0].kind, "método de compás");
    }

    #[test]
    fn unknown_receivers_offer_every_method_once() {
        let c = at("algo:|").unwrap();
        let l = labels(&c);
        assert_eq!(l.iter().filter(|n| *n == "steps").count(), 1);
        assert!(l.contains(&"slice".into()) && l.contains(&"reverse".into()));
    }

    #[test]
    fn fx_offers_effects_then_their_parameters_and_values() {
        let c = at(&format!("{SONG}track(\"d\", amen):fx(\"|")).unwrap();
        assert_eq!(labels(&c), ["filter", "lowpass", "highpass", "bitcrush"]);
        let c = at(&format!("{SONG}track(\"d\", amen):fx(\"bit|")).unwrap();
        assert_eq!(c.prefix, "bit");
        assert_eq!(labels(&c), ["bitcrush"]);

        let c = at(&format!("{SONG}track(\"d\", amen):fx(\"filter\", {{ |")).unwrap();
        assert_eq!(labels(&c), ["type", "cutoff", "q"]);
        assert_eq!(c.matches()[1].insert, "cutoff = ${3000}");
        let c = at(&format!(
            "{SONG}track(\"d\", amen):fx(\"bitcrush\", {{ bits = 4, d|"
        ))
        .unwrap();
        assert_eq!(labels(&c), ["downsample"]);
        // Valor de un parámetro: solo si tiene valores cerrados.
        let c = at(&format!(
            "{SONG}track(\"d\", amen):fx(\"filter\", {{ type = \"|"
        ))
        .unwrap();
        assert_eq!(labels(&c), ["lowpass", "highpass"]);
        assert!(
            at(&format!(
                "{SONG}track(\"d\", amen):fx(\"filter\", {{ cutoff = |"
            ))
            .is_none()
        );
    }

    #[test]
    fn sample_offers_the_wavs_of_the_sample_folder() {
        let wavs = ["amen.wav", "think.wav"];
        let c = at_with("local a = sample(\"|", &wavs).unwrap();
        assert_eq!(labels(&c), ["amen.wav", "think.wav"]);
        let c = at_with("local a = sample(\"th|", &wavs).unwrap();
        assert_eq!(labels(&c), ["think.wav"]);
        assert_eq!(c.prefix, "th");
    }

    #[test]
    fn track_offers_defined_tracks_and_chance_its_effects() {
        let text =
            format!("{SONG}track(\"drums\", amen)\ntrack(\"kick\", amen:slice_at(1))\ntrack(\"|");
        assert_eq!(labels(&at(&text).unwrap()), ["drums", "kick"]);
        let c = at(&format!("{SONG}track(\"d\", amen):chance(0.2, \"s|")).unwrap();
        assert!(labels(&c).is_empty());
        let c = at(&format!("{SONG}track(\"d\", amen):chance(0.2, \"|")).unwrap();
        assert_eq!(labels(&c), ["retrig", "reverse", "pitch", "gain", "mute"]);
    }

    #[test]
    fn steps_offers_labels_for_the_step_being_written() {
        let c = at(&format!("{SONG}track(\"d\", amen):steps(\"kick . s|")).unwrap();
        assert_eq!(c.prefix, "s");
        assert_eq!(labels(&c), ["snare"]);
        assert_eq!(c.matches()[0].kind, "etiqueta");
        let c = at(&format!("{SONG}track(\"d\", amen):steps(\"|")).unwrap();
        assert_eq!(labels(&c), ["kick", "snare", "hat"]);
        let c = at(&format!("{SONG}x:every(2, function(c) c:steps(\"1 h|")).unwrap();
        assert_eq!(labels(&c), ["hat"]);
        // Tras un número o dentro de un sufijo, nada.
        assert!(at(&format!("{SONG}track(\"d\", amen):steps(\"1 . 3|")).is_none());
        assert!(at(&format!("{SONG}track(\"d\", amen):steps(\"kick^|")).is_none());
        // Sin etiquetas conocidas, nada.
        let lines = vec!["x:steps(\"k".to_string()];
        assert!(context(&lines, 0, 10, Vec::new, &[], &[]).is_none());
    }

    #[test]
    fn scales_offer_their_methods_names_and_tonics() {
        let c = at("local menor = scale(\"minor\")\nmenor:|").unwrap();
        assert_eq!(labels(&c), ["pitch", "melody"]);
        assert_eq!(c.matches()[0].kind, "método de escala");
        let c = at("scale(\"mi|").unwrap();
        assert_eq!(labels(&c), ["minor", "mixolydian", "minor_pentatonic"]);
        let c = at("scale(\"minor\", \"|").unwrap();
        assert_eq!(labels(&c).len(), api::NOTES.len());
        let h = help("scale(\"minor\"):mel|ody(\"1\")").unwrap();
        assert_eq!(h.signature, "escala:melody(grados [, golpe])");
    }

    #[test]
    fn fill_offers_its_effects() {
        let c = at(&format!("{SONG}track(\"d\", amen):fill(\"r|")).unwrap();
        assert_eq!(labels(&c), ["roll", "reverse"]);
        assert_eq!(c.matches()[0].kind, "efecto de fill");
        let h = help(&format!("{SONG}track(\"d\", amen):fill(\"roll\", |")).unwrap();
        assert_eq!(h.description, "n: cada cuántos compases (1 a 64)");
    }

    #[test]
    fn finds_labels_named_in_the_text() {
        let text = "amen:name(1, \"kick\"):name(3,'snare')\nb:name(2, \"12\") -- no vale\namen:name(4, \"kick\")";
        assert_eq!(text_labels(text), ["kick", "snare"]);
    }

    #[test]
    fn nothing_inside_steps_other_strings_comments_or_numbers() {
        assert!(at(&format!("{SONG}track(\"d\", amen):steps(\"1 . 2|")).is_none());
        // Una palabra sin etiqueta que empiece así: sin opciones.
        assert!(
            at(&format!("{SONG}track(\"d\", amen):steps(\"1 . z|\")"))
                .unwrap()
                .matches()
                .is_empty()
        );
        assert!(at("print(\"hola |").is_none());
        assert!(at("-- un comentario tr|").is_none());
        assert!(at("bpm(17|").is_none());
        assert!(at("math.fl|").is_none());
    }

    #[test]
    fn comments_with_accents_keep_positions() {
        let c =
            at("-- línea con tildes: ñandú\ntrack(\"d\", sample(\"a.wav\"):slice(2)):g|").unwrap();
        assert_eq!(labels(&c), ["gain"]);
    }

    fn help(text: &str) -> Option<Help> {
        let lines: Vec<String> = text.lines().map(String::from).collect();
        let (row, line) = lines
            .iter()
            .enumerate()
            .find(|(_, l)| l.contains('|'))
            .expect("falta el cursor |");
        let col = line[..line.find('|').unwrap()].chars().count();
        let lines: Vec<String> = lines.iter().map(|l| l.replace('|', "")).collect();
        help_at(&lines, row, col)
    }

    #[test]
    fn help_for_the_word_under_the_cursor() {
        let h = help("b|pm(170)").unwrap();
        assert_eq!(h.signature, "bpm(n)");
        assert_eq!(h.description, "Tempo en pulsos por minuto, de 20 a 400.");
        let h = help(&format!("{SONG}track(\"d\", amen):ga|in(0.5)")).unwrap();
        assert_eq!(h.signature, "pista:gain(g) → pista");
        // Mismo nombre en el compás de every: su propia ayuda.
        let h = help(&format!(
            "{SONG}track(\"d\", amen):every(2, function(c) c:gain|(1) end)"
        ))
        .unwrap();
        assert_eq!(h.signature, "c:gain(g) → compás");
        assert!(help("local x| = 1").is_none());
        assert!(help("-- bpm|").is_none());
    }

    #[test]
    fn help_for_the_call_around_the_cursor_and_its_parameter() {
        let h = help(&format!(
            "{SONG}track(\"d\", amen):chance(0.2, \"retrig\", |4)"
        ))
        .unwrap();
        assert_eq!(h.signature, "pista:chance(p, efecto [, valor]) → pista");
        assert_eq!(
            h.description,
            "valor: el valor del efecto, si lo lleva · retrig: Retrigger del golpe"
        );
        let h = help(&format!("{SONG}track(\"d\", amen):steps(\"1 . |")).unwrap();
        assert_eq!(h.description, "texto: 16 pasos separados por espacios");
        let h = help("local a = sample(\"am|").unwrap();
        assert_eq!(h.signature, "sample(ruta) → sample");
        // En la tabla de fx: el efecto y, sobre una clave, su rango.
        let h = help(&format!("{SONG}track(\"d\", amen):fx(\"filter\", {{ |")).unwrap();
        assert_eq!(h.signature, "fx(\"filter\", { ... })");
        assert!(
            h.description.ends_with("Parámetros: type, cutoff, q"),
            "{}",
            h.description
        );
        let h = help(&format!(
            "{SONG}track(\"d\", amen):fx(\"filter\", {{ cutoff = |"
        ))
        .unwrap();
        assert_eq!(
            h.description,
            "cutoff: frecuencia de corte en Hz (20–20000, obligatorio)"
        );
        let h = help(&format!(
            "{SONG}track(\"d\", amen):fx(\"bitcrush\", {{ bits = 4, downsample = |"
        ))
        .unwrap();
        assert_eq!(
            h.description,
            "downsample: repite cada muestra n veces (1–64, por defecto 1)"
        );
        assert!(help("print(1, |").is_none());
    }

    #[test]
    fn finds_track_names_and_locals() {
        let text = "local a, b = 1, 2\nlocal function f() end\ntrack('x', a)\nmytrack(\"no\")\ntrack(\"y\")";
        assert_eq!(track_names(text), ["x", "y"]);
        assert_eq!(locals(text), ["a", "b", "f"]);
    }
}
