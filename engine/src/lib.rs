//! Motor de audio del sampler. Independiente de la interfaz.

pub mod arrangement;
pub mod automation;
pub mod bpm;
pub mod bus;
mod engine;
mod fx;
pub mod grain;
pub mod instrument;
mod interp;
mod limiter;
pub mod masking;
pub mod metronome;
pub mod notation;
pub mod offline;
mod output;
mod sample;
mod sequencer;
mod shift;
mod slice;
mod status;
pub mod stretch;

pub use arrangement::{Arrangement, MAX_ENTRIES, MAX_SECTIONS};
pub use automation::{Automation, Curve, LfoRate, MAX_AUTOMATIONS};
pub use bpm::{BpmGuess, detect_bpm};
pub use bus::{Buses, DelayParams, Division, ReverbParams};
pub use engine::{
    Batch, Controller, Engine, MAX_ATTACK_MS, MAX_CHOKE, MAX_RELEASE_MS, MAX_TRACKS, MidiNote,
    PlayMode, SendError, SliceShape, new_engine,
};
pub use fx::{
    DEFAULT_Q, DynEqParams, EQ_BANDS, EQ_DEFAULT_FREQS, EQ_DEFAULT_Q, EQ_FREQ_RANGE, EQ_GAIN_RANGE,
    EQ_Q_RANGE, EqBand, EqKind, FilterKind, Fx, MAX_FX, MAX_PARAMS,
};
pub use grain::{GRAIN_PARAMS, Granular, Knob, MAX_GRAINS};
pub use instrument::{Adsr, Instrument, MAX_VOICES};
pub use limiter::{LIMIT_THRESHOLD, soft_limit};
pub use output::{NullOutput, Output, open_null_output, open_output};
pub use sample::{ImportOptions, LoadError, Sample};
pub use sequencer::{
    DEFAULT_BPM, MAX_BPM, MAX_CHORD, MAX_CYCLE_BARS, MAX_DELAY, MAX_GAIN, MAX_RETRIG,
    MAX_SEMITONES, MAX_STEPS, MIN_BPM, Meter, Pattern, STEPS, Step, TempoRamp, bar_frames,
    bar_frames_steps, valid_bpm,
};
pub use shift::MAX_FORMANT;
pub use slice::{DEFAULT_MIN_GAP_SECONDS, DetectOptions, Detector, Slices};
pub use status::{MasterStatus, SectionStatus, Status, TrackStatus};
pub use stretch::{MAX_STRETCH, MIN_STRETCH};
