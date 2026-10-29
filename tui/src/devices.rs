//! Dispositivos: la salida de audio (`engine::AudioOutput`), el MIDI de
//! entrada y el de salida. Listarlos a petición (nunca se sondea en bucle),
//! elegir otro en caliente y recuperarse si el que suena desaparece. Todo
//! en el hilo de control: el audio cambia de dispositivo con un fundido
//! corto (ver `engine::device`) y el MIDI se conecta y desconecta sin tocar
//! el audio.
//!
//! El MIDI de cada lado es el puerto virtual («sampler entrada», «sampler
//! salida»; se conecta con `aconnect`) o un puerto del sistema, uno cada
//! vez. Los puertos se conocen por su nombre sin los números de ALSA del
//! final (` 24:0`), que cambian de una vez a otra.
//!
//! La lógica no sabe de tarjetas ni de ALSA: el audio viene de un
//! `engine::AudioBackend` y el MIDI de un [`MidiBackend`]; los tests usan
//! los falsos.

// El panel de dispositivos (que usa las listas) llega en la parte 3.
#![allow(dead_code)]

use std::any::Any;

use engine::{AudioDeviceInfo, AudioEvent, AudioOutput};

use crate::midi::{MidiSink, TimedEvent};

/// Mensajes que caben en la cola de entrada MIDI sin que la aplicación los
/// lea (si se llena, se pierden: nunca se bloquea).
pub const MIDI_QUEUE: usize = 1024;

/// Los puertos virtuales, como se ven en las listas.
pub const VIRTUAL_IN: &str = "sampler entrada (puerto virtual)";
pub const VIRTUAL_OUT: &str = "sampler salida (puerto virtual)";

/// Cliente ALSA de los puertos propios (no se listan: serían un bucle).
const OWN_CLIENT: &str = "sampler:";

/// El nombre de un puerto MIDI sin los números de cliente y puerto que
/// pone ALSA al final («Seaboard:Seaboard MIDI 1 24:0» → «Seaboard:Seaboard
/// MIDI 1»): así se guarda y se busca.
pub fn port_key(name: &str) -> &str {
    let Some((rest, last)) = name.rsplit_once(' ') else {
        return name;
    };
    let numbers = last.split_once(':').is_some_and(|(c, p)| {
        !c.is_empty()
            && !p.is_empty()
            && c.bytes().all(|b| b.is_ascii_digit())
            && p.bytes().all(|b| b.is_ascii_digit())
    });
    if numbers { rest } else { name }
}

/// Lo que sabe listar y abrir puertos MIDI: midir (ALSA) o el falso de los
/// tests.
pub trait MidiBackend {
    /// Puertos de los que se puede leer (con su nombre entero).
    fn inputs(&mut self) -> Result<Vec<String>, String>;
    /// Puertos a los que se puede escribir.
    fn outputs(&mut self) -> Result<Vec<String>, String>;
    /// Abre la entrada: el puerto `port` (su nombre entero) o, con `None`,
    /// el virtual. Lo que se entiende va a `queue`. La conexión dura lo que
    /// el valor devuelto.
    fn open_input(
        &mut self,
        port: Option<&str>,
        queue: rtrb::Producer<TimedEvent>,
    ) -> Result<Box<dyn Any>, String>;
    /// Abre la salida: el puerto `port` o, con `None`, el virtual.
    fn open_output(&mut self, port: Option<&str>) -> Result<Box<dyn MidiSink>, String>;
}

/// Cada una de las tres listas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Audio,
    MidiIn,
    MidiOut,
}

impl Kind {
    pub const ALL: [Kind; 3] = [Kind::Audio, Kind::MidiIn, Kind::MidiOut];

    /// Su nombre en `config.toml` y en el protocolo.
    pub fn key(self) -> &'static str {
        match self {
            Kind::Audio => "audio",
            Kind::MidiIn => "midi_entrada",
            Kind::MidiOut => "midi_salida",
        }
    }

    pub fn from_key(key: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.key() == key)
    }

    /// Título de la lista.
    pub fn title(self) -> &'static str {
        match self {
            Kind::Audio => "Audio de salida",
            Kind::MidiIn => "MIDI de entrada",
            Kind::MidiOut => "MIDI de salida",
        }
    }
}

/// Una fila de una lista.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Entry {
    /// Lo que se elige y se guarda.
    pub name: String,
    /// Canales y frecuencias (audio).
    pub detail: String,
    pub default: bool,
    /// Es el que está en uso.
    pub active: bool,
}

/// Lo que se ve en el panel de dispositivos (aquí o en su ventana).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DevicesView {
    pub audio: Vec<Entry>,
    pub midi_in: Vec<Entry>,
    pub midi_out: Vec<Entry>,
    /// La salida que suena: frecuencia, canales y latencia.
    pub status: String,
    /// Por qué una lista está vacía o apagada (sin `--allow-audio`, sin
    /// `--allow-midi`...), por lista.
    pub notes: [String; 3],
    /// Notas MIDI recibidas (el indicador parpadea cuando cambia).
    pub activity: u64,
    /// El último aviso de los dispositivos.
    pub notice: String,
}

impl DevicesView {
    pub fn list(&self, kind: Kind) -> &[Entry] {
        match kind {
            Kind::Audio => &self.audio,
            Kind::MidiIn => &self.midi_in,
            Kind::MidiOut => &self.midi_out,
        }
    }

    pub fn note(&self, kind: Kind) -> &str {
        &self.notes[kind as usize]
    }
}

/// Lo que la aplicación tiene que hacer tras una operación.
pub enum Change {
    /// Un aviso para la línea de estado.
    Message(String),
    /// La cola de la entrada MIDI nueva.
    Input(rtrb::Consumer<TimedEvent>),
    /// El destino nuevo de las notas.
    Output(Box<dyn MidiSink>),
}

/// Lo elegido, por nombre (lo que se guarda en `[dispositivos]`): `None`,
/// el predeterminado o el puerto virtual.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Choice {
    pub audio: Option<String>,
    pub midi_in: Option<String>,
    pub midi_out: Option<String>,
}

impl Choice {
    pub fn get(&self, kind: Kind) -> Option<&str> {
        match kind {
            Kind::Audio => self.audio.as_deref(),
            Kind::MidiIn => self.midi_in.as_deref(),
            Kind::MidiOut => self.midi_out.as_deref(),
        }
    }

    pub fn set(&mut self, kind: Kind, value: Option<String>) {
        match kind {
            Kind::Audio => self.audio = value,
            Kind::MidiIn => self.midi_in = value,
            Kind::MidiOut => self.midi_out = value,
        }
    }
}

/// Los dispositivos de una ventana principal.
pub struct Devices {
    /// La salida (la nula sin `--allow-audio`); `None` en los tests que no
    /// la usan.
    audio: Option<AudioOutput>,
    audio_allowed: bool,
    /// `None`: MIDI apagado (sin `--allow-midi` o sin la feature).
    midi: Option<Box<dyn MidiBackend>>,
    audio_list: Vec<AudioDeviceInfo>,
    audio_error: Option<String>,
    midi_inputs: Vec<String>,
    midi_outputs: Vec<String>,
    midi_error: Option<String>,
    /// La conexión de entrada abierta (se cierra al soltarla).
    input: Option<Box<dyn Any>>,
    /// Lo elegido y en uso (MIDI: el nombre sin números; `None`, el
    /// virtual).
    chosen: Choice,
    /// Hay salida MIDI abierta.
    output_open: bool,
    /// Notas MIDI recibidas.
    activity: u64,
    notice: String,
}

impl Devices {
    pub fn new(
        audio: Option<AudioOutput>,
        audio_allowed: bool,
        midi: Option<Box<dyn MidiBackend>>,
    ) -> Devices {
        let mut devices = Devices {
            audio,
            audio_allowed,
            midi,
            audio_list: Vec::new(),
            audio_error: None,
            midi_inputs: Vec::new(),
            midi_outputs: Vec::new(),
            midi_error: None,
            input: None,
            chosen: Choice::default(),
            output_open: false,
            activity: 0,
            notice: String::new(),
        };
        if devices.audio_allowed {
            devices.chosen.audio = devices.audio_info().map(|i| i.name);
        }
        devices.read_lists();
        devices
    }

    /// Lo elegido ahora (para guardarlo).
    pub fn choice(&self) -> &Choice {
        &self.chosen
    }

    pub fn midi_enabled(&self) -> bool {
        self.midi.is_some()
    }

    pub fn audio_info(&self) -> Option<engine::OutputInfo> {
        self.audio.as_ref().and_then(AudioOutput::info)
    }

    /// Abre el MIDI al arrancar: los puertos pedidos (por nombre, con o
    /// sin los números) o, si no están, los virtuales y un aviso.
    pub fn start_midi(&mut self, input: Option<&str>, output: Option<&str>) -> Vec<Change> {
        let mut changes = Vec::new();
        if self.midi.is_none() {
            return changes;
        }
        changes.extend(self.connect_input(input, true));
        changes.extend(self.connect_output(output, true));
        changes
    }

    /// Vuelve a leer las listas (a petición: acción de la paleta o `r` en
    /// el panel). Si el puerto MIDI de entrada en uso ya no está, vuelve
    /// al virtual.
    pub fn refresh(&mut self) -> Vec<Change> {
        self.read_lists();
        let mut changes = Vec::new();
        if let Some(port) = self.chosen.midi_in.clone()
            && self.midi_error.is_none()
            && self.find_port(&self.midi_inputs, &port).is_none()
        {
            changes.push(self.notice(format!(
                "MIDI de entrada: «{port}» ya no está; vuelve al puerto virtual"
            )));
            changes.extend(self.connect_input(None, false));
        }
        if let Some(port) = self.chosen.midi_out.clone()
            && self.midi_error.is_none()
            && self.find_port(&self.midi_outputs, &port).is_none()
        {
            changes.push(self.notice(format!(
                "MIDI de salida: «{port}» ya no está; vuelve al puerto virtual"
            )));
            changes.extend(self.connect_output(None, false));
        }
        if changes.is_empty() {
            let count = self.audio_list.len() + self.midi_inputs.len() + self.midi_outputs.len();
            changes.push(self.notice(format!("dispositivos: lista al día ({count} en total)")));
        }
        changes
    }

    fn read_lists(&mut self) {
        match self.audio.as_mut().filter(|_| self.audio_allowed) {
            Some(audio) => match audio.list() {
                Ok(list) => {
                    self.audio_list = list;
                    self.audio_error = None;
                }
                Err(e) => {
                    self.audio_list.clear();
                    self.audio_error = Some(e);
                }
            },
            None => self.audio_list.clear(),
        }
        if let Some(midi) = &mut self.midi {
            let inputs = midi.inputs();
            let outputs = midi.outputs();
            self.midi_error = inputs.as_ref().err().or(outputs.as_ref().err()).cloned();
            let own = |list: Vec<String>| -> Vec<String> {
                list.into_iter()
                    .filter(|p| !p.starts_with(OWN_CLIENT))
                    .collect()
            };
            self.midi_inputs = own(inputs.unwrap_or_default());
            self.midi_outputs = own(outputs.unwrap_or_default());
        }
    }

    /// El puerto de la lista que es `wanted` (con o sin los números).
    fn find_port(&self, list: &[String], wanted: &str) -> Option<String> {
        list.iter()
            .find(|p| p.as_str() == wanted)
            .or_else(|| list.iter().find(|p| port_key(p) == port_key(wanted)))
            .cloned()
    }

    fn notice(&mut self, text: String) -> Change {
        self.notice = text.clone();
        Change::Message(text)
    }

    /// Elige el dispositivo `name` de la lista `kind` (el nombre como se
    /// ve; el de los puertos virtuales, también).
    pub fn choose(&mut self, kind: Kind, name: &str) -> Vec<Change> {
        match kind {
            Kind::Audio => vec![self.choose_audio(name)],
            Kind::MidiIn | Kind::MidiOut if self.midi.is_none() => {
                vec![self.notice(
                    "MIDI apagado: abre con --allow-midi (o --midi-in / --midi-out)".into(),
                )]
            }
            Kind::MidiIn => {
                let port = (name != VIRTUAL_IN).then_some(name);
                self.connect_input(port, false)
            }
            Kind::MidiOut => {
                let port = (name != VIRTUAL_OUT).then_some(name);
                self.connect_output(port, false)
            }
        }
    }

    fn choose_audio(&mut self, name: &str) -> Change {
        if !self.audio_allowed {
            return self.notice(
                "sin --allow-audio no se abre la tarjeta: el motor avanza con la salida nula"
                    .into(),
            );
        }
        let Some(audio) = &mut self.audio else {
            return self.notice("sin salida de audio".into());
        };
        if audio.info().is_some_and(|i| i.name == name && !i.null) {
            return self.notice(format!("audio: ya suena por «{name}»"));
        }
        match audio.switch(Some(name)) {
            Ok(()) => {
                self.chosen.audio = Some(name.into());
                self.notice(format!("audio: cambiando a «{name}»…"))
            }
            Err(e) => self.notice(format!(
                "audio: no se pudo abrir «{name}» ({e}); sigue el que estaba"
            )),
        }
    }

    /// Conecta la entrada MIDI: `port` (por nombre) o el virtual. Si no se
    /// puede, la que había sigue (o, al arrancar, el virtual).
    fn connect_input(&mut self, port: Option<&str>, starting: bool) -> Vec<Change> {
        let mut changes = Vec::new();
        let found = match port {
            Some(wanted) => match self.find_port(&self.midi_inputs, wanted) {
                Some(found) => Some(found),
                None if starting => {
                    changes.push(self.notice(format!(
                        "MIDI de entrada: no está «{wanted}»; abre el puerto virtual"
                    )));
                    None
                }
                None => {
                    changes.push(self.notice(format!(
                        "MIDI de entrada: no está «{wanted}» (r para refrescar la lista)"
                    )));
                    return changes;
                }
            },
            None => None,
        };
        // Por si falla: la que había, para volver a ella.
        let previous = self
            .chosen
            .midi_in
            .clone()
            .and_then(|p| self.find_port(&self.midi_inputs, &p));
        let had_input = self.input.is_some();
        let Some(midi) = &mut self.midi else {
            return changes;
        };
        // La vieja se cierra antes de abrir la nueva: midir no deja dos
        // puertos virtuales con el mismo nombre.
        self.input = None;
        let shown = found.as_deref().map_or(VIRTUAL_IN, port_key).to_string();
        let (tx, rx) = rtrb::RingBuffer::new(MIDI_QUEUE);
        match midi.open_input(found.as_deref(), tx) {
            Ok(conn) => {
                self.input = Some(conn);
                self.chosen.midi_in = found.as_deref().map(|f| port_key(f).to_string());
                if !starting || found.is_some() {
                    changes.push(self.notice(format!("MIDI de entrada: «{shown}»")));
                }
                changes.push(Change::Input(rx));
            }
            Err(e) => {
                let (tx, back) = rtrb::RingBuffer::new(MIDI_QUEUE);
                let reopened = had_input
                    .then(|| midi.open_input(previous.as_deref(), tx).ok())
                    .flatten();
                let restored = reopened.is_some();
                if let Some(conn) = reopened {
                    self.input = Some(conn);
                    changes.push(Change::Input(back));
                }
                let rest = if restored {
                    "; sigue la que había"
                } else {
                    ""
                };
                changes.push(self.notice(format!(
                    "MIDI de entrada: no se pudo abrir «{shown}» ({e}){rest}"
                )));
            }
        }
        changes
    }

    /// Conecta la salida MIDI: `port` o el virtual. Si no se puede, sigue
    /// la que había.
    fn connect_output(&mut self, port: Option<&str>, starting: bool) -> Vec<Change> {
        let mut changes = Vec::new();
        let found = match port {
            Some(wanted) => match self.find_port(&self.midi_outputs, wanted) {
                Some(found) => Some(found),
                None if starting => {
                    changes.push(self.notice(format!(
                        "MIDI de salida: no está «{wanted}»; abre el puerto virtual"
                    )));
                    None
                }
                None => {
                    changes.push(self.notice(format!(
                        "MIDI de salida: no está «{wanted}» (r para refrescar la lista)"
                    )));
                    return changes;
                }
            },
            None => None,
        };
        let Some(midi) = &mut self.midi else {
            return changes;
        };
        let shown = found.as_deref().map_or(VIRTUAL_OUT, port_key).to_string();
        match midi.open_output(found.as_deref()) {
            Ok(sink) => {
                self.output_open = true;
                self.chosen.midi_out = found.as_deref().map(|f| port_key(f).to_string());
                if !starting || found.is_some() {
                    changes.push(self.notice(format!("MIDI de salida: «{shown}»")));
                }
                changes.push(Change::Output(sink));
            }
            Err(e) => changes
                .push(self.notice(format!("MIDI de salida: no se pudo abrir «{shown}» ({e})"))),
        }
        changes
    }

    /// Atiende los dispositivos en cada vuelta: termina un cambio de audio,
    /// se recupera si el que suena se fue y, si la salida MIDI dejó de
    /// aceptar mensajes (`output_failed`), vuelve al puerto virtual.
    pub fn poll(&mut self, output_failed: bool) -> Vec<Change> {
        let mut changes = Vec::new();
        let events = self
            .audio
            .as_mut()
            .map(AudioOutput::poll)
            .unwrap_or_default();
        for event in events {
            let text = match event {
                AudioEvent::Switched { name } => {
                    let latency = self.audio_info().map_or(0.0, |i| i.latency_ms());
                    format!("audio: suena por «{name}» (latencia ≈ {latency:.1} ms)")
                }
                AudioEvent::Lost { name, to, null } => {
                    self.chosen.audio = (!null).then(|| to.clone());
                    if null {
                        format!(
                            "audio: «{name}» ya no está y no hay otro que abrir: sigue sin \
                             sonido (salida nula); elige otro en el panel de dispositivos"
                        )
                    } else {
                        format!("audio: «{name}» ya no está; suena por «{to}»")
                    }
                }
                AudioEvent::Rerouted { name } => {
                    format!("audio: el sistema llevó «{name}» a otro dispositivo")
                }
            };
            changes.push(self.notice(text));
        }
        if output_failed && self.output_open && self.chosen.midi_out.is_some() {
            let port = self.chosen.midi_out.clone().unwrap_or_default();
            changes.push(self.notice(format!(
                "MIDI de salida: «{port}» no acepta mensajes; vuelve al puerto virtual"
            )));
            changes.extend(self.connect_output(None, false));
        }
        changes
    }

    /// Una nota MIDI que ha llegado (para el indicador de actividad).
    pub fn note_received(&mut self) {
        self.activity = self.activity.wrapping_add(1);
    }

    /// Lo que se ve en el panel.
    pub fn view(&self) -> DevicesView {
        let info = self.audio_info();
        let audio = if self.audio_allowed {
            self.audio_list
                .iter()
                .map(|d| Entry {
                    name: d.name.clone(),
                    detail: audio_detail(d),
                    default: d.default,
                    active: info.as_ref().is_some_and(|i| !i.null && i.name == d.name),
                })
                .collect()
        } else {
            vec![Entry {
                name: engine::NULL_NAME.into(),
                detail: "el motor avanza sin abrir la tarjeta".into(),
                default: true,
                active: true,
            }]
        };
        let ports = |list: &[String], virtual_name: &str, chosen: Option<&str>| {
            let mut out = vec![Entry {
                name: virtual_name.into(),
                detail: "se conecta con aconnect".into(),
                default: true,
                active: chosen.is_none(),
            }];
            out.extend(list.iter().map(|p| Entry {
                name: port_key(p).into(),
                detail: String::new(),
                default: false,
                active: chosen == Some(port_key(p)),
            }));
            out
        };
        let (midi_in, midi_out) = if self.midi.is_some() {
            (
                ports(
                    &self.midi_inputs,
                    VIRTUAL_IN,
                    self.chosen.midi_in.as_deref(),
                ),
                ports(
                    &self.midi_outputs,
                    VIRTUAL_OUT,
                    self.chosen.midi_out.as_deref(),
                ),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        let status = match &info {
            Some(i) if i.null => format!("{} · {} Hz", engine::NULL_NAME, i.sample_rate),
            Some(i) => {
                let mut text = format!(
                    "«{}» · {} Hz · {} canales · latencia ≈ {:.1} ms",
                    i.name,
                    i.sample_rate,
                    i.channels,
                    i.latency_ms()
                );
                if i.switching {
                    text.push_str(" · cambiando…");
                }
                if i.glitches > 0 {
                    text.push_str(&format!(" · {} cortes", i.glitches));
                }
                text
            }
            None => "sin salida de audio".into(),
        };
        let audio_note = if !self.audio_allowed {
            "sin --allow-audio: no se lista ni se abre la tarjeta".to_string()
        } else if let Some(e) = &self.audio_error {
            format!("no se pudo listar: {e}")
        } else if self.audio_list.is_empty() {
            "no hay dispositivos de salida (¿PipeWire en marcha?)".into()
        } else {
            String::new()
        };
        let midi_note = if self.midi.is_none() {
            "MIDI apagado: abre con --allow-midi".to_string()
        } else if let Some(e) = &self.midi_error {
            format!("no se pudo listar: {e}")
        } else {
            String::new()
        };
        DevicesView {
            audio,
            midi_in,
            midi_out,
            status,
            notes: [audio_note, midi_note.clone(), midi_note],
            activity: self.activity,
            notice: self.notice.clone(),
        }
    }

    /// Marca lo elegido sin abrir nada (lo pedido al arrancar que ya está
    /// abierto: el dispositivo de audio con el que se abrió la salida).
    pub fn set_chosen(&mut self, kind: Kind, value: Option<String>) {
        self.chosen.set(kind, value);
    }
}

/// «2 canales · 44100, 48000 Hz».
fn audio_detail(d: &AudioDeviceInfo) -> String {
    let rates: Vec<String> = d.sample_rates.iter().map(u32::to_string).collect();
    format!("{} canales · {} Hz", d.channels, rates.join(", "))
}

/// Un MIDI falso para los tests: puertos que se enchufan y desenchufan y
/// conexiones que se cuentan; las entradas se alimentan a mano.
#[cfg(test)]
pub mod fake {
    use std::cell::RefCell;
    use std::rc::Rc;

    use super::*;

    #[derive(Default)]
    pub struct State {
        pub inputs: Vec<String>,
        pub outputs: Vec<String>,
        /// Lo conectado ahora (`None`: el virtual).
        pub input: Option<Option<String>>,
        pub output: Option<Option<String>>,
        /// La cola de la entrada abierta.
        pub queue: Option<rtrb::Producer<TimedEvent>>,
        /// Lo que ha salido, por puerto.
        pub sent: Vec<(Option<String>, Vec<u8>)>,
        /// Los puertos que fallan al abrir.
        pub broken: Vec<String>,
        /// La salida falla al mandar.
        pub failing: bool,
    }

    #[derive(Clone, Default)]
    pub struct FakeMidi(pub Rc<RefCell<State>>);

    struct Input(Rc<RefCell<State>>);

    impl Drop for Input {
        fn drop(&mut self) {
            let mut state = self.0.borrow_mut();
            state.input = None;
            state.queue = None;
        }
    }

    struct Output(Rc<RefCell<State>>, Option<String>);

    impl MidiSink for Output {
        fn send(&mut self, bytes: &[u8]) {
            let mut state = self.0.borrow_mut();
            if !state.failing {
                state.sent.push((self.1.clone(), bytes.to_vec()));
            }
        }

        fn failed(&self) -> bool {
            self.0.borrow().failing
        }
    }

    impl FakeMidi {
        pub fn with_ports(inputs: &[&str], outputs: &[&str]) -> FakeMidi {
            let fake = FakeMidi::default();
            {
                let mut state = fake.0.borrow_mut();
                state.inputs = inputs.iter().map(|s| s.to_string()).collect();
                state.outputs = outputs.iter().map(|s| s.to_string()).collect();
            }
            fake
        }

        /// Llega un mensaje a la entrada abierta.
        pub fn receive(&self, bytes: &[u8]) {
            let mut state = self.0.borrow_mut();
            let event = crate::midi::parse(bytes).expect("mensaje válido");
            if let Some(q) = &mut state.queue {
                let _ = q.push(TimedEvent { micros: 0, event });
            }
        }

        pub fn backend(&self) -> Box<dyn MidiBackend> {
            Box::new(self.clone())
        }
    }

    impl MidiBackend for FakeMidi {
        fn inputs(&mut self) -> Result<Vec<String>, String> {
            Ok(self.0.borrow().inputs.clone())
        }

        fn outputs(&mut self) -> Result<Vec<String>, String> {
            Ok(self.0.borrow().outputs.clone())
        }

        fn open_input(
            &mut self,
            port: Option<&str>,
            queue: rtrb::Producer<TimedEvent>,
        ) -> Result<Box<dyn Any>, String> {
            let mut state = self.0.borrow_mut();
            if let Some(p) = port
                && (state.broken.iter().any(|b| b == p) || !state.inputs.iter().any(|i| i == p))
            {
                return Err("el puerto no responde".into());
            }
            assert!(state.input.is_none(), "dos entradas abiertas a la vez");
            state.input = Some(port.map(String::from));
            state.queue = Some(queue);
            Ok(Box::new(Input(self.0.clone())))
        }

        fn open_output(&mut self, port: Option<&str>) -> Result<Box<dyn MidiSink>, String> {
            let mut state = self.0.borrow_mut();
            if let Some(p) = port
                && (state.broken.iter().any(|b| b == p) || !state.outputs.iter().any(|o| o == p))
            {
                return Err("el puerto no responde".into());
            }
            state.output = Some(port.map(String::from));
            state.failing = false;
            Ok(Box::new(Output(self.0.clone(), port.map(String::from))))
        }
    }
}
