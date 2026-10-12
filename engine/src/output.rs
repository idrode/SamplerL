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
    // Bloque intermedio en la pila: sin reservas de memoria en el callback.
    let mut scratch = [0.0f32; 2048];
    let block = (scratch.len() / channels) * channels;

    let stream = device.build_output_stream(
        *config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
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
