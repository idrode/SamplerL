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
pub mod pitch;
mod sample;
mod sequencer;
mod shift;
mod slice;
mod slice_fx;
mod status;
pub mod stretch;
pub mod synth;

pub use arrangement::{Arrangement, MAX_ENTRIES, MAX_SECTIONS};
pub use automation::{
    Automation, Curve, LfoRate, MAX_AUTOMATIONS, MIX_GAIN, MIX_PAN, MIX_PARAMS, MIX_SEND, MixCurves,
};
pub use bpm::{BpmGuess, detect_bpm};
pub use bus::{Buses, DelayParams, Division, ReverbParams};
pub use engine::{
    Batch, Controller, Engine, MAX_ATTACK_MS, MAX_BEND, MAX_CHOKE, MAX_CROSSFADE_MS,
    MAX_RELEASE_MS, MAX_START_BAR, MAX_TRACKS, MidiNote, PlayMode, SHAPE_ATTACK, SHAPE_CROSSFADE,
    SHAPE_LENGTH, SHAPE_PAN, SHAPE_PARAMS, SHAPE_RELEASE, SendError, ShapeCurves, SliceShape,
    loop_seam_room, new_engine,
};
pub use fx::{
    DEFAULT_Q, DynEqParams, EQ_BANDS, EQ_DEFAULT_FREQS, EQ_DEFAULT_Q, EQ_FREQ_RANGE, EQ_GAIN_RANGE,
    EQ_Q_RANGE, EqBand, EqKind, FilterKind, Fx, MAX_FX, MAX_LEVEL, MAX_PARAMS,
};
pub use grain::{GRAIN_PARAMS, Granular, Knob, MAX_GRAINS};
pub use instrument::{Adsr, Instrument, MAX_VOICES};
pub use limiter::{LIMIT_THRESHOLD, soft_limit};
pub use output::{NullOutput, Output, open_null_output, open_output};
pub use pitch::{NoteGuess, detect_note, detect_notes, note_name};
pub use sample::{ImportOptions, LoadError, Sample};
pub use sequencer::{
    DEFAULT_BPM, MAX_BPM, MAX_CHORD, MAX_CYCLE_BARS, MAX_DELAY, MAX_GAIN, MAX_RETRIG,
    MAX_SEMITONES, MAX_STEPS, MIN_BPM, Meter, Pattern, STEPS, Step, TempoRamp, bar_frames,
    bar_frames_steps, valid_bpm,
};
pub use shift::MAX_FORMANT;
pub use slice::{DEFAULT_MIN_GAP_SECONDS, DetectOptions, Detector, Slices};
pub use slice_fx::SliceFx;
pub use status::{Cursor, MAX_TAG, MasterStatus, SectionStatus, Status, TrackStatus};
pub use stretch::{MAX_STRETCH, MIN_STRETCH};
