//! Puertos MIDI reales (ALSA, por `midir`; solo con la feature `midi` y
//! `--allow-midi`): los virtuales «sampler entrada» y «sampler salida» o
//! un puerto del sistema. Lo que llega va a la aplicación por una cola sin
//! bloqueos; la lógica está en `midi` y `devices` y se prueba sin
//! dispositivos.

use std::any::Any;

use crate::devices::MidiBackend;
use crate::midi::{MidiSink, TimedEvent, parse};

/// Nombre del cliente ALSA (los puertos propios no se listan).
const CLIENT: &str = "sampler";
const VIRTUAL_IN: &str = "sampler entrada";
const VIRTUAL_OUT: &str = "sampler salida";

/// Las notas de las pistas con `:midi()`: al puerto virtual (conéctalo a
/// Surge XT u otro synth con `aconnect`) o a un puerto del sistema.
pub struct Output {
    conn: midir::MidiOutputConnection,
    failed: bool,
}

impl MidiSink for Output {
    fn send(&mut self, bytes: &[u8]) {
        // Un fallo al mandar (el otro lado se fue) no para nada: se marca
        // y la aplicación vuelve al puerto virtual.
        if self.conn.send(bytes).is_err() {
            self.failed = true;
        }
    }

    fn failed(&self) -> bool {
        self.failed
    }
}

/// midir con ALSA: lista y abre puertos.
pub struct Midir;

fn names<P>(ports: Vec<P>, name: impl Fn(&P) -> Option<String>) -> Vec<String> {
    ports.iter().filter_map(name).collect()
}

impl MidiBackend for Midir {
    fn inputs(&mut self) -> Result<Vec<String>, String> {
        let input = midir::MidiInput::new(CLIENT).map_err(|e| e.to_string())?;
        Ok(names(input.ports(), |p| input.port_name(p).ok()))
    }

    fn outputs(&mut self) -> Result<Vec<String>, String> {
        let output = midir::MidiOutput::new(CLIENT).map_err(|e| e.to_string())?;
        Ok(names(output.ports(), |p| output.port_name(p).ok()))
    }

    fn open_input(
        &mut self,
        port: Option<&str>,
        mut queue: rtrb::Producer<TimedEvent>,
    ) -> Result<Box<dyn Any>, String> {
        use midir::os::unix::VirtualInput;
        let mut input = midir::MidiInput::new(CLIENT).map_err(|e| e.to_string())?;
        // Sin esto, midir se salta los pulsos de reloj.
        input.ignore(midir::Ignore::None);
        // Cada mensaje que se entiende va a la cola; si está llena, se
        // pierde (nunca se bloquea).
        let callback = move |micros: u64, bytes: &[u8], _: &mut ()| {
            if let Some(event) = parse(bytes) {
                let _ = queue.push(TimedEvent { micros, event });
            }
        };
        let conn = match port {
            None => input.create_virtual(VIRTUAL_IN, callback, ()),
            Some(name) => {
                let found = input
                    .ports()
                    .into_iter()
                    .find(|p| input.port_name(p).ok().as_deref() == Some(name))
                    .ok_or_else(|| format!("no hay ningún puerto «{name}»"))?;
                input.connect(&found, VIRTUAL_IN, callback, ())
            }
        };
        Ok(Box::new(conn.map_err(|e| e.to_string())?))
    }

    fn open_output(&mut self, port: Option<&str>) -> Result<Box<dyn MidiSink>, String> {
        use midir::os::unix::VirtualOutput;
        let output = midir::MidiOutput::new(CLIENT).map_err(|e| e.to_string())?;
        let conn = match port {
            None => output.create_virtual(VIRTUAL_OUT),
            Some(name) => {
                let found = output
                    .ports()
                    .into_iter()
                    .find(|p| output.port_name(p).ok().as_deref() == Some(name))
                    .ok_or_else(|| format!("no hay ningún puerto «{name}»"))?;
                output.connect(&found, VIRTUAL_OUT)
            }
        };
        Ok(Box::new(Output {
            conn: conn.map_err(|e| e.to_string())?,
            failed: false,
        }))
    }
}
