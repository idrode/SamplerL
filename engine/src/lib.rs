//! Motor de audio del sampler. Independiente de la interfaz.

mod engine;
mod output;
mod sample;
mod slice;

pub use engine::{Controller, Engine, SendError, new_engine};
pub use output::{Output, open_output};
pub use sample::{LoadError, Sample};
pub use slice::Slices;
