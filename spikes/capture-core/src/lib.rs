/*
 *  lib.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Storage and verification machinery shared by the Phase 0 spikes.
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

//! Storage and verification machinery shared by the Phase 0 spikes.
//!
//! S2 (`sqlite-capture-bench`) drives this with a synthetic real-time source to
//! answer REQUIREMENTS §48; S1 (`vinyl-audio-test`) drives the same code with a
//! live CPAL stream to answer §47. Keeping one implementation behind both means
//! the storage findings from S2 transfer to S1 without an asterisk, and it is
//! this code - not either binary - that gets ported into `crates/` at WP-02/04.

pub mod config;
pub mod db;
pub mod metrics;
pub mod producer;
pub mod readers;
pub mod verify;
pub mod writer;
