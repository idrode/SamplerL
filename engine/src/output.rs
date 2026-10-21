use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

use crate::engine::Engine;

type BoxError = Box<dyn std::error::Error>;

/// Stream de salida activo. El audio suena mientras exista.
pub struct Output {
    _stream: cpal::Stream,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Abre el dispositivo de salida por defecto y mueve el motor a su callback.
pub fn open_output(engine: Engine) -> Result<Output, BoxError> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no hay dispositivo de salida de audio")?;
    let supported = device.default_output_config()?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();
    let mut engine = engine;
    engine.set_sample_rate(config.sample_rate);

    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, &config, engine)?,
        SampleFormat::I16 => build::<i16>(&device, &config, engine)?,
        SampleFormat::I32 => build::<i32>(&device, &config, engine)?,
        SampleFormat::U16 => build::<u16>(&device, &config, engine)?,
        other => return Err(format!("formato de salida no soportado: {other}").into()),
    };
    stream.play()?;
    Ok(Output {
        _stream: stream,
        sample_rate: config.sample_rate,
        channels: config.channels,
    })
}

fn build<T>(
    device: &cpal::Device,
    config: &StreamConfig,
    mut engine: Engine,
) -> Result<cpal::Stream, BoxError>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    let sample_rate = config.sample_rate;
    // Bloque intermedio en la pila: sin reservas de memoria en el callback.
    let mut scratch = [0.0f32; 2048];
    let block = (scratch.len() / channels) * channels;

    let stream = device.build_output_stream(
        *config,
        move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
            // Lo que tarda en oírse el final de este bloque: hasta que suena
            // su principio (si la tarjeta lo dice) más el propio bloque.
            let stamp = info.timestamp();
            let latency = stamp.playback.duration_since(stamp.callback);
            let latency = (latency.as_secs_f64() * sample_rate as f64) as u64;
            engine.set_output_delay(latency + (data.len() / channels) as u64);
            for chunk in data.chunks_mut(block) {
                let tmp = &mut scratch[..chunk.len()];
                engine.process(tmp, channels);
                for (dst, &src) in chunk.iter_mut().zip(tmp.iter()) {
                    *dst = T::from_sample(src);
                }
            }
        },
        |err| eprintln!("error en el stream de audio: {err}"),
        None,
    )?;
    Ok(stream)
}

/// Salida nula: mueve el motor a ritmo de tiempo real en un hilo propio y
/// tira el audio. Sirve para usar la interfaz sin abrir la tarjeta de
/// sonido (el secuenciador avanza igual). Se para al soltarla.
pub struct NullOutput {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
    pub sample_rate: u32,
}

/// Frames por bloque de la salida nula.
const NULL_BLOCK: usize = 512;

pub fn open_null_output(mut engine: Engine, sample_rate: u32) -> NullOutput {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::time::{Duration, Instant};

    engine.set_sample_rate(sample_rate);
    let stop = std::sync::Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    let thread = std::thread::spawn(move || {
        let mut block = vec![0.0f32; NULL_BLOCK * 2];
        let period = Duration::from_secs_f64(NULL_BLOCK as f64 / sample_rate as f64);
        let mut deadline = Instant::now();
        while !flag.load(Ordering::Relaxed) {
            engine.process(&mut block, 2);
            // Plazos absolutos: el reloj no deriva aunque un bloque tarde.
            deadline += period;
            if let Some(wait) = deadline.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            } else {
                deadline = Instant::now();
            }
        }
    });
    NullOutput {
        stop,
        thread: Some(thread),
        sample_rate,
    }
}

impl Drop for NullOutput {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
