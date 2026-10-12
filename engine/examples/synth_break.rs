//! Genera un break sintético de un compás (8 golpes en corcheas a 140 BPM:
//! bombo, charles, caja, charles, bombo, bombo, caja, charles) a 44,1 kHz,
//! mono. Determinista: siempre produce el mismo archivo. Es el mismo audio
//! que usa el test `engine/tests/synth_break.rs`.
//!
//! Uso: cargo run -p engine --example synth_break -- <salida.wav>

#[path = "../tests/support/synth_break.rs"]
mod synth;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let out = std::env::args()
        .nth(1)
        .ok_or("uso: synth_break <salida.wav>")?;
    let (audio, hits) = synth::synth_break();
    engine::offline::write_wav(&out, &audio, 1, synth::RATE)?;
    println!(
        "{out}: {} golpes, {} frames a {} Hz ({:.2} s)",
        hits.len(),
        audio.len(),
        synth::RATE,
        audio.len() as f32 / synth::RATE as f32
    );
    Ok(())
}
