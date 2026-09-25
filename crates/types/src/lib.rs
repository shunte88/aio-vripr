//! Shared vocabulary for VCW.
//!
//! Every other crate in the workspace speaks these types, and this crate depends on
//! nothing but `serde` and `thiserror`. That is deliberate: it keeps CPAL, SQLite and
//! the network stack out of the dependency closure of crates that have no business
//! linking them. `vcw-signal` analysing samples should not pull in ALSA.
//!
//! The types here are the ones the specification already fixes - sample formats (§8),
//! capture modes (§9) and the hardware sample rates (§8). Nothing speculative lives
//! here; a type earns its place once a requirement or a decision pins it down.

pub mod format;
pub mod rate;

pub use format::{CaptureMode, SampleFormat, StorageFormat};
pub use rate::{STANDARD_RATES, SampleRate};
