//! Dispositivos de salida de audio: listarlos, abrir uno y cambiar a otro
//! en caliente sin perder la canción.
//!
//! El motor no vive dentro del callback de un stream, sino en una ranura
//! atómica ([`EngineSlot`]) que el callback toma y devuelve en cada bloque.
//! Así el mismo motor (con su canción, su transporte y su compás) pasa de un
//! dispositivo a otro: se abre el nuevo, el viejo baja a cero en un fundido
//! corto ([`FADE_SECONDS`]), se suelta, y el nuevo sube con otro fundido.
//! Todo lo que abre y cierra va en el hilo de control ([`AudioOutput`]);
//! el callback ([`Renderer::render`]) solo lee y escribe atómicos y llama a
//! [`Engine::process`]: sin reservas, mutex ni E/S.
//!
//! El cambio es al instante y no al compás: abrir un dispositivo tarda lo
//! que tarde (no se puede cuantizar) y un dispositivo que desaparece no
//! espera. La canción se queda quieta los milisegundos del cambio y sigue
//! en el mismo paso.
//!
//! El dispositivo nuevo se abre a la frecuencia del motor: los samples
//! están resampleados a ella al cargarlos (regla 6), así que uno que no la
//! admite se rechaza y sigue el anterior.

use std::any::Any;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::engine::Engine;

/// Lo que dura cada fundido del cambio de dispositivo (de salida y de
/// entrada): sin clic y sin que se note el hueco.
pub const FADE_SECONDS: f32 = 0.005;
/// Lo que se espera al fundido del dispositivo viejo antes de soltarlo sin
/// más (si ya no llama a su callback, se fue).
pub const HANDOFF_TIMEOUT: Duration = Duration::from_millis(150);
/// Frecuencias que se prueban para decir cuáles admite un dispositivo.
pub const COMMON_RATES: [u32; 7] = [22_050, 32_000, 44_100, 48_000, 88_200, 96_000, 192_000];

/// Un dispositivo de salida tal como se lista.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioDeviceInfo {
    /// El nombre que se ve y se guarda en `config.toml`.
    pub name: String,
    /// El identificador del sistema (en ALSA, el nombre del PCM): también
    /// vale para elegirlo.
    pub id: String,
    /// Canales de salida (los más que admite).
    pub channels: u16,
    /// Frecuencias admitidas, de menor a mayor (las habituales que caben
    /// en sus rangos y los extremos de cada rango).
    pub sample_rates: Vec<u32>,
    /// Es el predeterminado del sistema.
    pub default: bool,
}

impl AudioDeviceInfo {
    /// Si este dispositivo es el que se pide por `name` (su nombre o su
    /// identificador).
    pub fn matches(&self, name: &str) -> bool {
        self.name == name || self.id == name
    }
}

/// Frecuencias que se listan para unos rangos `(mín, máx)`.
pub fn listed_rates(ranges: &[(u32, u32)]) -> Vec<u32> {
    let mut rates: Vec<u32> = ranges
        .iter()
        .flat_map(|&(lo, hi)| {
            COMMON_RATES
                .into_iter()
                .filter(move |r| (lo..=hi).contains(r))
                .chain([lo, hi])
        })
        .collect();
    rates.sort_unstable();
    rates.dedup();
    rates
}

/// La ranura del motor: un puntero que el callback que está sonando toma
/// (lo deja nulo) y devuelve en cada bloque. Nadie más lo toca mientras un
/// stream esté activo; si un callback lo encuentra nulo, da silencio.
pub struct EngineSlot {
    engine: AtomicPtr<Engine>,
}

impl EngineSlot {
    pub fn new(engine: Engine) -> EngineSlot {
        EngineSlot {
            engine: AtomicPtr::new(Box::into_raw(Box::new(engine))),
        }
    }

    /// Hace algo con el motor desde el hilo de control. Solo vale sin
    /// ningún stream activo (si no, el callback podría tenerlo); `None` si
    /// está fuera.
    fn with<R>(&self, f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
        let p = self.engine.swap(ptr::null_mut(), Ordering::Acquire);
        if p.is_null() {
            return None;
        }
        // SAFETY: el puntero salió de `Box::into_raw` y, al cambiarlo por
        // nulo, nadie más lo tiene hasta que se devuelve.
        let result = f(unsafe { &mut *p });
        self.engine.store(p, Ordering::Release);
        Some(result)
    }
}

impl Drop for EngineSlot {
    fn drop(&mut self) {
        let p = self.engine.swap(ptr::null_mut(), Ordering::Acquire);
        if !p.is_null() {
            // SAFETY: ya no queda ningún callback (la ranura es la última
            // referencia) y el puntero es de `Box::into_raw`.
            drop(unsafe { Box::from_raw(p) });
        }
    }
}

/// Lo que comparten un stream y el hilo de control: todo atómicos.
#[derive(Default)]
pub struct StreamShared {
    /// Puede tomar el motor (lo pone el control cuando el anterior ya
    /// soltó el suyo).
    active: AtomicBool,
    /// Que baje a cero y suelte el motor.
    stop: AtomicBool,
    /// Ya bajó y no lo vuelve a tocar.
    done: AtomicBool,
    /// El dispositivo se fue (lo dice el error del backend).
    lost: AtomicBool,
    /// La ruta cambió y el backend lo llevó a otro dispositivo él solo.
    rerouted: AtomicBool,
    /// Errores que no paran el stream (underruns...).
    errors: AtomicU32,
    /// Frames que tarda en oírse el final del último bloque (latencia del
    /// dispositivo más el bloque), según el propio callback.
    latency: AtomicU64,
    /// Bloques servidos.
    callbacks: AtomicU64,
}

/// Un error que ha llegado del backend para un stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFault {
    /// El dispositivo ya no está (desenchufado, el servidor de audio
    /// cayó...): hay que cambiar a otro.
    Lost,
    /// El backend lo llevó a otro dispositivo él solo (sigue sonando).
    Rerouted,
    /// Un tropiezo (underrun...): se cuenta y sigue.
    Glitch,
}

/// El trabajo del callback de un stream: toma el motor de la ranura,
/// calcula el bloque, aplica los fundidos y lo devuelve. Lo crea
/// [`AudioOutput`] y se lo pasa al backend al abrir.
pub struct Renderer {
    slot: Arc<EngineSlot>,
    shared: Arc<StreamShared>,
    /// Ganancia del fundido, de 0 a 1, y lo que cambia por frame.
    gain: f32,
    fade_step: f32,
    done: bool,
}

impl Renderer {
    fn new(slot: Arc<EngineSlot>, shared: Arc<StreamShared>) -> Renderer {
        Renderer {
            slot,
            shared,
            gain: 0.0,
            fade_step: 1.0,
            done: false,
        }
    }

    /// Ajusta la duración de los fundidos a la frecuencia del stream (antes
    /// de arrancarlo).
    pub fn set_rate(&mut self, sample_rate: u32) {
        self.fade_step = 1.0 / (FADE_SECONDS * sample_rate.max(1) as f32).max(1.0);
    }

    /// Rellena `out` (entrelazado, `channels` canales). `latency` son los
    /// frames hasta que se oye el principio de este bloque, si el backend
    /// lo sabe. Sin reservas ni bloqueos: se llama desde el hilo de audio.
    pub fn render(&mut self, out: &mut [f32], channels: usize, latency: u64) {
        let channels = channels.max(1);
        let shared = &*self.shared;
        shared.callbacks.fetch_add(1, Ordering::Relaxed);
        let delay = latency + (out.len() / channels) as u64;
        shared.latency.store(delay, Ordering::Relaxed);
        if self.done || !shared.active.load(Ordering::Acquire) {
            out.fill(0.0);
            return;
        }
        let p = self.slot.engine.swap(ptr::null_mut(), Ordering::Acquire);
        if p.is_null() {
            out.fill(0.0);
            return;
        }
        // SAFETY: el puntero es de `Box::into_raw` y, al cambiarlo por nulo,
        // este callback es el único que lo tiene hasta devolverlo.
        let engine = unsafe { &mut *p };
        engine.set_output_delay(delay);
        engine.process(out, channels);
        let target = if shared.stop.load(Ordering::Acquire) {
            0.0
        } else {
            1.0
        };
        if self.gain != 1.0 || target != 1.0 {
            for frame in out.chunks_mut(channels) {
                self.gain = if target > self.gain {
                    (self.gain + self.fade_step).min(target)
                } else {
                    (self.gain - self.fade_step).max(target)
                };
                frame.iter_mut().for_each(|s| *s *= self.gain);
            }
        }
        self.slot.engine.store(p, Ordering::Release);
        if target == 0.0 && self.gain == 0.0 {
            // Primero se devuelve el motor, después se dice que se soltó.
            self.done = true;
            shared.done.store(true, Ordering::Release);
        }
    }

    /// Un error del backend para este stream. Solo atómicos: se puede
    /// llamar desde cualquier hilo.
    pub fn fault(&self, fault: StreamFault) {
        fault_on(&self.shared, fault);
    }

    /// Lo que hace falta para avisar de errores desde otro cierre (el
    /// callback de error de cpal no es el de datos).
    pub fn fault_handle(&self) -> FaultHandle {
        FaultHandle(self.shared.clone())
    }
}

/// Para avisar de errores de un stream desde otro hilo.
#[derive(Clone)]
pub struct FaultHandle(Arc<StreamShared>);

impl FaultHandle {
    pub fn fault(&self, fault: StreamFault) {
        fault_on(&self.0, fault);
    }
}

fn fault_on(shared: &StreamShared, fault: StreamFault) {
    match fault {
        StreamFault::Lost => shared.lost.store(true, Ordering::Release),
        StreamFault::Rerouted => shared.rerouted.store(true, Ordering::Release),
        StreamFault::Glitch => {
            shared.errors.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Un stream abierto por un backend. Suena mientras exista `guard`.
pub struct OpenStream {
    /// El nombre del dispositivo que se abrió (el de la lista).
    pub name: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// Tamaño del bloque pedido o declarado por el dispositivo (frames), si
    /// se sabe.
    pub buffer: Option<u32>,
    pub guard: Box<dyn Any>,
}

/// Lo que sabe listar y abrir dispositivos de salida: cpal, la salida
/// nula o uno falso en los tests.
pub trait AudioBackend {
    /// Cómo se llama (para los mensajes): «ALSA», «salida nula»...
    fn label(&self) -> String;
    /// Los dispositivos de salida que hay ahora.
    fn list(&mut self) -> Result<Vec<AudioDeviceInfo>, String>;
    /// Abre `device` (por su nombre o identificador; `None`, el
    /// predeterminado) a `sample_rate` (`None`: la suya por defecto) y
    /// llama a `renderer` desde su hilo de audio. Si el dispositivo no
    /// admite esa frecuencia, error.
    fn open(
        &mut self,
        device: Option<&str>,
        sample_rate: Option<u32>,
        renderer: Renderer,
    ) -> Result<OpenStream, String>;
}

/// No se pudo abrir ningún dispositivo: el porqué y el motor de vuelta.
pub struct OpenError {
    pub message: String,
    pub engine: Box<Engine>,
}

impl std::fmt::Debug for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// Un stream con lo que comparte con su callback.
struct Active {
    stream: OpenStream,
    shared: Arc<StreamShared>,
    /// Abierto en la salida nula (como último recurso).
    null: bool,
}

/// El dispositivo viejo bajando a cero mientras el nuevo espera.
struct Handoff {
    old: Active,
    deadline: Instant,
}

/// Lo que ha pasado con la salida (para el mensaje de la interfaz).
#[derive(Debug, Clone, PartialEq)]
pub enum AudioEvent {
    /// Ya suena por el dispositivo nuevo.
    Switched { name: String },
    /// El dispositivo se fue y se cambió a otro (`to`), o a la salida nula.
    Lost {
        name: String,
        to: String,
        null: bool,
    },
    /// El backend llevó el stream a otro dispositivo él solo.
    Rerouted { name: String },
}

/// Cómo está la salida ahora.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputInfo {
    /// Dispositivo que suena (el de la lista) o «salida nula».
    pub name: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// Latencia de salida medida por el callback (frames), y el bloque
    /// declarado (si se sabe).
    pub latency_frames: u64,
    pub buffer: Option<u32>,
    /// Sin dispositivo: la salida nula.
    pub null: bool,
    /// A medio cambio.
    pub switching: bool,
    /// Tropiezos del stream (underruns...).
    pub glitches: u32,
}

impl OutputInfo {
    pub fn latency_ms(&self) -> f64 {
        self.latency_frames as f64 * 1000.0 / self.sample_rate.max(1) as f64
    }
}

/// La salida de audio con su motor: abre, cambia de dispositivo y se
/// recupera si el que suena desaparece. Vive en el hilo de control; hay que
/// llamar a [`AudioOutput::poll`] a menudo (en cada vuelta de la
/// interfaz).
pub struct AudioOutput {
    backend: Box<dyn AudioBackend>,
    /// Último recurso si no se puede abrir ningún dispositivo.
    null: Box<dyn AudioBackend>,
    slot: Arc<EngineSlot>,
    sample_rate: u32,
    current: Option<Active>,
    handoff: Option<Handoff>,
    /// El dispositivo que sonaba antes del actual (para volver a él si el
    /// actual se va).
    previous: Option<String>,
}

impl AudioOutput {
    /// Abre `device` (`None`: el predeterminado) con el motor. La primera
    /// vez el motor toma la frecuencia del dispositivo. Si `device` no se
    /// puede abrir, prueba el predeterminado y lo dice en el aviso; si
    /// tampoco, error y se devuelve el motor.
    pub fn open(
        backend: Box<dyn AudioBackend>,
        engine: Engine,
        device: Option<&str>,
    ) -> Result<(AudioOutput, Option<String>), OpenError> {
        let mut output = AudioOutput {
            backend,
            null: Box::new(NullBackend),
            sample_rate: engine.sample_rate(),
            slot: Arc::new(EngineSlot::new(engine)),
            current: None,
            handoff: None,
            previous: None,
        };
        let mut warning = None;
        let opened = match output.start(device, None, false) {
            Ok(active) => Ok(active),
            Err(e) if device.is_some() => {
                warning = Some(format!(
                    "audio: no se pudo abrir «{}» ({e}); suena por el predeterminado",
                    device.unwrap_or_default()
                ));
                output.start(None, None, false)
            }
            Err(e) => Err(e),
        };
        match opened {
            Ok(active) => {
                let rate = active.stream.sample_rate;
                output.slot.with(|engine| engine.set_sample_rate(rate));
                output.sample_rate = rate;
                active.shared.active.store(true, Ordering::Release);
                output.current = Some(active);
                Ok((output, warning))
            }
            Err(message) => Err(OpenError {
                message,
                engine: Box::new(output.take_engine()),
            }),
        }
    }

    /// Abre la salida nula (sin tarjeta) a `sample_rate`: el motor avanza
    /// a ritmo real y el audio se tira.
    pub fn open_null(mut engine: Engine, sample_rate: u32) -> AudioOutput {
        engine.set_sample_rate(sample_rate);
        let mut output = AudioOutput {
            backend: Box::new(NullBackend),
            null: Box::new(NullBackend),
            sample_rate,
            slot: Arc::new(EngineSlot::new(engine)),
            current: None,
            handoff: None,
            previous: None,
        };
        let active = output
            .start(None, Some(sample_rate), true)
            .expect("la salida nula siempre abre");
        active.shared.active.store(true, Ordering::Release);
        output.current = Some(active);
        output
    }

    /// Saca el motor (tras parar los streams).
    fn take_engine(mut self) -> Engine {
        self.current = None;
        self.handoff = None;
        let p = self.slot.engine.swap(ptr::null_mut(), Ordering::Acquire);
        assert!(!p.is_null(), "el motor está en la ranura sin streams");
        // SAFETY: sin streams nadie más lo tiene; es de `Box::into_raw`.
        *unsafe { Box::from_raw(p) }
    }

    /// Abre un stream (sin activarlo).
    fn start(
        &mut self,
        device: Option<&str>,
        rate: Option<u32>,
        null: bool,
    ) -> Result<Active, String> {
        let shared = Arc::new(StreamShared::default());
        let renderer = Renderer::new(self.slot.clone(), shared.clone());
        let backend = if null {
            &mut self.null
        } else {
            &mut self.backend
        };
        let stream = backend.open(device, rate, renderer)?;
        Ok(Active {
            stream,
            shared,
            null,
        })
    }

    /// Los dispositivos de salida que hay ahora (a petición; no se sondea).
    pub fn list(&mut self) -> Result<Vec<AudioDeviceInfo>, String> {
        self.backend.list()
    }

    /// Cómo se llama el backend.
    pub fn backend_label(&self) -> String {
        self.backend.label()
    }

    /// La frecuencia del motor (la de todos los dispositivos que se abran).
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Cambia a `device` (`None`: el predeterminado). Si no se puede abrir,
    /// error y sigue el que estaba; si sí, el cambio termina en
    /// [`AudioOutput::poll`] (tras el fundido del viejo).
    pub fn switch(&mut self, device: Option<&str>) -> Result<(), String> {
        self.switch_to(device, false)
    }

    fn switch_to(&mut self, device: Option<&str>, null: bool) -> Result<(), String> {
        if self.handoff.is_some() {
            return Err("ya está cambiando de dispositivo".into());
        }
        let fresh = self.start(device, Some(self.sample_rate), null)?;
        match self.current.replace(fresh) {
            Some(old) => {
                if !old.null {
                    self.previous = Some(old.stream.name.clone());
                }
                old.shared.stop.store(true, Ordering::Release);
                self.handoff = Some(Handoff {
                    old,
                    deadline: Instant::now() + HANDOFF_TIMEOUT,
                });
            }
            None => self.activate(),
        }
        Ok(())
    }

    fn activate(&mut self) {
        if let Some(current) = &self.current {
            current.shared.active.store(true, Ordering::Release);
        }
    }

    /// Atiende la salida: termina un cambio cuando el viejo ya soltó el
    /// motor (o se le acabó el tiempo) y, si el que suena se fue, cambia al
    /// anterior, al predeterminado o, si no, a la salida nula. Nunca
    /// bloquea.
    pub fn poll(&mut self) -> Vec<AudioEvent> {
        let mut events = Vec::new();
        if let Some(handoff) = &self.handoff {
            let shared = &handoff.old.shared;
            let released = shared.done.load(Ordering::Acquire)
                || shared.lost.load(Ordering::Acquire)
                || Instant::now() >= handoff.deadline;
            if !released {
                return events;
            }
            // Se suelta el viejo (su callback deja de llamarse) y entra el
            // nuevo.
            self.handoff = None;
            self.activate();
            if let Some(current) = &self.current {
                events.push(AudioEvent::Switched {
                    name: current.stream.name.clone(),
                });
            }
        }
        let Some(current) = &self.current else {
            return events;
        };
        if current.shared.rerouted.swap(false, Ordering::AcqRel) {
            events.push(AudioEvent::Rerouted {
                name: current.stream.name.clone(),
            });
        }
        if current.shared.lost.load(Ordering::Acquire) {
            let gone = current.stream.name.clone();
            let previous = self.previous.clone().filter(|p| *p != gone);
            let attempts = previous
                .iter()
                .map(|p| Some(p.as_str()))
                .chain([None])
                .collect::<Vec<_>>();
            let mut switched = false;
            for attempt in attempts {
                if self.switch_to(attempt, false).is_ok() {
                    switched = true;
                    break;
                }
            }
            if !switched {
                // (La salida nula siempre abre.)
                let _ = self.switch_to(None, true);
            }
            // El que se fue no suelta el motor con un fundido (ya no lo
            // llaman): se da por soltado.
            if let Some(handoff) = &mut self.handoff {
                handoff.deadline = Instant::now();
            }
            self.previous = None;
            let current = self.current.as_ref().expect("acaba de abrir");
            events.push(AudioEvent::Lost {
                name: gone,
                to: current.stream.name.clone(),
                null: current.null,
            });
            events.extend(self.poll());
        }
        events
    }

    /// Cómo está la salida.
    pub fn info(&self) -> Option<OutputInfo> {
        let current = self.current.as_ref()?;
        Some(OutputInfo {
            name: current.stream.name.clone(),
            sample_rate: current.stream.sample_rate,
            channels: current.stream.channels,
            latency_frames: current.shared.latency.load(Ordering::Relaxed),
            buffer: current.stream.buffer,
            null: current.null,
            switching: self.handoff.is_some(),
            glitches: current.shared.errors.load(Ordering::Relaxed),
        })
    }

    /// Bloques que ha servido el stream actual (para saber si el
    /// dispositivo está vivo).
    pub fn callbacks(&self) -> u64 {
        self.current
            .as_ref()
            .map_or(0, |c| c.shared.callbacks.load(Ordering::Relaxed))
    }
}

/// La salida nula: un hilo que llama al renderer a ritmo real y tira el
/// audio (la interfaz sin tarjeta y el último recurso si todo falla).
pub struct NullBackend;

/// Nombre de la salida nula en la lista y los mensajes.
pub const NULL_NAME: &str = "salida nula";

/// Frames por bloque de la salida nula.
const NULL_BLOCK: usize = 512;

struct NullStream {
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for NullStream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl AudioBackend for NullBackend {
    fn label(&self) -> String {
        NULL_NAME.into()
    }

    fn list(&mut self) -> Result<Vec<AudioDeviceInfo>, String> {
        Ok(vec![AudioDeviceInfo {
            name: NULL_NAME.into(),
            id: "null".into(),
            channels: 2,
            sample_rates: COMMON_RATES.to_vec(),
            default: true,
        }])
    }

    fn open(
        &mut self,
        _device: Option<&str>,
        sample_rate: Option<u32>,
        mut renderer: Renderer,
    ) -> Result<OpenStream, String> {
        let sample_rate = sample_rate.unwrap_or(48_000);
        renderer.set_rate(sample_rate);
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let thread = std::thread::spawn(move || {
            let mut block = vec![0.0f32; NULL_BLOCK * 2];
            let period = Duration::from_secs_f64(NULL_BLOCK as f64 / sample_rate as f64);
            let mut deadline = Instant::now();
            while !flag.load(Ordering::Relaxed) {
                renderer.render(&mut block, 2, 0);
                // Plazos absolutos: el reloj no deriva aunque un bloque tarde.
                deadline += period;
                if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
                    std::thread::sleep(wait);
                } else {
                    deadline = Instant::now();
                }
            }
        });
        Ok(OpenStream {
            name: NULL_NAME.into(),
            sample_rate,
            channels: 2,
            buffer: Some(NULL_BLOCK as u32),
            guard: Box::new(NullStream {
                stop,
                thread: Some(thread),
            }),
        })
    }
}

/// Dispositivos de verdad por cpal (en Linux, ALSA; con PipeWire, sus
/// dispositivos `pipewire` y `default` van por él).
pub struct CpalBackend {
    host: cpal::Host,
}

impl Default for CpalBackend {
    fn default() -> Self {
        CpalBackend {
            host: cpal::default_host(),
        }
    }
}

mod cpal_backend {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{BufferSize, FromSample, SampleFormat, SizedSample, StreamConfig};

    use super::*;

    fn device_name(device: &cpal::Device) -> String {
        device
            .description()
            .map(|d| d.name().to_string())
            .unwrap_or_else(|_| device.to_string())
    }

    fn device_id(device: &cpal::Device) -> String {
        device.id().map(|id| id.to_string()).unwrap_or_default()
    }

    impl CpalBackend {
        fn find(&self, wanted: Option<&str>) -> Result<cpal::Device, String> {
            let Some(wanted) = wanted else {
                return self
                    .host
                    .default_output_device()
                    .ok_or_else(|| "no hay dispositivo de salida predeterminado".to_string());
            };
            let devices = self.host.output_devices().map_err(|e| e.to_string())?;
            devices
                .into_iter()
                .find(|d| device_name(d) == wanted || device_id(d) == wanted)
                .ok_or_else(|| format!("no hay ningún dispositivo «{wanted}»"))
        }
    }

    impl AudioBackend for CpalBackend {
        fn label(&self) -> String {
            format!("{:?}", self.host.id())
        }

        fn list(&mut self) -> Result<Vec<AudioDeviceInfo>, String> {
            let default = self.host.default_output_device().map(|d| device_id(&d));
            let devices = self.host.output_devices().map_err(|e| e.to_string())?;
            Ok(devices
                .into_iter()
                .map(|device| {
                    let id = device_id(&device);
                    let ranges: Vec<_> = device
                        .supported_output_configs()
                        .map(|c| c.collect())
                        .unwrap_or_default();
                    let rates: Vec<(u32, u32)> = ranges
                        .iter()
                        .map(|r: &cpal::SupportedStreamConfigRange| {
                            (r.min_sample_rate(), r.max_sample_rate())
                        })
                        .collect();
                    AudioDeviceInfo {
                        name: device_name(&device),
                        default: default.as_deref() == Some(id.as_str()),
                        id,
                        channels: ranges.iter().map(|r| r.channels()).max().unwrap_or(0),
                        sample_rates: listed_rates(&rates),
                    }
                })
                .collect())
        }

        fn open(
            &mut self,
            wanted: Option<&str>,
            sample_rate: Option<u32>,
            mut renderer: Renderer,
        ) -> Result<OpenStream, String> {
            let device = self.find(wanted)?;
            let default = device.default_output_config().map_err(|e| e.to_string())?;
            let supported = match sample_rate {
                None => default,
                Some(rate) if default.sample_rate() == rate => default,
                Some(rate) => {
                    let ranges = device
                        .supported_output_configs()
                        .map_err(|e| e.to_string())?;
                    // Mejor con los canales y el formato de siempre.
                    let mut fits: Vec<_> = ranges
                        .filter_map(|r| r.try_with_sample_rate(rate))
                        .collect();
                    fits.sort_by_key(|c| {
                        (
                            c.channels() != default.channels(),
                            c.sample_format() != SampleFormat::F32,
                        )
                    });
                    fits.into_iter()
                        .next()
                        .ok_or_else(|| format!("no admite {rate} Hz (la frecuencia del motor)"))?
                }
            };
            let format = supported.sample_format();
            let config: StreamConfig = supported.into();
            renderer.set_rate(config.sample_rate);
            let stream = match format {
                SampleFormat::F32 => build::<f32>(&device, &config, renderer),
                SampleFormat::I16 => build::<i16>(&device, &config, renderer),
                SampleFormat::I32 => build::<i32>(&device, &config, renderer),
                SampleFormat::U16 => build::<u16>(&device, &config, renderer),
                other => Err(format!("formato de salida no soportado: {other}")),
            }?;
            stream.play().map_err(|e| e.to_string())?;
            Ok(OpenStream {
                name: device_name(&device),
                sample_rate: config.sample_rate,
                channels: config.channels,
                buffer: match config.buffer_size {
                    BufferSize::Fixed(n) => Some(n),
                    BufferSize::Default => None,
                },
                guard: Box::new(stream),
            })
        }
    }

    fn build<T>(
        device: &cpal::Device,
        config: &StreamConfig,
        mut renderer: Renderer,
    ) -> Result<cpal::Stream, String>
    where
        T: SizedSample + FromSample<f32>,
    {
        let channels = config.channels as usize;
        let sample_rate = config.sample_rate;
        let faults = renderer.fault_handle();
        // Bloque intermedio en la pila: sin reservas de memoria en el callback.
        let mut scratch = [0.0f32; 2048];
        let block = (scratch.len() / channels.max(1)) * channels.max(1);
        device
            .build_output_stream(
                *config,
                move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
                    // Lo que tarda en oírse el principio de este bloque, si
                    // la tarjeta lo dice.
                    let stamp = info.timestamp();
                    let latency = stamp.playback.duration_since(stamp.callback);
                    let mut latency = (latency.as_secs_f64() * sample_rate as f64) as u64;
                    for chunk in data.chunks_mut(block) {
                        let tmp = &mut scratch[..chunk.len()];
                        renderer.render(tmp, channels, latency);
                        for (dst, &src) in chunk.iter_mut().zip(tmp.iter()) {
                            *dst = T::from_sample(src);
                        }
                        latency += (chunk.len() / channels) as u64;
                    }
                },
                move |err| {
                    use cpal::ErrorKind as K;
                    faults.fault(match err.kind() {
                        K::DeviceNotAvailable | K::StreamInvalidated => StreamFault::Lost,
                        K::DeviceChanged => StreamFault::Rerouted,
                        _ => StreamFault::Glitch,
                    });
                },
                None,
            )
            .map_err(|e| e.to_string())
    }
}

/// Un backend falso para los tests (sin tarjeta): dispositivos que se
/// enchufan y desenchufan, y streams que solo suenan cuando el test los
/// mueve con [`FakeAudio::pump`]. El mutex es solo de los tests: el hilo de
/// audio de verdad nunca pasa por aquí.
pub mod fake {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct State {
        devices: Vec<(AudioDeviceInfo, bool)>,
        streams: Vec<(String, usize, Renderer)>,
        opens: usize,
        /// Latencia que dice cada dispositivo (frames; 128 si no).
        latency: Vec<(String, u64)>,
    }

    /// El backend y su mando (se clonan y comparten el estado).
    #[derive(Clone, Default)]
    pub struct FakeAudio(Arc<Mutex<State>>);

    struct Guard(Arc<Mutex<State>>, usize);

    impl Drop for Guard {
        fn drop(&mut self) {
            if let Ok(mut state) = self.0.lock() {
                state.streams.retain(|(_, id, _)| *id != self.1);
            }
        }
    }

    impl FakeAudio {
        /// Añade un dispositivo (enchufado) con sus canales y frecuencias.
        pub fn add(&self, name: &str, channels: u16, rates: &[u32], default: bool) {
            let info = AudioDeviceInfo {
                name: name.into(),
                id: format!("fake:{name}"),
                channels,
                sample_rates: rates.to_vec(),
                default,
            };
            self.0.lock().unwrap().devices.push((info, true));
        }

        /// Desenchufa un dispositivo: deja de listarse y su stream recibe
        /// el error de dispositivo perdido.
        pub fn unplug(&self, name: &str) {
            let mut state = self.0.lock().unwrap();
            for (info, present) in &mut state.devices {
                if info.name == name {
                    *present = false;
                }
            }
            for (device, _, renderer) in &state.streams {
                if device == name {
                    renderer.fault(StreamFault::Lost);
                }
            }
        }

        /// La latencia que dice `name` (frames hasta que se oye el principio
        /// de cada bloque).
        pub fn set_latency(&self, name: &str, frames: u64) {
            self.0.lock().unwrap().latency.push((name.into(), frames));
        }

        pub fn plug(&self, name: &str) {
            for (info, present) in &mut self.0.lock().unwrap().devices {
                if info.name == name {
                    *present = true;
                }
            }
        }

        /// Mueve `frames` frames el stream abierto en `device` (si lo hay) y
        /// devuelve lo que sonó (entrelazado, en estéreo).
        pub fn pump(&self, device: &str, frames: usize) -> Option<Vec<f32>> {
            let mut out = vec![0.0; frames * 2];
            self.pump_into(device, &mut out).then_some(out)
        }

        /// Como [`FakeAudio::pump`], en `out` (estéreo) y sin reservar
        /// memoria: en bloques de 256 frames, como un callback. `false` si
        /// no hay stream en `device`.
        pub fn pump_into(&self, device: &str, out: &mut [f32]) -> bool {
            let mut state = self.0.lock().unwrap();
            let state = &mut *state;
            let latency = state
                .latency
                .iter()
                .rev()
                .find(|(d, _)| d == device)
                .map_or(128, |(_, l)| *l);
            let Some((_, _, renderer)) = state.streams.iter_mut().find(|(d, ..)| d == device)
            else {
                return false;
            };
            for chunk in out.chunks_mut(256 * 2) {
                renderer.render(chunk, 2, latency);
            }
            true
        }

        /// Dispositivos con un stream abierto ahora.
        pub fn open_streams(&self) -> Vec<String> {
            let state = self.0.lock().unwrap();
            state.streams.iter().map(|(d, ..)| d.clone()).collect()
        }

        pub fn backend(&self) -> Box<dyn AudioBackend> {
            Box::new(self.clone())
        }
    }

    impl AudioBackend for FakeAudio {
        fn label(&self) -> String {
            "falso".into()
        }

        fn list(&mut self) -> Result<Vec<AudioDeviceInfo>, String> {
            let state = self.0.lock().unwrap();
            Ok(state
                .devices
                .iter()
                .filter(|(_, present)| *present)
                .map(|(info, _)| info.clone())
                .collect())
        }

        fn open(
            &mut self,
            device: Option<&str>,
            sample_rate: Option<u32>,
            mut renderer: Renderer,
        ) -> Result<OpenStream, String> {
            let mut state = self.0.lock().unwrap();
            let info = state
                .devices
                .iter()
                .filter(|(_, present)| *present)
                .map(|(info, _)| info)
                .find(|info| match device {
                    Some(name) => info.matches(name),
                    None => info.default,
                })
                .cloned()
                .ok_or_else(|| match device {
                    Some(name) => format!("no hay ningún dispositivo «{name}»"),
                    None => "no hay dispositivo de salida predeterminado".to_string(),
                })?;
            let rate = sample_rate.unwrap_or(info.sample_rates[0]);
            if !info.sample_rates.contains(&rate) {
                return Err(format!("no admite {rate} Hz (la frecuencia del motor)"));
            }
            renderer.set_rate(rate);
            state.opens += 1;
            let id = state.opens;
            state.streams.push((info.name.clone(), id, renderer));
            Ok(OpenStream {
                name: info.name,
                sample_rate: rate,
                channels: info.channels,
                buffer: Some(256),
                guard: Box::new(Guard(self.0.clone(), id)),
            })
        }
    }
}
