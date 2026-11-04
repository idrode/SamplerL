//! Muestras para las visualizaciones (osciloscopio, espectro...): el hilo
//! de audio solo copia, en mono, lo que sale de cada pista (tras su
//! volumen y su pan) y del master (tras el limitador) a una cola lock-free
//! de un productor y un consumidor (`rtrb`) de capacidad fija, reservada
//! al crear el motor. Si la cola está llena, descarta: nunca bloquea ni
//! reserva. Y solo copia si la visualización de esa fuente está encendida;
//! apagada, no hace nada. Todo lo demás (ventanas, FFT, dibujo) va en el
//! hilo de la interfaz, que vacía las colas en cada `tick`.

use rtrb::{Consumer, Producer, RingBuffer};

use crate::MAX_TRACKS;

/// Fuentes: una por pista del motor y el master (la última).
pub const VIZ_SOURCES: usize = MAX_TRACKS + 1;
/// El master, como fuente.
pub const VIZ_MASTER: usize = MAX_TRACKS;
/// Muestras (mono) de cada cola: unos 0,34 s a 48 kHz.
pub const VIZ_CAPACITY: usize = 16_384;

/// Lo que escribe el hilo de audio en una fuente.
pub struct VizWriter {
    producer: Producer<f32>,
    /// Muestras descartadas por estar llena (para saberlo).
    dropped: u64,
}

/// Lo que lee la interfaz de una fuente.
pub struct VizReader {
    consumer: Consumer<f32>,
}

/// Una cola de `capacity` muestras.
pub fn viz_ring(capacity: usize) -> (VizWriter, VizReader) {
    let (producer, consumer) = RingBuffer::new(capacity);
    (
        VizWriter {
            producer,
            dropped: 0,
        },
        VizReader { consumer },
    )
}

impl VizWriter {
    /// Copia lo que cabe de `stereo` (entrelazado) en mono; lo que no
    /// cabe, se descarta. Sin reservas ni bloqueos.
    #[inline]
    pub fn push_stereo(&mut self, stereo: &[f32]) {
        let frames = stereo.len() / 2;
        let room = self.producer.slots().min(frames);
        if room < frames {
            self.dropped += (frames - room) as u64;
        }
        if room == 0 {
            return;
        }
        if let Ok(chunk) = self.producer.write_chunk_uninit(room) {
            let mono = stereo
                .as_chunks::<2>()
                .0
                .iter()
                .map(|f| 0.5 * (f[0] + f[1]));
            chunk.fill_from_iter(mono);
        }
    }

    /// Una muestra; si no cabe, se descarta.
    #[inline]
    pub fn push(&mut self, sample: f32) {
        if self.producer.push(sample).is_err() {
            self.dropped += 1;
        }
    }

    pub fn dropped(&self) -> u64 {
        self.dropped
    }
}

impl VizReader {
    /// Lo que haya en la cola, al final de `out` (hilo de control).
    pub fn drain_into(&mut self, out: &mut Vec<f32>) {
        let n = self.consumer.slots();
        if n == 0 {
            return;
        }
        if let Ok(chunk) = self.consumer.read_chunk(n) {
            out.extend(chunk);
        }
    }

    /// Las que hay ahora.
    pub fn len(&self) -> usize {
        self.consumer.slots()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Las fuentes del motor: sus colas y qué visualizaciones están encendidas
/// (las que entran con el lote, al compás).
pub(crate) struct VizTap {
    pub writers: [VizWriter; VIZ_SOURCES],
    pub on: [bool; VIZ_SOURCES],
    pub pending: Option<[bool; VIZ_SOURCES]>,
    pub staged: Option<[bool; VIZ_SOURCES]>,
}

impl VizTap {
    /// Las colas del motor y sus lectores (para el `Controller`).
    pub fn new() -> (VizTap, [VizReader; VIZ_SOURCES]) {
        let mut writers = Vec::with_capacity(VIZ_SOURCES);
        let mut readers = Vec::with_capacity(VIZ_SOURCES);
        for _ in 0..VIZ_SOURCES {
            let (w, r) = viz_ring(VIZ_CAPACITY);
            writers.push(w);
            readers.push(r);
        }
        let writers: [VizWriter; VIZ_SOURCES] = writers
            .try_into()
            .unwrap_or_else(|_| unreachable!("son VIZ_SOURCES"));
        let readers: [VizReader; VIZ_SOURCES] = readers
            .try_into()
            .unwrap_or_else(|_| unreachable!("son VIZ_SOURCES"));
        (
            VizTap {
                writers,
                on: [false; VIZ_SOURCES],
                pending: None,
                staged: None,
            },
            readers,
        )
    }

    /// Copia `stereo` en la fuente `source` si está encendida.
    #[inline]
    pub fn feed(&mut self, source: usize, stereo: &[f32]) {
        if self.on[source] {
            self.writers[source].push_stereo(stereo);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_drops_when_full_and_wraps_around() {
        let (mut w, mut r) = viz_ring(8);
        // 6 frames estéreo: entran 6 muestras mono.
        w.push_stereo(&[1.0, 1.0, 2.0, 2.0, 3.0, 3.0, 4.0, 4.0, 5.0, 5.0, 6.0, 6.0]);
        // 4 más: caben 2, las otras 2 se descartan.
        w.push_stereo(&[7.0, 7.0, 8.0, 8.0, 9.0, 9.0, 10.0, 10.0]);
        assert_eq!(w.dropped(), 2);
        let mut out = Vec::new();
        r.drain_into(&mut out);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0]);
        // Al vaciarse, vuelve a caber (dando la vuelta a la cola).
        for round in 0..20 {
            let v = round as f32;
            w.push_stereo(&[v, v, v + 0.5, v + 0.5, v + 0.25, v + 0.25]);
            out.clear();
            r.drain_into(&mut out);
            assert_eq!(out, [v, v + 0.5, v + 0.25]);
        }
        assert_eq!(w.dropped(), 2);
    }

    #[test]
    fn concurrent_producer_and_consumer_never_corrupt_or_lose() {
        // El productor escribe 0, 1, 2... en bloques; el consumidor vacía.
        // Lo que llega va siempre en orden; lo que se pierde es lo que se
        // descartó por estar llena, y nada más.
        let (mut w, mut r) = viz_ring(1024);
        const TOTAL: usize = 400_000;
        let writer = std::thread::spawn(move || {
            let mut next = 0usize;
            let mut block = [0.0f32; 128];
            while next < TOTAL {
                for (i, pair) in block.chunks_mut(2).enumerate() {
                    let v = (next + i) as f32;
                    pair[0] = v;
                    pair[1] = v;
                }
                w.push_stereo(&block);
                next += 64;
            }
            w.dropped()
        });
        let mut got: Vec<f32> = Vec::new();
        let start = std::time::Instant::now();
        loop {
            r.drain_into(&mut got);
            if writer.is_finished() && r.is_empty() {
                break;
            }
            assert!(start.elapsed().as_secs() < 30);
        }
        let dropped = writer.join().unwrap();
        assert!(got.windows(2).all(|w| w[1] > w[0]), "fuera de orden");
        assert_eq!(got.len() as u64 + dropped, TOTAL as u64);
    }
}
