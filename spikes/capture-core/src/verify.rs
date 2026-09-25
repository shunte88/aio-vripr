/*
 *  verify.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Post-hoc verification, used both after a clean run and after a deliberate
 *  kill.
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

//! Post-hoc verification, used both after a clean run and after a deliberate kill.
//!
//! Checksums alone would only prove the bytes survived the trip. Where the payload
//! is S2's deterministic pattern we can go further and prove the bytes are the
//! *right* ones - that nothing was reordered, duplicated or silently resampled on
//! the way in. A live capture (S1) has no such oracle, so `verify_live` checks
//! everything except the pattern and reports `pattern_checked: false` rather than
//! reporting a pass it did not actually perform.

use anyhow::{Context, Result};
use rusqlite::Connection;
use serde::Serialize;

use crate::config::Layout;
use crate::db;

#[derive(Debug, Serialize)]
pub struct VerifyReport {
    pub integrity_ok: bool,
    pub integrity_detail: String,
    pub capture_found: bool,
    pub rows: u64,
    pub blocks: u64,
    pub frames: u64,
    pub duration_secs: f64,
    pub checksum_failures: u64,
    /// False when the payload is real audio, for which no expected-value oracle exists.
    pub pattern_checked: bool,
    pub pattern_failures: u64,
    pub sequence_gaps: u64,
    pub last_good_sequence: i64,
    pub db_bytes: u64,
}

/// Verify a capture whose payload is S2's deterministic pattern.
pub fn verify(db_path: &str) -> Result<VerifyReport> {
    verify_inner(db_path, true)
}

/// Verify a live capture: integrity, checksums and sequencing, but no pattern -
/// real audio is not predictable, and pretending otherwise would be a false pass.
pub fn verify_live(db_path: &str) -> Result<VerifyReport> {
    verify_inner(db_path, false)
}

fn verify_inner(db_path: &str, check_pattern: bool) -> Result<VerifyReport> {
    let conn = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening {db_path}"))?;

    let integrity: String = conn
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .unwrap_or_else(|e| format!("integrity_check failed: {e}"));

    let cap: Option<(u32, u16, usize, String)> = conn
        .query_row(
            "SELECT sample_rate, channels, bytes_per_sample, layout FROM captures WHERE capture_id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get::<_, i64>(2)? as usize, r.get(3)?)),
        )
        .ok();

    let mut report = VerifyReport {
        integrity_ok: integrity == "ok",
        integrity_detail: integrity,
        capture_found: cap.is_some(),
        rows: 0,
        blocks: 0,
        frames: 0,
        duration_secs: 0.0,
        checksum_failures: 0,
        pattern_checked: check_pattern,
        pattern_failures: 0,
        sequence_gaps: 0,
        last_good_sequence: -1,
        db_bytes: crate::metrics::file_len(std::path::Path::new(db_path)),
    };

    let Some((rate, channels, bps, layout_s)) = cap else {
        return Ok(report);
    };
    let layout = if layout_s.eq_ignore_ascii_case("perchannel") {
        Layout::PerChannel
    } else {
        Layout::Interleaved
    };

    let mut stmt = conn.prepare(
        "SELECT c.sequence, c.channel, c.start_frame, c.frame_count, c.checksum, s.samples
           FROM capture_blocks c JOIN sampleblocks s ON s.blockid = c.blockid
          ORDER BY c.sequence, c.channel",
    )?;
    let mut rows = stmt.query([])?;

    let mut expect_seq: i64 = 0;
    let mut seen_seq: i64 = -1;
    while let Some(r) = rows.next()? {
        let sequence: i64 = r.get(0)?;
        let channel: i64 = r.get(1)?;
        let start_frame: i64 = r.get(2)?;
        let frame_count: i64 = r.get(3)?;
        let checksum: i64 = r.get(4)?;
        let samples: Vec<u8> = r.get(5)?;

        report.rows += 1;
        if sequence != seen_seq {
            report.blocks += 1;
            report.frames += frame_count as u64;
            if sequence != expect_seq {
                report.sequence_gaps += 1;
            }
            expect_seq = sequence + 1;
            seen_seq = sequence;
        }

        if crc32fast::hash(&samples) as i64 != checksum {
            report.checksum_failures += 1;
            continue;
        }
        if check_pattern
            && !pattern_matches(
                &samples,
                layout,
                channel,
                start_frame as u64,
                frame_count as u64,
                channels,
                bps,
            )
        {
            report.pattern_failures += 1;
            continue;
        }
        report.last_good_sequence = sequence;
    }

    report.duration_secs = report.frames as f64 / rate as f64;
    let _ = db::format_tag(bps);
    Ok(report)
}

fn pattern_matches(
    samples: &[u8],
    layout: Layout,
    channel: i64,
    start_frame: u64,
    frames: u64,
    channels: u16,
    bps: usize,
) -> bool {
    let mut buf = [0u8; 4];
    match layout {
        Layout::Interleaved => {
            if samples.len() != frames as usize * channels as usize * bps {
                return false;
            }
            let mut o = 0;
            for f in 0..frames {
                for ch in 0..channels {
                    let v = crate::config::Params::expected_sample(start_frame + f, ch);
                    crate::config::Params::write_sample(&mut buf, v, bps);
                    if samples[o..o + bps] != buf[..bps] {
                        return false;
                    }
                    o += bps;
                }
            }
            true
        }
        Layout::PerChannel => {
            if samples.len() != frames as usize * bps {
                return false;
            }
            let ch = channel.max(0) as u16;
            let mut o = 0;
            for f in 0..frames {
                let v = crate::config::Params::expected_sample(start_frame + f, ch);
                crate::config::Params::write_sample(&mut buf, v, bps);
                if samples[o..o + bps] != buf[..bps] {
                    return false;
                }
                o += bps;
            }
            true
        }
    }
}
