use std::sync::Arc;
use std::time::Duration;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args().nth(1).ok_or("uso: tui <archivo.wav>")?;

    let (mut controller, engine) = engine::new_engine();
    let output = engine::open_output(engine)?;
    let sample = Arc::new(engine::Sample::load_wav(&path, output.sample_rate)?);
    println!(
        "{path}: {} frames a {} Hz ({:.2} s)",
        sample.frames(),
        sample.sample_rate(),
        sample.frames() as f64 / sample.sample_rate() as f64
    );

    controller.play(sample);
    while controller.is_playing() {
        std::thread::sleep(Duration::from_millis(50));
        controller.collect_garbage();
    }
    Ok(())
}
