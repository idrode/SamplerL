//! Visualizaciones pedidas desde Lua: `pista:scope{...}`, `pista:spectrum`,
//! `pista:punchcard`, `pista:pianoroll`, `pista:pitchwheel`, `pista:spiral`,
//! `master():scope{...}`... y `viz_off()`. Solo se apuntan en el modelo
//! (`Song::viz`); la interfaz las dibuja y el lote enciende en el motor las
//! colas de muestras de las de audio, al compás. Si el script falla, siguen
//! las de la carga anterior.

use std::cell::RefCell;
use std::rc::Rc;

use mlua::{Lua, Table, UserData, UserDataMethods, Value};

use super::{Builder, api, fail};

/// Las que hay, por su nombre.
pub const KINDS: &[&str] = &[
    "punchcard",
    "pianoroll",
    "scope",
    "spectrum",
    "pitchwheel",
    "spiral",
];

/// Las que salen del patrón (no del audio): solo en una pista.
pub const PATTERN_KINDS: &[&str] = &["punchcard", "pianoroll"];

/// Filas por defecto, máximo, y bandas por defecto y su rango.
pub const DEFAULT_SIZE: u16 = 6;
pub const MAX_SIZE: u16 = 40;
pub const DEFAULT_BANDS: usize = 32;
pub const BANDS: std::ops::RangeInclusive<usize> = 4..=128;

/// Una visualización del modelo: qué, de qué pista (`None`: el master),
/// cuántas filas y cuántas bandas.
#[derive(Debug, Clone, PartialEq)]
pub struct VizDef {
    pub kind: &'static str,
    pub track: Option<String>,
    pub size: u16,
    pub bands: usize,
    /// Junto al código (`inline = true`), en la fila del texto donde se
    /// pidió; `None`, en su panel.
    pub inline: Option<usize>,
}

/// Lee `{ size = n, bands = n }` (claves del registro `api::VIZ_PARAMS`).
fn options(lua: &Lua, who: &str, table: Option<&Table>) -> mlua::Result<(u16, usize, bool)> {
    let (mut size, mut bands, mut inline) = (DEFAULT_SIZE, DEFAULT_BANDS, false);
    let Some(table) = table else {
        return Ok((size, bands, inline));
    };
    let known: Vec<&str> = api::VIZ_PARAMS.iter().map(|p| p.name).collect();
    for pair in table.pairs::<Value, Value>() {
        let (key, value) = pair?;
        let key = match &key {
            Value::String(k) => k.to_str()?.to_string(),
            other => format!("{other:?}"),
        };
        let integer = match value {
            Value::Integer(n) => Some(n),
            Value::Number(n) if n.fract() == 0.0 => Some(n as i64),
            _ => None,
        };
        match key.as_str() {
            "size" => {
                size = integer
                    .filter(|n| (1..=MAX_SIZE as i64).contains(n))
                    .ok_or_else(|| fail(lua, format!("{who}: size son filas, de 1 a {MAX_SIZE}")))?
                    as u16;
            }
            "inline" => {
                inline = match value {
                    Value::Boolean(on) => on,
                    _ => return Err(fail(lua, format!("{who}: inline es true o false"))),
                };
            }
            "bands" => {
                bands = integer
                    .filter(|n| BANDS.contains(&(*n as usize)) && *n > 0)
                    .ok_or_else(|| {
                        fail(
                            lua,
                            format!("{who}: bands va de {} a {}", BANDS.start(), BANDS.end()),
                        )
                    })? as usize;
            }
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
    Ok((size, bands, inline))
}

/// Apunta la visualización `kind` de `track` (sustituye a la misma de la
/// misma fuente).
pub(crate) fn add(
    lua: &Lua,
    builder: &mut Builder,
    kind: &'static str,
    track: Option<&str>,
    table: Option<&Table>,
) -> mlua::Result<()> {
    let who = match track {
        Some(_) => format!("{kind}()"),
        None => format!("master():{kind}()"),
    };
    if track.is_none() && PATTERN_KINDS.contains(&kind) {
        return Err(fail(
            lua,
            format!("{who}: sale del patrón de una pista; el master no tiene: pista:{kind}()"),
        ));
    }
    let (size, bands, inline) = options(lua, &who, table)?;
    let inline = if inline {
        Some(super::written::current_row(lua).ok_or_else(|| {
            fail(
                lua,
                format!("{who}: inline necesita la llamada escrita en el script"),
            )
        })?)
    } else {
        None
    };
    let track = track.map(str::to_string);
    builder
        .viz
        .retain(|v| !(v.kind == kind && v.track == track));
    builder.viz.push(VizDef {
        kind,
        track,
        size,
        bands,
        inline,
    });
    Ok(())
}

/// `master()` en Lua: las visualizaciones del master.
#[derive(Clone)]
pub(crate) struct MasterHandle {
    pub state: Rc<RefCell<Builder>>,
}

impl UserData for MasterHandle {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        for &kind in KINDS {
            methods.add_method(kind, move |lua, this, table: Option<Table>| {
                add(
                    lua,
                    &mut this.state.borrow_mut(),
                    kind,
                    None,
                    table.as_ref(),
                )?;
                Ok(this.clone())
            });
        }
        methods.add_method("viz_off", |_, this, ()| {
            this.state.borrow_mut().viz.retain(|v| v.track.is_some());
            Ok(this.clone())
        });
    }
}
