//! Criterio de éxito del detector por flujo espectral: en el break sintético
//! (el mismo que genera `scripts/render-demo.sh`), cargado a 48 kHz como en
//! el render, los 8 cortes caen a menos de 2 ms de su golpe.

mod support;

use engine::{DetectOptions, Detector, Sample, Slices};
use support::synth_break as synth;

const ENGINE_RATE: u32 = 48_000;
const TOLERANCE: usize = (ENGINE_RATE / 500) as usize;

#[test]
fn flux_finds_all_eight_hits_of_the_synthetic_break() {
    let (audio, hits) = synth::synth_break();
    let sample = Sample::from_interleaved(&audio, 1, synth::RATE, ENGINE_RATE).unwrap();
    let options = DetectOptions {
        detector: Detector::Flux,
        ..Default::default()
    };
    let slices = Slices::detect_with(&sample, 8, &options);
    assert_eq!(slices.len(), 8);

    for (i, (range, &hit)) in slices.iter().zip(&hits).enumerate() {
        // Posición del golpe a la frecuencia del motor.
        let expected = (hit as u64 * ENGINE_RATE as u64 / synth::RATE as u64) as usize;
        assert!(
            range.start.abs_diff(expected) <= TOLERANCE,
            "corte {} ({:?}): en {}, esperado {expected} (±{TOLERANCE})",
            i + 1,
            synth::BAR[i],
            range.start
        );
    }
}
