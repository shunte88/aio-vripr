/*
 *  soak.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  The long-run writer soak: the measurement D3 and WP-05 both depend on.
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

//! The long-run writer soak: the measurement D3 and WP-05 both depend on.
//!
//! WP-05's exit criterion is "90-minute 24/192 soak, zero loss, bounded WAL",
//! and D3's outstanding item is the same soak run against the *firmed*
//! configuration rather than the spike harness's defaults. This verb is where
//! both get satisfied, with the product code, on whatever machine it is pointed
//! at - which matters, because S2's numbers are x86_64 and the Pi 5 and Windows
//! runs are still open.
//!
//! The source is [`Simulated`] with [`Pattern::Deterministic`], which is the
//! only reason the run proves anything. Every sample is a pure function of its
//! frame and channel index, so after 90 minutes the verifier can recompute all
//! 8 GB of what should be there and compare it against what is, byte for byte.
//! A soak that only checked that the frame count looked right would pass just as
//! happily with the channels swapped.
//!
//! No hardware is involved and none is claimed: a simulated capture can never be
//! called bit-perfect, and the writer cannot tell the difference between this and
//! a turntable, which is the whole point of the `PcmSource` boundary.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use vcw_audio::capture::Negotiated;
use vcw_audio::source::{Faults, Pace, Pattern, Simulated, Source};
use vcw_project::persistence::{self, Checkpoint};
use vcw_project::{Project, validate};
use vcw_types::{CaptureState, SampleFormat, SampleRate, StorageFormat};

use crate::capture::Format;

/// WAL policies selectable on the command line.
#[derive(Clone, Copy, Debug, clap::ValueEnum)]
pub(crate) enum Wal {
    /// SQLite's own autocheckpoint. What S2 measured, and D3's choice.
    Automatic,
    /// Writer-issued `PASSIVE` checkpoints.
    Passive,
    /// Writer-issued `TRUNCATE` checkpoints.
    Truncate,
    /// None at all, to see how far the log actually grows.
    Never,
}

impl From<Wal> for Checkpoint {
    fn from(w: Wal) -> Self {
        match w {
            Wal::Automatic => Self::Automatic,
            Wal::Passive => Self::Passive,
            Wal::Truncate => Self::Truncate,
            Wal::Never => Self::Never,
        }
    }
}

/// Everything the soak was asked to do.
pub(crate) struct Options {
    /// Project to write. Must not already exist.
    pub(crate) project: PathBuf,
    /// Sample rate.
    pub(crate) rate: u32,
    /// Channel count.
    pub(crate) channels: u16,
    /// Sample format.
    pub(crate) format: Format,
    /// How long to run, in minutes.
    pub(crate) minutes: f64,
    /// Block duration. The D3 parameter under test.
    pub(crate) block_millis: u32,
    /// Blocks per transaction. The other D3 parameter under test.
    pub(crate) batch_blocks: usize,
    /// WAL policy.
    pub(crate) wal: Wal,
    /// Blocks between writer-issued checkpoints.
    pub(crate) checkpoint_blocks: u64,
    /// WAL ceiling in mebibytes, for the automatic policy.
    pub(crate) wal_mib: u64,
    /// Ring capacity in milliseconds.
    pub(crate) ring_millis: u32,
    /// Run flat out instead of in real time. Useful for a smoke test of this
    /// verb; useless as a measurement, and labelled as such in the output.
    pub(crate) fast: bool,
    /// Skip the byte-for-byte readback. Only sensible when the run is being
    /// killed deliberately.
    pub(crate) no_verify: bool,
    /// How often to print a progress line, in seconds. Zero for silence.
    pub(crate) every: u64,
    /// Machine-readable output.
    pub(crate) json: bool,
}

/// Runs the soak.
pub(crate) fn run(options: &Options) -> Result<()> {
    if options.project.exists() {
        // An 8 GB append into a project that already holds a side is not a
        // thing anyone means to ask for.
        bail!(
            "{} already exists; the soak writes a fresh project",
            options.project.display()
        );
    }

    let format = SampleFormat::from(options.format);
    let negotiated = Negotiated::simulated(SampleRate(options.rate), options.channels, format);
    let storage = negotiated.storage;
    let frame_bytes = negotiated.frame_bytes();
    let pace = if options.fast {
        Pace::Fast
    } else {
        Pace::RealTime
    };

    let config = persistence::Config {
        block_millis: options.block_millis,
        batch_blocks: options.batch_blocks,
        checkpoint: options.wal.into(),
        checkpoint_blocks: options.checkpoint_blocks,
        wal_bytes: options.wal_mib * 1024 * 1024,
        summaries: true,
        ..persistence::Config::default()
    };

    let (source, reader) = Simulated::start(
        negotiated.clone(),
        &Pattern::Deterministic,
        pace,
        Faults::none(),
        options.ring_millis,
    )
    .context("starting the simulated source")?;
    let info = source.info();

    let project = Project::create(&options.project)
        .with_context(|| format!("creating {}", options.project.display()))?;
    let handle = persistence::spawn(project, &info, config, reader)
        .context("starting the capture writer")?;
    let capture_id = handle.capture_id();
    let progress = std::sync::Arc::clone(handle.progress());

    if !options.json {
        println!(
            "soaking {} Hz, {} ch, {:?} ({} B/frame) for {:.1} min into {}",
            options.rate,
            options.channels,
            storage,
            frame_bytes,
            options.minutes,
            options.project.display(),
        );
        println!(
            "  config      {} ms blocks, batch {}, {:?} checkpointing at {} MiB, \
             {} ms ring, {} pace",
            config.block_millis,
            config.batch_blocks,
            config.checkpoint,
            options.wal_mib,
            options.ring_millis,
            if options.fast { "fast" } else { "real-time" },
        );
    }

    let started = Instant::now();
    let run_for = Duration::from_secs_f64(options.minutes * 60.0);
    let mut next_report = Duration::from_secs(options.every);
    while started.elapsed() < run_for {
        std::thread::sleep(Duration::from_millis(200));
        if options.every == 0 || options.json || started.elapsed() < next_report {
            continue;
        }
        next_report += Duration::from_secs(options.every);
        let elapsed = started.elapsed().as_secs_f64();
        println!(
            "  {:>6.0} s   {:.1} s written, rtf {:.5}, worst commit {:.1} ms, \
             wal {:.2} MiB, {} overrun(s)",
            elapsed,
            progress.frames() as f64 / f64::from(options.rate),
            progress.frames() as f64 / f64::from(options.rate) / elapsed,
            progress.worst_commit_micros() as f64 / 1_000.0,
            progress.peak_wal_bytes() as f64 / (1024.0 * 1024.0),
            source.diagnostics().overruns,
        );
    }
    let wall = started.elapsed();

    let diagnostics = source.stop();
    let delivered = diagnostics.dropped_frames;
    let state = if diagnostics.is_clean() {
        CaptureState::Finalised
    } else {
        CaptureState::Interrupted
    };
    handle.set_result(state, diagnostics);
    let outcome = handle.stop().context("stopping the capture writer")?;

    let project = Project::open(&options.project)
        .with_context(|| format!("reopening {}", options.project.display()))?;
    // Checksums on: they read every sample byte, which is exactly what a soak
    // has time for and what a routine open does not.
    let report = validate(
        &project,
        vcw_project::Options {
            verify_checksums: !options.no_verify,
        },
    )
    .context("validating")?;
    let verified = if options.no_verify {
        None
    } else {
        Some(verify(&project, capture_id, storage, options.channels).context("verifying")?)
    };
    let file_bytes = std::fs::metadata(&options.project)
        .map(|m| m.len())
        .unwrap_or(0);
    project.close().context("closing the project")?;

    let audio_secs = outcome.frames as f64 / f64::from(options.rate);
    let rtf = if wall.as_secs_f64() > 0.0 {
        audio_secs / wall.as_secs_f64()
    } else {
        0.0
    };
    let (p50, p95, p99, worst) = outcome.commit.summary().unwrap_or((0, 0, 0, 0));
    let passed = diagnostics.is_clean()
        && report.is_clean()
        && verified.is_none_or(|v| v)
        && outcome.commits_within_budget(&config);

    if options.json {
        let payload = serde_json::json!({
            "config": {
                "rate": options.rate, "channels": options.channels,
                "storage": format!("{storage:?}"), "frame_bytes": frame_bytes,
                "block_millis": config.block_millis, "batch_blocks": config.batch_blocks,
                "checkpoint": format!("{:?}", config.checkpoint),
                "checkpoint_blocks": config.checkpoint_blocks,
                "wal_bytes": config.wal_bytes,
                "ring_millis": options.ring_millis,
                "pace": if options.fast { "fast" } else { "real-time" },
                "requested_minutes": options.minutes,
            },
            "wall_secs": wall.as_secs_f64(),
            "audio_secs": audio_secs,
            "real_time_factor": rtf,
            "written": {
                "blocks": outcome.blocks, "frames": outcome.frames, "bytes": outcome.bytes,
                "commits": outcome.commits, "checkpoints": outcome.checkpoints,
                "file_bytes": file_bytes, "peak_wal_bytes": outcome.peak_wal_bytes,
            },
            "commit_micros": { "p50": p50, "p95": p95, "p99": p99, "max": worst,
                               "budget": config.commit_granularity_millis() * 1_000 },
            "prepare_micros": outcome.prepare.summary()
                .map(|(a, b, c, d)| serde_json::json!({ "p50": a, "p95": b, "p99": c, "max": d })),
            "checkpoint_micros": outcome.checkpoint.summary()
                .map(|(a, b, c, d)| serde_json::json!({ "p50": a, "p95": b, "p99": c, "max": d })),
            "final_checkpoint_micros": outcome.final_checkpoint_micros,
            "diagnostics": diagnostics,
            "dropped_frames": delivered,
            "validate_clean": report.is_clean(),
            "bytes_verified": verified,
            "passed": passed,
        });
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    println!(
        "  ran         {:.1} s wall, {:.1} s of audio, real-time factor {rtf:.5}",
        wall.as_secs_f64(),
        audio_secs,
    );
    println!(
        "  written     {} frames, {} blocks, {:.2} GiB of samples in {} commits",
        outcome.frames,
        outcome.blocks,
        outcome.bytes as f64 / (1024.0 * 1024.0 * 1024.0),
        outcome.commits,
    );
    println!(
        "  commit      p50 {:.1} ms, p95 {:.1} ms, p99 {:.1} ms, max {:.1} ms, budget {} ms",
        p50 as f64 / 1_000.0,
        p95 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        worst as f64 / 1_000.0,
        config.commit_granularity_millis(),
    );
    if let Some((a, _, _, d)) = outcome.prepare.summary() {
        println!(
            "  prepare     p50 {:.1} ms, max {:.1} ms (deinterleave, summaries, crc)",
            a as f64 / 1_000.0,
            d as f64 / 1_000.0,
        );
    }
    println!(
        "  wal         peak {:.2} MiB, {} writer checkpoint(s){}",
        outcome.peak_wal_bytes as f64 / (1024.0 * 1024.0),
        outcome.checkpoints,
        outcome
            .checkpoint
            .max()
            .map_or_else(String::new, |m| format!(
                ", worst {:.1} ms",
                m as f64 / 1_000.0
            )),
    );
    println!(
        "  file        {:.2} GiB",
        file_bytes as f64 / (1024.0 * 1024.0 * 1024.0)
    );
    println!(
        "  counters    {} overruns, {} underruns, {} dropped frames, {} stream errors",
        diagnostics.overruns,
        diagnostics.underruns,
        diagnostics.dropped_frames,
        diagnostics.stream_errors,
    );
    println!(
        "  validate    {}",
        if report.is_clean() {
            "clean".to_owned()
        } else {
            format!("{} problem(s)", report.findings.len())
        }
    );
    for finding in report.findings.iter().take(5) {
        println!("              - {}: {}", finding.code, finding.detail);
    }
    println!(
        "  bytes       {}",
        match verified {
            None => "not checked (--no-verify)".to_owned(),
            Some(true) => format!(
                "every one of {} matches what the source generated",
                outcome.bytes
            ),
            Some(false) => "MISMATCH - see above".to_owned(),
        }
    );
    println!(
        "  verdict     {}",
        if passed {
            "pass: zero loss, bounded WAL, every byte accounted for"
        } else {
            "FAIL"
        }
    );

    if passed {
        Ok(())
    } else {
        bail!("the soak did not pass")
    }
}

/// Recomputes every sample the source should have produced and compares it with
/// what landed in the project.
///
/// Block by block rather than all at once: a 90-minute 24/192 capture is 8 GB,
/// and a verifier that needed it in memory could not check the run it was
/// written for. The frame index is taken from `start_frame`, so a block written
/// out of order or at the wrong offset fails here rather than passing on its own
/// internal consistency.
fn verify(
    project: &Project,
    capture_id: i64,
    storage: StorageFormat,
    channels: u16,
) -> Result<bool> {
    let width = storage.bytes_per_sample();
    let mut stmt = project.conn().prepare(
        "SELECT b.channel, b.start_frame, b.frame_count, s.samples
         FROM capture_blocks b JOIN sampleblocks s ON s.blockid = b.blockid
         WHERE b.capture_id = ?1 ORDER BY b.sequence, b.channel",
    )?;
    let mut rows = stmt.query([capture_id])?;
    let mut expected_next = vec![0u64; channels as usize];
    let mut ok = true;
    while let Some(row) = rows.next()? {
        let channel: i64 = row.get(0)?;
        let start_frame: i64 = row.get(1)?;
        let frame_count: i64 = row.get(2)?;
        let samples: Vec<u8> = row.get(3)?;

        if channel < 0 || channel as usize >= expected_next.len() {
            println!("  !           a block claims channel {channel} of {channels}");
            return Ok(false);
        }
        let slot = &mut expected_next[channel as usize];
        if start_frame as u64 != *slot {
            println!(
                "  !           channel {channel} block starts at frame {start_frame}, \
                 expected {slot}"
            );
            ok = false;
        }
        if samples.len() != frame_count as usize * width {
            println!(
                "  !           channel {channel} block at {start_frame} holds {} bytes for \
                 {frame_count} frames",
                samples.len()
            );
            ok = false;
        }
        for i in 0..frame_count as usize {
            let frame = start_frame as u64 + i as u64;
            let want = Simulated::expected_sample(frame, channel as u16).to_le_bytes();
            let got = &samples[i * width..(i + 1) * width];
            if got != &want[..width] {
                println!(
                    "  !           channel {channel} frame {frame}: stored {got:02X?}, \
                     source produced {:02X?}",
                    &want[..width]
                );
                return Ok(false);
            }
        }
        *slot = start_frame as u64 + frame_count as u64;
    }
    Ok(ok)
}
