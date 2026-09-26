/*
 *  detection.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  The detection workers: provisional boundaries live, the whole side afterwards.
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

//! The detection workers: provisional boundaries live, the whole side afterwards.
//!
//! §22 asks for both halves: "Live analysis creates provisional markers. A post-capture
//! pass may refine them using the complete recording." [`Detectors`] is the first, a
//! worker on a lossy tap of the capture stream in the shape of [`crate::metering`].
//! [`refine`] is the second, and reads the committed audio back out of the project.
//!
//! What each half can afford is what separates them:
//!
//! - The live pass runs levels only. One number per 100 ms window, no FFT, and it
//!   announces a boundary only once nothing later can move it - about 1.2 s of lag -
//!   so a marker never has to be retracted in front of someone.
//! - The refine pass runs all three of §22's detectors over one spectral extraction of
//!   the whole side, and puts their observations through
//!   [`vcw_signal::resolve`]. It costs an FFT per window and can afford it: the record
//!   has stopped turning.
//!
//! Neither of them edits anything, which is §23: "analysis subsystems shall publish
//! observations rather than directly modifying tracks". The live pass publishes
//! [`Event::Detected`]. The refine pass returns [`Refined`] to its caller. Turning
//! either into tracks is the project's decision to make and WP-13's code to make it in.
//!
//! Requirements: §22 (live and refine), §23 (observations), §24 (provenance), §36 (a
//! worker per job).

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use vcw_audio::buffers::Tap;
use vcw_project::pcm::{Layout, Reader};
use vcw_project::{Project, Result as ProjectResult};
use vcw_signal::features::{DEFAULT_WINDOW_MILLIS, Frame, Shape, Windows};
use vcw_signal::regions::{Config, Diagnostics, Outcome, Trace};
use vcw_signal::resolve::{Decision, Tolerance, resolve};
use vcw_signal::{hmm, silence, spectral};
use vcw_types::{BoundaryObservation, CaptureInfo, Provenance, Span};

use crate::events::{Bus, Event};

/// How often the tap is drained and the trace re-scanned.
///
/// A window is 100 ms and the settling lag is 1.2 s, so nothing is lost by looking
/// every quarter second - and a re-scan is the whole detector over the whole side,
/// which is cheap but not free. At 4 Hz a forty-minute side costs a few milliseconds
/// of this thread per second of audio.
const DRAIN: Duration = Duration::from_millis(250);

/// How much audio the tap can hold before it starts dropping: two seconds.
///
/// Ten times the drain interval. Larger than the meter's because this worker sleeps
/// for longer between reads, and because what it loses it loses permanently - a meter
/// that misses 200 ms shows a stale needle, while a detector that misses 200 ms has a
/// hole in its level trace and will place a boundary from audio it never saw.
///
/// Losing audio here still cannot cost the recording a frame: the tap is lossy by
/// construction, which is §10's rule that no consumer may cost a sample, and the
/// refine pass reads the committed audio rather than this stream.
const TAP_MILLIS: u32 = 2_000;

/// A running live-detection worker.
///
/// Dropping it stops the thread without waiting. [`Detectors::stop`] waits and returns
/// what the pass had found, which is what the engine wants on the way through
/// `Stopped`: the provisional markers are worth keeping even though the refine pass is
/// about to look again, because a capture that is never refined still has them.
pub struct Detectors {
    thread: Option<JoinHandle<Vec<BoundaryObservation>>>,
    running: Arc<AtomicBool>,
}

impl std::fmt::Debug for Detectors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Detectors")
            .field("running", &self.running.load(Ordering::Relaxed))
            .finish()
    }
}

impl Detectors {
    /// Starts a live pass on this tap.
    ///
    /// The capture's own format is everything the detector needs to know, exactly as
    /// with the meter, so there is nothing here that can be configured wrongly. The
    /// detector's own settings are [`Config::default`] - VRipr's numbers - because a
    /// live pass has nothing to tune against yet.
    #[must_use]
    pub fn spawn(tap: Tap, info: &CaptureInfo, bus: Bus) -> Self {
        Self::spawn_with(tap, info, bus, Config::default())
    }

    /// Starts a live pass with detector settings of your own.
    #[must_use]
    pub fn spawn_with(tap: Tap, info: &CaptureInfo, bus: Bus, cfg: Config) -> Self {
        let shape = Shape::new(
            info.rate,
            info.channels as usize,
            info.storage_format,
            DEFAULT_WINDOW_MILLIS,
        );
        let running = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&running);
        let scratch = bytes_for(info, TAP_MILLIS / 4);
        let thread = std::thread::Builder::new()
            .name("vcw-detect".into())
            .spawn(move || run(tap, &shape, cfg, &bus, &flag, scratch))
            .ok();
        Self { thread, running }
    }

    /// Stops the worker, waits for it, and returns every boundary it announced.
    #[must_use]
    pub fn stop(mut self) -> Vec<BoundaryObservation> {
        self.running.store(false, Ordering::Relaxed);
        self.thread
            .take()
            .and_then(|thread| thread.join().ok())
            .unwrap_or_default()
    }
}

impl Drop for Detectors {
    fn drop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
    }
}

/// How big a tap this capture's live detector wants, in bytes.
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

/// The live worker loop.
fn run(
    mut tap: Tap,
    shape: &Shape,
    cfg: Config,
    bus: &Bus,
    running: &AtomicBool,
    scratch: usize,
) -> Vec<BoundaryObservation> {
    let rate = f64::from(shape.rate.hz()).max(1.0);
    let mut live = silence::Live::new(shape, cfg);
    let mut buffer = vec![0u8; scratch];
    let mut announced: Vec<BoundaryObservation> = Vec::new();

    loop {
        let carry_on = running.load(Ordering::Relaxed);
        // Drain everything waiting rather than one buffer of it, so a worker that was
        // late does not stay late and lose audio to the tap wrapping.
        loop {
            let read = tap.read(&mut buffer);
            if read == 0 {
                break;
            }
            live.push(&buffer[..read]);
        }

        for boundary in live.fresh() {
            bus.publish(&Event::Detected {
                frame: boundary.at,
                seconds: boundary.at as f64 / rate,
                edge: boundary.edge,
                confidence: boundary.confidence,
                provenance: boundary.provenance,
            });
            announced.push(boundary);
        }

        if !carry_on || (tap.is_abandoned() && tap.available() == 0) {
            break;
        }
        std::thread::sleep(DRAIN);
    }
    announced
}

/// What the refine pass found.
#[derive(Debug, Clone, PartialEq)]
pub struct Refined {
    /// One decision per boundary, after every detector has been heard.
    pub decisions: Vec<Decision>,
    /// Every observation that went into them, kept because §24's evidence is only
    /// useful if the thing it is evidence *of* can still be examined.
    pub observations: Vec<BoundaryObservation>,
    /// What each detector reported about the pass itself, in the order
    /// [`Provenance`] puts them.
    pub diagnostics: Vec<(Provenance, Diagnostics)>,
    /// How many analysis windows the side came to.
    pub windows: usize,
    /// How long the extraction and the three detectors took.
    pub took: Duration,
}

impl Refined {
    /// The decisions for one edge, in order.
    #[must_use]
    pub fn edges(&self, edge: vcw_types::Edge) -> Vec<&Decision> {
        self.decisions
            .iter()
            .filter(|decision| decision.edge == edge)
            .collect()
    }
}

/// Runs the post-capture pass over a whole capture in an open project.
///
/// One extraction, three detectors, one resolver. The extraction is where the cost is,
/// at an FFT per window, and all three detectors read the frames it produced - the
/// level-only one included. That gives a property worth having: a disagreement between
/// two detectors on the same side cannot be a disagreement about what they were looking
/// at.
///
/// Boundaries a user has placed are not read from the project here, because nothing
/// writes them yet - that is WP-13. When it does, they belong in `already`, and §24's
/// locking then applies without a change to this function: see
/// [`vcw_signal::resolve::resolve`].
///
/// # Errors
///
/// If the capture is not in this project, or its audio cannot be read.
pub fn refine(
    project: &Project,
    capture_id: i64,
    cfg: &Config,
    already: &[BoundaryObservation],
) -> ProjectResult<Refined> {
    let began = Instant::now();
    let layout = Layout::of(project.conn(), capture_id)?;
    let shape = Shape::new(
        layout.rate,
        layout.channels as usize,
        layout.format,
        DEFAULT_WINDOW_MILLIS,
    );

    let mut windows = Windows::spectral(&shape);
    let mut frames: Vec<Frame> = Vec::new();
    let mut reader = Reader::open(project.conn(), capture_id, Span::whole(layout.frames))?;
    // A second of audio per read. Big enough that the per-call overhead disappears
    // against the FFTs, small enough that a forty-minute side is never held in memory
    // twice over.
    let mut buffer = vec![0u8; (layout.frame_bytes() * layout.rate.hz() as usize).max(4_096)];
    loop {
        let read = reader.fill(&mut buffer)?;
        if read == 0 {
            break;
        }
        windows.push(&buffer[..read], &mut frames);
    }
    windows.flush(&mut frames);

    let trace = Trace::from_windows(&frames, &windows);
    let passes: [(Provenance, Outcome); 3] = [
        (Provenance::Silence, silence::scan(&trace, cfg)),
        (Provenance::SpectralChange, spectral::scan(&trace, cfg)),
        (Provenance::Hmm, hmm::scan(&trace, cfg)),
    ];

    let mut observations: Vec<BoundaryObservation> = already.to_vec();
    let mut diagnostics = Vec::with_capacity(passes.len());
    for (provenance, outcome) in passes {
        diagnostics.push((provenance, outcome.diagnostics));
        observations.extend(outcome.boundaries);
    }

    let decisions = resolve(&observations, Tolerance::default_at(layout.rate));
    Ok(Refined {
        decisions,
        observations,
        diagnostics,
        windows: frames.len(),
        took: began.elapsed(),
    })
}

/// Opens a project read-only and runs [`refine`] over one of its captures.
///
/// The form a caller outside the engine wants - the CLI, and the command handler a UI
/// will reach this through - because analysis needs no write access and asking for none
/// means a side can be examined while another one is being recorded.
///
/// # Errors
///
/// If the project cannot be opened, or [`refine`] fails.
pub fn refine_project(
    path: &Path,
    capture_id: i64,
    cfg: &Config,
    already: &[BoundaryObservation],
) -> ProjectResult<Refined> {
    let project = Project::open_read_only(path)?;
    let refined = refine(&project, capture_id, cfg, already)?;
    project.close()?;
    Ok(refined)
}
