//! Shared test scaffolding: synthesising a capture the way the writer thread will.
//!
//! WP-05 builds the real writer. Until then the tests need blocks that are
//! *shaped* correctly - contiguous, per-channel, checksummed, the declared width -
//! so that validation and round-tripping are exercised against something the
//! production path could plausibly have written.
//!
//! Each test binary compiles this separately, so not every one uses all of it.

#![allow(dead_code)]

use vcw_project::{Project, block_checksum};
use vcw_types::StorageFormat;

/// The shape of a synthetic capture.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Blocks {
    /// How samples are stored.
    pub(crate) format: StorageFormat,
    /// Channels, each with its own block sequence (per-channel layout, D3).
    pub(crate) channels: u16,
    /// Blocks per channel.
    pub(crate) blocks: u32,
    /// Frames in each block.
    pub(crate) frames: u32,
}

impl Blocks {
    /// A capture of `blocks` blocks per channel.
    pub(crate) fn new(format: StorageFormat, channels: u16, blocks: u32, frames: u32) -> Self {
        Self {
            format,
            channels,
            blocks,
            frames,
        }
    }
}

/// Deterministic pseudo-sample bytes, distinct per block.
///
/// Not silence and not random: a round-trip that compares two buffers of zeroes
/// proves nothing, and a random buffer makes a failure hard to reproduce.
pub(crate) fn sample_bytes(format: StorageFormat, frames: u32, seed: u32) -> Vec<u8> {
    let len = frames as usize * format.bytes_per_sample();
    let mut out = Vec::with_capacity(len);
    let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
    for _ in 0..len {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        out.push((x >> 24) as u8);
    }
    out
}

/// Writes one capture and its blocks. Returns `(blockid, samples)` per block.
pub(crate) fn insert_capture(project: &mut Project, shape: Blocks) -> Vec<(i64, Vec<u8>)> {
    let now = 1_700_000_000i64;
    let conn = project.conn_mut();
    conn.execute(
        "INSERT INTO captures
            (capture_id, sample_rate, channels, storage_format, capture_mode, started_at,
             finished_at, frames, state)
         VALUES (1, 48000, ?1, ?2, 'Exclusive', ?3, ?4, ?5, 'finalised')",
        rusqlite::params![
            shape.channels,
            shape.format.code(),
            now,
            now + 60,
            shape.blocks * shape.frames,
        ],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO capture_diagnostics (capture_id, updated_at) VALUES (1, ?1)",
        [now],
    )
    .unwrap();

    let mut written = Vec::new();
    let tx = conn.transaction().unwrap();
    // Channels interleave their block ids in one autoincrement sequence, exactly
    // as Audacity's do (S5) and as the per-channel layout S2 measured implies.
    for sequence in 0..shape.blocks {
        for channel in 0..shape.channels {
            let seed = sequence * u32::from(shape.channels) + u32::from(channel);
            let samples = sample_bytes(shape.format, shape.frames, seed);
            tx.execute(
                "INSERT INTO sampleblocks (sampleformat, samples) VALUES (?1, ?2)",
                rusqlite::params![shape.format.code(), samples],
            )
            .unwrap();
            let blockid = tx.last_insert_rowid();
            tx.execute(
                "INSERT INTO capture_blocks
                    (blockid, capture_id, channel, sequence, start_frame, frame_count,
                     checksum, committed_at)
                 VALUES (?1, 1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    blockid,
                    channel,
                    sequence,
                    sequence * shape.frames,
                    shape.frames,
                    block_checksum(&samples),
                    now,
                ],
            )
            .unwrap();
            written.push((blockid, samples));
        }
    }
    tx.commit().unwrap();
    written
}
