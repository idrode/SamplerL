//! Paleta de comandos (F1): todas las acciones del editor en una lista que
//! se filtra escribiendo. Las acciones nuevas van aquí en vez de gastar
//! atajos de teclado. La tabla de la paleta de `docs/GUIA.md` se genera
//! desde [`ACTIONS`] y los snippets del registro de la API (ver
//! [`guide_markdown`]).

use script::api;

/// Acción de la paleta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Undo,
    Redo,
    ToggleComment,
    DuplicateLines,
    MoveLinesUp,
    MoveLinesDown,
    Complete,
    SaveAndLoad,
    Load,
    Transport,
    GotoError,
    NextTrack,
    PlayMode,
    ReloadExternal,
    Export,
    Commands,
    LabelSlice,
    Mute,
    Solo,
    ClearMuteSolo,
    ResetMeters,
    Metronome,
    DetectBpm,
    Preview,
    SliceEarlier,
    SliceLater,
    ClearSliceAdjust,
    NextSlice,
    PreviousSlice,
    SplitSlice,
    JoinSlice,
    Resample,
    SaveBlock,
    InsertBlock,
    UpdateBlock,
    DeleteBlock,
    YankSlot,
    PasteSlot,
    Quit,
}

/// Una acción con su nombre, descripción y atajo (si tiene).
#[derive(Debug, Clone, Copy)]
pub struct ActionDef {
    pub action: Action,
    pub name: &'static str,
    pub description: &'static str,
    /// Atajo de teclado, o vacío.
    pub keys: &'static str,
}

/// Acciones de la paleta, en el orden en que se muestran.
pub const ACTIONS: &[ActionDef] = &[
    ActionDef {
        action: Action::Undo,
        name: "Deshacer",
        description: "Deshace el último cambio del texto",
        keys: "Ctrl-Z o Ctrl-U",
    },
    ActionDef {
        action: Action::Redo,
        name: "Rehacer",
        description: "Rehace lo último deshecho",
        keys: "Ctrl-R",
    },
    ActionDef {
        action: Action::ToggleComment,
        name: "Comentar o descomentar líneas",
        description: "Pone o quita `-- ` en la línea del cursor o en las seleccionadas",
        keys: "",
    },
    ActionDef {
        action: Action::DuplicateLines,
        name: "Duplicar líneas",
        description: "Copia la línea del cursor (o las seleccionadas) debajo",
        keys: "",
    },
    ActionDef {
        action: Action::MoveLinesUp,
        name: "Subir líneas",
        description: "Mueve la línea del cursor (o las seleccionadas) una arriba",
        keys: "",
    },
    ActionDef {
        action: Action::MoveLinesDown,
        name: "Bajar líneas",
        description: "Mueve la línea del cursor (o las seleccionadas) una abajo",
        keys: "",
    },
    ActionDef {
        action: Action::Complete,
        name: "Autocompletar",
        description: "Abre el autocompletado en el cursor",
        keys: "F4",
    },
    ActionDef {
        action: Action::SaveAndLoad,
        name: "Guardar y cargar",
        description: "Guarda el script y lo carga (entra en el siguiente compás)",
        keys: "Ctrl-S",
    },
    ActionDef {
        action: Action::Load,
        name: "Cargar sin guardar",
        description: "Carga el texto del editor sin guardarlo",
        keys: "Ctrl-L o F5",
    },
    ActionDef {
        action: Action::Transport,
        name: "Marcha / paro",
        description: "Arranca o para el secuenciador",
        keys: "Ctrl-T o F8",
    },
    ActionDef {
        action: Action::GotoError,
        name: "Ir al error",
        description: "Lleva el cursor a la línea del error",
        keys: "Ctrl-G",
    },
    ActionDef {
        action: Action::NextTrack,
        name: "Siguiente pista",
        description: "Selecciona la pista siguiente (vista del sample y modo tocar)",
        keys: "Ctrl-O",
    },
    ActionDef {
        action: Action::PlayMode,
        name: "Modo tocar",
        description: "Entra o sale del modo tocar",
        keys: "Esc",
    },
    ActionDef {
        action: Action::ReloadExternal,
        name: "Recargar del disco",
        description: "Recarga el archivo si cambió fuera del editor",
        keys: "F3",
    },
    ActionDef {
        action: Action::Export,
        name: "Exportar la mezcla",
        description: "Renderiza la canción entera a `<script>-mezcla.wav`",
        keys: "",
    },
    ActionDef {
        action: Action::Commands,
        name: "Línea de órdenes",
        description: "Abre la línea de órdenes (exportar, guardar, abrir, muestras)",
        keys: "F2",
    },
    ActionDef {
        action: Action::Mute,
        name: "Mute de la pista",
        description: "Silencia la pista seleccionada, o la vuelve a dejar sonar (en vivo, sin esperar al compás)",
        keys: "m (modo tocar)",
    },
    ActionDef {
        action: Action::Solo,
        name: "Solo de la pista",
        description: "Pone la pista seleccionada en solo, o se lo quita (en vivo)",
        keys: "s (modo tocar)",
    },
    ActionDef {
        action: Action::ClearMuteSolo,
        name: "Quitar mute y solo",
        description: "Quita todos los mute y solo en vivo (los del script siguen)",
        keys: "",
    },
    ActionDef {
        action: Action::DetectBpm,
        name: "Detectar el BPM del sample",
        description: "Propone el tempo y los compases del sample de la pista; Enter pone bpm() en el script",
        keys: "",
    },
    ActionDef {
        action: Action::Metronome,
        name: "Metrónomo",
        description: "Enciende o apaga el clic en cada pulso (no entra en lo exportado salvo que se pida)",
        keys: "",
    },
    ActionDef {
        action: Action::ResetMeters,
        name: "Reiniciar medidores",
        description: "Pone a cero los medidores y quita los avisos de limitador y saturación",
        keys: "",
    },
    ActionDef {
        action: Action::LabelSlice,
        name: "Etiquetar el corte seleccionado",
        description: "Abre «etiquetar N » para escribir la etiqueta (kick, snare...); se guarda en el proyecto",
        keys: "",
    },
    ActionDef {
        action: Action::Preview,
        name: "Preescuchar el corte seleccionado",
        description: "Lo toca en una voz aparte, sin pasar por la pista ni tocar el script",
        keys: "espacio (modo tocar)",
    },
    ActionDef {
        action: Action::SliceEarlier,
        name: "Adelantar el corte 5 ms",
        description: "Mueve el inicio del corte seleccionado 5 ms antes (se guarda en el proyecto y manda sobre el detector)",
        keys: ", (modo tocar; < 1 ms)",
    },
    ActionDef {
        action: Action::SliceLater,
        name: "Retrasar el corte 5 ms",
        description: "Mueve el inicio del corte seleccionado 5 ms después",
        keys: ". (modo tocar; > 1 ms)",
    },
    ActionDef {
        action: Action::ClearSliceAdjust,
        name: "Quitar el ajuste del corte",
        description: "El corte seleccionado vuelve a empezar donde lo pone el detector",
        keys: "",
    },
    ActionDef {
        action: Action::NextSlice,
        name: "Corte siguiente",
        description: "Selecciona el corte siguiente de la pista (sin tocarlo)",
        keys: "",
    },
    ActionDef {
        action: Action::PreviousSlice,
        name: "Corte anterior",
        description: "Selecciona el corte anterior de la pista (sin tocarlo)",
        keys: "",
    },
    ActionDef {
        action: Action::SplitSlice,
        name: "Partir el corte seleccionado",
        description: "Añade un corte en su mitad (se guarda en el proyecto; las etiquetas y ajustes siguientes se corren)",
        keys: "",
    },
    ActionDef {
        action: Action::JoinSlice,
        name: "Unir el corte seleccionado con el anterior",
        description: "Quita su marca: el anterior se alarga hasta donde acababa este (se guarda en el proyecto)",
        keys: "",
    },
    ActionDef {
        action: Action::Resample,
        name: "Remuestrear la pista seleccionada",
        description: "Abre «remuestrear pista »: graba su salida (render offline) como un WAV nuevo en la carpeta de samples",
        keys: "",
    },
    ActionDef {
        action: Action::SaveBlock,
        name: "Guardar la selección como bloque",
        description: "Abre «bloque » para escribir nombre, etiquetas y `-- descripción`; va a la biblioteca global",
        keys: "",
    },
    ActionDef {
        action: Action::InsertBlock,
        name: "Insertar un bloque",
        description: "Lista filtrable (nombre, etiquetas, descripción) con vista previa; Tab salta entre sus campos `${...}`",
        keys: "",
    },
    ActionDef {
        action: Action::UpdateBlock,
        name: "Actualizar un bloque con la selección",
        description: "Elige un bloque y sustituye su texto por la selección (conserva nombre, descripción y etiquetas); Enter dos veces",
        keys: "",
    },
    ActionDef {
        action: Action::DeleteBlock,
        name: "Borrar un bloque",
        description: "Elige un bloque y borra su archivo; Enter dos veces",
        keys: "",
    },
    ActionDef {
        action: Action::YankSlot,
        name: "Guardar en una ranura (1-9)",
        description: "Guarda la selección (o la línea del cursor) en la ranura del número que se pulse, como los registros de vim",
        keys: "",
    },
    ActionDef {
        action: Action::PasteSlot,
        name: "Pegar una ranura (1-9)",
        description: "Pega la ranura del número que se pulse; las líneas enteras van encima de la del cursor",
        keys: "",
    },
    ActionDef {
        action: Action::Quit,
        name: "Salir",
        description: "Sale (pide confirmación si hay cambios sin guardar)",
        keys: "Ctrl-Q",
    },
];

/// Lo que ejecuta una entrada de la paleta.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Item {
    Action(Action),
    /// Insertar un snippet del registro de la API.
    Snippet(&'static str),
}

/// Entrada de la paleta.
#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    pub item: Item,
    pub name: String,
    pub description: String,
    pub keys: String,
}

/// Todas las entradas; la última usada (si la hay) va primero.
pub fn entries(last: Option<Item>) -> Vec<Entry> {
    let mut all: Vec<Entry> = ACTIONS
        .iter()
        .map(|a| Entry {
            item: Item::Action(a.action),
            name: a.name.into(),
            description: a.description.into(),
            keys: a.keys.into(),
        })
        .collect();
    all.extend(api::SNIPPETS.iter().map(|s| Entry {
        item: Item::Snippet(s.name),
        name: format!("Snippet: {}", s.name),
        description: s.summary.into(),
        keys: String::new(),
    }));
    if let Some(i) = all.iter().position(|e| Some(e.item) == last) {
        let entry = all.remove(i);
        all.insert(0, entry);
    }
    all
}

/// Minúsculas y sin tildes, para comparar.
fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            c => c,
        })
        .collect()
}

/// Entradas cuyo nombre contiene cada palabra de la búsqueda (sin
/// distinguir mayúsculas ni tildes), en su orden.
pub fn filter(entries: &[Entry], query: &str) -> Vec<Entry> {
    let words: Vec<String> = query.split_whitespace().map(fold).collect();
    entries
        .iter()
        .filter(|e| {
            let name = fold(&e.name);
            words.iter().all(|w| name.contains(w.as_str()))
        })
        .cloned()
        .collect()
}

/// Marcas de la tabla de la paleta en la guía.
#[cfg(test)]
const GUIDE_BEGIN: &str = "<!-- paleta:inicio (generado desde tui/src/palette.rs) -->";
#[cfg(test)]
const GUIDE_END: &str = "<!-- paleta:fin -->";

/// Tabla de la paleta para `docs/GUIA.md` (la comprueba un test).
#[cfg(test)]
fn guide_markdown() -> String {
    let mut out = String::from("| Acción | Qué hace | Atajo |\n| --- | --- | --- |\n");
    for a in ACTIONS {
        let keys = if a.keys.is_empty() { "—" } else { a.keys };
        out.push_str(&format!("| {} | {} | {keys} |\n", a.name, a.description));
    }
    let names: Vec<String> = api::SNIPPETS
        .iter()
        .map(|s| format!("`{}`", s.name))
        .collect();
    out.push_str(&format!(
        "| Snippet: {} | Inserta el snippet en el cursor (ver «Snippets» en la sección 4) | — |\n",
        names.join(", ")
    ));
    out
}

/// Sustituye la tabla generada de la guía. `None` si faltan las marcas.
#[cfg(test)]
fn update_guide(guide: &str) -> Option<String> {
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

    fn names(entries: &[Entry]) -> Vec<String> {
        entries.iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn filters_by_every_word_without_accents() {
        let all = entries(None);
        assert_eq!(
            names(&filter(&all, "lineas")),
            [
                "Comentar o descomentar líneas",
                "Duplicar líneas",
                "Subir líneas",
                "Bajar líneas"
            ]
        );
        assert_eq!(names(&filter(&all, "BAJ lin")), ["Bajar líneas"]);
        assert_eq!(filter(&all, "").len(), ACTIONS.len() + api::SNIPPETS.len());
        assert_eq!(names(&filter(&all, "snip gho")), ["Snippet: ghost"]);
        assert!(filter(&all, "nada de nada").is_empty());
    }

    #[test]
    fn the_last_action_goes_first() {
        let all = entries(Some(Item::Action(Action::MoveLinesDown)));
        assert_eq!(all[0].name, "Bajar líneas");
        let all = entries(Some(Item::Snippet("fill")));
        assert_eq!(all[0].name, "Snippet: fill");
    }

    #[test]
    fn the_guide_lists_the_palette() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/GUIA.md");
        let guide = std::fs::read_to_string(path).unwrap();
        let updated = update_guide(&guide).expect("faltan las marcas paleta:inicio/paleta:fin");
        if std::env::var_os("ACTUALIZAR_GUIA").is_some() {
            std::fs::write(path, &updated).unwrap();
            return;
        }
        assert!(
            updated == guide,
            "docs/GUIA.md no coincide con tui/src/palette.rs: \
             ACTUALIZAR_GUIA=1 cargo test -p tui the_guide_lists_the_palette"
        );
    }
}
