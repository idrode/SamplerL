//! Motor de audio del sampler. Independiente de la interfaz.

pub mod automation;
pub mod bpm;
pub mod bus;
mod engine;
mod fx;
mod limiter;
pub mod metronome;
pub mod notation;
pub mod offline;
mod output;
mod sample;
mod sequencer;
mod slice;
mod status;
pub mod stretch;

pub use automation::{Automation, Curve, LfoRate, MAX_AUTOMATIONS};
pub use bpm::{BpmGuess, detect_bpm};
pub use bus::{Buses, DelayParams, Division, ReverbParams};
pub use engine::{
    Batch, Controller, Engine, MAX_ATTACK_MS, MAX_CHOKE, MAX_RELEASE_MS, MAX_TRACKS, PlayMode,
    SendError, SliceShape, new_engine,
};
pub use fx::{DEFAULT_Q, FilterKind, Fx, MAX_FX};
pub use limiter::{LIMIT_THRESHOLD, soft_limit};
pub use output::{NullOutput, Output, open_null_output, open_output};
pub use sample::{ImportOptions, LoadError, Sample};
pub use sequencer::{
    DEFAULT_BPM, MAX_BPM, MAX_CYCLE_BARS, MAX_DELAY, MAX_GAIN, MAX_RETRIG, MAX_SEMITONES,
    MAX_STEPS, MIN_BPM, Pattern, STEPS, Step, bar_frames, valid_bpm,
};
pub use slice::{DEFAULT_MIN_GAP_SECONDS, DetectOptions, Detector, Slices};
pub use status::{MasterStatus, Status, TrackStatus};
pub use stretch::{MAX_STRETCH, MIN_STRETCH};
