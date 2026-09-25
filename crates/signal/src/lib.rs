//! Metering, waveform summaries and track-boundary detection.
//!
//! Pure analysis over PCM: no device access, no database, no network. That isolation
//! is what makes the detector port testable against the labelled corpus without an
//! audio stack present, which is the whole basis of the WP-11 A/B harness.
//!
//! Requirements: §17-§19 (metering and waveform), §22-§24 (detection).

pub mod hmm;
pub mod meter;
pub mod silence;
pub mod spectral;
pub mod waveform;
