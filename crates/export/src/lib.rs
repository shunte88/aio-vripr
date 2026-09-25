//! Producing deliverables from a finished capture.
//!
//! Requirements: §33 (export), and the naming templates ported from VRipr.
//!
//! Exports are derived, never authoritative: the blocks stay untouched and the
//! output is reproducible from blocks plus edit instructions. WP-14 verifies the WAV
//! path is bit-exact against the source blocks, which is the end-to-end proof that
//! the bit-perfect claim survived the whole pipeline and not just the callback.

pub mod encoder;
pub mod splitter;
pub mod tagging;
