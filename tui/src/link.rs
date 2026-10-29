//! Enlace entre la ventana principal y el editor en su propia ventana
//! (fase L), y entre la principal y el panel de dispositivos en la suya
//! (fase U, en un socket aparte): un socket Unix local, solo del usuario,
//! con un protocolo de líneas de texto. Sin hilos: los dos extremos se leen y escriben sin
//! bloquear desde el bucle de la interfaz, y el audio no pasa por aquí.
//!
//! Cada mensaje es una línea: el verbo y sus campos separados por
//! tabuladores, con `\\`, `\t`, `\n` y `\r` escapados dentro de cada campo.

use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::devices::{DevicesView, Entry, Kind};
use crate::highlight::Highlight;

/// Versión del protocolo: los dos extremos tienen que tener la misma (la
/// 2 añade el traspaso del editor: `historial`, `devuelve` y `vuelvo`; la
/// 3, lo que suena para resaltarlo en el código: `suena`; la 4, el panel de
/// dispositivos en su ventana, en su propio socket: [`ToDevices`] y
/// [`FromDevices`]; la 5, la línea de los mandos MIDI en la vista del panel
/// de dispositivos).
pub const VERSION: u32 = 5;

/// Lo que se deja pendiente de escribir antes de dar el otro extremo por
/// perdido (no lee).
const MAX_PENDING: usize = 8 << 20;
/// Línea más larga que se acepta.
const MAX_LINE: usize = 8 << 20;
/// Cada cuánto vuelve a intentar conectar el editor sin principal.
pub const RETRY: Duration = Duration::from_secs(1);

/// Del editor a la principal.
#[derive(Debug, Clone, PartialEq)]
pub enum ToMain {
    /// Saludo con la versión del protocolo.
    Hello { version: u32 },
    /// El texto del editor (sin cargar), con su revisión.
    Text { rev: u64, text: String },
    /// Cargar este texto: entra al compás siguiente.
    Load { rev: u64, text: String },
    /// Una acción de la paleta que se hace en la principal (por su nombre).
    Action(String),
    /// Una orden de la línea de órdenes (F2).
    Command(String),
    /// El editor vuelve a la ventana principal con su estado (y se
    /// cierra).
    Return(Handover),
}

/// Lo que se lleva el editor al cambiar de ventana: el texto, el cursor y
/// el historial de deshacer como textos enteros.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Handover {
    pub text: String,
    /// Fila y columna (en caracteres).
    pub cursor: (usize, usize),
    /// Lo que deshace Ctrl-Z, del más antiguo al anterior al actual.
    pub undo: Vec<String>,
    /// Lo que rehace Ctrl-Y, del siguiente al más lejano.
    pub redo: Vec<String>,
}

impl Handover {
    fn fields(&self) -> Vec<String> {
        let mut out = vec![
            self.cursor.0.to_string(),
            self.cursor.1.to_string(),
            self.undo.len().to_string(),
            self.text.clone(),
        ];
        out.extend(self.undo.iter().cloned());
        out.extend(self.redo.iter().cloned());
        out
    }

    fn from_fields(f: &[String], v: &str) -> Result<Handover, String> {
        let undo: usize = number(field(f, 2, v)?, v)?;
        let history = f.get(4..).unwrap_or_default();
        if undo > history.len() {
            return Err(format!("«{v}»: faltan textos del historial"));
        }
        Ok(Handover {
            cursor: (number(field(f, 0, v)?, v)?, number(field(f, 1, v)?, v)?),
            text: field(f, 3, v)?.into(),
            undo: history[..undo].to_vec(),
            redo: history[undo..].to_vec(),
        })
    }
}

/// De la principal al editor.
#[derive(Debug, Clone, PartialEq)]
pub enum ToEditor {
    /// Respuesta al saludo: la ruta del script, la carpeta de los samples,
    /// la última revisión del editor que tiene y su texto.
    Welcome {
        version: u32,
        path: PathBuf,
        samples: PathBuf,
        rev: u64,
        text: String,
    },
    /// La principal cambió el texto, a partir de la revisión `base` del
    /// editor.
    Text { base: u64, text: String },
    /// Resultado de la última carga: el error, o ninguno.
    Error(Option<String>),
    /// El mensaje de la ventana principal.
    Notice(String),
    /// Dónde va la reproducción y qué hay seleccionado.
    State(RemoteState),
    /// Etiquetas de corte que conoce la principal (para el autocompletado
    /// y el diagnóstico).
    Labels(Vec<String>),
    /// La principal cierra esta conexión (y por qué): no se reintenta.
    Bye(String),
    /// El estado del editor de la principal, que acaba de sacarlo a esta
    /// ventana: si el texto es el mismo, se queda con su cursor e
    /// historial.
    History(Handover),
    /// La principal pide el editor de vuelta: el editor contesta con
    /// [`ToMain::Return`] y se cierra.
    GiveBack,
    /// Los tokens que se oyen ahora (resaltado en vivo), en el orden en que
    /// se dibujan, sobre el texto de la revisión `base` del editor (vacío
    /// si no suena nada o el texto no es el cargado).
    Sounding { base: u64, marks: Vec<Highlight> },
}

/// Estado de la reproducción que ve el editor.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RemoteState {
    pub running: bool,
    pub bpm: f64,
    pub bar: u64,
    pub step: Option<usize>,
    /// Pista y corte seleccionados (el corte desde 1).
    pub track: String,
    pub slice: usize,
    /// El cue como se ve (vacío: no hay).
    pub cue: String,
}

/// Del panel de dispositivos (en su ventana) a la principal.
#[derive(Debug, Clone, PartialEq)]
pub enum FromDevices {
    Hello {
        version: u32,
    },
    /// Volver a leer las listas.
    Refresh,
    /// Usar el dispositivo `name` de la lista `kind`.
    Choose {
        kind: Kind,
        name: String,
    },
}

/// De la principal al panel de dispositivos.
#[derive(Debug, Clone, PartialEq)]
pub enum ToDevices {
    /// Respuesta al saludo: la versión y el script (para el título).
    Welcome { version: u32, title: String },
    /// Las listas y el estado, cada vez que cambian.
    View(DevicesView),
    /// La principal cierra esta conexión (y por qué): no se reintenta.
    Bye(String),
    /// La principal vuelve a meter el panel: la ventana se cierra.
    GiveBack,
}

/// Escapa un campo.
fn escape(field: &str, out: &mut String) {
    for c in field.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
}

fn unescape(field: &str) -> Result<String, String> {
    let mut out = String::with_capacity(field.len());
    let mut chars = field.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('\\') => out.push('\\'),
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            other => return Err(format!("escape no válido: \\{}", other.unwrap_or(' '))),
        }
    }
    Ok(out)
}

/// Une el verbo y los campos en una línea (con su salto).
fn line(verb: &str, fields: &[&str]) -> String {
    let mut out = String::from(verb);
    for f in fields {
        out.push('\t');
        escape(f, &mut out);
    }
    out.push('\n');
    out
}

fn fields_line(verb: &str, fields: &[String]) -> String {
    let fields: Vec<&str> = fields.iter().map(String::as_str).collect();
    line(verb, &fields)
}

/// Separa una línea (sin el salto) en verbo y campos.
fn split(line: &str) -> Result<(String, Vec<String>), String> {
    let mut parts = line.split('\t');
    let verb = parts.next().unwrap_or_default().to_string();
    let fields = parts.map(unescape).collect::<Result<_, _>>()?;
    Ok((verb, fields))
}

fn field<'a>(fields: &'a [String], i: usize, verb: &str) -> Result<&'a str, String> {
    fields
        .get(i)
        .map(String::as_str)
        .ok_or_else(|| format!("«{verb}»: falta el campo {}", i + 1))
}

fn number<T: std::str::FromStr>(text: &str, verb: &str) -> Result<T, String> {
    text.parse()
        .map_err(|_| format!("«{verb}»: número no válido «{text}»"))
}

/// Un mensaje que se puede mandar por el enlace.
pub trait Message: Sized {
    fn encode(&self) -> String;
    fn decode(line: &str) -> Result<Self, String>;
}

impl Message for ToMain {
    fn encode(&self) -> String {
        match self {
            ToMain::Hello { version } => line("hola", &[&version.to_string()]),
            ToMain::Text { rev, text } => line("texto", &[&rev.to_string(), text]),
            ToMain::Load { rev, text } => line("cargar", &[&rev.to_string(), text]),
            ToMain::Action(name) => line("accion", &[name]),
            ToMain::Command(command) => line("orden", &[command]),
            ToMain::Return(handover) => fields_line("vuelvo", &handover.fields()),
        }
    }

    fn decode(text: &str) -> Result<Self, String> {
        let (verb, f) = split(text)?;
        let v = verb.as_str();
        Ok(match v {
            "hola" => ToMain::Hello {
                version: number(field(&f, 0, v)?, v)?,
            },
            "texto" => ToMain::Text {
                rev: number(field(&f, 0, v)?, v)?,
                text: field(&f, 1, v)?.into(),
            },
            "cargar" => ToMain::Load {
                rev: number(field(&f, 0, v)?, v)?,
                text: field(&f, 1, v)?.into(),
            },
            "accion" => ToMain::Action(field(&f, 0, v)?.into()),
            "orden" => ToMain::Command(field(&f, 0, v)?.into()),
            "vuelvo" => ToMain::Return(Handover::from_fields(&f, v)?),
            other => return Err(format!("mensaje desconocido «{other}»")),
        })
    }
}

impl Message for ToEditor {
    fn encode(&self) -> String {
        match self {
            ToEditor::Welcome {
                version,
                path,
                samples,
                rev,
                text,
            } => line(
                "hola",
                &[
                    &version.to_string(),
                    &path.to_string_lossy(),
                    &samples.to_string_lossy(),
                    &rev.to_string(),
                    text,
                ],
            ),
            ToEditor::Text { base, text } => line("texto", &[&base.to_string(), text]),
            ToEditor::Error(error) => line("error", &[error.as_deref().unwrap_or("")]),
            ToEditor::Notice(text) => line("aviso", &[text]),
            ToEditor::State(s) => line(
                "estado",
                &[
                    if s.running { "1" } else { "0" },
                    &s.bpm.to_string(),
                    &s.bar.to_string(),
                    &s.step.map_or(String::new(), |s| s.to_string()),
                    &s.track,
                    &s.slice.to_string(),
                    &s.cue,
                ],
            ),
            ToEditor::Labels(labels) => {
                let fields: Vec<&str> = labels.iter().map(String::as_str).collect();
                line("etiquetas", &fields)
            }
            ToEditor::Bye(reason) => line("adios", &[reason]),
            ToEditor::History(handover) => fields_line("historial", &handover.fields()),
            ToEditor::GiveBack => line("devuelve", &[]),
            ToEditor::Sounding { base, marks } => {
                // Cada token: fila, columnas, color y si es de la seleccionada.
                let mut fields = vec![base.to_string()];
                fields.extend(marks.iter().map(|h| {
                    format!(
                        "{},{},{},{},{}",
                        h.row,
                        h.cols.start,
                        h.cols.end,
                        h.color,
                        u8::from(h.selected)
                    )
                }));
                fields_line("suena", &fields)
            }
        }
    }

    fn decode(text: &str) -> Result<Self, String> {
        let (verb, f) = split(text)?;
        let v = verb.as_str();
        Ok(match v {
            "hola" => ToEditor::Welcome {
                version: number(field(&f, 0, v)?, v)?,
                path: field(&f, 1, v)?.into(),
                samples: field(&f, 2, v)?.into(),
                rev: number(field(&f, 3, v)?, v)?,
                text: field(&f, 4, v)?.into(),
            },
            "texto" => ToEditor::Text {
                base: number(field(&f, 0, v)?, v)?,
                text: field(&f, 1, v)?.into(),
            },
            "error" => {
                let error = field(&f, 0, v)?;
                ToEditor::Error((!error.is_empty()).then(|| error.into()))
            }
            "aviso" => ToEditor::Notice(field(&f, 0, v)?.into()),
            "estado" => {
                let step = field(&f, 3, v)?;
                ToEditor::State(RemoteState {
                    running: field(&f, 0, v)? == "1",
                    bpm: number(field(&f, 1, v)?, v)?,
                    bar: number(field(&f, 2, v)?, v)?,
                    step: if step.is_empty() {
                        None
                    } else {
                        Some(number(step, v)?)
                    },
                    track: field(&f, 4, v)?.into(),
                    slice: number(field(&f, 5, v)?, v)?,
                    // (Una principal de antes no lo manda.)
                    cue: f.get(6).cloned().unwrap_or_default(),
                })
            }
            "etiquetas" => ToEditor::Labels(f),
            "adios" => ToEditor::Bye(field(&f, 0, v)?.into()),
            "historial" => ToEditor::History(Handover::from_fields(&f, v)?),
            "devuelve" => ToEditor::GiveBack,
            "suena" => ToEditor::Sounding {
                base: number(field(&f, 0, v)?, v)?,
                marks: f[1..]
                    .iter()
                    .map(|mark| {
                        let n: Vec<&str> = mark.split(',').collect();
                        let [row, start, end, color, selected] = n[..] else {
                            return Err(format!("«{v}»: token no válido «{mark}»"));
                        };
                        Ok(Highlight {
                            row: number(row, v)?,
                            cols: number(start, v)?..number(end, v)?,
                            color: number(color, v)?,
                            selected: selected == "1",
                        })
                    })
                    .collect::<Result<_, String>>()?,
            },
            other => return Err(format!("mensaje desconocido «{other}»")),
        })
    }
}

impl Message for FromDevices {
    fn encode(&self) -> String {
        match self {
            FromDevices::Hello { version } => line("hola", &[&version.to_string()]),
            FromDevices::Refresh => line("refrescar", &[]),
            FromDevices::Choose { kind, name } => line("elegir", &[kind.key(), name]),
        }
    }

    fn decode(text: &str) -> Result<Self, String> {
        let (verb, f) = split(text)?;
        let v = verb.as_str();
        Ok(match v {
            "hola" => FromDevices::Hello {
                version: number(field(&f, 0, v)?, v)?,
            },
            "refrescar" => FromDevices::Refresh,
            "elegir" => {
                let key = field(&f, 0, v)?;
                FromDevices::Choose {
                    kind: Kind::from_key(key)
                        .ok_or_else(|| format!("«{v}»: lista desconocida «{key}»"))?,
                    name: field(&f, 1, v)?.into(),
                }
            }
            other => return Err(format!("mensaje desconocido «{other}»")),
        })
    }
}

impl Message for ToDevices {
    fn encode(&self) -> String {
        match self {
            ToDevices::Welcome { version, title } => line("hola", &[&version.to_string(), title]),
            ToDevices::View(view) => {
                // El estado, el aviso, la actividad, los mandos, las tres
                // notas y cada
                // lista: cuántas filas y, por fila, nombre, detalle y marcas
                // (`a` en uso, `d` predeterminado).
                let mut fields = vec![
                    view.status.clone(),
                    view.notice.clone(),
                    view.activity.to_string(),
                    view.knobs.clone(),
                ];
                fields.extend(view.notes.iter().cloned());
                for kind in Kind::ALL {
                    let list = view.list(kind);
                    fields.push(list.len().to_string());
                    for e in list {
                        let mut flags = String::new();
                        if e.active {
                            flags.push('a');
                        }
                        if e.default {
                            flags.push('d');
                        }
                        fields.extend([e.name.clone(), e.detail.clone(), flags]);
                    }
                }
                fields_line("vista", &fields)
            }
            ToDevices::Bye(reason) => line("adios", &[reason]),
            ToDevices::GiveBack => line("devuelve", &[]),
        }
    }

    fn decode(text: &str) -> Result<Self, String> {
        let (verb, f) = split(text)?;
        let v = verb.as_str();
        Ok(match v {
            "hola" => ToDevices::Welcome {
                version: number(field(&f, 0, v)?, v)?,
                title: field(&f, 1, v)?.into(),
            },
            "vista" => {
                let mut view = DevicesView {
                    status: field(&f, 0, v)?.into(),
                    notice: field(&f, 1, v)?.into(),
                    activity: number(field(&f, 2, v)?, v)?,
                    knobs: field(&f, 3, v)?.into(),
                    notes: [
                        field(&f, 4, v)?.into(),
                        field(&f, 5, v)?.into(),
                        field(&f, 6, v)?.into(),
                    ],
                    ..DevicesView::default()
                };
                let mut at = 7;
                for kind in Kind::ALL {
                    let count: usize = number(field(&f, at, v)?, v)?;
                    at += 1;
                    let mut list = Vec::new();
                    for _ in 0..count {
                        let flags = field(&f, at + 2, v)?;
                        list.push(Entry {
                            name: field(&f, at, v)?.into(),
                            detail: field(&f, at + 1, v)?.into(),
                            active: flags.contains('a'),
                            default: flags.contains('d'),
                        });
                        at += 3;
                    }
                    match kind {
                        Kind::Audio => view.audio = list,
                        Kind::MidiIn => view.midi_in = list,
                        Kind::MidiOut => view.midi_out = list,
                    }
                }
                ToDevices::View(view)
            }
            "adios" => ToDevices::Bye(field(&f, 0, v)?.into()),
            "devuelve" => ToDevices::GiveBack,
            other => return Err(format!("mensaje desconocido «{other}»")),
        })
    }
}

/// Una conexión sin bloqueos: lo leído se junta hasta tener líneas
/// enteras y lo que no se pudo escribir espera a la siguiente vuelta.
pub struct Conn {
    stream: UnixStream,
    input: Vec<u8>,
    output: Vec<u8>,
}

impl Conn {
    pub fn new(stream: UnixStream) -> io::Result<Conn> {
        stream.set_nonblocking(true)?;
        Ok(Conn {
            stream,
            input: Vec::new(),
            output: Vec::new(),
        })
    }

    /// Pone un mensaje en la cola y escribe lo que se pueda.
    pub fn send(&mut self, message: &impl Message) -> io::Result<()> {
        self.send_line(&message.encode())
    }

    /// Lo mismo con un mensaje ya escrito.
    fn send_line(&mut self, line: &str) -> io::Result<()> {
        self.output.extend_from_slice(line.as_bytes());
        self.flush()
    }

    /// Escribe lo pendiente sin bloquear. Error si el otro extremo se fue o
    /// lleva demasiado sin leer.
    pub fn flush(&mut self) -> io::Result<()> {
        while !self.output.is_empty() {
            match self.stream.write(&self.output) {
                Ok(0) => return Err(ErrorKind::WriteZero.into()),
                Ok(n) => {
                    self.output.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        if self.output.len() > MAX_PENDING {
            return Err(io::Error::other("el otro extremo no lee"));
        }
        Ok(())
    }

    /// Escribe todo lo pendiente, esperando como mucho `timeout` (para
    /// mandar lo último antes de cerrarse).
    pub fn drain(&mut self, timeout: Duration) -> io::Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            self.flush()?;
            if self.output.is_empty() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(ErrorKind::TimedOut.into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Lee lo que haya y devuelve los mensajes enteros. Error si el otro
    /// extremo cerró o mandó algo que no se entiende.
    pub fn receive<M: Message>(&mut self) -> io::Result<Vec<M>> {
        let mut buf = [0u8; 16 * 1024];
        loop {
            match self.stream.read(&mut buf) {
                Ok(0) => {
                    // Lo que llegó antes de cerrar todavía vale.
                    let messages = self.lines()?;
                    if messages.is_empty() {
                        return Err(ErrorKind::UnexpectedEof.into());
                    }
                    return Ok(messages);
                }
                Ok(n) => {
                    self.input.extend_from_slice(&buf[..n]);
                    if self.input.len() > MAX_LINE && !self.input.contains(&b'\n') {
                        return Err(io::Error::other("línea demasiado larga"));
                    }
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(e),
            }
        }
        self.lines()
    }

    fn lines<M: Message>(&mut self) -> io::Result<Vec<M>> {
        let mut messages = Vec::new();
        while let Some(end) = self.input.iter().position(|&b| b == b'\n') {
            let raw: Vec<u8> = self.input.drain(..=end).collect();
            let text = std::str::from_utf8(&raw[..end])
                .map_err(|_| io::Error::new(ErrorKind::InvalidData, "texto que no es UTF-8"))?;
            let message = M::decode(text).map_err(|e| io::Error::new(ErrorKind::InvalidData, e))?;
            messages.push(message);
        }
        Ok(messages)
    }

    /// Usuario del otro extremo.
    pub fn peer_uid(&self) -> io::Result<u32> {
        peer_uid(&self.stream)
    }
}

/// Usuario efectivo de este proceso.
pub fn my_uid() -> u32 {
    // SAFETY: geteuid no tiene precondiciones y no falla.
    unsafe { libc::geteuid() }
}

/// Usuario del proceso al otro lado de un socket Unix (lo da el núcleo; no
/// se puede falsear).
#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` y `len` son válidos y del tamaño que pide SO_PEERCRED.
    let r = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(cred.uid)
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
    let (mut uid, mut gid) = (0, 0);
    // SAFETY: punteros válidos a dos enteros.
    let r = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(uid)
}

/// Carpeta de los sockets: `$XDG_RUNTIME_DIR/sampler` (o, sin ella,
/// `<tmp>/sampler-<uid>`), creada 0700 y comprobada: tiene que ser nuestra
/// y sin permisos para nadie más.
pub fn runtime_dir() -> io::Result<PathBuf> {
    let dir = match std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        Some(base) => PathBuf::from(base).join("sampler"),
        None => std::env::temp_dir().join(format!("sampler-{}", my_uid())),
    };
    private_dir(&dir)?;
    Ok(dir)
}

/// Crea (si falta) una carpeta 0700 y comprueba que es del usuario y que
/// nadie más puede entrar.
pub fn private_dir(dir: &Path) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.uid() != my_uid() {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            format!("{} no es una carpeta tuya", dir.display()),
        ));
    }
    if meta.mode() & 0o077 != 0 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Socket de la instancia de un script: un nombre por ruta (la del script,
/// también si se abrió un proyecto), para poder tener varias canciones
/// abiertas a la vez.
pub fn socket_path(dir: &Path, script: &Path) -> PathBuf {
    let full = std::fs::canonicalize(script)
        .or_else(|_| std::path::absolute(script))
        .unwrap_or_else(|_| script.to_path_buf());
    let stem = full
        .file_stem()
        .map_or_else(|| "script".into(), |s| s.to_string_lossy().into_owned());
    // Solo letras, números, - y _ (y corto: el límite de sun_path es ~100).
    let stem: String = stem
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(24)
        .collect();
    dir.join(format!(
        "{stem}-{:016x}.sock",
        fnv1a(full.to_string_lossy().as_bytes())
    ))
}

/// Socket del panel de dispositivos de la instancia de un script (aparte
/// del del editor: los dos pueden estar fuera a la vez).
pub fn devices_socket_path(dir: &Path, script: &Path) -> PathBuf {
    let editor = socket_path(dir, script);
    let stem = editor
        .file_stem()
        .map_or_else(String::new, |s| s.to_string_lossy().into_owned());
    dir.join(format!("{stem}-disp.sock"))
}

/// Hash FNV-1a de 64 bits (estable entre versiones de Rust, a diferencia
/// de `DefaultHasher`: los dos procesos pueden ser binarios distintos).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |h, &b| {
        (h ^ b as u64).wrapping_mul(0x0100_0000_01b3)
    })
}

/// El lado de la principal: escucha y atiende a un editor cada vez.
pub struct Server {
    path: PathBuf,
    listener: UnixListener,
    client: Option<Conn>,
    /// Lo que se le dice al cliente que se queda fuera cuando llega otro.
    replaced: String,
}

impl Server {
    /// Escucha en `path` (a un editor). Si ya hay otra instancia viva ahí,
    /// error; si el socket es de una que se cayó, lo sustituye.
    pub fn bind(path: &Path) -> io::Result<Server> {
        let replaced = ToEditor::Bye("se ha conectado otro editor a esta ventana principal".into());
        Server::bind_with(path, replaced.encode())
    }

    /// Como [`Server::bind`], diciendo `replaced` (un mensaje ya escrito) al
    /// cliente que se queda fuera cuando se conecta otro.
    pub fn bind_with(path: &Path, replaced: String) -> io::Result<Server> {
        if path.exists() {
            if UnixStream::connect(path).is_ok() {
                return Err(io::Error::new(
                    ErrorKind::AddrInUse,
                    format!(
                        "ya hay una ventana principal de este script ({})",
                        path.display()
                    ),
                ));
            }
            std::fs::remove_file(path)?;
        }
        let listener = UnixListener::bind(path)?;
        // La carpeta ya es 0700; el socket, además, solo para el usuario.
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(Server {
            path: path.to_path_buf(),
            listener,
            client: None,
            replaced,
        })
    }

    /// Acepta las conexiones nuevas: las de otro usuario se cierran sin
    /// más. Devuelve si hay una conexión nueva (sustituye a la que
    /// hubiera) y si se rechazó alguna.
    pub fn accept(&mut self) -> (bool, Option<String>) {
        let mut fresh = false;
        let mut rejected = None;
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    let conn = match Conn::new(stream) {
                        Ok(conn) => conn,
                        Err(_) => continue,
                    };
                    match conn.peer_uid() {
                        Ok(uid) if uid == my_uid() => {
                            if let Some(mut old) = self.client.replace(conn) {
                                let _ = old.send_line(&self.replaced);
                            }
                            fresh = true;
                        }
                        Ok(uid) => rejected = Some(format!("conexión del usuario {uid} rechazada")),
                        Err(e) => rejected = Some(format!("conexión rechazada: {e}")),
                    }
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        (fresh, rejected)
    }

    pub fn client(&mut self) -> Option<&mut Conn> {
        self.client.as_mut()
    }

    pub fn connected(&self) -> bool {
        self.client.is_some()
    }

    /// Suelta el editor (se fue o falló).
    pub fn drop_client(&mut self) {
        self.client = None;
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// El lado del editor: conecta, y si se pierde la principal, reintenta.
pub struct Client {
    path: PathBuf,
    conn: Option<Conn>,
    last_try: Option<Instant>,
    /// El saludo que se manda al conectar (ya escrito).
    hello: String,
    /// La principal cerró a propósito (otro editor, otra versión): no se
    /// reintenta solo.
    pub stopped: bool,
}

impl Client {
    /// El lado de un editor.
    pub fn new(path: &Path) -> Client {
        Client::with_hello(path, ToMain::Hello { version: VERSION }.encode())
    }

    /// Un cliente que saluda con `hello` (un mensaje ya escrito).
    pub fn with_hello(path: &Path, hello: String) -> Client {
        Client {
            path: path.to_path_buf(),
            conn: None,
            last_try: None,
            hello,
            stopped: false,
        }
    }

    /// Intenta conectar si no lo está (como mucho una vez por `RETRY`,
    /// salvo `now`). Devuelve la conexión si es nueva.
    pub fn connect(&mut self, now: bool) -> Option<&mut Conn> {
        if self.conn.is_some() || (self.stopped && !now) {
            return None;
        }
        if !now && self.last_try.is_some_and(|t| t.elapsed() < RETRY) {
            return None;
        }
        self.last_try = Some(Instant::now());
        let stream = UnixStream::connect(&self.path).ok()?;
        let mut conn = Conn::new(stream).ok()?;
        // Solo con una principal del mismo usuario.
        if conn.peer_uid().ok() != Some(my_uid()) {
            return None;
        }
        conn.send_line(&self.hello).ok()?;
        self.stopped = false;
        self.conn = Some(conn);
        self.conn.as_mut()
    }

    pub fn conn(&mut self) -> Option<&mut Conn> {
        self.conn.as_mut()
    }

    pub fn connected(&self) -> bool {
        self.conn.is_some()
    }

    pub fn disconnect(&mut self) {
        self.conn = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip<M: Message + PartialEq + std::fmt::Debug>(message: M) {
        let line = message.encode();
        assert!(line.ends_with('\n'));
        assert_eq!(line.matches('\n').count(), 1, "una sola línea: {line:?}");
        assert_eq!(M::decode(line.trim_end_matches('\n')).unwrap(), message);
    }

    #[test]
    fn every_message_survives_the_wire() {
        let text = "track(\"a\")\n\t:steps(\"1 . \\\\ 2\")\r\n-- ñ ✓\n".to_string();
        roundtrip(ToMain::Hello { version: VERSION });
        roundtrip(ToMain::Text {
            rev: 7,
            text: text.clone(),
        });
        roundtrip(ToMain::Load {
            rev: u64::MAX,
            text: String::new(),
        });
        roundtrip(ToMain::Action("Transport".into()));
        roundtrip(ToMain::Command("etiquetar 2 caja\tfuerte".into()));
        roundtrip(ToEditor::Welcome {
            version: VERSION,
            path: "/home/a b/canción.lua".into(),
            samples: "/home/a b".into(),
            rev: 3,
            text: text.clone(),
        });
        roundtrip(ToEditor::Text {
            base: 3,
            text: text.clone(),
        });
        roundtrip(ToEditor::Error(Some("song.lua:3: algo\nmás".into())));
        roundtrip(ToEditor::Error(None));
        roundtrip(ToEditor::Notice("cargado".into()));
        roundtrip(ToEditor::State(RemoteState {
            running: true,
            bpm: 172.5,
            bar: 12,
            step: Some(15),
            track: "drums".into(),
            slice: 3,
            cue: "drop (entrada 2)".into(),
        }));
        roundtrip(ToEditor::State(RemoteState::default()));
        roundtrip(ToEditor::Labels(vec!["kick".into(), "caja\tgorda".into()]));
        roundtrip(ToEditor::Labels(vec![]));
        roundtrip(ToEditor::Bye("otro editor".into()));
        let handover = Handover {
            text: text.clone(),
            cursor: (2, 7),
            undo: vec![String::new(), "a\tb".into(), "a\n".into()],
            redo: vec!["ñ\\".into()],
        };
        roundtrip(ToEditor::History(handover.clone()));
        roundtrip(ToMain::Return(handover));
        roundtrip(ToMain::Return(Handover::default()));
        roundtrip(ToEditor::GiveBack);
        roundtrip(ToEditor::Sounding {
            base: 9,
            marks: vec![
                Highlight {
                    row: 3,
                    cols: 20..23,
                    color: 1,
                    selected: false,
                },
                Highlight {
                    row: 2,
                    cols: 0..1,
                    color: 0,
                    selected: true,
                },
            ],
        });
        roundtrip(ToEditor::Sounding {
            base: 0,
            marks: vec![],
        });
    }

    #[test]
    fn the_devices_messages_survive_the_wire() {
        roundtrip(FromDevices::Hello { version: VERSION });
        roundtrip(FromDevices::Refresh);
        roundtrip(FromDevices::Choose {
            kind: Kind::MidiIn,
            name: "Seaboard:Seaboard MIDI 1\tñ".into(),
        });
        roundtrip(ToDevices::Welcome {
            version: VERSION,
            title: "canción.lua".into(),
        });
        roundtrip(ToDevices::Bye("otra versión".into()));
        roundtrip(ToDevices::GiveBack);
        roundtrip(ToDevices::View(DevicesView::default()));
        let entry = |name: &str, active, default| Entry {
            name: name.into(),
            detail: "2 canales · 48000 Hz".into(),
            active,
            default,
        };
        roundtrip(ToDevices::View(DevicesView {
            audio: vec![
                entry("Altavoces", false, true),
                entry("USB\n«x»", true, false),
            ],
            midi_in: vec![entry("virtual", true, true)],
            midi_out: vec![],
            status: "«USB» · 48000 Hz".into(),
            notes: ["".into(), "MIDI apagado".into(), "otra".into()],
            activity: 42,
            notice: "audio: suena por «USB»".into(),
            knobs: "2 mandos · último CC 21 (canal 1) = 64".into(),
        }));
        assert!(FromDevices::decode("elegir\tbluetooth\tx").is_err());
        assert!(ToDevices::decode("vista\ta\tb\t1\t\t\t\t\t2\tsolo uno").is_err());
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(ToMain::decode("").is_err());
        assert!(ToMain::decode("nada\t1").is_err());
        assert!(ToMain::decode("texto\tsiete\thola").is_err());
        assert!(ToMain::decode("texto\t1").is_err());
        assert!(ToMain::decode("vuelvo\t0\t0\t3\ttexto\tuno").is_err());
        assert!(ToEditor::decode("aviso\tmal \\x").is_err());
        assert!(ToEditor::decode("estado\t1\t120").is_err());
        assert!(ToEditor::decode("suena\t1\t2,3,4").is_err());
        assert!(ToEditor::decode("suena\t1\t2,3,x,0,1").is_err());
        assert!(ToEditor::decode("suena").is_err());
    }

    #[test]
    fn a_connection_joins_partial_lines_and_reports_hangups() {
        let (a, b) = UnixStream::pair().unwrap();
        let mut main = Conn::new(a).unwrap();
        let mut raw = b;
        assert!(main.receive::<ToMain>().unwrap().is_empty());
        let line = ToMain::Action("Mute".into()).encode();
        let (first, second) = line.split_at(4);
        raw.write_all(first.as_bytes()).unwrap();
        assert!(main.receive::<ToMain>().unwrap().is_empty());
        raw.write_all(second.as_bytes()).unwrap();
        raw.write_all(ToMain::Hello { version: 1 }.encode().as_bytes())
            .unwrap();
        assert_eq!(
            main.receive::<ToMain>().unwrap(),
            [ToMain::Action("Mute".into()), ToMain::Hello { version: 1 }]
        );
        // Lo último antes de colgar llega; después, error.
        raw.write_all(ToMain::Command("bpm 120".into()).encode().as_bytes())
            .unwrap();
        drop(raw);
        assert_eq!(
            main.receive::<ToMain>().unwrap(),
            [ToMain::Command("bpm 120".into())]
        );
        assert!(main.receive::<ToMain>().is_err());
        assert!(
            main.send(&ToEditor::Notice("¿hola?".into())).is_err()
                || main.send(&ToEditor::Notice("¿hola?".into())).is_err()
        );
    }

    #[test]
    fn the_peer_is_this_user() {
        let (a, _b) = UnixStream::pair().unwrap();
        assert_eq!(Conn::new(a).unwrap().peer_uid().unwrap(), my_uid());
    }

    fn temp_dir(test: &str) -> PathBuf {
        let dir = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../target/tmp"))
            .join(format!("link_{test}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.parent().unwrap()).unwrap();
        dir
    }

    #[test]
    fn the_socket_is_private_and_named_after_the_script() {
        let dir = temp_dir("private");
        private_dir(&dir).unwrap();
        let mode = std::fs::metadata(&dir).unwrap().mode() & 0o777;
        assert_eq!(mode, 0o700);
        // Una carpeta que se abrió de más se cierra.
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        private_dir(&dir).unwrap();
        assert_eq!(std::fs::metadata(&dir).unwrap().mode() & 0o777, 0o700);

        let a = socket_path(&dir, Path::new("/x/canción rara.lua"));
        let b = socket_path(&dir, Path::new("/y/canción rara.lua"));
        assert_ne!(a, b);
        assert_eq!(a, socket_path(&dir, Path::new("/x/canción rara.lua")));
        assert!(
            a.file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("cancinrara-")
        );

        let server = Server::bind(&a).unwrap();
        assert_eq!(std::fs::metadata(&a).unwrap().mode() & 0o777, 0o600);
        // Una segunda principal del mismo script no puede.
        assert!(Server::bind(&a).is_err());
        drop(server);
        assert!(!a.exists(), "al cerrar se borra el socket");
    }

    #[test]
    fn a_stale_socket_is_replaced() {
        let dir = temp_dir("stale");
        private_dir(&dir).unwrap();
        let path = dir.join("viejo.sock");
        // Un socket sin nadie escuchando (como el de una principal muerta).
        let listener = UnixListener::bind(&path).unwrap();
        drop(listener);
        assert!(path.exists());
        let _server = Server::bind(&path).unwrap();
    }

    #[test]
    fn server_and_client_talk_and_survive_each_other() {
        let dir = temp_dir("talk");
        private_dir(&dir).unwrap();
        let path = dir.join("s.sock");
        let mut client = Client::new(&path);
        // Sin principal: no conecta, y no lo reintenta antes de tiempo.
        assert!(client.connect(false).is_none());
        let mut server = Server::bind(&path).unwrap();
        assert!(client.connect(false).is_none(), "espera RETRY");
        assert!(client.connect(true).is_some());
        let (fresh, rejected) = server.accept();
        assert!(fresh && rejected.is_none());
        let conn = server.client().unwrap();
        assert_eq!(
            conn.receive::<ToMain>().unwrap(),
            [ToMain::Hello { version: VERSION }]
        );
        conn.send(&ToEditor::Notice("hola".into())).unwrap();
        assert_eq!(
            client.conn().unwrap().receive::<ToEditor>().unwrap(),
            [ToEditor::Notice("hola".into())]
        );

        // Un segundo editor sustituye al primero, que recibe adiós.
        let mut other = Client::new(&path);
        assert!(other.connect(true).is_some());
        assert!(server.accept().0);
        assert_eq!(
            client.conn().unwrap().receive::<ToEditor>().unwrap(),
            [ToEditor::Bye(
                "se ha conectado otro editor a esta ventana principal".into()
            )]
        );

        // Se cae la principal: el editor lo nota y vuelve a conectar con
        // la siguiente.
        drop(server);
        assert!(other.conn().unwrap().receive::<ToEditor>().is_err());
        other.disconnect();
        let mut server = Server::bind(&path).unwrap();
        assert!(other.connect(true).is_some());
        assert!(server.accept().0);
    }
}
