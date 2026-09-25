//! WAV and FLAC encoding (D5).
//!
//! Filled by WP-14. Our own WAV writer - trivial to write, and it avoids `hound`'s
//! format limits at 24/192. FLAC through `flacenc`, pure Rust and Apache-2.0. MP3 and
//! Ogg Vorbis are Phase 2 and both carry LGPL relink obligations, which is why they
//! are candidates for optional cargo features rather than defaults.
