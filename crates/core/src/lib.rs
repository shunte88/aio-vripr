/*
 *  lib.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  The application engine - everything but the user interface.
 *
 * MIT License
 *
 * Copyright (c) 2026 Stue Hunter
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 *
 */

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
