use std::sync::Arc;
use std::time::Duration;

const USAGE: &str = "uso: tui <archivo.wav> [--render <salida.wav>]";
/// Frecuencia del motor cuando se renderiza sin dispositivo.
const RENDER_RATE: u32 = 48_000;
/// Límite del render offline.
const MAX_RENDER_SECONDS: usize = 600;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (path, render_to) = match args.as_slice() {
        [path] => (path, None),
        [path, flag, out] if flag == "--render" => (path, Some(out)),
        [flag, out, path] if flag == "--render" => (path, Some(out)),
        _ => return Err(USAGE.into()),
    };

    match render_to {
        Some(out) => render(path, out),
        None => play(path),
    }
}

fn load(path: &str, rate: u32) -> Result<Arc<engine::Sample>, engine::LoadError> {
    let sample = engine::Sample::load_wav(path, rate)?;
    println!(
        "{path}: {} frames a {} Hz ({:.2} s)",
        sample.frames(),
        sample.sample_rate(),
        sample.frames() as f64 / sample.sample_rate() as f64
    );
    Ok(Arc::new(sample))
}

fn play(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    // `open_output` ajusta la frecuencia del motor a la del dispositivo.
    let (mut controller, engine) = engine::new_engine(RENDER_RATE);
    let output = engine::open_output(engine)?;
    controller.play(load(path, output.sample_rate)?)?;
    while controller.is_playing() {
        std::thread::sleep(Duration::from_millis(50));
        controller.collect_garbage();
    }
    Ok(())
}

fn render(path: &str, out: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (mut controller, mut engine) = engine::new_engine(RENDER_RATE);
    controller.play(load(path, RENDER_RATE)?)?;
    let max_frames = MAX_RENDER_SECONDS * RENDER_RATE as usize;
    let audio = engine::offline::render_until_stopped(&controller, &mut engine, 2, max_frames);
    engine::offline::write_wav(out, &audio, 2, RENDER_RATE)?;
    println!("{out}: {} frames renderizados", audio.len() / 2);
    Ok(())
}
