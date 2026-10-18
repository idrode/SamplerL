//! Estado del motor visible desde la interfaz.
//!
//! El hilo de audio solo escribe valores atómicos sueltos (sin bloqueos ni
//! reservas); la interfaz lee una instantánea con [`Controller::status`].
//! Los campos se leen por separado, así que una instantánea puede mezclar
//! valores de dos bloques consecutivos: vale para mostrar, no para decidir.
//!
//! [`Controller::status`]: crate::Controller::status

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

use crate::engine::MAX_TRACKS;

/// Instantánea del estado del motor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Status {
    /// El secuenciador está en marcha.
    pub running: bool,
    /// Tempo que está sonando.
    pub bpm: f64,
    /// Compás del último paso disparado (desde que arrancó).
    pub bar: u64,
    /// Último paso disparado (desde 0); `None` antes del primero.
    pub step: Option<usize>,
    /// Semicorchea de ese paso desde que arrancó (la que leen las pistas).
    pub global: u64,
    /// Pasos del compás que suena (16 en 4/4).
    pub bar_steps: usize,
    pub tracks: [TrackStatus; MAX_TRACKS],
    pub master: MasterStatus,
    /// Con un arreglo: dónde va la canción en la línea de tiempo.
    pub section: Option<SectionStatus>,
    /// Al entrar el último arreglo, la sección que sonaba ya no estaba y
    /// se volvió al principio (se pone a `false` al leerlo).
    pub section_lost: bool,
}

/// Posición en la línea de tiempo de un arreglo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SectionStatus {
    /// Entrada de la línea de tiempo (desde 0).
    pub entry: usize,
    /// Compás dentro de la entrada (desde 0).
    pub bar: u32,
    /// Se repite la entrada en vez de seguir.
    pub looping: bool,
}

/// Nivel del master desde la lectura anterior.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MasterStatus {
    /// Pico de la suma antes del limitador: por encima de 1 satura (el
    /// limitador lo evita comprimiendo).
    pub peak: f32,
    /// Frames en los que actuó el limitador (la suma pasó de
    /// `LIMIT_THRESHOLD`).
    pub limited_frames: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct TrackStatus {
    /// Frame de inicio en el sample del último golpe disparado (identifica
    /// el corte); `None` si la pista no ha sonado.
    pub hit_start: Option<usize>,
    /// Pico absoluto de la pista (tras su volumen) desde la lectura anterior.
    pub peak: f32,
    /// Instrumento: notas que han sonado a la vez como máximo desde la
    /// lectura anterior (sin contar las robadas que hacen su fundido).
    pub voices: usize,
    /// Granos que han nacido desde la lectura anterior.
    pub grains: u32,
}

const NONE: u64 = u64::MAX;

pub(crate) struct SharedStatus {
    running: AtomicBool,
    bpm: AtomicU64,
    bar: AtomicU64,
    step: AtomicU64,
    global: AtomicU64,
    bar_steps: AtomicU64,
    /// Posición de la canción en pasos (bits de un f64) al acabar el último
    /// bloque; `NONE` con el transporte parado.
    position: AtomicU64,
    hit_start: [AtomicU64; MAX_TRACKS],
    peak: [AtomicU32; MAX_TRACKS],
    voices: [AtomicU32; MAX_TRACKS],
    grains: [AtomicU32; MAX_TRACKS],
    master_peak: AtomicU32,
    limited: AtomicU64,
    section_entry: AtomicU64,
    section_bar: AtomicU64,
    section_looping: AtomicBool,
    section_lost: AtomicBool,
}

impl SharedStatus {
    pub fn new(bpm: f64) -> Self {
        SharedStatus {
            running: AtomicBool::new(false),
            bpm: AtomicU64::new(bpm.to_bits()),
            bar: AtomicU64::new(0),
            step: AtomicU64::new(NONE),
            global: AtomicU64::new(0),
            bar_steps: AtomicU64::new(16),
            position: AtomicU64::new(NONE),
            hit_start: std::array::from_fn(|_| AtomicU64::new(NONE)),
            peak: std::array::from_fn(|_| AtomicU32::new(0)),
            voices: std::array::from_fn(|_| AtomicU32::new(0)),
            grains: std::array::from_fn(|_| AtomicU32::new(0)),
            master_peak: AtomicU32::new(0),
            limited: AtomicU64::new(0),
            section_entry: AtomicU64::new(NONE),
            section_bar: AtomicU64::new(0),
            section_looping: AtomicBool::new(false),
            section_lost: AtomicBool::new(false),
        }
    }

    // --- Escritura (hilo de audio) ---

    /// Posición en pasos (con decimales); `None`, parado.
    pub fn set_position(&self, steps: Option<f64>) {
        self.position
            .store(steps.map_or(NONE, f64::to_bits), Relaxed);
    }

    /// La posición, sin tocar nada más (los picos no se vacían).
    pub fn position(&self) -> Option<f64> {
        let bits = self.position.load(Relaxed);
        (bits != NONE).then(|| f64::from_bits(bits))
    }

    pub fn set_transport(&self, running: bool, bpm: f64) {
        self.running.store(running, Relaxed);
        self.bpm.store(bpm.to_bits(), Relaxed);
    }

    pub fn set_step(&self, bar: u64, step: usize, global: u64, bar_steps: usize) {
        self.bar.store(bar, Relaxed);
        self.step.store(step as u64, Relaxed);
        self.global.store(global, Relaxed);
        self.bar_steps.store(bar_steps as u64, Relaxed);
    }

    pub fn clear_step(&self) {
        self.step.store(NONE, Relaxed);
    }

    pub fn set_hit(&self, track: usize, start: usize) {
        self.hit_start[track].store(start as u64, Relaxed);
    }

    /// Acumula el pico: los f32 positivos se ordenan igual que sus bits.
    pub fn add_peak(&self, track: usize, peak: f32) {
        if peak > 0.0 && peak.is_finite() {
            self.peak[track].fetch_max(peak.to_bits(), Relaxed);
        }
    }

    /// Notas del instrumento que suenan en la pista.
    pub fn add_voices(&self, track: usize, voices: usize) {
        if voices > 0 {
            self.voices[track].fetch_max(voices as u32, Relaxed);
        }
    }

    /// Posición en la línea de tiempo: (entrada, compás, repitiendo);
    /// `None` sin arreglo.
    pub fn set_section(&self, place: Option<(usize, u32, bool)>) {
        let (entry, bar, looping) = place.map_or((NONE, 0, false), |(e, b, l)| (e as u64, b, l));
        self.section_bar.store(bar as u64, Relaxed);
        self.section_looping.store(looping, Relaxed);
        self.section_entry.store(entry, Relaxed);
    }

    /// La posición en la línea de tiempo, sin tocar nada más.
    pub fn section(&self) -> Option<SectionStatus> {
        let entry = self.section_entry.load(Relaxed);
        (entry != NONE).then(|| SectionStatus {
            entry: entry as usize,
            bar: self.section_bar.load(Relaxed) as u32,
            looping: self.section_looping.load(Relaxed),
        })
    }

    /// La sección que sonaba ya no está en el arreglo nuevo.
    pub fn set_lost(&self, lost: bool) {
        if lost {
            self.section_lost.store(true, Relaxed);
        }
    }

    /// Granos que han nacido en la pista.
    pub fn add_grains(&self, track: usize, grains: u32) {
        if grains > 0 {
            self.grains[track].fetch_add(grains, Relaxed);
        }
    }

    /// Pico del master antes del limitador y frames en que actuó.
    pub fn add_master(&self, peak: f32, limited_frames: u64) {
        if peak > 0.0 && peak.is_finite() {
            self.master_peak.fetch_max(peak.to_bits(), Relaxed);
        }
        if limited_frames > 0 {
            self.limited.fetch_add(limited_frames, Relaxed);
        }
    }

    // --- Lectura (interfaz) ---

    /// Instantánea; los picos se ponen a cero al leerlos.
    pub fn snapshot(&self) -> Status {
        let opt = |v: u64| (v != NONE).then_some(v);
        Status {
            running: self.running.load(Relaxed),
            bpm: f64::from_bits(self.bpm.load(Relaxed)),
            bar: self.bar.load(Relaxed),
            step: opt(self.step.load(Relaxed)).map(|s| s as usize),
            global: self.global.load(Relaxed),
            bar_steps: self.bar_steps.load(Relaxed) as usize,
            tracks: std::array::from_fn(|t| TrackStatus {
                hit_start: opt(self.hit_start[t].load(Relaxed)).map(|s| s as usize),
                peak: f32::from_bits(self.peak[t].swap(0, Relaxed)),
                voices: self.voices[t].swap(0, Relaxed) as usize,
                grains: self.grains[t].swap(0, Relaxed),
            }),
            master: MasterStatus {
                peak: f32::from_bits(self.master_peak.swap(0, Relaxed)),
                limited_frames: self.limited.swap(0, Relaxed),
            },
            section: opt(self.section_entry.load(Relaxed)).map(|entry| SectionStatus {
                entry: entry as usize,
                bar: self.section_bar.load(Relaxed) as u32,
                looping: self.section_looping.load(Relaxed),
            }),
            section_lost: self.section_lost.swap(false, Relaxed),
        }
    }
}
