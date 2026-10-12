//! Motor de audio del sampler. Independiente de la interfaz.

mod engine;
pub mod notation;
pub mod offline;
mod output;
mod sample;
mod sequencer;
mod slice;

pub use engine::{Batch, Controller, Engine, MAX_TRACKS, SendError, new_engine};
pub use output::{Output, open_output};
pub use sample::{LoadError, Sample};
pub use sequencer::{
    DEFAULT_BPM, MAX_BPM, MAX_GAIN, MAX_SEMITONES, MIN_BPM, Pattern, STEPS, Step, bar_frames,
};
pub use slice::{DEFAULT_MIN_GAP_SECONDS, DetectOptions, Detector, Slices};
