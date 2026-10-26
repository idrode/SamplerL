//! Expresión por nota del MIDI de entrada (fase Q): de los mensajes de
//! bend, presión y controladores de cada canal a los ejes de cada nota que
//! suena (`engine::synth::AXES`). Sin dispositivos: se prueba con mensajes
//! simulados.
//!
//! Con MPE (zona baja), cada nota llega en su canal (del 2 al 16) y lo que
//! llega por ese canal es suyo: el bend ([`MidiConfig::bend`] semitonos
//! por bend entero, 48 por defecto), la presión del canal y el CC74
//! («slide» o «timbre»). El canal 1 es el maestro: su bend (±2 semitonos)
//! se suma al de todas las notas. Lo que un canal trae antes de la nota
//! (el bend del dedo al caer, por ejemplo) es con lo que la nota empieza.
//!
//! Sin MPE, lo de un canal va a todas sus notas (un teclado de siempre:
//! el bend mueve el acorde entero, ±2 por defecto), y la presión por nota
//! (aftertouch polifónico) a la suya.

use engine::synth::{AXES, BEND, PRESSURE, SLIDE};

use crate::midi::{MidiConfig, MidiEvent};

/// El controlador del slide en MPE (y el «brillo» de muchos teclados).
pub const SLIDE_CC: u8 = 74;
/// Semitonos del bend del canal maestro en MPE (lo que dice MPE).
const MASTER_BEND: f32 = 2.0;
/// El canal maestro de la zona baja de MPE.
const MASTER: u8 = 1;

/// Lo que se sabe de cada canal: bend (de -1 a 1), presión y slide (de 0 a
/// 1).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Channel {
    bend: f32,
    pressure: f32,
    slide: f32,
}

/// Una nota que suena: su canal, su tecla y su presión propia (la del
/// aftertouch polifónico, si llega).
#[derive(Debug, Clone, Copy, PartialEq)]
struct Held {
    channel: u8,
    key: u8,
    pressure: Option<f32>,
}

/// Un cambio de un eje de una nota.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Change {
    pub key: u8,
    pub axis: usize,
    pub value: f32,
}

/// El estado de la expresión de lo que entra por MIDI.
#[derive(Debug, Clone)]
pub struct Expression {
    mpe: bool,
    /// Semitonos del bend entero de una nota.
    range: f32,
    channels: [Channel; 16],
    held: Vec<Held>,
}

impl Expression {
    pub fn new(config: &MidiConfig) -> Expression {
        let range = config
            .bend
            .map_or(if config.mpe { 48.0 } else { 2.0 }, f32::from);
        Expression {
            mpe: config.mpe,
            range,
            channels: [Channel::default(); 16],
            held: Vec::new(),
        }
    }

    fn channel(&self, channel: u8) -> Channel {
        self.channels[(channel.clamp(1, 16) - 1) as usize]
    }

    /// Los ejes de una nota del canal `channel` ahora.
    fn axes(&self, held: &Held) -> [f32; AXES] {
        let own = self.channel(held.channel);
        let mut bend = own.bend * self.range;
        if self.mpe && held.channel != MASTER {
            bend += self.channel(MASTER).bend * MASTER_BEND;
        }
        let mut axes = [0.0; AXES];
        axes[BEND] = bend.clamp(-engine::MAX_BEND, engine::MAX_BEND);
        axes[PRESSURE] = held.pressure.unwrap_or(own.pressure);
        axes[SLIDE] = own.slide;
        axes
    }

    /// Empieza a sonar la nota `key` del canal `channel`: con lo que el
    /// canal ya trae.
    pub fn note_on(&mut self, channel: u8, key: u8) -> [f32; AXES] {
        self.held.retain(|h| h.key != key);
        let held = Held {
            channel,
            key,
            pressure: None,
        };
        self.held.push(held);
        self.axes(&held)
    }

    /// Se suelta la nota `key`. En MPE, su canal vuelve a empezar de cero
    /// para la nota siguiente (salvo que la traiga ya).
    pub fn note_off(&mut self, key: u8) {
        self.held.retain(|h| h.key != key);
    }

    /// Un mensaje: los cambios de los ejes de las notas que suenan.
    pub fn event(&mut self, event: &MidiEvent) -> Vec<Change> {
        let (channel, touched): (u8, &[usize]) = match *event {
            MidiEvent::PitchBend { channel, value } => {
                self.channels[(channel - 1) as usize].bend = value as f32 / 8192.0;
                (channel, &[BEND])
            }
            MidiEvent::ChannelPressure { channel, value } => {
                self.channels[(channel - 1) as usize].pressure = value as f32 / 127.0;
                (channel, &[PRESSURE])
            }
            MidiEvent::Controller {
                channel,
                number: SLIDE_CC,
                value,
            } => {
                self.channels[(channel - 1) as usize].slide = value as f32 / 127.0;
                (channel, &[SLIDE])
            }
            MidiEvent::PolyPressure {
                channel,
                note,
                value,
            } => {
                let mut out = Vec::new();
                for held in self
                    .held
                    .iter_mut()
                    .filter(|h| h.key == note && h.channel == channel)
                {
                    held.pressure = Some(value as f32 / 127.0);
                    out.push(Change {
                        key: note,
                        axis: PRESSURE,
                        value: value as f32 / 127.0,
                    });
                }
                return out;
            }
            _ => return Vec::new(),
        };
        // El bend del maestro mueve todas las notas; lo de otro canal, las
        // suyas.
        let all = self.mpe && channel == MASTER && touched == [BEND];
        self.held
            .iter()
            .filter(|h| all || h.channel == channel)
            .flat_map(|h| {
                let axes = self.axes(h);
                touched.iter().map(move |&axis| Change {
                    key: h.key,
                    axis,
                    value: axes[axis],
                })
            })
            .collect()
    }
}

/// Deja el último cambio de cada eje de cada nota (lo que llega entre dos
/// vueltas de la interfaz no tiene que ir entero a la cola del motor).
pub fn coalesce(changes: &mut Vec<Change>) {
    let mut kept: Vec<Change> = Vec::with_capacity(changes.len());
    for change in changes.drain(..).rev() {
        if !kept
            .iter()
            .any(|k| k.key == change.key && k.axis == change.axis)
        {
            kept.push(change);
        }
    }
    kept.reverse();
    *changes = kept;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mpe() -> Expression {
        Expression::new(&MidiConfig {
            mpe: true,
            ..MidiConfig::default()
        })
    }

    fn bend(channel: u8, value: i16) -> MidiEvent {
        MidiEvent::PitchBend { channel, value }
    }

    #[test]
    fn in_mpe_each_channel_moves_its_own_note() {
        let mut e = mpe();
        assert_eq!(e.note_on(2, 60), [0.0, 0.0, 0.0]);
        e.note_on(3, 64);
        // Medio bend arriba en el canal 2: 24 semitonos, solo en la 60.
        let changes = e.event(&bend(2, 4096));
        assert_eq!(
            changes,
            [Change {
                key: 60,
                axis: BEND,
                value: 24.0
            }]
        );
        let changes = e.event(&MidiEvent::ChannelPressure {
            channel: 3,
            value: 127,
        });
        assert_eq!(
            changes,
            [Change {
                key: 64,
                axis: PRESSURE,
                value: 1.0
            }]
        );
        let changes = e.event(&MidiEvent::Controller {
            channel: 3,
            number: 74,
            value: 0,
        });
        assert_eq!(changes[0].axis, SLIDE);
        // Otro controlador no es expresión.
        assert!(
            e.event(&MidiEvent::Controller {
                channel: 3,
                number: 1,
                value: 9
            })
            .is_empty()
        );
        // Un canal sin nota: se recuerda para la que llegue.
        assert!(e.event(&bend(4, -8192)).is_empty());
        assert_eq!(e.note_on(4, 67)[BEND], -48.0);
    }

    #[test]
    fn the_master_bend_adds_to_every_note() {
        let mut e = mpe();
        e.note_on(2, 60);
        e.note_on(3, 64);
        e.event(&bend(2, 8192 / 4));
        let changes = e.event(&bend(1, 8192 / 2));
        // ±2 del maestro: +1 en las dos, sobre el +12 de la 60.
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].value, 13.0);
        assert_eq!(changes[1].value, 1.0);
    }

    #[test]
    fn without_mpe_a_channel_moves_all_its_notes() {
        let mut e = Expression::new(&MidiConfig::default());
        e.note_on(1, 60);
        e.note_on(1, 64);
        e.note_on(2, 67);
        // ±2 por defecto: el bend entero son 2 semitonos, en el acorde.
        let changes = e.event(&bend(1, -8192));
        assert_eq!(
            changes.iter().map(|c| (c.key, c.value)).collect::<Vec<_>>(),
            [(60, -2.0), (64, -2.0)]
        );
        // La presión por nota, a la suya (y manda sobre la del canal).
        let changes = e.event(&MidiEvent::PolyPressure {
            channel: 1,
            note: 64,
            value: 127,
        });
        assert_eq!(changes.len(), 1);
        let changes = e.event(&MidiEvent::ChannelPressure {
            channel: 1,
            value: 0,
        });
        assert_eq!(
            changes.iter().map(|c| (c.key, c.value)).collect::<Vec<_>>(),
            [(60, 0.0), (64, 1.0)]
        );
        // Soltada, ya no se mueve.
        e.note_off(60);
        assert_eq!(e.event(&bend(1, 0)).len(), 1);
    }

    #[test]
    fn the_bend_range_comes_from_the_config() {
        let mut e = Expression::new(&MidiConfig {
            mpe: true,
            bend: Some(24),
            ..MidiConfig::default()
        });
        e.note_on(5, 50);
        assert_eq!(e.event(&bend(5, 8192 / 2))[0].value, 12.0);
    }

    #[test]
    fn only_the_last_change_of_each_axis_goes() {
        let change = |key, axis, value| Change { key, axis, value };
        let mut changes = vec![
            change(60, BEND, 1.0),
            change(60, PRESSURE, 0.5),
            change(60, BEND, 2.0),
            change(61, BEND, 3.0),
        ];
        coalesce(&mut changes);
        assert_eq!(
            changes,
            [
                change(60, PRESSURE, 0.5),
                change(60, BEND, 2.0),
                change(61, BEND, 3.0)
            ]
        );
    }
}
