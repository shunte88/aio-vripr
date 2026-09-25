//! Capture sessions: one per contiguous recording, with its diagnostics counters.
//!
//! Filled by WP-02. §10 requires overruns, underruns, dropped frames and stream
//! errors to be counted *and persisted* - they belong to the recording, not to the
//! process that made it.
