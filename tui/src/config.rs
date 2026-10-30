//! Configuración del usuario: `~/.config/sampler/config.toml` (o
//! `$XDG_CONFIG_HOME/sampler/config.toml`), con los atajos de teclado
//! (`[teclas]`), los colores (`[colores]`) y el resaltado en vivo del
//! código (`[resaltado]`).
//!
//! ```toml
//! [teclas]
//! guardar = "alt-s"
//! marcha = ["ctrl-t", "f8"]
//! paleta = "ctrl-p"
//!
//! [colores]
//! acento = "#ff8700"
//! sonando = "22"
//! ```
//!
//! Nunca falla: si falta o está roto, valores por defecto y un aviso; lo
//! que no se entiende se avisa y se ignora. F1 abre siempre la paleta
//! (aunque se le dé otra tecla): así ninguna configuración la deja
//! inaccesible.

use std::path::{Path, PathBuf};

use ratatui_textarea::{Input, Key};

use crate::midi::MidiConfig;
use crate::palette::Action;
use crate::theme::{self, Theme};

/// A qué lleva una tecla.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    Action(Action),
    /// Abrir la paleta de comandos.
    Palette,
}

/// Lo que se puede atar a una tecla en `[teclas]`, con sus teclas por
/// defecto.
pub const BINDABLE: &[(&str, Bound, &[&str])] = &[
    ("paleta", Bound::Palette, &["f1"]),
    ("guardar", Bound::Action(Action::SaveAndLoad), &["ctrl-s"]),
    ("cargar", Bound::Action(Action::Load), &["ctrl-l", "f5"]),
    (
        "marcha",
        Bound::Action(Action::Transport),
        &["ctrl-space", "ctrl-t", "f8"],
    ),
    (
        "cue",
        Bound::Action(Action::Cue),
        &["shift-ctrl-space", "f9"],
    ),
    ("ir_al_error", Bound::Action(Action::GotoError), &["ctrl-g"]),
    ("ir_a_linea", Bound::Action(Action::GotoLine), &["alt-g"]),
    ("buscar", Bound::Action(Action::Find), &["ctrl-f"]),
    ("buscar_siguiente", Bound::Action(Action::FindNext), &["f6"]),
    (
        "buscar_anterior",
        Bound::Action(Action::FindPrevious),
        &["f7"],
    ),
    (
        "siguiente_pista",
        Bound::Action(Action::NextTrack),
        &["ctrl-o"],
    ),
    ("recargar", Bound::Action(Action::ReloadExternal), &["f3"]),
    ("ordenes", Bound::Action(Action::Commands), &["f2"]),
    ("completar", Bound::Action(Action::Complete), &["f4"]),
    ("salir", Bound::Action(Action::Quit), &["ctrl-q"]),
    ("modo_tocar", Bound::Action(Action::PlayMode), &["esc"]),
    (
        "deshacer",
        Bound::Action(Action::Undo),
        &["ctrl-z", "ctrl-u"],
    ),
    ("rehacer", Bound::Action(Action::Redo), &["ctrl-r"]),
    ("explorador", Bound::Action(Action::Browser), &[]),
    ("tap_tempo", Bound::Action(Action::TapTempo), &[]),
    ("macros", Bound::Action(Action::Macros), &[]),
    ("metronomo", Bound::Action(Action::Metronome), &[]),
    ("exportar", Bound::Action(Action::Export), &[]),
    ("seccion_siguiente", Bound::Action(Action::NextSection), &[]),
    (
        "seccion_anterior",
        Bound::Action(Action::PreviousSection),
        &[],
    ),
    ("repetir_seccion", Bound::Action(Action::LoopSection), &[]),
    ("variante", Bound::Action(Action::ToggleVariant), &[]),
    ("resaltado", Bound::Action(Action::Highlight), &[]),
    ("ocupacion", Bound::Action(Action::Occupancy), &[]),
    ("panico", Bound::Action(Action::Panic), &["f12"]),
    (
        "pausa_panico",
        Bound::Action(Action::PanicPause),
        &["shift-f12"],
    ),
];

/// `[midi] macros`: el controlador (CC) de cada macro, por orden; `None`
/// si no es una lista de números de 0 a 127 (como mucho uno por macro).
fn midi_macros(value: &toml::Value) -> Option<[Option<u8>; engine::MAX_MACROS]> {
    let list = value.as_array()?;
    if list.len() > engine::MAX_MACROS {
        return None;
    }
    let mut out = [None; engine::MAX_MACROS];
    for (slot, v) in out.iter_mut().zip(list) {
        let n = v.as_integer().filter(|n| (0..=127).contains(n))?;
        *slot = Some(n as u8);
    }
    Some(out)
}

/// `[resaltado]`: el resaltado en vivo del código al sonar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HighlightConfig {
    /// Encendido al arrancar (`activo`).
    pub enabled: bool,
    /// Milisegundos que se suman a la latencia que dice la salida de audio
    /// (`retardo_ms`): para salidas que la dicen de menos (Bluetooth...).
    pub delay_ms: f64,
}

impl Default for HighlightConfig {
    fn default() -> Self {
        HighlightConfig {
            enabled: true,
            delay_ms: 0.0,
        }
    }
}

/// `[ocupacion]`: la vista de ocupación por paso.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct OccupancyConfig {
    /// Se ve al arrancar (`visible`).
    pub visible: bool,
}

/// Claves de `[ocupacion]`, qué hacen y su valor por defecto (para la
/// guía).
#[cfg(test)]
const OCCUPANCY_KEYS: &[(&str, &str, &str)] = &[(
    "visible",
    "enseñar la vista de ocupación al arrancar (`true`/`false`; la paleta la enseña y la oculta en vivo); sus colores, `hueco` y `sugerencia` en `[colores]`",
    "`false`",
)];

/// `[voces]`: cuántas voces suenan a la vez.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VoicesConfig {
    /// Notas de instrumento y voces de sintetizador a la vez entre todas
    /// las pistas (`limite`); al llegar, la nueva roba la más antigua.
    pub limit: usize,
}

impl Default for VoicesConfig {
    fn default() -> Self {
        VoicesConfig {
            limit: engine::DEFAULT_VOICE_LIMIT,
        }
    }
}

/// Claves de `[voces]` (para la guía).
#[cfg(test)]
const VOICES_KEYS: &[(&str, &str, &str)] = &[(
    "limite",
    "notas de instrumento y voces de sintetizador que suenan a la vez entre todas las pistas (de 1 a 256); al llegar, una nota nueva roba la más antigua de cualquier pista (con el fundido mínimo). Aparte va el límite de cada pista (`voices` en el script)",
    "`64`",
)];

/// Claves de `[dispositivos]` (para la guía).
#[cfg(test)]
const DEVICES_KEYS: &[(&str, &str, &str)] = &[
    (
        "audio",
        "dispositivo de salida de audio, por su nombre (el de `tui --list-devices` o el panel de dispositivos); se guarda solo al elegirlo en el panel; `--audio-device` manda sobre él",
        "el predeterminado",
    ),
    (
        "midi_entrada",
        "puerto MIDI de entrada, por su nombre sin los números de ALSA del final (`\"Seaboard:Seaboard MIDI 1\"`); `--midi-in` manda sobre él",
        "el puerto virtual",
    ),
    (
        "midi_salida",
        "puerto MIDI de salida, igual; `--midi-out` manda sobre él",
        "el puerto virtual",
    ),
    (
        "terminal",
        "comando que abre una terminal nueva para el panel de dispositivos en su ventana (`\"kitty --class sampler-dispositivos -e\"`); detrás va `tui --window-devices <script>`",
        "la de `[editor] terminal`",
    ),
];

/// Retardo a mano más largo del resaltado, en ms.
const MAX_HIGHLIGHT_DELAY_MS: f64 = 1000.0;

/// Claves de `[resaltado]`, qué hacen y su valor por defecto (para la
/// guía).
#[cfg(test)]
const HIGHLIGHT_KEYS: &[(&str, &str, &str)] = &[
    (
        "activo",
        "resaltar lo que suena al arrancar (`true`/`false`; la paleta lo cambia en vivo)",
        "`true`",
    ),
    (
        "estilo",
        "`\"fondo\"`, `\"subrayado\"` o `\"invertido\"`",
        "`\"fondo\"`",
    ),
    (
        "colores",
        "un color o una lista, uno por pista en el orden del script (si hay menos, se repiten)",
        "8 de la paleta de 256",
    ),
    (
        "retardo_ms",
        "ms que se suman a la latencia que dice la salida de audio (de 0 a 1000)",
        "`0`",
    ),
];

/// Una combinación de teclas: `ctrl-s`, `alt-x`, `f5`, `esc`,
/// `shift-ctrl-space`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyCombo {
    pub key: Key,
    pub ctrl: bool,
    pub alt: bool,
    /// Con Shift. Solo distingue si la terminal lo dice (Shift-Ctrl-Espacio
    /// llega como Ctrl-Espacio sin el protocolo de teclado de kitty o las
    /// teclas extendidas de tmux); una combinación sin Shift vale también
    /// con él si no hay otra que lo pida.
    pub shift: bool,
}

impl KeyCombo {
    /// Lee `ctrl-s`, `alt-x`, `shift-ctrl-space`: modificadores `ctrl-`,
    /// `alt-` y `shift-` y una letra, un número, `space`, `f1`-`f12` o
    /// `esc`. Sin `ctrl-` ni `alt-` solo valen las F y Esc (una letra o
    /// un espacio solos escribirían texto).
    pub fn parse(text: &str) -> Result<KeyCombo, String> {
        let lower = text.trim().to_lowercase();
        let (mut ctrl, mut alt, mut shift) = (false, false, false);
        let mut rest = lower.as_str();
        loop {
            if let Some(r) = rest.strip_prefix("ctrl-") {
                ctrl = true;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("alt-") {
                alt = true;
                rest = r;
            } else if let Some(r) = rest.strip_prefix("shift-") {
                shift = true;
                rest = r;
            } else {
                break;
            }
        }
        let key = match rest {
            "esc" => Key::Esc,
            "space" | "espacio" => {
                if !ctrl && !alt {
                    return Err(format!(
                        "«{text}»: un espacio sin ctrl- ni alt- escribiría texto"
                    ));
                }
                Key::Char(' ')
            }
            f if f.starts_with('f') && f.len() > 1 => {
                let n: u8 = f[1..]
                    .parse()
                    .ok()
                    .filter(|n| (1..=12).contains(n))
                    .ok_or_else(|| format!("«{text}»: tecla desconocida"))?;
                Key::F(n)
            }
            c if c.chars().count() == 1 && c.chars().all(|c| c.is_ascii_alphanumeric()) => {
                if !ctrl && !alt {
                    return Err(format!(
                        "«{text}»: una letra sin ctrl- ni alt- escribiría texto"
                    ));
                }
                Key::Char(c.chars().next().unwrap())
            }
            _ => return Err(format!("«{text}»: tecla desconocida")),
        };
        Ok(KeyCombo {
            key,
            ctrl,
            alt,
            shift,
        })
    }

    /// Si `input` es esta combinación. Sin `shift`, también con Shift (ver
    /// [`Keymap::lookup`]: antes se busca la que lo pide).
    pub fn matches(&self, input: &Input) -> bool {
        let key = match input.key {
            Key::Char(c) => Key::Char(c.to_ascii_lowercase()),
            k => k,
        };
        key == self.key
            && input.ctrl == self.ctrl
            && input.alt == self.alt
            && (input.shift || !self.shift)
    }

    /// Como se ve: `Ctrl-S`, `Alt-X`, `F5`, `Esc`, `Shift-Ctrl-Espacio`.
    pub fn text(&self) -> String {
        let mut out = String::new();
        if self.shift {
            out.push_str("Shift-");
        }
        if self.ctrl {
            out.push_str("Ctrl-");
        }
        if self.alt {
            out.push_str("Alt-");
        }
        match self.key {
            Key::Char(' ') => out.push_str("Espacio"),
            Key::Char(c) => out.push(c.to_ascii_uppercase()),
            Key::F(n) => out.push_str(&format!("F{n}")),
            Key::Esc => out.push_str("Esc"),
            _ => out.push('?'),
        }
        out
    }
}

/// Las teclas de cada acción.
#[derive(Debug, Clone, PartialEq)]
pub struct Keymap {
    bindings: Vec<(KeyCombo, Bound)>,
}

/// F1: la paleta, siempre.
const PALETTE_KEY: KeyCombo = KeyCombo {
    key: Key::F(1),
    ctrl: false,
    alt: false,
    shift: false,
};

impl Default for Keymap {
    fn default() -> Self {
        let mut bindings = Vec::new();
        for (_, bound, keys) in BINDABLE {
            for key in *keys {
                bindings.push((KeyCombo::parse(key).expect("tecla por defecto"), *bound));
            }
        }
        Keymap { bindings }
    }
}

impl Keymap {
    /// A qué lleva esta tecla (F1, siempre a la paleta).
    pub fn lookup(&self, input: &Input) -> Option<Bound> {
        if PALETTE_KEY.matches(input) {
            return Some(Bound::Palette);
        }
        // Primero la que pide Shift (Shift-Ctrl-Espacio antes que
        // Ctrl-Espacio), luego las demás.
        self.bindings
            .iter()
            .filter(|(k, _)| k.shift)
            .chain(self.bindings.iter().filter(|(k, _)| !k.shift))
            .find(|(k, _)| k.matches(input))
            .map(|(_, b)| *b)
    }

    /// Las teclas de una acción, como se ven: `Ctrl-S o F5` (vacío si no
    /// tiene).
    pub fn describe(&self, bound: Bound) -> String {
        let mut keys: Vec<String> = Vec::new();
        if bound == Bound::Palette {
            keys.push(PALETTE_KEY.text());
        }
        for (k, b) in &self.bindings {
            if *b == bound && !keys.contains(&k.text()) {
                keys.push(k.text());
            }
        }
        keys.join(" o ")
    }

    /// La primera tecla de una acción, como se ve (`Ctrl-S`).
    pub fn first(&self, bound: Bound) -> Option<String> {
        if bound == Bound::Palette {
            return Some(PALETTE_KEY.text());
        }
        self.bindings
            .iter()
            .find(|(_, b)| *b == bound)
            .map(|(k, _)| k.text())
    }

    /// Línea de ayuda con los atajos principales (`Ctrl-S guardar y cargar
    /// · ...`): la del modo editor o la del modo tocar. Sin `editor` (la
    /// ventana principal con el editor en la suya), el modo tocar no dice
    /// cómo volver al editor: ahí no hay.
    pub fn help(&self, playing: bool, editor: bool) -> String {
        let action = |a| Bound::Action(a);
        let items: &[(Bound, &str)] = if playing && !editor {
            &[
                (action(Action::NextTrack), "pista"),
                (action(Action::Transport), "marcha/paro"),
                (Bound::Palette, "paleta"),
            ]
        } else if playing {
            &[
                (action(Action::NextTrack), "pista"),
                (action(Action::Transport), "marcha/paro"),
                (Bound::Palette, "paleta"),
                (action(Action::PlayMode), "editor"),
            ]
        } else {
            &[
                (action(Action::SaveAndLoad), "guardar y cargar"),
                (action(Action::Load), "cargar"),
                (action(Action::Transport), "marcha/paro"),
                (action(Action::GotoError), "ir al error"),
                (action(Action::NextTrack), "pista"),
                (action(Action::PlayMode), "tocar"),
                (Bound::Palette, "paleta"),
                (action(Action::Commands), "órdenes"),
                (action(Action::Complete), "completar"),
                (action(Action::Quit), "salir"),
            ]
        };
        let parts: Vec<String> = items
            .iter()
            .filter_map(|(b, what)| self.first(*b).map(|k| format!("{k} {what}")))
            .collect();
        if playing {
            format!(
                "TOCAR: 1-0 q-i cortes · ←→ elegir · espacio oír · m mute · s solo · {}",
                parts.join(" · ")
            )
        } else {
            parts.join(" · ")
        }
    }

    /// Aplica `[teclas]`: las acciones que aparecen cambian sus teclas por
    /// esas; lo que no vale o choca, se avisa.
    fn apply(&mut self, table: &toml::Table, warnings: &mut Vec<String>) {
        let mut user: Vec<(KeyCombo, Bound, String)> = Vec::new();
        let mut rebound: Vec<Bound> = Vec::new();
        for (name, value) in table {
            let Some((_, bound, _)) = BINDABLE.iter().find(|(n, _, _)| n == name) else {
                warnings.push(format!(
                    "[teclas] «{name}» no existe (hay: {})",
                    BINDABLE
                        .iter()
                        .map(|(n, _, _)| *n)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
                continue;
            };
            let texts: Vec<String> = match value {
                toml::Value::String(s) => vec![s.clone()],
                toml::Value::Array(list) => list
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
                _ => {
                    warnings.push(format!("[teclas] {name}: una tecla o una lista de teclas"));
                    continue;
                }
            };
            let before = user.len();
            for text in texts {
                match KeyCombo::parse(&text) {
                    Ok(combo) if combo == PALETTE_KEY && *bound != Bound::Palette => warnings.push(
                        format!("[teclas] {name}: F1 es siempre la paleta de comandos"),
                    ),
                    Ok(combo) => match user.iter().find(|(k, _, _)| *k == combo) {
                        Some((_, _, other)) => warnings.push(format!(
                            "[teclas] {name}: {} ya es de {other}; no se usa",
                            combo.text()
                        )),
                        None => user.push((combo, *bound, name.clone())),
                    },
                    Err(e) => warnings.push(format!("[teclas] {name}: {e}")),
                }
            }
            // Con alguna tecla buena, esas sustituyen a las de siempre; si
            // no, se queda con las suyas.
            if user.len() > before {
                rebound.push(*bound);
            }
        }
        // Las de la configuración primero; las por defecto de las demás
        // acciones, si su tecla no la ha cogido otra.
        let mut bindings: Vec<(KeyCombo, Bound)> = user.iter().map(|(k, b, _)| (*k, *b)).collect();
        for (combo, bound) in &self.bindings {
            if rebound.contains(bound) {
                continue;
            }
            match user.iter().find(|(k, _, _)| k == combo) {
                Some((_, _, other)) => {
                    let name = BINDABLE.iter().find(|(_, b, _)| b == bound).unwrap().0;
                    warnings.push(format!(
                        "[teclas] {} era de {name}; ahora es de {other}",
                        combo.text()
                    ));
                }
                None => bindings.push((*combo, *bound)),
            }
        }
        self.bindings = bindings;
    }
}

/// `[editor]`, además de `terminal`: el ratón y el modo vim.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EditorConfig {
    /// El ratón en el editor (`raton`): clic, arrastrar y rueda. Sin él,
    /// la terminal selecciona como siempre.
    pub mouse: bool,
    /// `modo = "vim"` (si no, `"clasico"`, el de siempre).
    pub vim: bool,
    /// Las dos teclas que salen de Insert en vim (`salir_insert`).
    pub insert_exit: [char; 2],
}

impl Default for EditorConfig {
    fn default() -> Self {
        EditorConfig {
            mouse: true,
            vim: false,
            insert_exit: crate::app::VIM_EXIT,
        }
    }
}

/// Lo que se lee del archivo.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Config {
    pub keys: Keymap,
    pub theme: Theme,
    /// `[midi]`: nota del corte 1, canal y reloj.
    pub midi: MidiConfig,
    /// `[editor] terminal`: el comando que abre una terminal nueva para el
    /// editor en su ventana (`tui --edit … --split`); detrás va
    /// `tui --window-editor <script>`.
    pub editor_terminal: Option<Vec<String>>,
    /// `[editor]`: el ratón.
    pub editor: EditorConfig,
    /// `[resaltado]`: encendido y retardo (los colores y el estilo van en
    /// `theme`).
    pub highlight: HighlightConfig,
    /// `[ocupacion]`: si se ve al arrancar.
    pub occupancy: OccupancyConfig,
    /// `[voces]`: el límite de voces a la vez.
    pub voices: VoicesConfig,
    /// `[dispositivos]`: los elegidos la última vez.
    pub devices: crate::devices::Choice,
    /// `[dispositivos] terminal`: la terminal del panel de dispositivos en
    /// su ventana (si no, la de `[editor] terminal`).
    pub devices_terminal: Option<Vec<String>>,
    /// `[midi] mandos`: los mandos MIDI de un proyecto que no tiene los
    /// suyos (MIDI learn; ver `script::learn`).
    pub learned: Vec<script::learn::Learned>,
    /// Avisos (archivo que falta o roto, claves desconocidas, choques...).
    pub warnings: Vec<String>,
}

/// Carpeta de configuración: `$XDG_CONFIG_HOME/sampler` o
/// `~/.config/sampler`.
pub fn dir() -> Option<PathBuf> {
    script::blocks::global_dir().and_then(|blocks| blocks.parent().map(Path::to_path_buf))
}

impl Config {
    /// Lee el archivo (si falta o está roto, por defecto y un aviso).
    pub fn load(path: &Path, truecolor: bool) -> Config {
        match std::fs::read_to_string(path) {
            Ok(text) => Config::parse(&text, truecolor, &path.display().to_string()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Config {
                warnings: vec![format!(
                    "sin {}: teclas y colores por defecto (ver la guía)",
                    path.display()
                )],
                ..Config::default()
            },
            Err(e) => Config {
                warnings: vec![format!("{}: {e}; valores por defecto", path.display())],
                ..Config::default()
            },
        }
    }

    /// Lee el texto de un `config.toml`.
    pub fn parse(text: &str, truecolor: bool, name: &str) -> Config {
        let mut config = Config::default();
        let table: toml::Table = match text.parse() {
            Ok(table) => table,
            Err(e) => {
                let first = e.to_string().lines().next().unwrap_or("").to_string();
                config
                    .warnings
                    .push(format!("{name} está roto ({first}); valores por defecto"));
                return config;
            }
        };
        for (section, value) in &table {
            let Some(inner) = value.as_table() else {
                config.warnings.push(format!(
                    "{name}: «{section}» tendría que ser una sección [{section}]"
                ));
                continue;
            };
            match section.as_str() {
                "teclas" => config.keys.apply(inner, &mut config.warnings),
                "colores" => {
                    for (key, value) in inner {
                        let color = value
                            .as_str()
                            .map(String::from)
                            .or_else(|| value.as_integer().map(|n| n.to_string()))
                            .and_then(|text| theme::parse(&text, truecolor));
                        match color {
                            Some(color) if config.theme.set(key, color) => {}
                            Some(_) => config.warnings.push(format!(
                                "[colores] «{key}» no existe (hay: {})",
                                theme::NAMES.join(", ")
                            )),
                            None => config
                                .warnings
                                .push(format!("[colores] {key}: color no válido")),
                        }
                    }
                }
                "midi" => {
                    for (key, value) in inner {
                        let number = value.as_integer();
                        match (key.as_str(), number, value.as_bool()) {
                            ("nota_base", Some(n), _) if (0..=127).contains(&n) => {
                                config.midi.base_note = n as u8
                            }
                            ("canal", Some(n), _) if (1..=16).contains(&n) => {
                                config.midi.channel = Some(n as u8)
                            }
                            ("reloj", _, Some(on)) => config.midi.clock = on,
                            ("mpe", _, Some(on)) => config.midi.mpe = on,
                            ("bend", Some(n), _) if (1..=96).contains(&n) => {
                                config.midi.bend = Some(n as u8)
                            }
                            ("mandos", _, _) => {
                                let lines = value.as_array().map(|list| {
                                    list.iter()
                                        .map(|v| {
                                            v.as_str()
                                                .ok_or_else(|| "cada mando es un texto".to_string())
                                                .and_then(script::learn::Learned::parse)
                                        })
                                        .collect::<Vec<_>>()
                                });
                                match lines {
                                    Some(lines) => {
                                        for line in lines {
                                            match line {
                                                Ok(l) => config.learned.push(l),
                                                Err(e) => config
                                                    .warnings
                                                    .push(format!("[midi] mandos: {e}")),
                                            }
                                        }
                                    }
                                    None => config.warnings.push(
                                        "[midi] mandos: una lista de textos \
                                         [\"1 21 drums lowpass.cutoff 200 8000 log\"]"
                                            .into(),
                                    ),
                                }
                            }
                            ("macros", _, _) => match midi_macros(value) {
                                Some(list) => config.midi.macros = list,
                                None => config.warnings.push(format!(
                                    "[midi] macros: una lista de hasta {} controladores (0-127), \
                                     uno por macro: [21, 22, 23]",
                                    engine::MAX_MACROS
                                )),
                            },
                            ("nota_base" | "canal" | "reloj" | "mpe" | "bend", _, _) => {
                                config.warnings.push(format!(
                                    "[midi] {key}: valor no válido (nota_base 0-127, canal 1-16, \
                                     reloj true/false, mpe true/false, bend 1-96 semitonos)"
                                ))
                            }
                            _ => config.warnings.push(format!(
                                "[midi] «{key}» no existe (hay: nota_base, canal, reloj, mpe, bend, macros, mandos)"
                            )),
                        }
                    }
                }
                "editor" => {
                    for (key, value) in inner {
                        match (key.as_str(), terminal_words(value)) {
                            ("terminal", Some(words)) => config.editor_terminal = Some(words),
                            ("terminal", _) => config.warnings.push(
                                "[editor] terminal: un comando, como \"kitty --class sampler-editor -e\" o una lista de palabras".into(),
                            ),
                            ("raton", _) => match value.as_bool() {
                                Some(on) => config.editor.mouse = on,
                                None => config
                                    .warnings
                                    .push("[editor] raton: true o false".into()),
                            },
                            ("modo", _) => match value.as_str() {
                                Some("vim") => config.editor.vim = true,
                                Some("clasico" | "clásico") => config.editor.vim = false,
                                _ => config
                                    .warnings
                                    .push("[editor] modo: \"clasico\" o \"vim\"".into()),
                            },
                            ("salir_insert", _) => {
                                let keys: Vec<char> =
                                    value.as_str().map_or(Vec::new(), |s| s.chars().collect());
                                match keys[..] {
                                    [a, b] if a.is_alphanumeric() && b.is_alphanumeric() => {
                                        config.editor.insert_exit = [a, b]
                                    }
                                    _ => config.warnings.push(
                                        "[editor] salir_insert: dos letras seguidas, como \"jk\"".into(),
                                    ),
                                }
                            }
                            _ => config.warnings.push(format!(
                                "[editor] «{key}» no existe (hay: terminal, raton, modo, salir_insert)"
                            )),
                        }
                    }
                }
                "resaltado" => config.apply_highlight(inner, truecolor),
                "ocupacion" | "ocupación" => {
                    for (key, value) in inner {
                        match (key.as_str(), value.as_bool()) {
                            ("visible", Some(on)) => config.occupancy.visible = on,
                            ("visible", None) => config
                                .warnings
                                .push("[ocupacion] visible: true o false".into()),
                            _ => config.warnings.push(format!(
                                "[ocupacion] «{key}» no existe (hay: visible; los colores, hueco y sugerencia, en [colores])"
                            )),
                        }
                    }
                }
                "voces" => {
                    for (key, value) in inner {
                        let limit = value
                            .as_integer()
                            .filter(|n| (1..=engine::MAX_VOICE_LIMIT as i64).contains(n));
                        match (key.as_str(), limit) {
                            ("limite" | "límite", Some(n)) => config.voices.limit = n as usize,
                            ("limite" | "límite", None) => config.warnings.push(format!(
                                "[voces] limite: un número de 1 a {}",
                                engine::MAX_VOICE_LIMIT
                            )),
                            _ => config
                                .warnings
                                .push(format!("[voces] «{key}» no existe (hay: limite)")),
                        }
                    }
                }
                "dispositivos" => {
                    for (key, value) in inner {
                        if key == "terminal" {
                            match terminal_words(value) {
                                Some(words) => config.devices_terminal = Some(words),
                                None => config.warnings.push(
                                    "[dispositivos] terminal: un comando, como \"kitty --class sampler-dispositivos -e\" o una lista de palabras".into(),
                                ),
                            }
                            continue;
                        }
                        let kind = crate::devices::Kind::from_key(key);
                        match (kind, value.as_str()) {
                            (Some(kind), Some(name)) => config
                                .devices
                                .set(kind, (!name.is_empty()).then(|| name.to_string())),
                            (Some(_), None) => config.warnings.push(format!(
                                "[dispositivos] {key}: el nombre entre comillas"
                            )),
                            (None, _) => config.warnings.push(format!(
                                "[dispositivos] «{key}» no existe (hay: audio, midi_entrada, midi_salida, terminal)"
                            )),
                        }
                    }
                }
                other => config.warnings.push(format!(
                    "{name}: sección [{other}] desconocida (hay: teclas, colores, midi, editor, resaltado, ocupacion, voces, dispositivos)"
                )),
            }
        }
        config
    }
}

impl Config {
    /// Aplica `[resaltado]`; lo que no vale se avisa y se queda como está.
    fn apply_highlight(&mut self, table: &toml::Table, truecolor: bool) {
        for (key, value) in table {
            match key.as_str() {
                "activo" => match value.as_bool() {
                    Some(on) => self.highlight.enabled = on,
                    None => self
                        .warnings
                        .push("[resaltado] activo: true o false".into()),
                },
                "estilo" => {
                    let mode = value.as_str().and_then(|text| {
                        theme::HighlightMode::NAMES
                            .iter()
                            .find(|(name, _)| *name == text)
                    });
                    match mode {
                        Some((_, mode)) => self.theme.highlight_mode = *mode,
                        None => self.warnings.push(
                            "[resaltado] estilo: \"fondo\", \"subrayado\" o \"invertido\"".into(),
                        ),
                    }
                }
                "colores" => {
                    let list = match value {
                        toml::Value::Array(list) => list.clone(),
                        other => vec![other.clone()],
                    };
                    let colors: Option<Vec<_>> = list
                        .iter()
                        .map(|v| {
                            let text = v
                                .as_str()
                                .map(String::from)
                                .or_else(|| v.as_integer().map(|n| n.to_string()))?;
                            theme::parse(&text, truecolor)
                        })
                        .collect();
                    match colors {
                        Some(colors) if !colors.is_empty() => {
                            self.theme.set_highlight(&colors, truecolor)
                        }
                        _ => self.warnings.push(
                            "[resaltado] colores: una lista de colores (nombre, #rrggbb o un número de la paleta de 256)".into(),
                        ),
                    }
                }
                "retardo_ms" => {
                    let ms = value
                        .as_float()
                        .or_else(|| value.as_integer().map(|n| n as f64))
                        .filter(|ms| (0.0..=MAX_HIGHLIGHT_DELAY_MS).contains(ms));
                    match ms {
                        Some(ms) => self.highlight.delay_ms = ms,
                        None => self
                            .warnings
                            .push("[resaltado] retardo_ms: de 0 a 1000".into()),
                    }
                }
                _ => self.warnings.push(format!(
                    "[resaltado] «{key}» no existe (hay: activo, estilo, colores, retardo_ms)"
                )),
            }
        }
    }
}

/// Marcas de la tabla de la configuración en la guía.
#[cfg(test)]
const GUIDE_BEGIN: &str = "<!-- config:inicio (generado desde tui/src/config.rs) -->";
#[cfg(test)]
const GUIDE_END: &str = "<!-- config:fin -->";

/// Tablas de `[teclas]` y `[colores]` para `docs/GUIA.md`.
#[cfg(test)]
fn guide_markdown() -> String {
    let mut out = String::from("| En `[teclas]` | Acción | Por defecto |\n| --- | --- | --- |\n");
    for (name, bound, keys) in BINDABLE {
        let what = match bound {
            Bound::Palette => "Abrir la paleta de comandos (F1 la abre siempre)".to_string(),
            Bound::Action(a) => crate::palette::ACTIONS
                .iter()
                .find(|d| d.action == *a)
                .map_or(String::new(), |d| d.name.to_string()),
        };
        let keys: Vec<String> = keys.iter().map(|k| format!("`{k}`")).collect();
        let keys = if keys.is_empty() {
            "—".to_string()
        } else {
            keys.join(", ")
        };
        out.push_str(&format!("| `{name}` | {what} | {keys} |\n"));
    }
    out.push_str("\n| En `[colores]` | Para |\n| --- | --- |\n");
    for (name, what) in theme::NAMES.iter().zip(theme::DESCRIPTIONS) {
        out.push_str(&format!("| `{name}` | {what} |\n"));
    }
    out.push_str("\n| En `[resaltado]` | Para | Por defecto |\n| --- | --- | --- |\n");
    for (name, what, default) in HIGHLIGHT_KEYS {
        out.push_str(&format!("| `{name}` | {what} | {default} |\n"));
    }
    out.push_str("\n| En `[ocupacion]` | Para | Por defecto |\n| --- | --- | --- |\n");
    for (name, what, default) in OCCUPANCY_KEYS {
        out.push_str(&format!("| `{name}` | {what} | {default} |\n"));
    }
    out.push_str("\n| En `[voces]` | Para | Por defecto |\n| --- | --- | --- |\n");
    for (name, what, default) in VOICES_KEYS {
        out.push_str(&format!("| `{name}` | {what} | {default} |\n"));
    }
    out.push_str("\n| En `[dispositivos]` | Para | Por defecto |\n| --- | --- | --- |\n");
    for (name, what, default) in DEVICES_KEYS {
        out.push_str(&format!("| `{name}` | {what} | {default} |\n"));
    }
    out
}

/// Un comando de terminal: un texto (se parte por los espacios) o una
/// lista de palabras; `None` si no es ninguna de las dos o está vacío.
fn terminal_words(value: &toml::Value) -> Option<Vec<String>> {
    let words: Option<Vec<String>> = match value {
        toml::Value::String(s) => Some(s.split_whitespace().map(String::from).collect()),
        toml::Value::Array(list) => list.iter().map(|v| v.as_str().map(String::from)).collect(),
        _ => None,
    };
    words.filter(|w| !w.is_empty())
}

/// Escribe `[midi] mandos` en `path` con `learned` sin tocar lo demás
/// (comentarios incluidos): sustituye la lista que haya o la pone al final
/// de `[midi]` (o añade la sección). En un temporal y luego en su sitio.
pub fn save_learned(path: &Path, learned: &[script::learn::Learned]) -> std::io::Result<()> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let updated = with_learned(&text, learned);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, updated)?;
    std::fs::rename(&tmp, path)
}

/// El texto de un `config.toml` con `[midi] mandos` puesto a `learned`.
pub fn with_learned(text: &str, learned: &[script::learn::Learned]) -> String {
    let header = |line: &str| {
        let line = line.split('#').next().unwrap_or("").trim();
        line.starts_with('[')
            .then(|| line.trim_matches(['[', ']']).trim().to_string())
    };
    let mut entry = vec!["mandos = [".to_string()];
    for l in learned {
        entry.push(format!("  {},", toml_string(&l.to_line())));
    }
    entry.push("]".to_string());
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let Some(start) = lines
        .iter()
        .position(|l| header(l).as_deref() == Some("midi"))
    else {
        let mut out = text.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("[midi]\n");
        for line in entry {
            out.push_str(&line);
            out.push('\n');
        }
        return out;
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| header(l).is_some())
        .map_or(lines.len(), |i| start + 1 + i);
    // La lista que haya (en una línea o en varias, hasta su `]`).
    let old = (start + 1..end).find(|&i| {
        lines[i]
            .split_once('=')
            .is_some_and(|(k, _)| k.trim().trim_matches('"') == "mandos")
    });
    match old {
        Some(first) => {
            let last = (first..end)
                .find(|&i| lines[i].split('#').next().unwrap_or("").contains(']'))
                .unwrap_or(first);
            lines.splice(first..=last, entry);
        }
        None => {
            // Detrás de la última clave de la sección.
            let at = (start + 1..end)
                .rev()
                .find(|&i| !lines[i].trim().is_empty())
                .map_or(start + 1, |i| i + 1);
            lines.splice(at..at, entry);
        }
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Guarda lo elegido en `[dispositivos]` de `path` sin tocar lo demás
/// (comentarios incluidos): cambia o quita las líneas de esas claves, o
/// añade la sección al final. Se escribe entero en un temporal y se cambia
/// por el de verdad.
pub fn save_devices(path: &Path, choice: &crate::devices::Choice) -> std::io::Result<()> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let updated = with_devices(&text, choice);
    if updated == text {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, updated)?;
    std::fs::rename(&tmp, path)
}

/// El texto de un `config.toml` con `[dispositivos]` puesto a `choice`.
pub fn with_devices(text: &str, choice: &crate::devices::Choice) -> String {
    use crate::devices::Kind;
    let header = |line: &str| {
        let line = line.trim();
        let line = line.split('#').next().unwrap_or("").trim();
        line.starts_with('[')
            .then(|| line.trim_matches(['[', ']']).trim().to_string())
    };
    let key_of = |line: &str| {
        let (key, _) = line.split_once('=')?;
        let key = key.trim().trim_matches('"');
        Kind::from_key(key)
    };
    let entry = |kind: Kind, value: &str| format!("{} = {}", kind.key(), toml_string(value));
    let mut lines: Vec<String> = text.lines().map(String::from).collect();
    let section = lines
        .iter()
        .position(|l| header(l).as_deref() == Some("dispositivos"));
    let Some(start) = section else {
        let wanted: Vec<String> = Kind::ALL
            .into_iter()
            .filter_map(|k| choice.get(k).map(|v| entry(k, v)))
            .collect();
        if wanted.is_empty() {
            return text.to_string();
        }
        let mut out = text.to_string();
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str("[dispositivos]\n");
        for line in wanted {
            out.push_str(&line);
            out.push('\n');
        }
        return out;
    };
    let end = lines[start + 1..]
        .iter()
        .position(|l| header(l).is_some())
        .map_or(lines.len(), |i| start + 1 + i);
    let mut seen = Vec::new();
    let mut body: Vec<String> = Vec::new();
    for line in &lines[start + 1..end] {
        match key_of(line) {
            Some(kind) if !seen.contains(&kind) => {
                seen.push(kind);
                if let Some(value) = choice.get(kind) {
                    body.push(entry(kind, value));
                }
            }
            // Una clave repetida (TOML no la deja): fuera.
            Some(_) => {}
            None => body.push(line.clone()),
        }
    }
    // Las que faltan, tras la última clave (antes de las líneas en blanco
    // del final de la sección).
    let tail = body
        .iter()
        .rev()
        .take_while(|l| l.trim().is_empty())
        .count();
    let at = body.len() - tail;
    let missing: Vec<String> = Kind::ALL
        .into_iter()
        .filter(|k| !seen.contains(k))
        .filter_map(|k| choice.get(k).map(|v| entry(k, v)))
        .collect();
    body.splice(at..at, missing);
    lines.splice(start + 1..end, body);
    let mut out = lines.join("\n");
    if text.ends_with('\n') || text.is_empty() {
        out.push('\n');
    }
    out
}

/// Una cadena de TOML con sus escapes.
fn toml_string(value: &str) -> String {
    let mut out = String::from('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn the_guide_lists_the_config() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../docs/GUIA.md");
        let guide = std::fs::read_to_string(path).unwrap();
        let begin = guide
            .find(GUIDE_BEGIN)
            .expect("faltan las marcas config:inicio/config:fin")
            + GUIDE_BEGIN.len();
        let end = guide[begin..].find(GUIDE_END).unwrap() + begin;
        let updated = format!(
            "{}\n\n{}\n{}",
            &guide[..begin],
            guide_markdown(),
            &guide[end..]
        );
        if std::env::var_os("ACTUALIZAR_GUIA").is_some() {
            std::fs::write(path, &updated).unwrap();
            return;
        }
        assert!(
            updated == guide,
            "docs/GUIA.md no coincide con tui/src/config.rs: \
             ACTUALIZAR_GUIA=1 cargo test -p tui the_guide_lists_the_config"
        );
    }

    #[test]
    fn the_devices_section_names_them() {
        let config = Config::parse(
            "[dispositivos]\naudio = \"Auriculares USB\"\nmidi_entrada = \"Seaboard:Seaboard MIDI 1\"\n\
             midi_salida = \"\"\n",
            true,
            "c",
        );
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
        assert_eq!(config.devices.audio.as_deref(), Some("Auriculares USB"));
        assert_eq!(
            config.devices.midi_in.as_deref(),
            Some("Seaboard:Seaboard MIDI 1")
        );
        // Vacío: el puerto virtual.
        assert_eq!(config.devices.midi_out, None);
        let config = Config::parse("[dispositivos]\naudio = 2\nmidi = \"x\"\n", true, "c");
        assert_eq!(config.warnings.len(), 2, "{:?}", config.warnings);
        assert_eq!(config.devices, crate::devices::Choice::default());
    }

    #[test]
    fn saving_the_devices_keeps_everything_else() {
        use crate::devices::Choice;
        let choice = Choice {
            audio: Some("Auriculares \"USB\"".into()),
            midi_in: Some("Seaboard:Seaboard MIDI 1".into()),
            midi_out: None,
        };
        // Sin sección: se añade al final.
        let text = "# mis teclas\n[teclas]\nguardar = \"alt-s\" # así\n";
        let saved = with_devices(text, &choice);
        assert_eq!(
            saved,
            "# mis teclas\n[teclas]\nguardar = \"alt-s\" # así\n\n[dispositivos]\n\
             audio = \"Auriculares \\\"USB\\\"\"\nmidi_entrada = \"Seaboard:Seaboard MIDI 1\"\n"
        );
        let config = Config::parse(&saved, true, "c");
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
        assert_eq!(config.devices, choice);
        // Otra vez lo mismo: no cambia nada.
        assert_eq!(with_devices(&saved, &choice), saved);

        // Con sección: cambia sus líneas, quita las que pasan a por
        // defecto, añade las que faltan y deja comentarios y lo de después.
        let text = "[dispositivos] # elegidos en el panel\n# el bueno:\naudio = \"viejo\"\n\
                    midi_salida = \"Surge\"\n\n[midi]\ncanal = 2\n";
        let saved = with_devices(text, &choice);
        assert_eq!(
            saved,
            "[dispositivos] # elegidos en el panel\n# el bueno:\naudio = \"Auriculares \\\"USB\\\"\"\n\
             midi_entrada = \"Seaboard:Seaboard MIDI 1\"\n\n[midi]\ncanal = 2\n"
        );
        assert_eq!(Config::parse(&saved, true, "c").devices, choice);
        assert_eq!(Config::parse(&saved, true, "c").midi.channel, Some(2));
        // Nada elegido y sin sección: igual que estaba.
        assert_eq!(with_devices("[midi]\n", &Choice::default()), "[midi]\n");
        assert_eq!(with_devices("", &Choice::default()), "");
    }

    #[test]
    fn the_knobs_replace_their_list_and_keep_the_rest() {
        let knob = script::learn::Learned::parse("1 21 drums gain 0 1 lin").unwrap();
        let text = "[midi]\nmandos = [\n  \"1 9 x pan -1 1 lin\",\n] # viejos\ncanal = 2\n[editor]\nraton = true\n";
        let out = with_learned(text, std::slice::from_ref(&knob));
        assert_eq!(
            out,
            "[midi]\nmandos = [\n  \"1 21 drums gain 0 1 lin\",\n]\ncanal = 2\n[editor]\nraton = true\n"
        );
        let config = Config::parse(&out, true, "c");
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
        assert_eq!(config.learned, std::slice::from_ref(&knob));
        assert_eq!(config.midi.channel, Some(2));
        // Sin sección, al final.
        assert_eq!(
            with_learned("[editor]\nraton = true\n", &[knob]),
            "[editor]\nraton = true\n\n[midi]\nmandos = [\n  \"1 21 drums gain 0 1 lin\",\n]\n"
        );
        // Un mando mal escrito avisa y no cuenta.
        let config = Config::parse("[midi]\nmandos = [\"1 21 drums\"]\n", true, "c");
        assert!(
            config.warnings[0].starts_with("[midi] mandos: «1 21 drums»"),
            "{:?}",
            config.warnings
        );
        assert!(config.learned.is_empty());
    }

    #[test]
    fn save_devices_writes_the_file() {
        let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../target/tmp"))
            .join("config_save_devices");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("sampler/config.toml");
        let choice = crate::devices::Choice {
            midi_out: Some("Surge XT:Surge XT MIDI In".into()),
            ..Default::default()
        };
        // Sin archivo (ni carpeta): se crea.
        save_devices(&path, &choice).unwrap();
        let config = Config::load(&path, true);
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
        assert_eq!(config.devices, choice);
        assert!(!path.with_extension("toml.tmp").exists());
    }

    #[test]
    fn the_occupancy_section_and_its_colors() {
        let config = Config::parse(
            "[ocupacion]\nvisible = true\n[colores]\nhueco = \"#330000\"\nsugerencia = \"22\"\n",
            true,
            "c",
        );
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
        assert!(config.occupancy.visible);
        assert_eq!(config.theme.gap, Color::Rgb(0x33, 0, 0));
        assert_eq!(config.theme.suggestion, Color::Indexed(22));
        assert!(!Config::default().occupancy.visible);
        let config = Config::parse("[ocupacion]\nvisible = 1\ncolor = 2\n", false, "c");
        assert_eq!(config.warnings.len(), 2, "{:?}", config.warnings);
        assert!(!config.occupancy.visible);
    }

    #[test]
    fn the_highlight_section_sets_its_switch_style_colors_and_delay() {
        let config = Config::parse(
            "[resaltado]\nactivo = false\nestilo = \"subrayado\"\n\
             colores = [\"#ff0000\", \"45\"]\nretardo_ms = 40\n",
            true,
            "c",
        );
        assert!(config.warnings.is_empty(), "{:?}", config.warnings);
        assert!(!config.highlight.enabled);
        assert_eq!(config.highlight.delay_ms, 40.0);
        assert_eq!(config.theme.highlight_mode, theme::HighlightMode::Underline);
        // Dos colores para ocho pistas: se repiten; los apagados, en RGB.
        assert_eq!(config.theme.highlight[0], Color::Rgb(255, 0, 0));
        assert_eq!(config.theme.highlight[1], Color::Indexed(45));
        assert_eq!(config.theme.highlight[6], Color::Rgb(255, 0, 0));
        assert_eq!(config.theme.highlight_dim[0], Color::Rgb(115, 0, 0));
        // Sin truecolor, los apagados también de la paleta de 256.
        let config = Config::parse("[resaltado]\ncolores = \"#ff0000\"\n", false, "c");
        assert_eq!(config.theme.highlight[3], Color::Indexed(196));
        assert!(matches!(config.theme.highlight_dim[3], Color::Indexed(_)));
        // Por defecto: encendido, fondo, sin retardo.
        let config = Config::parse("", false, "c");
        assert_eq!(config.highlight, HighlightConfig::default());
        assert!(config.highlight.enabled);
        assert_eq!(
            config.theme.highlight_mode,
            theme::HighlightMode::Background
        );
        // Lo que no vale se avisa y no cambia nada.
        let config = Config::parse(
            "[resaltado]\nactivo = 1\nestilo = \"recuadro\"\ncolores = [\"rosa chicle\"]\n\
             retardo_ms = 5000\notra = 1\n",
            false,
            "c",
        );
        assert_eq!(config.warnings.len(), 5, "{:?}", config.warnings);
        assert_eq!(config.highlight, HighlightConfig::default());
        assert_eq!(config.theme, Theme::default());
    }

    fn input(key: Key, ctrl: bool, alt: bool) -> Input {
        Input {
            key,
            ctrl,
            alt,
            ..Input::default()
        }
    }

    #[test]
    fn keys_parse_and_describe_themselves() {
        let k = KeyCombo::parse("Ctrl-S").unwrap();
        assert_eq!(k.text(), "Ctrl-S");
        assert!(k.matches(&input(Key::Char('s'), true, false)));
        assert!(k.matches(&input(Key::Char('S'), true, false)));
        assert!(!k.matches(&input(Key::Char('s'), false, false)));
        assert_eq!(KeyCombo::parse("f12").unwrap().key, Key::F(12));
        assert_eq!(KeyCombo::parse("ctrl-alt-k").unwrap().text(), "Ctrl-Alt-K");
        assert!(
            KeyCombo::parse("s")
                .unwrap_err()
                .contains("escribiría texto")
        );
        assert!(KeyCombo::parse("f13").is_err());
        assert!(KeyCombo::parse("ctrl-enter").is_err());
        let keys = Keymap::default();
        assert_eq!(keys.describe(Bound::Action(Action::Load)), "Ctrl-L o F5");
        assert_eq!(keys.describe(Bound::Palette), "F1");
    }

    #[test]
    fn the_config_rebinds_and_warns_about_conflicts() {
        let text = r##"
[teclas]
guardar = "alt-s"
paleta = "ctrl-p"
marcha = ["ctrl-g", "f8"]
cargar = "f1"
explorador = ["alt-b", "alt-s"]
inventada = "alt-x"
salir = "q"

[colores]
acento = "#ff8700"
sonando = 22
tenue = "fucsia"
brillo = "red"
"##;
        let config = Config::parse(text, true, "config.toml");
        let keys = &config.keys;
        let at = |k, c, a| keys.lookup(&input(k, c, a));
        assert_eq!(
            at(Key::Char('s'), false, true),
            Some(Bound::Action(Action::SaveAndLoad))
        );
        assert_eq!(at(Key::Char('s'), true, false), None, "ctrl-s ya no guarda");
        assert_eq!(at(Key::Char('p'), true, false), Some(Bound::Palette));
        assert_eq!(
            at(Key::F(1), false, false),
            Some(Bound::Palette),
            "F1, siempre"
        );
        assert_eq!(
            at(Key::Char('g'), true, false),
            Some(Bound::Action(Action::Transport))
        );
        assert_eq!(
            at(Key::Char('b'), false, true),
            Some(Bound::Action(Action::Browser))
        );
        // salir = "q" no vale: se queda con su tecla por defecto.
        assert_eq!(
            at(Key::Char('q'), true, false),
            Some(Bound::Action(Action::Quit))
        );
        assert_eq!(keys.describe(Bound::Palette), "F1 o Ctrl-P");
        assert_eq!(
            config.warnings,
            [
                "[teclas] cargar: F1 es siempre la paleta de comandos",
                "[teclas] explorador: Alt-S ya es de guardar; no se usa",
                "[teclas] «inventada» no existe (hay: paleta, guardar, cargar, marcha, cue, ir_al_error, ir_a_linea, buscar, buscar_siguiente, buscar_anterior, siguiente_pista, recargar, ordenes, completar, salir, modo_tocar, deshacer, rehacer, explorador, tap_tempo, macros, metronomo, exportar, seccion_siguiente, seccion_anterior, repetir_seccion, variante, resaltado, ocupacion, panico, pausa_panico)",
                "[teclas] salir: «q»: una letra sin ctrl- ni alt- escribiría texto",
                "[teclas] Ctrl-G era de ir_al_error; ahora es de marcha",
                "[colores] tenue: color no válido",
                "[colores] «brillo» no existe (hay: acento, tenue, error, linea_error, aviso_pasos, aviso, onda, cortes, sonando, nivel, hueco, sugerencia)",
            ]
        );
        assert_eq!(config.theme.accent, Color::Rgb(0xff, 0x87, 0x00));
        assert_eq!(config.theme.playing, Color::Indexed(22));
        // Sin truecolor, el más cercano de 256.
        let plain = Config::parse(text, false, "config.toml");
        assert_eq!(plain.theme.accent, Color::Indexed(208));
    }

    #[test]
    fn the_editor_terminal_is_a_command_or_a_list_of_words() {
        let text = "[editor]\nterminal = \"kitty --class sampler-editor -e\"\n";
        let config = Config::parse(text, true, "c.toml");
        assert_eq!(
            config.editor_terminal.unwrap(),
            ["kitty", "--class", "sampler-editor", "-e"]
        );
        let text = "[editor]\nterminal = [\"foot\", \"--app-id\", \"sampler editor\"]\n";
        let config = Config::parse(text, true, "c.toml");
        assert_eq!(
            config.editor_terminal.unwrap(),
            ["foot", "--app-id", "sampler editor"]
        );
        assert!(config.warnings.is_empty());
        let config = Config::parse("[editor]\nterminal = 3\nventana = 1\n", true, "c.toml");
        assert_eq!(config.editor_terminal, None);
        assert_eq!(config.warnings.len(), 2, "{:?}", config.warnings);
        assert!(config.warnings[1].contains("(hay: terminal, raton, modo, salir_insert)"));
    }

    #[test]
    fn a_broken_or_missing_file_gives_defaults_and_a_warning() {
        let broken = Config::parse("[teclas\nguardar = ", true, "config.toml");
        assert_eq!(broken.keys, Keymap::default());
        assert_eq!(broken.theme, Theme::default());
        assert!(
            broken.warnings[0].starts_with("config.toml está roto ("),
            "{:?}",
            broken.warnings
        );
        let missing = Config::load(Path::new("/no/existe/config.toml"), true);
        assert_eq!(missing.keys, Keymap::default());
        assert!(missing.warnings[0].starts_with("sin /no/existe/config.toml"));
        let odd = Config::parse("teclas = 3\n[otra]\nx = 1\n", true, "c.toml");
        assert_eq!(odd.warnings.len(), 2, "{:?}", odd.warnings);
    }
}
