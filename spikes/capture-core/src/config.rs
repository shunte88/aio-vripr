//! Benchmark parameters.
//!
//! Every knob REQUIREMENTS.md §48 asks us to sweep is represented here, plus the
//! block *layout* question that fell out of the AUP4-superset decision (D1): AUP4
//! stores one channel per block, while a bit-perfect capture arrives interleaved.
//! Deinterleaving preserves every sample value, so both layouts are legitimate —
//! which one costs less is a measurement, not an opinion.

use clap::{Args, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Layout {
    /// One row per block holding the interleaved stream exactly as the device gave it.
    Interleaved,
    /// One row per channel per block, AUP4-style. Sample values are unchanged.
    PerChannel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Journal {
    Wal,
    Delete,
    Truncate,
    Memory,
}

impl Journal {
    pub fn as_pragma(self) -> &'static str {
        match self {
            Journal::Wal => "WAL",
            Journal::Delete => "DELETE",
            Journal::Truncate => "TRUNCATE",
            Journal::Memory => "MEMORY",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Sync {
    Off,
    Normal,
    Full,
}

impl Sync {
    pub fn as_pragma(self) -> &'static str {
        match self {
            Sync::Off => "OFF",
            Sync::Normal => "NORMAL",
            Sync::Full => "FULL",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Checkpoint {
    /// Leave SQLite's own autocheckpoint alone.
    Auto,
    /// Writer issues PASSIVE checkpoints itself; autocheckpoint disabled.
    Passive,
    /// Writer issues TRUNCATE checkpoints itself; autocheckpoint disabled.
    Truncate,
    /// No checkpointing at all — shows unbounded WAL growth.
    Off,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Summaries {
    /// Store no waveform summaries.
    None,
    /// Compute and store summary256 + summary64k, as AUP3/AUP4 do.
    Aup4,
}

#[derive(Debug, Clone, Args)]
pub struct Params {
    /// Sample rate in Hz.
    #[arg(long, default_value_t = 192_000)]
    pub rate: u32,

    /// Channel count.
    #[arg(long, default_value_t = 2)]
    pub channels: u16,

    /// Bytes per sample as delivered by the device: 2 = i16, 3 = packed 24, 4 = i32/f32.
    #[arg(long, default_value_t = 4)]
    pub bytes_per_sample: usize,

    /// Audio block duration in milliseconds.
    #[arg(long, default_value_t = 1000)]
    pub block_ms: u32,

    /// Blocks committed per SQLite transaction.
    #[arg(long, default_value_t = 4)]
    pub batch_blocks: usize,

    /// Simulated device callback size in frames.
    #[arg(long, default_value_t = 512)]
    pub callback_frames: u32,

    /// Bounded ring capacity in milliseconds of audio.
    #[arg(long, default_value_t = 500)]
    pub ring_ms: u32,

    #[arg(long, value_enum, default_value_t = Layout::Interleaved)]
    pub layout: Layout,

    #[arg(long, value_enum, default_value_t = Journal::Wal)]
    pub journal: Journal,

    #[arg(long, value_enum, default_value_t = Sync::Normal)]
    pub sync: Sync,

    #[arg(long, value_enum, default_value_t = Checkpoint::Auto)]
    pub checkpoint: Checkpoint,

    /// Blocks between writer-issued checkpoints (ignored unless --checkpoint passive|truncate).
    #[arg(long, default_value_t = 64)]
    pub checkpoint_blocks: u64,

    #[arg(long, value_enum, default_value_t = Summaries::Aup4)]
    pub summaries: Summaries,

    /// SQLite page size in bytes.
    #[arg(long, default_value_t = 4096)]
    pub page_size: u32,

    /// Concurrent analysis reader threads (waveform + fingerprint simulation).
    #[arg(long, default_value_t = 2)]
    pub readers: usize,

    /// Capture duration in seconds.
    #[arg(long, default_value_t = 60)]
    pub duration: u64,

    /// Produce audio faster than real time. 1.0 = real time; 4.0 = deliberate abuse.
    #[arg(long, default_value_t = 1.0)]
    pub rate_multiplier: f64,
}

impl Params {
    /// Parameters for a live capture, where the device dictates rate, channel
    /// count and sample width rather than a sweep choosing them. Storage
    /// settings start at S2's measured recommendation (see
    /// `docs/spikes/S2-sqlite-capture.md`): 250 ms blocks committed one at a
    /// time, WAL, `synchronous=FULL` — chosen for recovery granularity, since
    /// S2 showed throughput is not the binding constraint.
    pub fn default_for(rate: u32, channels: u16, bytes_per_sample: usize) -> Self {
        Self {
            rate,
            channels,
            bytes_per_sample,
            block_ms: 250,
            batch_blocks: 1,
            callback_frames: 512,
            ring_ms: 500,
            layout: Layout::PerChannel,
            journal: Journal::Wal,
            sync: Sync::Full,
            checkpoint: Checkpoint::Auto,
            checkpoint_blocks: 64,
            summaries: Summaries::Aup4,
            page_size: 4096,
            readers: 0,
            duration: 60,
            rate_multiplier: 1.0,
        }
    }

    pub fn frame_bytes(&self) -> usize {
        self.bytes_per_sample * self.channels as usize
    }

    pub fn block_frames(&self) -> u64 {
        (self.rate as u64 * self.block_ms as u64) / 1000
    }

    pub fn ring_bytes(&self) -> usize {
        let frames = (self.rate as u64 * self.ring_ms as u64) / 1000;
        frames as usize * self.frame_bytes()
    }

    pub fn byte_rate(&self) -> f64 {
        self.rate as f64 * self.frame_bytes() as f64
    }

    /// Deterministic sample value for (frame, channel), so that any block can be
    /// re-derived and compared byte for byte during verification and after a crash.
    pub fn expected_sample(frame: u64, channel: u16) -> u32 {
        let x = frame
            .wrapping_mul(2_654_435_761)
            .wrapping_add(channel as u64 + 1);
        (x ^ (x >> 29)) as u32
    }

    pub fn write_sample(buf: &mut [u8], value: u32, bytes_per_sample: usize) {
        let le = value.to_le_bytes();
        buf[..bytes_per_sample].copy_from_slice(&le[..bytes_per_sample]);
    }
}
