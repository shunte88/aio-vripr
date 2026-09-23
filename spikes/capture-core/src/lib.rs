//! Storage and verification machinery shared by the Phase 0 spikes.
//!
//! S2 (`sqlite-capture-bench`) drives this with a synthetic real-time source to
//! answer REQUIREMENTS §48; S1 (`vinyl-audio-test`) drives the same code with a
//! live CPAL stream to answer §47. Keeping one implementation behind both means
//! the storage findings from S2 transfer to S1 without an asterisk, and it is
//! this code — not either binary — that gets ported into `crates/` at WP-02/04.

pub mod config;
pub mod db;
pub mod metrics;
pub mod producer;
pub mod readers;
pub mod verify;
pub mod writer;
