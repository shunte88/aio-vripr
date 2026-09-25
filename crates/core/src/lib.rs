//! The application engine - everything but the user interface.
//!
//! Requirements: §11 (recording state), §35 (command and event surface), §36
//! (concurrency), §4.5 (a cross-platform Rust core).
//!
//! §2 is the rule this crate exists to enforce: the core is independent of any UI
//! framework. `vcw-cli` drives the whole application through this surface, and the
//! Tauri shell (WP-15) is one more consumer of the same commands and events, with
//! no logic of its own. CI asserts that no crate below `app/` depends on Tauri.
//!
//! Concurrency follows D8: dedicated OS threads for capture, metering, waveform and
//! detection, tokio confined to network and export I/O. No async on the RT path.

pub mod commands;
pub mod engine;
pub mod events;
pub mod state;
