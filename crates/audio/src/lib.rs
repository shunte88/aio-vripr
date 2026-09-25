//! Audio device management, bit-perfect capture and playback.
//!
//! This is the only crate in the workspace that links CPAL. Everything downstream
//! of the callback deals in [`vcw_types`] vocabulary and plain PCM, so the rest of
//! the tree neither knows nor cares which host API delivered the bytes.
//!
//! Requirements: §7 (devices), §8 (capture configuration), §9 (bit-perfect capture),
//! §10 (the real-time pipeline), §21 (playback).

pub mod buffers;
pub mod capture;
pub mod devices;
pub mod playback;
