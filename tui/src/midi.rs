//! MIDI sin dispositivos: leer los mensajes que llegan (notas, reloj y la
//! expresión: pitch bend, presión y controladores),
//! pasar notas a cortes y el reloj a un tempo estable. Los puertos reales
//! (ALSA, con `--allow-midi`) están en `midi_io`; todo esto se prueba con
//! mensajes simulados.

// Sin la feature `midi` no hay puerto que construya mensajes (se usan en
// los tests y en la aplicación igual).
#![cfg_attr(not(feature = "midi"), allow(dead_code))]

use std::collections::VecDeque;

/// Un mensaje MIDI de los que se usan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiEvent {
    /// Canal desde 1.
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
    },
    /// Pitch bend del canal: de -8192 a 8191 (0, en el centro).
    PitchBend {
        channel: u8,
        value: i16,
    },
    /// Presión (aftertouch) del canal entero; en MPE, la de la nota de ese
    /// canal.
    ChannelPressure {
        channel: u8,
        value: u8,
    },
    /// Presión de una nota (aftertouch polifónico).
    PolyPressure {
        channel: u8,
        note: u8,
        value: u8,
    },
    /// Un controlador (CC): en MPE, el 74 es el «slide» (timbre) de la nota.
    Controller {
        channel: u8,
        number: u8,
        value: u8,
    },
    /// Pulso de reloj (24 por negra).
    Clock,
    Start,
    Continue,
    Stop,
}

/// Un mensaje con el instante en que llegó (µs, del puerto).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TimedEvent {
    pub micros: u64,
    pub event: MidiEvent,
}

/// Lee un mensaje. Una nota con velocidad 0 es un note off. Lo demás
/// (cambio de programa, sysex...), `None`; también un byte de datos con el
/// bit alto (no es un mensaje bien formado).
pub fn parse(bytes: &[u8]) -> Option<MidiEvent> {
    let status = *bytes.first()?;
    let channel = (status & 0x0f) + 1;
    if bytes[1..].iter().any(|b| b & 0x80 != 0) && status < 0xf0 {
        return None;
    }
    match (status & 0xf0, bytes.get(1), bytes.get(2)) {
        (0xe0, Some(&lsb), Some(&msb)) => Some(MidiEvent::PitchBend {
            channel,
            value: (((msb as i16) << 7) | lsb as i16) - 8192,
        }),
        (0xd0, Some(&value), _) => Some(MidiEvent::ChannelPressure { channel, value }),
        (0xa0, Some(&note), Some(&value)) => Some(MidiEvent::PolyPressure {
            channel,
            note,
            value,
        }),
        (0xb0, Some(&number), Some(&value)) => Some(MidiEvent::Controller {
            channel,
            number,
            value,
        }),
        (0x90, Some(&note), Some(&0)) => Some(MidiEvent::NoteOff { channel, note }),
        (0x90, Some(&note), Some(&velocity)) => Some(MidiEvent::NoteOn {
            channel,
            note,
            velocity,
        }),
        (0x80, Some(&note), Some(_)) => Some(MidiEvent::NoteOff { channel, note }),
        _ => match status {
            0xf8 => Some(MidiEvent::Clock),
            0xfa => Some(MidiEvent::Start),
            0xfb => Some(MidiEvent::Continue),
            0xfc => Some(MidiEvent::Stop),
            _ => None,
        },
    }
}

/// Adónde van las notas MIDI que salen de las pistas: el puerto virtual
/// (`midi_io`) o, en los tests, una lista.
pub trait MidiSink {
    fn send(&mut self, bytes: &[u8]);
}

/// Un destino que guarda lo que recibe (para probar sin dispositivos).
#[cfg(test)]
#[derive(Debug, Clone, Default)]
pub struct Recorder(pub std::rc::Rc<std::cell::RefCell<Vec<Vec<u8>>>>);

#[cfg(test)]
impl MidiSink for Recorder {
    fn send(&mut self, bytes: &[u8]) {
        self.0.borrow_mut().push(bytes.to_vec());
    }
}

/// Qué se hace con el MIDI que entra (`[midi]` de la configuración).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiConfig {
    /// La nota que toca el corte 1 (36, el bombo de General MIDI).
    pub base_note: u8,
    /// Solo este canal (desde 1); `None`, todos.
    pub channel: Option<u8>,
    /// El reloj MIDI pone el tempo (y Start/Stop, la marcha).
    pub clock: bool,
    /// MPE: cada nota llega en su canal (del 2 al 16; el 1 es el maestro)
    /// con su bend, su presión y su slide (ver `crate::expression`). Con
    /// MPE, `channel` no filtra (las notas van por todos).
    pub mpe: bool,
    /// Semitonos del bend entero de una nota (el que dice el controlador);
    /// `None`: 48 con MPE, 2 sin él.
    pub bend: Option<u8>,
}

impl Default for MidiConfig {
    fn default() -> Self {
        MidiConfig {
            base_note: 36,
            channel: None,
            clock: false,
            mpe: false,
            bend: None,
        }
    }
}

impl MidiConfig {
    /// El corte (desde 0) que toca una nota de un canal, si toca alguno.
    pub fn slice_for(&self, channel: u8, note: u8) -> Option<usize> {
        if self.channel.is_some_and(|c| c != channel) {
            return None;
        }
        note.checked_sub(self.base_note).map(usize::from)
    }
}

/// Pulsos de reloj por negra.
pub const PPQN: u64 = 24;
/// Pulsos que se miran (cuatro negras).
const WINDOW: usize = 96;
/// Un hueco mayor que este entre pulsos empieza otra cuenta.
const GAP_MICROS: u64 = 500_000;
/// Cambio mínimo para dar un tempo nuevo (por debajo, es jitter).
const HYSTERESIS: f64 = 0.5;

/// El tempo de un reloj MIDI: el intervalo entre pulsos por mínimos
/// cuadrados sobre los últimos (la recta que mejor los ajusta, que pesa
/// todos y no solo los extremos), redondeado a una décima, y solo cuando
/// cambia de verdad (más que [`HYSTERESIS`]): el jitter de los pulsos no
/// mueve el tempo.
#[derive(Debug, Clone)]
pub struct ClockTempo {
    pulses: VecDeque<u64>,
    last: Option<f64>,
}

impl Default for ClockTempo {
    fn default() -> Self {
        ClockTempo {
            pulses: VecDeque::with_capacity(WINDOW + 1),
            last: None,
        }
    }
}

impl ClockTempo {
    /// Un pulso en `micros`. Devuelve el tempo nuevo cuando hay al menos
    /// una negra de pulsos y ha cambiado.
    pub fn pulse(&mut self, micros: u64) -> Option<f64> {
        if self
            .pulses
            .back()
            .is_some_and(|last| micros.saturating_sub(*last) > GAP_MICROS)
        {
            self.pulses.clear();
        }
        self.pulses.push_back(micros);
        if self.pulses.len() > WINDOW + 1 {
            self.pulses.pop_front();
        }
        let intervals = self.pulses.len() - 1;
        if intervals < PPQN as usize {
            return None;
        }
        // Pendiente de los instantes frente a su número de pulso.
        let n = self.pulses.len() as f64;
        let first = *self.pulses.front()? as f64;
        let mean_i = (n - 1.0) / 2.0;
        let mean_t = self.pulses.iter().map(|&t| t as f64 - first).sum::<f64>() / n;
        let (mut num, mut den) = (0.0, 0.0);
        for (i, &t) in self.pulses.iter().enumerate() {
            let di = i as f64 - mean_i;
            num += di * (t as f64 - first - mean_t);
            den += di * di;
        }
        let beat = num / den * PPQN as f64;
        let bpm = (60e6 / beat * 10.0).round() / 10.0;
        if !engine::valid_bpm(bpm) {
            return None;
        }
        match self.last {
            Some(last) if (bpm - last).abs() < HYSTERESIS => None,
            _ => {
                self.last = Some(bpm);
                Some(bpm)
            }
        }
    }

    /// Empezar otra cuenta (Start).
    pub fn reset(&mut self) {
        self.pulses.clear();
        self.last = None;
    }

    /// El último tempo dado.
    #[cfg(test)]
    pub fn bpm(&self) -> Option<f64> {
        self.last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_parse() {
        assert_eq!(
            parse(&[0x90, 36, 100]),
            Some(MidiEvent::NoteOn {
                channel: 1,
                note: 36,
                velocity: 100
            })
        );
        assert_eq!(
            parse(&[0x99, 38, 0]),
            Some(MidiEvent::NoteOff {
                channel: 10,
                note: 38
            })
        );
        assert_eq!(
            parse(&[0x83, 40, 64]),
            Some(MidiEvent::NoteOff {
                channel: 4,
                note: 40
            })
        );
        assert_eq!(parse(&[0xf8]), Some(MidiEvent::Clock));
        assert_eq!(parse(&[0xfa]), Some(MidiEvent::Start));
        assert_eq!(parse(&[0xfc]), Some(MidiEvent::Stop));
        assert_eq!(parse(&[0x90, 36]), None, "incompleto");
        assert_eq!(parse(&[0xc0, 5]), None, "cambio de programa");
        assert_eq!(parse(&[0x90, 0x80, 1]), None, "dato con el bit alto");
        assert_eq!(parse(&[]), None);
    }

    #[test]
    fn expression_messages_parse() {
        // Bend: 14 bits, LSB primero, centrado en 0x2000.
        let bend = |lsb, msb| parse(&[0xe1, lsb, msb]);
        assert_eq!(
            bend(0, 0x40),
            Some(MidiEvent::PitchBend {
                channel: 2,
                value: 0
            })
        );
        assert_eq!(
            bend(0, 0),
            Some(MidiEvent::PitchBend {
                channel: 2,
                value: -8192
            })
        );
        assert_eq!(
            bend(0x7f, 0x7f),
            Some(MidiEvent::PitchBend {
                channel: 2,
                value: 8191
            })
        );
        assert_eq!(
            bend(1, 0x40),
            Some(MidiEvent::PitchBend {
                channel: 2,
                value: 1
            })
        );
        assert_eq!(
            parse(&[0xdf, 90]),
            Some(MidiEvent::ChannelPressure {
                channel: 16,
                value: 90
            })
        );
        assert_eq!(
            parse(&[0xa3, 60, 33]),
            Some(MidiEvent::PolyPressure {
                channel: 4,
                note: 60,
                value: 33
            })
        );
        assert_eq!(
            parse(&[0xb4, 74, 127]),
            Some(MidiEvent::Controller {
                channel: 5,
                number: 74,
                value: 127
            })
        );
        assert_eq!(parse(&[0xd0]), None, "presión sin valor");
        assert_eq!(parse(&[0xe0, 0]), None, "bend incompleto");
    }

    #[test]
    fn notes_map_to_slices() {
        let config = MidiConfig::default();
        assert_eq!(config.slice_for(1, 36), Some(0));
        assert_eq!(config.slice_for(16, 43), Some(7));
        assert_eq!(config.slice_for(1, 35), None);
        let ch10 = MidiConfig {
            base_note: 60,
            channel: Some(10),
            ..MidiConfig::default()
        };
        assert_eq!(ch10.slice_for(10, 62), Some(2));
        assert_eq!(ch10.slice_for(1, 62), None);
    }

    /// Pulsos de reloj a `bpm` desde `start` µs, con un jitter de hasta
    /// ±`jitter` µs (determinista).
    fn pulses(bpm: f64, count: u64, start: u64, jitter: i64) -> Vec<u64> {
        let interval = 60e6 / bpm / PPQN as f64;
        let mut x = 0x2545_f491u64;
        (0..count)
            .map(|k| {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                let j = (x % (2 * jitter as u64 + 1)) as i64 - jitter;
                ((start as f64 + k as f64 * interval) as i64 + j).max(0) as u64
            })
            .collect()
    }

    #[test]
    fn the_clock_tempo_ignores_jitter_and_follows_changes() {
        let mut clock = ClockTempo::default();
        // 120 BPM con ±2 ms de jitter (un pulso son 20,8 ms).
        let mut given: Vec<f64> = Vec::new();
        for t in pulses(120.0, 24 * 16, 10_000, 2000) {
            given.extend(clock.pulse(t));
        }
        // Un solo tempo (o casi) en 16 negras, y cerca de 120.
        assert!(given.len() <= 2, "{given:?}");
        let bpm = clock.bpm().unwrap();
        assert!((bpm - 120.0).abs() < 0.5, "{bpm}");
        // No antes de una negra de pulsos.
        let mut fresh = ClockTempo::default();
        let early: Vec<f64> = pulses(120.0, 24, 0, 0)
            .into_iter()
            .filter_map(|t| fresh.pulse(t))
            .collect();
        assert!(early.is_empty());
        // Sube a 140: en unas negras lo sigue.
        let start = pulses(120.0, 24 * 16, 10_000, 2000).last().unwrap() + 20_833;
        for t in pulses(140.0, 24 * 5, start, 2000) {
            clock.pulse(t);
        }
        assert!(
            (clock.bpm().unwrap() - 140.0).abs() < 0.5,
            "{:?}",
            clock.bpm()
        );
        // Un hueco largo empieza otra cuenta.
        assert_eq!(clock.pulse(start + 10_000_000), None);
    }
}
