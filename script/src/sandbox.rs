//! Lo que se cambia de la biblioteca de Lua para que el límite de tiempo
//! no se pueda rodear (N4 de `docs/AUDITORIA-2.md`).
//!
//! El hook de [`crate::ScriptHost::eval`] lanza un error al pasar el
//! límite. Tres cosas lo esquivaban:
//!
//! - `pcall` y `xpcall` se tragaban ese error: un bucle que llama a `pcall`
//!   no acababa nunca. Aquí, una vez pasado el límite, devuelven el error
//!   del límite en vez de atraparlo (lo marca [`Expired`]).
//! - Un finalizador (`__gc`) corre con los hooks apagados: un bucle dentro
//!   no se cortaba (ni al cerrar el intérprete). `setmetatable` rechaza una
//!   metatabla con `__gc` (un `__gc` añadido después no cuenta en Lua 5.4:
//!   el objeto solo se marca al poner la metatabla).
//! - Las funciones de patrones de `string` son C: el hook no las ve, y un
//!   patrón con muchos cuantificadores sobre una cadena larga hace vuelta
//!   atrás durante horas. Se rechaza antes de empezar si su coste en el
//!   peor caso pasa de [`MAX_PATTERN_COST`] (ver [`pattern_cost`]).
//!
//! Lo que quede (otra función de C que tarde) lo para el vigilante de
//! [`crate::ScriptHost::eval`], que abandona la carga.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use mlua::{Function, Lua, MultiValue, Table, Value};

/// El mensaje del límite, una vez pasado (lo pone el hook). Mientras sea
/// `Some`, `pcall` y `xpcall` no atrapan nada.
pub(crate) type Expired = Rc<RefCell<Option<String>>>;

/// Coste máximo, en el peor caso, de buscar un patrón (ver
/// [`pattern_cost`]): unos pocos segundos de C.
pub const MAX_PATTERN_COST: f64 = 1e12;

/// Líneas de `print` que llegan a los avisos de una carga.
pub const MAX_PRINTS: usize = 20;
/// Caracteres de cada línea de `print` que se muestran.
const PRINT_CHARS: usize = 200;

/// Lo que se quita o se cambia de la biblioteca base (N9 de
/// `docs/AUDITORIA-2.md`): `dofile` y `loadfile` leían cualquier archivo
/// como Lua (el script es uno solo y el vigilante de cambios no veía esos
/// archivos), y `print` escribía en la salida estándar, encima de la
/// pantalla de la interfaz. Ahora `print` deja su línea en los avisos de la
/// carga (`warnings`), con dónde se llamó; como mucho [`MAX_PRINTS`].
pub(crate) fn restrict(lua: &Lua, warnings: &Rc<RefCell<Vec<String>>>) -> mlua::Result<()> {
    let globals = lua.globals();
    globals.raw_remove("dofile")?;
    globals.raw_remove("loadfile")?;
    let tostring: Function = globals.get("tostring")?;
    let warnings = warnings.clone();
    let printed = Rc::new(Cell::new(0usize));
    globals.set(
        "print",
        lua.create_function(move |lua, args: MultiValue| {
            let n = printed.get();
            printed.set(n + 1);
            if n < MAX_PRINTS {
                let mut parts = Vec::new();
                for value in args {
                    parts.push(tostring.call::<String>(value)?);
                }
                let mut text: String = parts.join("\t").chars().take(PRINT_CHARS).collect();
                if text.chars().count() == PRINT_CHARS {
                    text.push('…');
                }
                warnings
                    .borrow_mut()
                    .push(format!("{}print: {text}", crate::location(lua)));
            } else if n == MAX_PRINTS {
                warnings.borrow_mut().push(format!(
                    "print: hay más líneas (solo se muestran las {MAX_PRINTS} primeras)"
                ));
            }
            Ok(())
        })?,
    )?;
    Ok(())
}

/// Instala los cambios en `lua`.
pub(crate) fn harden(lua: &Lua, expired: &Expired) -> mlua::Result<()> {
    let globals = lua.globals();
    for name in ["pcall", "xpcall"] {
        let original: Function = globals.get(name)?;
        let expired = expired.clone();
        globals.set(
            name,
            lua.create_function(move |_, args: MultiValue| {
                let result: MultiValue = original.call(args)?;
                match expired.borrow().as_ref() {
                    Some(message) => Err(mlua::Error::runtime(message.clone())),
                    None => Ok(result),
                }
            })?,
        )?;
    }

    let original: Function = globals.get("setmetatable")?;
    globals.set(
        "setmetatable",
        lua.create_function(move |lua, (table, meta): (Table, Value)| {
            if let Value::Table(meta) = &meta
                && !meta.raw_get::<Value>("__gc")?.is_nil()
            {
                return Err(crate::fail(
                    lua,
                    "setmetatable: __gc no está permitido (un finalizador no tiene límite de \
                     tiempo)",
                ));
            }
            original.call::<Table>((table, meta))
        })?,
    )?;

    let string: Table = globals.get("string")?;
    // (función, si admite `init` en el tercer argumento y `plain` en el
    // cuarto, como `find`)
    for (name, plain) in [
        ("find", true),
        ("match", false),
        ("gmatch", false),
        ("gsub", false),
    ] {
        let original: Function = string.get(name)?;
        let qualified = format!("string.{name}");
        string.set(
            name,
            lua.create_function(move |lua, args: MultiValue| {
                let subject = args.front().and_then(as_bytes);
                let pattern = args.get(1).and_then(as_bytes);
                let is_plain = plain && args.get(3).is_some_and(|v| v.as_boolean() == Some(true));
                if let (Some(subject), Some(pattern), false) = (subject, pattern, is_plain) {
                    let cost = pattern_cost(subject.len(), &pattern);
                    if cost > MAX_PATTERN_COST {
                        return Err(crate::fail(
                            lua,
                            format!(
                                "{qualified}: el patrón es demasiado costoso para una cadena de \
                                 {} caracteres (con tantos cuantificadores, la búsqueda podría \
                                 no acabar)",
                                subject.len()
                            ),
                        ));
                    }
                }
                original.call::<MultiValue>(args)
            })?,
        )?;
    }
    Ok(())
}

/// Los bytes de una cadena o de un número (Lua convierte los números en
/// las funciones de `string`).
fn as_bytes(value: &Value) -> Option<Vec<u8>> {
    match value {
        Value::String(s) => Some(s.as_bytes().to_vec()),
        Value::Integer(i) => Some(i.to_string().into_bytes()),
        Value::Number(n) => Some(n.to_string().into_bytes()),
        _ => None,
    }
}

/// Pasos de la búsqueda de `pattern` en una cadena de `len` bytes, en el
/// peor caso: cada cuantificador (`*`, `+`, `-`, `?`) puede probar todas
/// las longitudes, y sin `^` se prueba desde cada posición, así que
/// `len^(cuantificadores + 1)` (o `len^cuantificadores` anclado). Con uno
/// o ninguno, la búsqueda es como mucho cuadrática y no se limita (una
/// cadena cabe en la memoria del intérprete): solo cuenta con dos o más.
pub fn pattern_cost(len: usize, pattern: &[u8]) -> f64 {
    let anchored = pattern.first() == Some(&b'^');
    let mut quantifiers = 0i32;
    let mut i = usize::from(anchored);
    while i < pattern.len() {
        let single = match pattern[i] {
            b'%' => {
                // `%b()` y `%f[...]` no son una clase de un carácter.
                match pattern.get(i + 1) {
                    Some(b'b') => {
                        i += 4;
                        false
                    }
                    Some(b'f') => {
                        i += 2;
                        i = skip_set(pattern, i);
                        false
                    }
                    _ => {
                        i += 2;
                        true
                    }
                }
            }
            b'[' => {
                i = skip_set(pattern, i);
                true
            }
            b'(' | b')' => {
                i += 1;
                false
            }
            _ => {
                i += 1;
                true
            }
        };
        if single && matches!(pattern.get(i), Some(b'*' | b'+' | b'-' | b'?')) {
            quantifiers += 1;
            i += 1;
        }
    }
    if quantifiers < 2 {
        return 0.0;
    }
    let exponent = quantifiers + i32::from(!anchored);
    (len.max(1) as f64).powi(exponent)
}

/// Salta un conjunto `[...]` que empieza en `i` (con `]` y `^` al principio
/// y `%` escapando); devuelve dónde acaba.
fn skip_set(pattern: &[u8], mut i: usize) -> usize {
    if pattern.get(i) != Some(&b'[') {
        return i;
    }
    i += 1;
    if pattern.get(i) == Some(&b'^') {
        i += 1;
    }
    // Un `]` justo al principio es parte del conjunto.
    if pattern.get(i) == Some(&b']') {
        i += 1;
    }
    while i < pattern.len() && pattern[i] != b']' {
        i += if pattern[i] == b'%' { 2 } else { 1 };
    }
    i + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cost_counts_quantifiers_of_single_classes() {
        // Uno o ninguno: sin límite.
        assert_eq!(pattern_cost(1_000_000, b"%s+"), 0.0);
        assert_eq!(pattern_cost(1_000_000, b"abc"), 0.0);
        assert_eq!(pattern_cost(1_000_000, b"^(%w+)"), 0.0);
        // Dos, sin anclar: len³.
        assert_eq!(pattern_cost(100, b"(%w+)=(%w+)"), 1e6);
        // Anclado: len².
        assert_eq!(pattern_cost(100, b"^(%w+)=(%w*)$"), 1e4);
        // Dentro de un conjunto, `-` y `*` no cuentan.
        assert_eq!(pattern_cost(100, b"[a-z*]+[%-]?"), 1e6);
        // El caso de la auditoría: 25 cuantificadores.
        assert!(pattern_cost(5000, "a-".repeat(25).as_bytes()) > MAX_PATTERN_COST);
        // `%b()` y `%f[%w]` no son clases de un carácter.
        assert_eq!(pattern_cost(100, b"%b()%f[%w]"), 0.0);
    }
}
