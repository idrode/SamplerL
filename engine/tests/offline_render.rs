//! Renderizado offline: carga un WAV sintético, lo reproduce con el motor
//! en un buffer y comprueba longitud y ausencia de silencio.

use std::sync::Arc;

use engine::{Sample, new_engine, offline};

const SOURCE_RATE: u32 = 44_100;
const ENGINE_RATE: u32 = 48_000;
const CHANNELS: usize = 2;
const BLOCK_FRAMES: usize = 512;

fn write_sine_wav(path: &std::path::Path, seconds: f32) {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: SOURCE_RATE,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec).unwrap();
    let frames = (seconds * SOURCE_RATE as f32) as usize;
    for i in 0..frames {
        let t = i as f32 / SOURCE_RATE as f32;
        let v = 0.5 * (2.0 * std::f32::consts::PI * 440.0 * t).sin();
        writer.write_sample((v * i16::MAX as f32) as i16).unwrap();
    }
    writer.finalize().unwrap();
}

fn rms(samples: &[f32]) -> f32 {
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

#[test]
fn renders_loaded_wav_with_correct_length_and_no_silence() {
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("sine_440.wav");
    write_sine_wav(&path, 0.5);

    // 0.5 s a 44.1 kHz -> 0.5 s a 48 kHz.
    let sample = Arc::new(Sample::load_wav(&path, ENGINE_RATE).unwrap());
    let expected_frames = ENGINE_RATE as usize / 2;
    assert_eq!(sample.frames(), expected_frames);

    let (mut controller, mut engine) = new_engine(ENGINE_RATE);
    controller.play(sample).unwrap();

    // Renderizar en bloques, como haría el callback, más un margen tras el final.
    let total_frames = expected_frames + 4 * BLOCK_FRAMES;
    let mut rendered = vec![0.0f32; total_frames * CHANNELS];
    for block in rendered.chunks_mut(BLOCK_FRAMES * CHANNELS) {
        engine.process(block, CHANNELS);
    }
    assert!(!controller.is_playing());

    // Sin silencio dentro del sample: cada ventana de 10 ms tiene energía
    // (seno de amplitud 0.5 -> RMS ~0.35).
    let window = (ENGINE_RATE as usize / 100) * CHANNELS;
    let (body, tail) = rendered.split_at(expected_frames * CHANNELS);
    for (i, w) in body.chunks(window).enumerate() {
        let level = rms(w);
        assert!(level > 0.2, "ventana {i} demasiado baja: rms = {level}");
    }

    // Después del final, silencio exacto: la reproducción dura lo que el sample.
    assert!(tail.iter().all(|&s| s == 0.0));

    // Ambos canales iguales (fuente mono duplicada).
    assert!(body.as_chunks::<2>().0.iter().all(|[l, r]| l == r));
}

#[test]
fn render_until_stopped_writes_a_readable_wav() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"));
    let input = dir.join("sine_440_render_in.wav");
    let output = dir.join("sine_440_render_out.wav");
    write_sine_wav(&input, 0.5);

    let sample = Arc::new(Sample::load_wav(&input, ENGINE_RATE).unwrap());
    let frames = sample.frames();
    let (mut controller, mut engine) = new_engine(ENGINE_RATE);
    controller.play(sample).unwrap();
    let audio = offline::render_until_stopped(&controller, &mut engine, CHANNELS, 10 * frames);

    // Se detiene en el primer bloque tras el final.
    assert!(audio.len() / CHANNELS >= frames);
    assert!(audio.len() / CHANNELS < frames + offline::BLOCK_FRAMES);

    offline::write_wav(&output, &audio, CHANNELS as u16, ENGINE_RATE).unwrap();
    let mut reader = hound::WavReader::open(&output).unwrap();
    assert_eq!(reader.spec().sample_rate, ENGINE_RATE);
    assert_eq!(reader.spec().channels as usize, CHANNELS);
    let read: Vec<i16> = reader.samples::<i16>().map(Result::unwrap).collect();
    assert_eq!(read.len(), audio.len());
    for (&r, &a) in read.iter().zip(&audio) {
        assert!((r as f32 / i16::MAX as f32 - a).abs() <= 1.0 / i16::MAX as f32);
    }
}

/// El render a disco por bloques (`render_into`) da el mismo audio que
/// `render` en memoria, y en 16 bits sin dither el mismo archivo que
/// `write_wav` (N13 de `docs/AUDITORIA-2.md`: el render ya no tiene que
/// guardar la mezcla entera).
#[test]
fn rendering_into_a_file_is_the_same_as_in_memory() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("render_into");
    std::fs::create_dir_all(&dir).unwrap();
    let source = dir.join("sine.wav");
    write_sine_wav(&source, 0.5);
    let frames = 3 * BLOCK_FRAMES + 77;
    let run = || {
        let (mut controller, engine) = new_engine(ENGINE_RATE);
        controller
            .play(Arc::new(Sample::load_wav(&source, ENGINE_RATE).unwrap()))
            .unwrap();
        engine
    };
    let audio = offline::render(&mut run(), frames, CHANNELS);
    offline::write_wav(dir.join("memoria.wav"), &audio, 2, ENGINE_RATE).unwrap();
    let mut out = offline::WavOut::create(
        dir.join("disco.wav"),
        2,
        ENGINE_RATE,
        offline::WavFormat::Int16,
        false,
    )
    .unwrap();
    offline::render_into(&mut run(), frames, &mut out).unwrap();
    out.finalize().unwrap();
    assert_eq!(
        std::fs::read(dir.join("memoria.wav")).unwrap(),
        std::fs::read(dir.join("disco.wav")).unwrap()
    );
    // En 32 bits en coma flotante, las muestras tal cual.
    let mut out = offline::WavOut::create(
        dir.join("float.wav"),
        2,
        ENGINE_RATE,
        offline::WavFormat::Float32,
        false,
    )
    .unwrap();
    offline::render_into(&mut run(), frames, &mut out).unwrap();
    out.finalize().unwrap();
    let read: Vec<f32> = hound::WavReader::open(dir.join("float.wav"))
        .unwrap()
        .samples::<f32>()
        .map(Result::unwrap)
        .collect();
    assert_eq!(read, audio);
}

/// 24 bits (M6 de la primera auditoría): cada muestra a su escalón más
/// cercano; con dither, a uno o dos escalones como mucho y distinto de sin
/// él, pero el mismo en dos renders (determinista).
#[test]
fn twenty_four_bits_and_dither() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("wav_formats");
    std::fs::create_dir_all(&dir).unwrap();
    let audio: Vec<f32> = (0..4000).map(|i| 0.3 * (i as f32 * 0.01).sin()).collect();
    let write = |name: &str, format, dither| {
        let path = dir.join(name);
        let mut out = offline::WavOut::create(&path, 2, ENGINE_RATE, format, dither).unwrap();
        out.write(&audio).unwrap();
        out.finalize().unwrap();
        let reader = hound::WavReader::open(&path).unwrap();
        let bits = reader.spec().bits_per_sample;
        let samples: Vec<i32> = reader.into_samples::<i32>().map(Result::unwrap).collect();
        (bits, samples)
    };
    let full = 8_388_607.0f32;
    let (bits, plain) = write("24.wav", offline::WavFormat::Int24, false);
    assert_eq!(bits, 24);
    for (&s, &x) in plain.iter().zip(&audio) {
        assert_eq!(s, (x * full).round() as i32);
    }
    let (_, dithered) = write("24d.wav", offline::WavFormat::Int24, true);
    assert_ne!(dithered, plain);
    assert!(
        dithered
            .iter()
            .zip(&audio)
            .all(|(&s, &x)| (s as f32 - x * full).abs() <= 2.0)
    );
    let (_, again) = write("24d2.wav", offline::WavFormat::Int24, true);
    assert_eq!(dithered, again);
    assert_eq!(
        offline::WavFormat::parse("32f"),
        Some(offline::WavFormat::Float32)
    );
    assert_eq!(offline::WavFormat::parse("8"), None);
}
