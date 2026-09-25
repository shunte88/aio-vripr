/*
 *  metering.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  The meter worker: levels from the capture stream to the UI (§17, §18, §36).
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

//! The meter worker: levels from the capture stream to the UI (§17, §18, §36).
//!
//! §36 lists a meter worker among the eleven, and this is it: one thread that
//! owns a [`Tap`] on the capture stream, a [`Meter`], and a clone of the
//! [`Bus`]. It reads whatever the tap has, feeds it to the meter, and every
//! [`INTERVAL`] publishes an [`Event::Meter`].
//!
//! # Why it cannot cost a sample
//!
//! The tap is lossy by construction (see [`vcw_audio::buffers::Tee`]). This
//! thread can stall, deadlock, or be starved by the scheduler and the recording
//! is unaffected - the worst case is a needle that stops moving. That asymmetry
//! is deliberate and is the reason the meter is not fed from the callback:
//! §10's rule is that no consumer may cost a frame, and the only way to be sure
//! of it is to make dropping the cheap option.
//!
//! # Why it runs while armed
//!
//! §50's workflow sets the level *before* the needle goes down, so the meters
//! have to be live in `Armed`. They already are, for free: the writer runs
//! paused rather than stopped, so it is draining the ring the whole time, and
//! everything it drains goes past the tap whether it is being committed or not.
//! The meter measures what the *device* is producing, which is what a level
//! control acts on; the transport phase is not its business.
//!
//! # Rate
//!
//! [`INTERVAL`] is 20 ms, 50 Hz, the middle of §17's 30-60. S3 measured the
//! Tauri boundary carrying far more than this, so the constraint is the
//! browser's rendering rather than the traffic, and a rate the UI can drop
//! frames from is better than one it has to interpolate.
//!
//! The tap is drained more often than that - a snapshot every 20 ms is only
//! honest if the samples between snapshots actually reached the meter.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use vcw_audio::buffers::Tap;
use vcw_signal::meter::{Config, Meter};
use vcw_types::CaptureInfo;

use crate::events::{Bus, Event};

/// How often a snapshot is published: 50 Hz, the middle of §17's 30-60.
pub const INTERVAL: Duration = Duration::from_millis(20);

/// How often the tap is drained. Four times per snapshot, so a snapshot is
/// never made from a quarter of the audio it claims to cover.
const DRAIN: Duration = Duration::from_millis(5);

/// How much audio the tap can hold before it starts dropping: 200 ms.
///
/// Ten snapshots' worth. Enough that an ordinary scheduling hiccup costs
/// nothing, small enough that a genuinely stuck worker shows a stale needle
/// rather than catching up through a backlog of history nobody wants to see.
const TAP_MILLIS: u32 = 200;

/// A running meter worker.
///
/// Dropping it stops the thread but does not wait for it. [`Meters::stop`]
/// waits, which is what the engine wants on the way through `Stopped` so that
/// no snapshot arrives after `capture-finished`.
pub struct Meters {
    thread: Option<JoinHandle<()>>,
    running: Arc<AtomicBool>,
}

impl std::fmt::Debug for Meters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Meters")
            .field("running", &self.running.load(Ordering::Relaxed))
            .finish()
    }
}

impl Meters {
    /// Starts a meter worker on this tap.
    ///
    /// The capture's own format decides everything the meter needs to know -
    /// rate, channels and where full scale is - so there is nothing to
    /// configure and nothing that can be configured wrongly.
    #[must_use]
    pub fn spawn(tap: Tap, info: &CaptureInfo, bus: Bus) -> Self {
        let config = Config::new(info.channels as usize, info.rate.0, info.storage_format);
        let running = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&running);
        // A modest scratch: the tap holds 200 ms and this drains every 5 ms, so
        // reading a tenth of the tap at a time keeps up with a wide margin and
        // still copies in one call in the ordinary case.
        let scratch = bytes_for(info, TAP_MILLIS / 10);
        let thread = std::thread::Builder::new()
            .name("vcw-meter".into())
            .spawn(move || run(tap, config, &bus, &flag, scratch))
            .ok();
        Self { thread, running }
    }

    /// Stops the worker and waits for it.
    pub fn stop(mut self) {
        self.running.store(false, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            // A meter thread that will not join is not worth a failed capture:
            // it holds nothing the project needs, and it cannot be holding the
            // device, because all it has is the reading end of a ring.
            let _ = thread.join();
        }
    }
}

impl Drop for Meters {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

/// How big a tap this capture's meter wants, in bytes.
///
/// Public because the [`Tee`](vcw_audio::buffers::Tee) is built where the
/// source is, which is not here - and the size is this module's business
/// rather than the engine's.
#[must_use]
pub fn tap_bytes(info: &CaptureInfo) -> usize {
    bytes_for(info, TAP_MILLIS)
}

/// Bytes of interleaved audio this capture produces in `millis`.
fn bytes_for(info: &CaptureInfo, millis: u32) -> usize {
    let frame = info.storage_format.bytes_per_sample() * info.channels as usize;
    let frames = (u64::from(info.rate.0) * u64::from(millis)).div_ceil(1000);
    (frames as usize * frame).max(frame)
}

/// The worker loop.
fn run(mut tap: Tap, config: Config, bus: &Bus, running: &AtomicBool, scratch: usize) {
    let mut meter = Meter::new(config);
    let mut buffer = vec![0u8; scratch];
    let mut due = Instant::now() + INTERVAL;
    let mut reported = 0;

    while running.load(Ordering::Relaxed) {
        // Drain everything waiting, not one buffer of it, so a worker that was
        // late does not stay late.
        loop {
            let read = tap.read(&mut buffer);
            if read == 0 {
                break;
            }
            meter.feed(&buffer[..read]);
        }

        // Nothing new means nothing to say. Reading a snapshot resets the
        // instantaneous peak, so publishing on a tick that found no audio
        // would report silence the stream never contained - a needle flicking
        // to the floor every time this thread woke a millisecond early.
        // Genuine silence still reports: a quiet device sends zeros, and zeros
        // are frames.
        if Instant::now() >= due && meter.frames() > reported {
            reported = meter.frames();
            bus.publish(&Event::Meter {
                levels: meter.snapshot(),
            });
            due += INTERVAL;
            // If the thread was descheduled for longer than a snapshot, catch
            // the clock up rather than firing a burst of empty snapshots at the
            // UI to make up the difference.
            let now = Instant::now();
            if due < now {
                due = now + INTERVAL;
            }
        }

        if tap.is_abandoned() && tap.available() == 0 {
            break;
        }
        std::thread::sleep(DRAIN);
    }

    // One last snapshot on the way out, so a UI holding the final levels is
    // holding the levels at the end rather than the levels 20 ms before it.
    if meter.frames() > reported {
        bus.publish(&Event::Meter {
            levels: meter.snapshot(),
        });
    }
}
