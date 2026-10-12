//! Motor de audio del sampler. Independiente de la interfaz.

mod engine;
mod fx;
mod limiter;
pub mod notation;
pub mod offline;
mod output;
mod sample;
mod sequencer;
mod slice;

pub use engine::{Batch, Controller, Engine, MAX_TRACKS, SendError, new_engine};
pub use fx::{DEFAULT_Q, FilterKind, Fx, MAX_FX};
pub use limiter::{LIMIT_THRESHOLD, soft_limit};
pub use output::{Output, open_output};
pub use sample::{LoadError, Sample};
pub use sequencer::{
    DEFAULT_BPM, MAX_BPM, MAX_CYCLE_BARS, MAX_GAIN, MAX_RETRIG, MAX_SEMITONES, MIN_BPM, Pattern,
    STEPS, Step, bar_frames, valid_bpm,
};
pub use slice::{DEFAULT_MIN_GAP_SECONDS, DetectOptions, Detector, Slices};
