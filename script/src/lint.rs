//! El linter del editor: avisa de lo que está mal en el texto **sin
//! ejecutarlo**, mientras se escribe. Dos capas:
//!
//! 1. **Sintaxis** ([`syntax`]): el texto se compila (no se ejecuta) con un
//!    intérprete de Lua desechable, sin bibliotecas, solo en modo texto (un
//!    trozo precompilado se rechaza) y con el límite de memoria del script.
//!    El mensaje de Lua se traduce ([`spanish`]).
//! 2. **API** ([`api_lints`]): contra el registro único ([`crate::api`]),
//!    solo con lo que es literal en el texto: funciones y métodos que no
//!    existen (con «¿quisiste decir…?»), claves desconocidas en las tablas
//!    de parámetros, valores de una lista cerrada que no están en ella,
//!    tipos evidentemente erróneos, curvas donde no se admiten y números
//!    fuera del rango documentado; y, como pista, un método que no es del
//!    objeto sobre el que se llama cuando se sabe seguro qué objeto es.
//!
//! Conservador: lo que no se sabe sin ejecutar (variables, código generado,
//! nombres que el script define, `_G`, `load`, `setmetatable`...) se deja
//! en paz. Nada de aquí toca el motor ni lo que suena.

use std::ops::Range;

use mlua::chunk::ChunkMode;
use mlua::{Lua, LuaOptions, StdLib};

use crate::api::{self, Effect, FxParam, Receiver};

/// Lo grave que es.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    /// Falla seguro al cargar (o al llegar a esa línea).
    Error,
    /// Muy probablemente falla (un número fuera del rango documentado).
    Warning,
    /// Parece un despiste, pero no se puede saber del todo.
    Hint,
}

impl Severity {
    pub fn label(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "aviso",
            Severity::Hint => "pista",
        }
    }
}

/// Un diagnóstico del linter.
#[derive(Debug, Clone, PartialEq)]
pub struct Lint {
    /// Fila (desde 0).
    pub row: usize,
    /// Columnas (en caracteres) que se subrayan; vacío, ninguna.
    pub cols: Range<usize>,
    pub severity: Severity,
    pub message: String,
}

/// Los dos análisis, ordenados por posición. `memory` es el límite de
/// memoria del intérprete de la sintaxis (el de `[script] memoria`).
pub fn check(source: &str, memory: usize) -> Vec<Lint> {
    let mut out: Vec<Lint> = syntax(source, memory).into_iter().collect();
    out.extend(api_lints(source));
    out.sort_by_key(|l| (l.row, l.cols.start, l.severity));
    out.dedup_by(|a, b| a.row == b.row && a.cols == b.cols && a.message == b.message);
    out
}

// ---------------------------------------------------------------------
// Sintaxis

/// El nombre del trozo en los mensajes de Lua (`texto:3: ...`).
const CHUNK: &str = "texto";

/// El error de sintaxis del texto, si lo hay. Compila sin ejecutar nada.
/// `None` también si el análisis no puede hacerse (sin memoria...): no se
/// marca nada que no se sepa.
pub fn syntax(source: &str, memory: usize) -> Option<Lint> {
    let lua = Lua::new_with(StdLib::NONE, LuaOptions::new()).ok()?;
    lua.set_memory_limit(memory.max(1 << 20)).ok()?;
    let result = lua
        .load(source)
        .set_name(format!("={CHUNK}"))
        .set_mode(ChunkMode::Text)
        .into_function();
    match result {
        Ok(_) => None,
        Err(mlua::Error::SyntaxError { message, .. }) => Some(locate(&message, source)),
        Err(_) => None,
    }
}

/// Fila, columnas y mensaje en español de un error de sintaxis de Lua.
fn locate(message: &str, source: &str) -> Lint {
    let lines: Vec<&str> = source.split('\n').collect();
    let rest = message
        .strip_prefix(CHUNK)
        .and_then(|r| r.strip_prefix(':'))
        .unwrap_or(message);
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let line: usize = digits.parse().unwrap_or(1);
    let text = rest[digits.len()..].trim_start_matches(':').trim();
    let mut row = line.saturating_sub(1).min(lines.len().saturating_sub(1));
    let mut cols = 0..0;
    // Lo que no se cerró, en la línea que lo abre (más útil que el final).
    if let Some((what, opener, at)) = to_close(text)
        && at >= 1
        && at <= lines.len()
    {
        row = at - 1;
        cols = find_word(lines[row], &opener).unwrap_or(0..0);
        return Lint {
            row,
            cols,
            severity: Severity::Error,
            message: format!("sintaxis: falta «{what}» para cerrar el «{opener}» de la línea {at}"),
        };
    }
    if let Some(token) = near(text) {
        let line_text = lines.get(row).copied().unwrap_or("");
        if token == "<eof>" {
            let len = line_text.chars().count();
            cols = len.saturating_sub(1)..len;
        } else if let Some(c) = find_text(line_text, token) {
            cols = c;
        }
    }
    if cols.is_empty() {
        let len = lines.get(row).map_or(0, |l| l.chars().count());
        cols = 0..len;
    }
    Lint {
        row,
        cols,
        severity: Severity::Error,
        message: format!("sintaxis: {}", spanish(text)),
    }
}

/// `'end' expected (to close 'function' at line 3) near <eof>`: lo que
/// falta, lo que abre y su línea.
fn to_close(text: &str) -> Option<(String, String, usize)> {
    let what = text.strip_prefix('\'')?;
    let (what, rest) = what.split_once('\'')?;
    let rest = rest.strip_prefix(" expected (to close '")?;
    let (opener, rest) = rest.split_once('\'')?;
    let rest = rest.strip_prefix(" at line ")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    Some((what.to_string(), opener.to_string(), digits.parse().ok()?))
}

/// El token de «near X» (sin sus comillas).
fn near(text: &str) -> Option<&str> {
    let at = text.rfind(" near ")?;
    let token = &text[at + 6..];
    Some(
        token
            .strip_prefix('\'')
            .and_then(|t| t.strip_suffix('\''))
            .unwrap_or(token),
    )
}

/// El mensaje de un error de sintaxis de Lua, en español (lo que no se
/// conoce se deja como viene).
pub fn spanish(text: &str) -> String {
    let (body, token) = match text.rfind(" near ") {
        Some(at) => (&text[..at], Some(&text[at + 6..])),
        None => (text, None),
    };
    let token = token.map(|t| match t {
        "<eof>" => "el final del texto".to_string(),
        t => format!("«{}»", t.trim_matches('\'')),
    });
    let at = |phrase: &str| match &token {
        Some(t) => format!("{phrase} {t}"),
        None => String::new(),
    };
    let quoted = |s: &str| s.trim_matches('\'').to_string();
    if body == "unexpected symbol" {
        format!("símbolo inesperado{}", at(":"))
    } else if body == "unfinished string" {
        "cadena sin cerrar (falta la comilla del final en esta línea)".into()
    } else if body == "unfinished long string" {
        "cadena larga [[ ]] sin cerrar".into()
    } else if body == "unfinished long comment" {
        "comentario largo --[[ ]] sin cerrar".into()
    } else if body == "malformed number" {
        format!("número mal escrito{}", at(":"))
    } else if body == "syntax error" {
        format!("error de sintaxis{}", at(" en"))
    } else if body == "function arguments expected" {
        format!("faltan los argumentos de la llamada{}", at(" antes de"))
    } else if body == "<name> expected" {
        format!("falta un nombre{}", at(" antes de"))
    } else if body == "'=' or 'in' expected" {
        format!("falta «=» o «in» en el for{}", at(" antes de"))
    } else if body == "invalid escape sequence" || body.starts_with("invalid escape sequence") {
        format!("secuencia de escape no válida{}", at(":"))
    } else if let Some(name) = body.strip_suffix(" expected") {
        format!("falta «{}»{}", quoted(name), at(" antes de"))
    } else if let Some(rest) = body.strip_prefix("break outside a loop") {
        let _ = rest;
        "break fuera de un bucle".into()
    } else if body.starts_with("cannot use '...' outside a vararg function") {
        "«...» fuera de una función con argumentos variables".into()
    } else if let Some(rest) = body.strip_prefix("no visible label ") {
        format!(
            "no hay ninguna etiqueta {} a la vista del goto",
            rest.trim_end_matches(" for goto")
        )
    } else if let Some(rest) = body.strip_prefix("attempt to assign to const variable ") {
        format!("{} es constante: no se le puede asignar", rest)
    } else {
        match &token {
            Some(t) => format!("{body} (cerca de {t})"),
            None => body.to_string(),
        }
    }
}

// ---------------------------------------------------------------------
// Léxico

#[derive(Debug, Clone, PartialEq)]
enum Kind {
    Name(String),
    /// Cadena, con su contenido (sin comillas ni escapes resueltos).
    Str(String),
    Num(String),
    Sym(String),
}

#[derive(Debug, Clone)]
struct Token {
    kind: Kind,
    row: usize,
    /// Columnas en su fila (una cadena de varias líneas: hasta el final de
    /// la primera).
    cols: Range<usize>,
}

impl Token {
    fn sym(&self, s: &str) -> bool {
        matches!(&self.kind, Kind::Sym(x) if x == s)
    }
    fn name(&self) -> Option<&str> {
        match &self.kind {
            Kind::Name(n) => Some(n),
            _ => None,
        }
    }
    fn keyword(&self, k: &str) -> bool {
        self.name() == Some(k)
    }
}

const KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in",
    "local", "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

/// Los tokens del texto, sin comentarios. Tolerante: lo que no entiende
/// lo salta (la sintaxis la dice la otra capa).
fn lex(source: &str) -> Vec<Token> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Vec::new();
    let (mut i, mut row, mut line_start) = (0usize, 0usize, 0usize);
    // Salta un bloque largo `[==[ ... ]==]` que empieza en `i` (en el `[`
    // de dentro ya); devuelve dónde acaba.
    let long_end = |from: usize, level: usize, row: &mut usize, line_start: &mut usize| {
        let mut j = from;
        while j < chars.len() {
            if chars[j] == '\n' {
                *row += 1;
                *line_start = j + 1;
            }
            if chars[j] == ']'
                && chars.get(j + 1 + level) == Some(&']')
                && (1..=level).all(|k| chars.get(j + k) == Some(&'='))
            {
                return j + level + 2;
            }
            j += 1;
        }
        chars.len()
    };
    // `[`, `=`*, `[` en `at`: el nivel.
    let long_open = |at: usize| -> Option<usize> {
        if chars.get(at) != Some(&'[') {
            return None;
        }
        let mut level = 0;
        while chars.get(at + 1 + level) == Some(&'=') {
            level += 1;
        }
        (chars.get(at + 1 + level) == Some(&'[')).then_some(level)
    };
    while i < chars.len() {
        let c = chars[i];
        if c == '\n' {
            row += 1;
            line_start = i + 1;
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        let col = i - line_start;
        if c == '-' && chars.get(i + 1) == Some(&'-') {
            if let Some(level) = long_open(i + 2) {
                i = long_end(i + 4 + level, level, &mut row, &mut line_start);
            } else {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            continue;
        }
        if let Some(level) = long_open(i) {
            let start_row = row;
            let begin = i + 2 + level;
            let end = long_end(begin, level, &mut row, &mut line_start);
            let content: String = chars[begin..end.saturating_sub(level + 2).max(begin)]
                .iter()
                .collect();
            let end_col = if row == start_row {
                end - line_start
            } else {
                col + 2
            };
            out.push(Token {
                kind: Kind::Str(content),
                row: start_row,
                cols: col..end_col,
            });
            i = end;
            continue;
        }
        if c == '"' || c == '\'' {
            let mut j = i + 1;
            let mut content = String::new();
            while j < chars.len() && chars[j] != c && chars[j] != '\n' {
                if chars[j] == '\\' && j + 1 < chars.len() {
                    content.push(chars[j]);
                    content.push(chars[j + 1]);
                    if chars[j + 1] == '\n' {
                        row += 1;
                        line_start = j + 2;
                    }
                    j += 2;
                    continue;
                }
                content.push(chars[j]);
                j += 1;
            }
            let end = (j + 1).min(chars.len());
            out.push(Token {
                kind: Kind::Str(content),
                row,
                cols: col..(end - line_start).max(col + 1),
            });
            i = end;
            continue;
        }
        if c.is_ascii_digit() || (c == '.' && chars.get(i + 1).is_some_and(char::is_ascii_digit)) {
            let mut j = i;
            while j < chars.len() {
                let d = chars[j];
                let exponent = matches!(d, '+' | '-')
                    && j > i
                    && matches!(chars[j - 1], 'e' | 'E' | 'p' | 'P')
                    && !(chars[i..j].iter().any(|&x| x == 'x' || x == 'X')
                        && matches!(chars[j - 1], 'e' | 'E'));
                if d.is_ascii_alphanumeric() || d == '.' || exponent {
                    j += 1;
                } else {
                    break;
                }
            }
            out.push(Token {
                kind: Kind::Num(chars[i..j].iter().collect()),
                row,
                cols: col..j - line_start,
            });
            i = j;
            continue;
        }
        if c.is_ascii_alphabetic() || c == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            out.push(Token {
                kind: Kind::Name(chars[i..j].iter().collect()),
                row,
                cols: col..j - line_start,
            });
            i = j;
            continue;
        }
        let three: String = chars[i..(i + 3).min(chars.len())].iter().collect();
        let two: String = chars[i..(i + 2).min(chars.len())].iter().collect();
        let sym = if three == "..." {
            three
        } else if ["==", "~=", "<=", ">=", "//", "::", "..", "<<", ">>"].contains(&two.as_str()) {
            two
        } else {
            c.to_string()
        };
        let len = sym.chars().count();
        out.push(Token {
            kind: Kind::Sym(sym),
            row,
            cols: col..col + len,
        });
        i += len;
    }
    out
}

// ---------------------------------------------------------------------
// API

/// Lo que se sabe del texto entero.
struct Scope {
    /// Nombres que el script define de alguna forma: asignados, locales,
    /// funciones, parámetros, variables de `for`, campos de tablas.
    defined: std::collections::HashSet<String>,
    /// Parámetros, variables de `for` y nombres de funciones (su tipo no se
    /// sabe).
    opaque: std::collections::HashSet<String>,
    /// Cuántas veces se asigna cada nombre (`x = ...` o `local x = ...`).
    assigned: std::collections::HashMap<String, usize>,
    /// El texto usa algo que define nombres en tiempo de ejecución (`_G`,
    /// `load`, `setmetatable`...): no se avisa de nombres desconocidos.
    dynamic: bool,
}

fn scope(tokens: &[Token]) -> Scope {
    let mut s = Scope {
        defined: Default::default(),
        opaque: Default::default(),
        assigned: Default::default(),
        dynamic: false,
    };
    for (i, t) in tokens.iter().enumerate() {
        let next = tokens.get(i + 1);
        let prev = i.checked_sub(1).map(|p| &tokens[p]);
        if let Some(name) = t.name() {
            if matches!(
                name,
                "_G" | "_ENV" | "load" | "rawset" | "setmetatable" | "getmetatable" | "require"
            ) {
                s.dynamic = true;
            }
            if next.is_some_and(|n| n.sym("=")) {
                s.defined.insert(name.to_string());
                if !prev.is_some_and(|p| p.sym(".") || p.sym(":")) {
                    *s.assigned.entry(name.to_string()).or_default() += 1;
                }
            }
        }
        // `local a, b <const>, c`
        if t.keyword("local") {
            let mut j = i + 1;
            if tokens.get(j).is_some_and(|t| t.keyword("function")) {
                continue;
            }
            while let Some(name) = tokens.get(j).and_then(Token::name) {
                s.defined.insert(name.to_string());
                if !tokens.get(j + 1).is_some_and(|t| t.sym("=")) {
                    *s.assigned.entry(name.to_string()).or_default() += 1;
                }
                j += 1;
                if tokens.get(j).is_some_and(|t| t.sym("<")) {
                    j += 3;
                }
                if tokens.get(j).is_some_and(|t| t.sym(",")) {
                    j += 1;
                } else {
                    break;
                }
            }
        }
        // `function a.b:c(x, y)` y `function(x, y)`
        if t.keyword("function") {
            let mut j = i + 1;
            while let Some(tok) = tokens.get(j) {
                if let Some(name) = tok.name() {
                    s.defined.insert(name.to_string());
                    s.opaque.insert(name.to_string());
                } else if !(tok.sym(".") || tok.sym(":")) {
                    break;
                }
                j += 1;
            }
            if tokens.get(j).is_some_and(|t| t.sym("(")) {
                j += 1;
                while let Some(tok) = tokens.get(j) {
                    if tok.sym(")") {
                        break;
                    }
                    if let Some(name) = tok.name() {
                        s.defined.insert(name.to_string());
                        s.opaque.insert(name.to_string());
                    }
                    j += 1;
                }
            }
        }
        // `for a, b in` y `for i =`
        if t.keyword("for") {
            let mut j = i + 1;
            while let Some(tok) = tokens.get(j) {
                if let Some(name) = tok.name().filter(|n| *n != "in") {
                    s.defined.insert(name.to_string());
                    s.opaque.insert(name.to_string());
                } else if !tok.sym(",") {
                    break;
                }
                j += 1;
            }
        }
        // `["clave"] = ...`
        if t.sym("[")
            && let (Some(Kind::Str(key)), Some(close), Some(eq)) = (
                tokens.get(i + 1).map(|t| &t.kind),
                tokens.get(i + 2),
                tokens.get(i + 3),
            )
            && close.sym("]")
            && eq.sym("=")
        {
            s.defined.insert(key.clone());
        }
    }
    s
}

/// El índice del token que cierra el que abre en `open` (`(`, `{` o `[`).
fn closing(tokens: &[Token], open: usize) -> Option<usize> {
    let (a, b) = match &tokens[open].kind {
        Kind::Sym(s) if s == "(" => ("(", ")"),
        Kind::Sym(s) if s == "{" => ("{", "}"),
        Kind::Sym(s) if s == "[" => ("[", "]"),
        _ => return None,
    };
    let mut depth = 0usize;
    for (j, t) in tokens.iter().enumerate().skip(open) {
        if t.sym(a) {
            depth += 1;
        } else if t.sym(b) {
            depth -= 1;
            if depth == 0 {
                return Some(j);
            }
        }
    }
    None
}

/// El índice del token que abre el que cierra en `close`.
fn opening(tokens: &[Token], close: usize) -> Option<usize> {
    let (a, b) = match &tokens[close].kind {
        Kind::Sym(s) if s == ")" => ("(", ")"),
        Kind::Sym(s) if s == "}" => ("{", "}"),
        Kind::Sym(s) if s == "]" => ("[", "]"),
        _ => return None,
    };
    let mut depth = 0usize;
    for j in (0..=close).rev() {
        if tokens[j].sym(b) {
            depth += 1;
        } else if tokens[j].sym(a) {
            depth -= 1;
            if depth == 0 {
                return Some(j);
            }
        }
    }
    None
}

/// Si el token en `at` empieza los argumentos de una llamada: `(`, `{` o
/// una cadena.
fn starts_call(tokens: &[Token], at: usize) -> bool {
    tokens
        .get(at)
        .is_some_and(|t| t.sym("(") || t.sym("{") || matches!(t.kind, Kind::Str(_)))
}

/// Los argumentos de la llamada cuyos argumentos empiezan en `at`: los
/// rangos de tokens de cada uno, y el índice tras la llamada.
fn call_args(tokens: &[Token], at: usize) -> Option<(Vec<Range<usize>>, usize)> {
    let t = tokens.get(at)?;
    if matches!(t.kind, Kind::Str(_)) {
        return Some((std::iter::once(at..at + 1).collect(), at + 1));
    }
    let close = closing(tokens, at)?;
    if t.sym("{") {
        return Some((std::iter::once(at..close + 1).collect(), close + 1));
    }
    Some((split(tokens, at + 1, close), close + 1))
}

/// Parte `from..to` por las comas (o `;`) de primer nivel.
fn split(tokens: &[Token], from: usize, to: usize) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut start = from;
    for (j, t) in tokens.iter().enumerate().take(to).skip(from) {
        if t.sym("(") || t.sym("{") || t.sym("[") {
            depth += 1;
        } else if t.sym(")") || t.sym("}") || t.sym("]") {
            depth -= 1;
        } else if depth == 0 && (t.sym(",") || t.sym(";")) {
            out.push(start..j);
            start = j + 1;
        }
    }
    if start < to {
        out.push(start..to);
    }
    out
}

/// Las funciones globales de la API.
fn global_names() -> Vec<&'static str> {
    api::functions(Receiver::Global).map(|f| f.name).collect()
}

/// Los métodos de la API (de cualquier objeto).
fn method_names() -> Vec<&'static str> {
    let mut names: Vec<&str> = api::FUNCTIONS
        .iter()
        .filter(|f| f.receiver != Receiver::Global)
        .map(|f| f.name)
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Distancia de edición (con trasposiciones) entre dos nombres.
fn distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// El nombre de `candidates` más parecido a `name`, si lo hay y es único:
/// a 1 de distancia (2 en los nombres de más de 5 letras), o igual sin
/// distinguir mayúsculas.
pub fn suggestion<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    let limit = if name.chars().count() > 5 { 2 } else { 1 };
    let lower = name.to_lowercase();
    let mut best: Option<(usize, &str)> = None;
    let mut tie = false;
    for &c in candidates {
        let d = if c.to_lowercase() == lower {
            0
        } else {
            distance(name, c)
        };
        if d > limit {
            continue;
        }
        match best {
            Some((b, other)) if d == b && other != c => tie = true,
            Some((b, _)) if d >= b => {}
            _ => {
                best = Some((d, c));
                tie = false;
            }
        }
    }
    best.filter(|_| !tie).map(|(_, c)| c)
}

fn did_you_mean(name: &str, candidates: &[&str]) -> String {
    match suggestion(name, candidates) {
        Some(s) => format!("; ¿quisiste decir «{s}»?"),
        None => String::new(),
    }
}

/// Lo que el linter sabe del texto: los tokens, los nombres y los del
/// registro.
struct Ctx {
    tokens: Vec<Token>,
    scope: Scope,
    globals: Vec<&'static str>,
    methods: Vec<&'static str>,
    out: Vec<Lint>,
}

impl Ctx {
    fn push(&mut self, at: &Token, severity: Severity, message: String) {
        self.out.push(Lint {
            row: at.row,
            cols: at.cols.clone(),
            severity,
            message,
        });
    }

    /// Un nombre de la API o de Lua que el script no ha redefinido.
    fn api_global(&self, name: &str) -> bool {
        self.globals.contains(&name) && !self.scope.defined.contains(name)
    }
}

/// Los diagnósticos de la API del texto (ver la cabecera del módulo).
pub fn api_lints(source: &str) -> Vec<Lint> {
    let tokens = lex(source);
    let scope = scope(&tokens);
    let mut ctx = Ctx {
        tokens,
        scope,
        globals: global_names(),
        methods: method_names(),
        out: Vec::new(),
    };
    for i in 0..ctx.tokens.len() {
        names(&mut ctx, i);
        tables(&mut ctx, i);
    }
    ctx.out
}

/// Funciones y métodos que no existen, y métodos de otro objeto.
fn names(ctx: &mut Ctx, i: usize) {
    let tokens = &ctx.tokens;
    let Some(name) = tokens[i].name() else { return };
    if KEYWORDS.contains(&name) || !starts_call(tokens, i + 1) {
        return;
    }
    let prev = i.checked_sub(1).map(|p| &tokens[p]);
    // En la declaración de una función (`function a:b()`), no es llamada.
    let declared = {
        let mut j = i;
        while j > 0 && (tokens[j - 1].sym(".") || tokens[j - 1].sym(":")) && j >= 2 {
            j -= 2;
        }
        j > 0 && tokens[j - 1].keyword("function")
    };
    if declared {
        return;
    }
    let name = name.to_string();
    if prev.is_some_and(|p| p.sym(".")) {
        return;
    }
    if prev.is_some_and(|p| p.sym(":")) {
        // Un método.
        let known = ctx.methods.contains(&name.as_str())
            || api::LUA_STRING_METHODS.contains(&name.as_str())
            || ctx.scope.defined.contains(&name);
        if !known {
            if ctx.scope.dynamic {
                return;
            }
            let hint = did_you_mean(&name, &ctx.methods.clone());
            let at = tokens[i].clone();
            ctx.push(
                &at,
                Severity::Error,
                format!("ningún objeto de la API tiene el método «{name}»{hint}"),
            );
            return;
        }
        // El método existe, pero ¿en este objeto?
        if ctx.scope.defined.contains(&name) {
            return;
        }
        if let Some(receiver) = i.checked_sub(2).and_then(|end| receiver_type(ctx, end, 0))
            && api::function(receiver, &name).is_none()
        {
            let owners: Vec<&str> = [
                Receiver::Sample,
                Receiver::Slices,
                Receiver::Track,
                Receiver::Bar,
                Receiver::Scale,
                Receiver::Instrument,
                Receiver::Group,
                Receiver::Master,
            ]
            .into_iter()
            .filter(|r| api::function(*r, &name).is_some())
            .map(Receiver::label)
            .collect();
            if owners.is_empty() {
                return;
            }
            let at = tokens[i].clone();
            ctx.push(
                &at,
                Severity::Hint,
                format!(
                    "«{name}» es un método de {} y aquí se llama sobre {} {}",
                    owners.join(" o de "),
                    article(receiver),
                    receiver.label()
                ),
            );
        }
        return;
    }
    // Una función global.
    let known = ctx.globals.contains(&name.as_str())
        || api::LUA_BASE.contains(&name.as_str())
        || api::LUA_LIBRARIES.contains(&name.as_str())
        || ctx.scope.defined.contains(&name);
    if known || ctx.scope.dynamic {
        return;
    }
    let mut candidates = ctx.globals.clone();
    candidates.extend(api::LUA_BASE);
    let hint = did_you_mean(&name, &candidates);
    let at = tokens[i].clone();
    ctx.push(
        &at,
        Severity::Error,
        format!("«{name}» no es una función de la API ni está definida en el script{hint}"),
    );
}

fn article(r: Receiver) -> &'static str {
    match r {
        Receiver::Slices => "unos",
        Receiver::Track | Receiver::Scale => "una",
        _ => "un",
    }
}

/// El objeto que es la expresión que acaba en el token `end` (el de antes
/// del `:` de un método), si se sabe seguro.
fn receiver_type(ctx: &Ctx, end: usize, depth: usize) -> Option<Receiver> {
    if depth > 8 {
        return None;
    }
    let tokens = &ctx.tokens;
    let last = tokens.get(end)?;
    // Una llamada: `nombre(...)`, `nombre{...}` o `nombre"..."`.
    let callee = if last.sym(")") || last.sym("}") {
        opening(tokens, end)?.checked_sub(1)?
    } else if matches!(last.kind, Kind::Str(_)) {
        end.checked_sub(1)?
    } else {
        let var = last.name()?;
        if end > 0 && (tokens[end - 1].sym(".") || tokens[end - 1].sym(":")) {
            return None;
        }
        return variable_type(ctx, var, depth + 1);
    };
    let name = tokens.get(callee)?.name()?;
    let before = callee.checked_sub(1).map(|b| &tokens[b]);
    match before {
        Some(b) if b.sym(":") => {
            let receiver = receiver_type(ctx, callee - 2, depth + 1)?;
            api::function(receiver, name)?.returns
        }
        Some(b) if b.sym(".") => None,
        _ if ctx.api_global(name) => api::function(Receiver::Global, name)?.returns,
        _ => None,
    }
}

/// El objeto de una variable asignada una sola vez con `local x = ...`
/// (con una expresión que es solo una cadena de llamadas).
fn variable_type(ctx: &Ctx, var: &str, depth: usize) -> Option<Receiver> {
    if ctx.scope.opaque.contains(var) || ctx.scope.assigned.get(var) != Some(&1) {
        return None;
    }
    let tokens = &ctx.tokens;
    let at = (1..tokens.len()).find(|&j| {
        tokens[j].name() == Some(var)
            && tokens[j - 1].keyword("local")
            && tokens.get(j + 1).is_some_and(|t| t.sym("="))
    })?;
    // La expresión: de tras el `=` hasta donde acaba la cadena de llamadas.
    let mut j = at + 2;
    tokens.get(j)?.name()?;
    j += 1;
    loop {
        match tokens.get(j) {
            Some(t) if t.sym("(") || t.sym("{") => j = closing(tokens, j)? + 1,
            Some(t) if matches!(t.kind, Kind::Str(_)) => j += 1,
            Some(t) if t.sym(":") => {
                tokens.get(j + 1)?.name()?;
                j += 2;
            }
            _ => break,
        }
    }
    // Lo que sigue tiene que empezar otra sentencia (nada de `or`, `..`).
    if let Some(next) = tokens.get(j) {
        let ends = matches!(&next.kind, Kind::Name(n) if !matches!(n.as_str(), "and" | "or"))
            || next.sym(";");
        if !ends {
            return None;
        }
    }
    receiver_type(ctx, j - 1, depth)
}

/// Las tablas literales de parámetros de las llamadas de la API.
fn tables(ctx: &mut Ctx, i: usize) {
    let tokens = &ctx.tokens;
    let Some(name) = tokens[i].name() else { return };
    if !starts_call(tokens, i + 1) {
        return;
    }
    let method = i > 0 && tokens[i - 1].sym(":");
    if i > 0 && tokens[i - 1].sym(".") {
        return;
    }
    // Una función del script con ese nombre no es la de la API.
    if ctx.scope.defined.contains(name) || (!method && !ctx.globals.contains(&name)) {
        return;
    }
    let name = name.to_string();
    let Some((args, _)) = call_args(tokens, i + 1) else {
        return;
    };
    let strings: Vec<Option<String>> = args
        .iter()
        .map(|a| match (&tokens[a.start].kind, a.len()) {
            (Kind::Str(s), 1) => Some(s.clone()),
            _ => None,
        })
        .collect();
    let strs: Vec<Option<&str>> = strings.iter().map(Option::as_deref).collect();
    // Nombres de una lista cerrada en el primer argumento.
    if !method && let (Some(first), Some(Some(value))) = (args.first(), strs.first()) {
        let at = tokens[first.start].clone();
        let (what, names): (&str, Vec<&str>) = match name.as_str() {
            "synth" => ("sintetizador", api::SYNTHS.iter().map(|e| e.name).collect()),
            "bus" => ("bus", api::BUSES.iter().map(|e| e.name).collect()),
            _ => ("", Vec::new()),
        };
        if !names.is_empty() && !names.contains(value) {
            let hint = did_you_mean(value, &names);
            ctx.push(
                &at,
                Severity::Error,
                format!(
                    "{name}: no hay ningún {what} «{value}» (hay: {}){hint}",
                    names.join(", ")
                ),
            );
            return;
        }
    }
    for (k, arg) in args.iter().enumerate() {
        let tokens = &ctx.tokens;
        if !(tokens[arg.start].sym("{") && closing(tokens, arg.start) == Some(arg.end - 1)) {
            continue;
        }
        let Some(effect) = api::call_table(&name, k, &strs) else {
            continue;
        };
        let layer = api::layer_call(&name, k, &strs);
        check_table(ctx, arg.start, effect, layer);
    }
}

/// Lo que es un valor escrito tal cual.
enum Literal {
    Number(f64),
    Text(String),
    Bool,
    /// `ramp(...)`, `lfo(...)`, `lane(...)` o `macro(...)`.
    Curve,
    /// Una tabla `{ ... }` (su `{`).
    Table(usize),
    Other,
}

fn literal(ctx: &Ctx, value: Range<usize>) -> Literal {
    let tokens = &ctx.tokens;
    let parts = &tokens[value.clone()];
    match parts {
        [t] => match &t.kind {
            Kind::Num(n) => number(n).map_or(Literal::Other, Literal::Number),
            Kind::Str(s) => Literal::Text(s.clone()),
            Kind::Name(n) if n == "true" || n == "false" => Literal::Bool,
            _ => Literal::Other,
        },
        [minus, t] if minus.sym("-") => match &t.kind {
            Kind::Num(n) => number(n).map_or(Literal::Other, |n| Literal::Number(-n)),
            _ => Literal::Other,
        },
        [first, ..] if first.sym("{") && closing(tokens, value.start) == Some(value.end - 1) => {
            Literal::Table(value.start)
        }
        [callee, open, ..]
            if matches!(callee.name(), Some("ramp" | "lfo" | "lane" | "macro"))
                && ctx.api_global(callee.name().unwrap_or_default())
                && open.sym("(")
                && closing(tokens, value.start + 1) == Some(value.end - 1) =>
        {
            Literal::Curve
        }
        _ => Literal::Other,
    }
}

/// Un número de Lua (decimal o hexadecimal entero).
fn number(text: &str) -> Option<f64> {
    if let Some(hex) = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
        return i64::from_str_radix(hex, 16).ok().map(|n| n as f64);
    }
    text.parse().ok()
}

/// El rango de un parámetro numérico, si su `valid` es exactamente `a–b`.
fn range(p: &FxParam) -> Option<(f64, f64)> {
    if !p.values.is_empty() {
        return None;
    }
    let (a, b) = p.valid.split_once('–')?;
    Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
}

fn shown(n: f64) -> String {
    let text = format!("{n}");
    text.replace('.', ",")
}

/// Las claves y los valores de una tabla de parámetros literal.
fn check_table(ctx: &mut Ctx, open: usize, effect: &'static Effect, layer: bool) {
    let Some(close) = closing(&ctx.tokens, open) else {
        return;
    };
    let who = match api::effect(effect.name) {
        Some(_) => format!("fx(\"{}\")", effect.name),
        None => effect.name.to_string(),
    };
    for item in split(&ctx.tokens, open + 1, close) {
        let tokens = &ctx.tokens;
        // `clave = valor` (los elementos sin clave no se miran).
        let (key, key_at, value) = match &tokens[item.clone()] {
            [k, eq, ..] if eq.sym("=") && k.name().is_some() => (
                k.name().unwrap_or_default().to_string(),
                k.clone(),
                item.start + 2..item.end,
            ),
            [open, k, close, eq, ..] if open.sym("[") && close.sym("]") && eq.sym("=") => {
                match &k.kind {
                    Kind::Str(s) => (s.clone(), k.clone(), item.start + 4..item.end),
                    _ => continue,
                }
            }
            _ => continue,
        };
        if value.is_empty() {
            continue;
        }
        let Some(param) = effect.param(&key) else {
            let names: Vec<&str> = effect.params.iter().map(|p| p.name).collect();
            let hint = match suggestion(&key, &names) {
                Some(s) => format!("; ¿quisiste decir «{s}»?"),
                None => format!(" (admite: {})", names.join(", ")),
            };
            ctx.push(
                &key_at,
                Severity::Error,
                format!("{who}: no tiene el parámetro «{key}»{hint}"),
            );
            continue;
        };
        let value_at = Token {
            kind: Kind::Sym(String::new()),
            row: tokens[value.start].row,
            cols: tokens[value.start].cols.start
                ..if tokens[value.end - 1].row == tokens[value.start].row {
                    tokens[value.end - 1].cols.end
                } else {
                    tokens[value.start].cols.end
                },
        };
        match literal(ctx, value) {
            Literal::Curve => {
                if let api::Auto::No(why) = param.automate {
                    ctx.push(
                        &value_at,
                        Severity::Error,
                        format!("{who}: {key} no admite ramp() ni lfo(): {why}"),
                    );
                }
            }
            Literal::Text(text) if !param.values.is_empty() => {
                if !param.values.contains(&text.as_str()) {
                    let hint = match suggestion(&text, param.values) {
                        Some(s) => format!("; ¿quisiste decir «{s}»?"),
                        None => String::new(),
                    };
                    ctx.push(
                        &value_at,
                        Severity::Error,
                        format!(
                            "{who}: {key} = «{text}» no vale (valen: {}){hint}",
                            param.values.join(", ")
                        ),
                    );
                }
            }
            Literal::Text(text) => {
                if let Some((a, b)) = range(param) {
                    match text.trim().parse::<f64>() {
                        Ok(n) => out_of_range(ctx, &value_at, &who, &key, n, (a, b)),
                        Err(_) => ctx.push(
                            &value_at,
                            Severity::Error,
                            format!(
                                "{who}: {key} es un número ({}), no el texto «{text}»",
                                param.valid
                            ),
                        ),
                    }
                }
            }
            Literal::Bool => {
                if range(param).is_some() {
                    ctx.push(
                        &value_at,
                        Severity::Error,
                        format!(
                            "{who}: {key} es un número ({}), no true ni false",
                            param.valid
                        ),
                    );
                }
            }
            Literal::Number(n) => {
                if let Some(bounds) = range(param) {
                    out_of_range(ctx, &value_at, &who, &key, n, bounds);
                }
            }
            Literal::Table(at) => {
                if layer {
                    check_table(ctx, at, &api::LAYER_TABLE, false);
                }
            }
            Literal::Other => {}
        }
    }
}

fn out_of_range(ctx: &mut Ctx, at: &Token, who: &str, key: &str, n: f64, (a, b): (f64, f64)) {
    if n < a || n > b {
        ctx.push(
            at,
            Severity::Warning,
            format!(
                "{who}: {key} = {} queda fuera del rango ({} a {})",
                shown(n),
                shown(a),
                shown(b)
            ),
        );
    }
}

/// Las columnas (en caracteres) de la primera vez que sale `needle` en
/// `line`.
fn find_text(line: &str, needle: &str) -> Option<Range<usize>> {
    let byte = line.find(needle)?;
    let start = line[..byte].chars().count();
    Some(start..start + needle.chars().count())
}

/// Como [`find_text`], solo como palabra entera.
fn find_word(line: &str, word: &str) -> Option<Range<usize>> {
    let mut from = 0;
    while let Some(at) = line[from..].find(word) {
        let byte = from + at;
        let before = line[..byte].chars().last();
        let after = line[byte + word.len()..].chars().next();
        let ident = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        if !ident(before) && !ident(after) {
            let start = line[..byte].chars().count();
            return Some(start..start + word.chars().count());
        }
        from = byte + word.len();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const MB: usize = 64 << 20;

    fn lints(source: &str) -> Vec<Lint> {
        check(source, MB)
    }

    fn one(source: &str) -> Lint {
        let l = lints(source);
        assert_eq!(l.len(), 1, "{source}\n{l:#?}");
        l.into_iter().next().unwrap()
    }

    #[test]
    fn syntax_errors_say_where_and_what_in_spanish() {
        let l = one("bpm(120)\ntrack(\"a\" amen)\n");
        assert_eq!((l.row, l.severity), (1, Severity::Error));
        assert!(
            l.message.starts_with("sintaxis: falta «)» antes de «amen»"),
            "{}",
            l.message
        );
        assert_eq!(l.cols, 10..14);
        // Lo que no se cierra, en la línea que lo abre.
        let l = one("track(\"a\"):every(4, function(t)\n  t:reverse()\n");
        assert_eq!(l.row, 0);
        assert!(
            l.message
                .contains("falta «end» para cerrar el «function» de la línea 1"),
            "{}",
            l.message
        );
        assert_eq!(l.cols, 20..28);
        let l = one("local x = \"abc\nbpm(1)\n");
        assert_eq!(l.row, 0);
        assert!(l.message.contains("cadena sin cerrar"), "{}", l.message);
        let l = one("bpm(120))\n");
        assert!(l.message.contains("símbolo inesperado"), "{}", l.message);
        let l = one("local x = 3..4\n");
        assert!(l.message.contains("número mal escrito"), "{}", l.message);
        assert!(lints("bpm(120)\n-- [[ comentario\n").is_empty());
    }

    #[test]
    fn syntax_never_runs_the_script() {
        // Un bucle infinito arriba del todo: se compila y ya.
        let start = std::time::Instant::now();
        assert!(lints("while true do end\nbpm(120)\n").is_empty());
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        // Sin bibliotecas: nada se podría llamar aunque se ejecutara, y no
        // hace falta.
        assert!(lints("os.exit(1)\nio.open('x', 'w')\n").is_empty());
        // Código precompilado: rechazado (solo texto).
        assert!(syntax("\x1bLua\x54\x00", MB).is_some());
    }

    #[test]
    fn unknown_functions_and_methods_with_a_suggestion() {
        let l = one("trak(\"a\")\n");
        assert_eq!(l.severity, Severity::Error);
        assert!(
            l.message.contains("¿quisiste decir «track»?"),
            "{}",
            l.message
        );
        assert_eq!((l.row, l.cols.clone()), (0, 0..4));
        let l = one("track(\"a\"):stesp(\"1 . 1 .\")\n");
        assert!(l.message.contains("método «stesp»"), "{}", l.message);
        assert!(
            l.message.contains("¿quisiste decir «steps»?"),
            "{}",
            l.message
        );
        // Lo que define el script, Lua y las cadenas: nada.
        let ok = "local function rep(x) return x end\n\
                  rep(1)\n\
                  local s = (\"x\"):rep(3):upper()\n\
                  print(string.format(\"%d\", math.floor(1.5)))\n\
                  local t = { go = function(self) end }\n\
                  t:go()\n\
                  function t.hola() end\n\
                  t.hola()\n";
        assert!(lints(ok).is_empty(), "{:#?}", lints(ok));
        // Con _G o load, los nombres pueden venir de cualquier parte.
        assert!(lints("_G[\"trak\"] = track\ntrak(\"a\")\n").is_empty());
    }

    #[test]
    fn unknown_table_keys_values_types_curves_and_ranges() {
        let l = one("track(\"a\"):fx(\"filter\", { cutof = 800 })\n");
        assert_eq!(l.severity, Severity::Error);
        assert!(
            l.message.contains("«cutof»; ¿quisiste decir «cutoff»?"),
            "{}",
            l.message
        );
        let l = one("track(\"a\"):fx(\"filter\", { type = \"lowpas\" })\n");
        assert!(
            l.message.contains("¿quisiste decir «lowpass»?"),
            "{}",
            l.message
        );
        let l = one("track(\"a\"):fx(\"filter\", { cutoff = \"mucho\" })\n");
        assert!(l.message.contains("es un número"), "{}", l.message);
        let l = one("track(\"a\"):fx(\"filter\", { cutoff = 50000 })\n");
        assert_eq!(l.severity, Severity::Warning);
        assert!(
            l.message.contains("fuera del rango (20 a 20000)"),
            "{}",
            l.message
        );
        let l = one("track(\"a\"):fx(\"filter\", { type = ramp(0, 1, 4) })\n");
        assert!(l.message.contains("no admite ramp()"), "{}", l.message);
        // Bien escrito, variables y curvas donde se admiten: nada.
        let ok = "local c = 3000\n\
                  track(\"a\"):fx(\"filter\", { type = \"highpass\", cutoff = c, q = 0.7 })\n\
                  track(\"a\"):fx(\"filter\", { cutoff = ramp(200, 4000, 8) })\n\
                  track(\"a\"):fx(\"filter\", { cutoff = \"800\" })\n";
        assert!(lints(ok).is_empty(), "{:#?}", lints(ok));
        // En los sintetizadores y en sus capas.
        let l = one("track(\"b\", synth(\"acdi\", {}))\n");
        assert!(
            l.message.contains("¿quisiste decir «acid»?"),
            "{}",
            l.message
        );
        let l = one("track(\"b\", subharm{ o1 = { dvi = 2 } })\n");
        assert!(l.message.contains("«dvi»"), "{}", l.message);
    }

    #[test]
    fn a_method_of_another_object_is_a_hint_only_when_sure() {
        let l = one("local amen = sample(\"amen.wav\"):slice(8)\namen:steps(\"1 . 1 .\")\n");
        assert_eq!(l.severity, Severity::Hint);
        assert!(
            l.message.contains("«steps» es un método de pista"),
            "{}",
            l.message
        );
        assert!(l.message.contains("unos cortes"), "{}", l.message);
        // Asignada dos veces, parámetro o con `or`: no se sabe.
        let unsure = "local amen = sample(\"amen.wav\"):slice(8)\n\
                      amen = track(\"a\", amen)\n\
                      amen:steps(\"1 . 1 .\")\n\
                      track(\"t\"):every(4, function(amen) amen:steps(\"1\") end)\n\
                      local x = sample(\"a.wav\") or track(\"b\")\n\
                      x:steps(\"1\")\n";
        assert!(lints(unsure).is_empty(), "{:#?}", lints(unsure));
    }

    #[test]
    fn suggestions_are_unique_and_close() {
        assert_eq!(suggestion("trak", &["track", "bpm"]), Some("track"));
        assert_eq!(suggestion("Track", &["track"]), Some("track"));
        assert_eq!(suggestion("zzz", &["track", "bpm"]), None);
        // Dos igual de cerca: ninguna.
        assert_eq!(suggestion("ab", &["ac", "ad"]), None);
    }

    #[test]
    fn lexing_skips_comments_and_long_strings() {
        let src = "--[[ trak(1)\n]] bpm(1) -- trak(2)\nlocal s = [==[ trak(3) ]==]\n";
        assert!(lints(src).is_empty(), "{:#?}", lints(src));
    }
}
