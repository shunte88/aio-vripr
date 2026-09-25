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
//! Today it carries `doctor`, `devices` and `formats`: enough to answer "what can
//! this machine record, and through which path", which is WP-03's whole question.
//! The capture verbs arrive with the engine at WP-07.

mod devices;

use clap::{Parser, Subcommand};
use vcw_types::STANDARD_RATES;

use crate::devices::Which;

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
