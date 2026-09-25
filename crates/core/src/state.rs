//! The recording state machine: `Idle -> Armed -> Recording <-> Paused -> Stopped` (§11).
//!
//! Filled by WP-07. §11 says invalid transitions "shall be impossible", and WP-07's
//! exit criterion reads that literally: unrepresentable in the type system, not
//! rejected at runtime.
