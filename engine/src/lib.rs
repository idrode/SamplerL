//! Motor de audio del sampler. Independiente de la interfaz.

mod engine;
pub mod offline;
mod output;
mod sample;
mod sequencer;
mod slice;

pub use engine::{Controller, Engine, SendError, new_engine};
pub use output::{Output, open_output};
pub use sample::{LoadError, Sample};
pub use sequencer::{DEFAULT_BPM, MAX_BPM, MIN_BPM, Pattern, STEPS, bar_frames};
pub use slice::{DEFAULT_MIN_GAP_SECONDS, DetectOptions, Detector, Slices};
