//! `vcw` - the headless driver.
//!
//! §4.5 and WP-07 make this the primary interface to the core: the whole capture
//! workflow has to be drivable from here, with no UI present. That is not a
//! convenience, it is how the architectural rule in §2 gets tested - a core that
//! cannot be driven headlessly has leaked into its shell.
//!
//! Today it carries one subcommand, `doctor`, because a scaffold that builds and
//! runs is worth more than one that only builds.

use clap::{Parser, Subcommand};
use vcw_types::STANDARD_RATES;

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
}

fn main() -> anyhow::Result<()> {
    match Cli::parse().command {
        Command::Doctor => doctor(),
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
