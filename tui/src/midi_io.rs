//! Puertos MIDI reales (ALSA, por `midir`; solo con la feature `midi` y
//! `--allow-midi`). Lo que llega va a la aplicación por una cola sin
//! bloqueos; la lógica está en `midi` y se prueba sin dispositivos.

use crate::midi::{TimedEvent, parse};

/// Mensajes que caben en la cola de entrada sin que la aplicación los lea.
pub const QUEUE: usize = 1024;

/// Abre el puerto virtual de entrada «sampler entrada» (se conecta con
/// `aconnect` o desde el teclado o el secuenciador). Cada mensaje que se
/// entiende va a la cola; si está llena, se pierde (nunca se bloquea).
pub fn open_input(
    mut queue: rtrb::Producer<TimedEvent>,
) -> Result<midir::MidiInputConnection<()>, String> {
    use midir::os::unix::VirtualInput;
    let mut input = midir::MidiInput::new("sampler").map_err(|e| e.to_string())?;
    // Sin esto, midir se salta los pulsos de reloj.
    input.ignore(midir::Ignore::None);
    input
        .create_virtual(
            "sampler entrada",
            move |micros, bytes, _| {
                if let Some(event) = parse(bytes) {
                    let _ = queue.push(TimedEvent { micros, event });
                }
            },
            (),
        )
        .map_err(|e| e.to_string())
}
