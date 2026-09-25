//! The event surface the core reports through (§35).
//!
//! Filled by WP-07. S3 sized this: meter and waveform frames at 750 Hz crossed the
//! IPC boundary with 12.5x headroom and zero loss, so events are not coalesced on
//! the way out. Paints are coalesced instead, in the webview, one read of the latest
//! state per frame.
