//! Multi-resolution waveform pyramid, built progressively during capture (§19).
//!
//! Filled by WP-09. §37 requires render cost independent of total recording length,
//! which is what the pyramid buys. S3 added the other half of the answer: the cost
//! that actually bites is main-thread drawing in the webview, not producing these
//! summaries or shipping them across the IPC boundary.
