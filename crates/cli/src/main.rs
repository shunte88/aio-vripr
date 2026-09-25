/*
 *  main.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  `vcw` - the headless driver.
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

//! `vcw` - the headless driver.
//!
//! §4.5 and WP-07 make this the primary interface to the core: the whole capture
//! workflow has to be drivable from here, with no UI present. That is not a
//! convenience, it is how the architectural rule in §2 gets tested - a core that
//! cannot be driven headlessly has leaked into its shell.
//!
//! Today it carries `doctor`, `devices` and `formats`, which answer "what can
//! this machine record, and through which path" (WP-03), `capture`, which
//! answers "and what did it actually do" (WP-04), and `soak`, which is how
//! WP-05's writer is measured on a machine before it is trusted with a side.
//! `session` arrives with the engine at WP-07 and is the one that matters: a
//! whole capture, driven by transport commands, with no UI present. The
//! editing and export verbs follow at WP-10 onwards.

mod capture;
mod devices;
mod recover;
mod session;
mod soak;

use clap::{Parser, Subcommand};
use vcw_types::STANDARD_RATES;

use crate::capture::{Format, Mode};
use crate::devices::Which;
use crate::soak::Wal;

#[derive(Parser)]
#[command(name = "vcw", version, about = "VCW - The Vinyl Capture Workstation")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report what this build can see: host APIs, SQLite, supported rates.
    Doctor,

    /// List audio devices, their ids, and what each one will record (§7).
    Devices {
        /// Restrict to capture or playback devices.
        #[arg(long, value_enum, default_value_t = Which::Both)]
        which: Which,
        /// Only direct-hardware paths: the ones that can be bit-perfect.
        #[arg(long)]
        hardware: bool,
        /// Machine-readable output, for comparing across machines.
        #[arg(long)]
        json: bool,
        /// Include drivers, interfaces and every advertised range.
        #[arg(short, long)]
        verbose: bool,
    },

    /// Show the §8 configurations one device accepts. Select it by id.
    Formats {
        /// Device id, as printed by `vcw devices`. A name works only if unique.
        device: String,
        /// Restrict to capture or playback.
        #[arg(long, value_enum, default_value_t = Which::Both)]
        which: Which,
        /// Open the device once per configuration to find out which it really
        /// accepts. Intrusive: it will fail while another application holds it.
        #[arg(long)]
        confirm: bool,
        /// Channel ceiling for --confirm. A plug PCM advertises 64 counts at
        /// every rate and format; confirming all of them opens the device
        /// thousands of times.
        #[arg(long, default_value_t = 8)]
        max_channels: u16,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },

    /// Record from a device and report what was really negotiated (§9).
    ///
    /// With --project the samples are written; without one there is nowhere to
    /// put them, so they are drained, counted and discarded.
    Capture {
        /// Device id, as printed by `vcw devices`. A name works only if unique.
        device: String,
        /// Sample rate in Hz. Omit to take the best the device offers.
        #[arg(long)]
        rate: Option<u32>,
        /// Channel count. Omit to take the best on offer.
        #[arg(long)]
        channels: Option<u16>,
        /// Sample format. Omit to take the widest integer format available.
        #[arg(long, value_enum)]
        format: Option<Format>,
        /// How to open the device. Only exclusive can be bit-perfect (§9).
        #[arg(long, value_enum, default_value_t = Mode::Exclusive)]
        mode: Mode,
        /// How long to record.
        #[arg(long, default_value_t = 5.0)]
        seconds: f64,
        /// Ring capacity in milliseconds. Raised to the 500 ms floor if lower.
        #[arg(long, default_value_t = 1000)]
        ring_millis: u32,
        /// Project file to record the session in. Created if absent.
        #[arg(long)]
        project: Option<std::path::PathBuf>,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },

    /// Drive a whole capture session from the transport commands (§11, §35).
    ///
    /// Reads one verb per line from stdin, or from --script. The verbs are
    /// arm, record, pause, resume, stop, reset, disarm, poll and quit, plus
    /// `sleep <seconds>` for a script that wants to record for a while. Every
    /// event the core publishes is printed as it happens.
    ///
    /// This is WP-07's exit criterion: a full capture, with no UI present.
    Session {
        /// Project to record into. Created if it does not exist.
        project: std::path::PathBuf,
        /// Device id, as printed by `vcw devices`. Omit for the simulated
        /// source, which needs nothing plugged in.
        #[arg(long)]
        device: Option<String>,
        /// Sample rate in Hz. Omit to take the best the device offers.
        #[arg(long)]
        rate: Option<u32>,
        /// Channel count. Omit to take the best on offer.
        #[arg(long)]
        channels: Option<u16>,
        /// Sample format. Omit to take the widest integer format available.
        #[arg(long, value_enum)]
        format: Option<Format>,
        /// How to open the device. Only exclusive can be bit-perfect (§9).
        #[arg(long, value_enum, default_value_t = Mode::Exclusive)]
        mode: Mode,
        /// Ring capacity in milliseconds. Raised to the 500 ms floor if lower.
        #[arg(long)]
        ring_millis: Option<u32>,
        /// A whole session on one line: --script "arm,record,sleep 2,stop".
        #[arg(long)]
        script: Option<String>,
        /// Machine-readable output: one JSON object per line.
        #[arg(long)]
        json: bool,
        /// Print the 50 Hz level meters too. Loud, and off by default.
        #[arg(long)]
        meters: bool,
    },

    /// Find unfinished captures left by a crash and close them honestly (§15).
    ///
    /// Reports by default and writes nothing. A recording that survived a
    /// crash is worth more than the convenience of not typing --apply.
    Recover {
        /// Project to examine.
        project: std::path::PathBuf,
        /// Write the reconstructed frame count, state and end time.
        #[arg(long)]
        apply: bool,
        /// Also delete blocks stranded past the recoverable end. Implies
        /// --apply, and is the only way to make recovery discard audio.
        #[arg(long)]
        repair: bool,
        /// Recompute every block's checksum afterwards. Reads the whole
        /// project, which is minutes for a full side.
        #[arg(long)]
        verify: bool,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },

    /// Drive the writer from a simulated source for a long time and check
    /// every byte that lands (WP-05, D3).
    Soak {
        /// Project to write. Must not already exist.
        project: std::path::PathBuf,
        /// Sample rate in Hz.
        #[arg(long, default_value_t = 192_000)]
        rate: u32,
        /// Channel count.
        #[arg(long, default_value_t = 2)]
        channels: u16,
        /// Sample format.
        #[arg(long, value_enum, default_value_t = Format::S24)]
        format: Format,
        /// How long to run, in minutes.
        #[arg(long, default_value_t = 90.0)]
        minutes: f64,
        /// Block duration in milliseconds. D3 says 250.
        #[arg(long, default_value_t = 250)]
        block_millis: u32,
        /// Blocks per transaction. D3 says 1.
        #[arg(long, default_value_t = 1)]
        batch_blocks: usize,
        /// WAL policy. D3 says automatic.
        #[arg(long, value_enum, default_value_t = Wal::Automatic)]
        wal: Wal,
        /// Blocks between writer-issued checkpoints, for the non-automatic
        /// policies.
        #[arg(long, default_value_t = 64)]
        checkpoint_blocks: u64,
        /// WAL ceiling in MiB, for the automatic policy. SQLite counts pages;
        /// VCW's are 64 KiB, so the stock threshold would be a 64 MiB log.
        #[arg(long, default_value_t = 4)]
        wal_mib: u64,
        /// Ring capacity in milliseconds. Raised to the 500 ms floor if lower.
        #[arg(long, default_value_t = 1000)]
        ring_millis: u32,
        /// Run flat out. Fast, and worthless as a timing measurement.
        #[arg(long)]
        fast: bool,
        /// Skip the byte-for-byte readback.
        #[arg(long)]
        no_verify: bool,
        /// Seconds between progress lines. Zero for silence.
        #[arg(long, default_value_t = 60)]
        every: u64,
        /// Machine-readable output.
        #[arg(long)]
        json: bool,
    },
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Doctor => doctor(),
        Command::Devices {
            which,
            hardware,
            json,
            verbose,
        } => devices::list(which, hardware, json, verbose),
        Command::Formats {
            device,
            which,
            confirm,
            max_channels,
            json,
        } => devices::formats(&device, which, confirm, max_channels, json),
        Command::Capture {
            device,
            rate,
            channels,
            format,
            mode,
            seconds,
            ring_millis,
            project,
            json,
        } => capture::run(&capture::Options {
            device,
            rate,
            channels,
            format,
            mode,
            seconds,
            ring_millis,
            project,
            json,
        }),
        Command::Session {
            project,
            device,
            rate,
            channels,
            format,
            mode,
            ring_millis,
            script,
            json,
            meters,
        } => session::run(&session::Args {
            project,
            device,
            rate,
            channels,
            format,
            mode,
            ring_millis,
            script,
            json,
            meters,
        }),
        Command::Recover {
            project,
            apply,
            repair,
            verify,
            json,
        } => recover::run(&recover::Args {
            project,
            apply,
            repair,
            verify,
            json,
        }),
        Command::Soak {
            project,
            rate,
            channels,
            format,
            minutes,
            block_millis,
            batch_blocks,
            wal,
            checkpoint_blocks,
            wal_mib,
            ring_millis,
            fast,
            no_verify,
            every,
            json,
        } => soak::run(&soak::Options {
            project,
            rate,
            channels,
            format,
            minutes,
            block_millis,
            batch_blocks,
            wal,
            checkpoint_blocks,
            wal_mib,
            ring_millis,
            fast,
            no_verify,
            every,
            json,
        }),
    }
}

fn doctor() -> anyhow::Result<()> {
    println!("vcw {}", env!("CARGO_PKG_VERSION"));
    println!("target      {}", std::env::consts::ARCH);
    println!("os          {}", std::env::consts::OS);
    println!(
        "sqlite      {} (bundled)",
        vcw_project::sqlite::runtime_version()
    );

    let hosts = vcw_audio::devices::available_hosts();
    println!("audio hosts {}", hosts.join(", "));

    let rates: Vec<String> = STANDARD_RATES.iter().map(|r| r.hz().to_string()).collect();
    println!("rates (§8)  {}", rates.join(", "));

    Ok(())
}
