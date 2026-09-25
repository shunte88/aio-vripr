//! Structural validation and integrity checking (§15, §16).
//!
//! Two different questions, deliberately separated. [`integrity_check`] asks
//! SQLite whether the *file* is sound - pages, indices, foreign keys. [`validate`]
//! asks whether the *project* is sound: whether blocks are where the timeline says
//! they are, whether declared frame counts match the bytes actually stored, and
//! whether a capture's rows agree with each other.
//!
//! Both collect every problem rather than stopping at the first. A recovery tool
//! that reports one fault at a time turns a diagnosis into a guessing game.

use rusqlite::Connection;
use vcw_types::StorageFormat;

use crate::error::Result;
use crate::schema::REQUIRED_TABLES;
use crate::sqlite::{Project, block_checksum};

/// One problem found in a project.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Stable machine-readable identifier, for tests and for the UI.
    pub code: &'static str,
    /// Human-readable detail, naming the rows involved.
    pub detail: String,
}

/// What a validation run found.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Problems, in the order the checks ran.
    pub findings: Vec<Finding>,
    /// Captures examined.
    pub captures: u64,
    /// Blocks examined.
    pub blocks: u64,
    /// Whether sample checksums were recomputed.
    pub checksums_verified: bool,
}

impl Report {
    /// Whether the project is sound.
    pub fn is_clean(&self) -> bool {
        self.findings.is_empty()
    }

    /// Whether any finding carries this code.
    pub fn has(&self, code: &str) -> bool {
        self.findings.iter().any(|f| f.code == code)
    }
}

/// How thorough a validation run should be.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    /// Recompute every block's CRC-32 and compare it with the stored value.
    ///
    /// Off by default because it reads every sample byte in the project - minutes
    /// for a full-length rip. Worth it when recovering, wasteful on every open.
    pub verify_checksums: bool,
}

/// Runs the structural checks.
pub fn validate(project: &Project, options: Options) -> Result<Report> {
    let conn = project.conn();
    let mut report = Report {
        checksums_verified: options.verify_checksums,
        ..Report::default()
    };

    check_tables(conn, &mut report)?;
    if !report.findings.is_empty() {
        // Every later check reads those tables. Reporting "no such table" nine
        // more times helps nobody.
        return Ok(report);
    }

    check_meta(conn, &mut report)?;
    check_dangling_blocks(conn, &mut report)?;
    check_orphan_blocks(conn, &mut report)?;
    check_block_sizes(conn, &mut report)?;
    check_timeline(conn, &mut report)?;
    check_captures(conn, &mut report)?;
    if options.verify_checksums {
        check_checksums(conn, &mut report)?;
    }

    report.captures = count(conn, "captures")?;
    report.blocks = count(conn, "capture_blocks")?;
    Ok(report)
}

/// Asks SQLite whether the file itself is sound.
pub fn integrity_check(project: &Project) -> Result<Report> {
    let conn = project.conn();
    let mut report = Report::default();

    let mut stmt = conn.prepare("PRAGMA integrity_check")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for row in rows {
        let row = row?;
        if row != "ok" {
            report.findings.push(Finding {
                code: "sqlite-integrity",
                detail: row,
            });
        }
    }

    let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, Option<i64>>(1)?,
            r.get::<_, String>(2)?,
        ))
    })?;
    for row in rows {
        let (table, rowid, parent) = row?;
        report.findings.push(Finding {
            code: "foreign-key",
            detail: format!("{table} rowid {rowid:?} has no parent in {parent}"),
        });
    }

    Ok(report)
}

fn count(conn: &Connection, table: &str) -> rusqlite::Result<u64> {
    let n: i64 = conn.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?;
    Ok(n as u64)
}

fn check_tables(conn: &Connection, report: &mut Report) -> rusqlite::Result<()> {
    let mut stmt =
        conn.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
    let present = stmt
        .query_map([], |r| r.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for required in REQUIRED_TABLES {
        if !present.iter().any(|p| p == required) {
            report.findings.push(Finding {
                code: "missing-table",
                detail: format!("table {required} is absent"),
            });
        }
    }
    Ok(())
}

fn check_meta(conn: &Connection, report: &mut Report) -> rusqlite::Result<()> {
    for key in crate::meta::REQUIRED_KEYS {
        if crate::meta::get(conn, key)?.is_none() {
            report.findings.push(Finding {
                code: "missing-meta",
                detail: format!("§16 requires meta key {key}, which is not set"),
            });
        }
    }
    Ok(())
}

/// A `capture_blocks` row pointing at a `sampleblocks` row that is not there.
///
/// The foreign key should make this impossible, but `PRAGMA foreign_keys` is
/// per-connection and defaults off, so a project touched by another tool can carry
/// one. S5 found the equivalent in Audacity projects and it is the single most
/// damaging shape of corruption: the timeline claims audio that does not exist.
fn check_dangling_blocks(conn: &Connection, report: &mut Report) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT cb.blockid FROM capture_blocks cb
         LEFT JOIN sampleblocks sb ON sb.blockid = cb.blockid
         WHERE sb.blockid IS NULL ORDER BY cb.blockid",
    )?;
    for id in stmt.query_map([], |r| r.get::<_, i64>(0))? {
        report.findings.push(Finding {
            code: "dangling-block",
            detail: format!("capture_blocks references blockid {} with no samples", id?),
        });
    }
    Ok(())
}

/// Samples nothing refers to. Wasted space rather than lost audio, but in schema
/// v1 `capture_blocks` is the only referrer, so an orphan means a write that was
/// not finished.
fn check_orphan_blocks(conn: &Connection, report: &mut Report) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT sb.blockid FROM sampleblocks sb
         LEFT JOIN capture_blocks cb ON cb.blockid = sb.blockid
         WHERE cb.blockid IS NULL ORDER BY sb.blockid",
    )?;
    for id in stmt.query_map([], |r| r.get::<_, i64>(0))? {
        report.findings.push(Finding {
            code: "orphan-block",
            detail: format!("sampleblocks {} is referenced by nothing", id?),
        });
    }
    Ok(())
}

/// `frame_count` against the bytes actually stored, and the format code against
/// the ones we know.
///
/// The AUP4 equivalent - `waveblock/@length` versus `length(samples)` - matched in
/// all 5,664 cases S5 checked, which is what makes it a good invariant: cheap,
/// and it has never been wrong when the file was sound.
fn check_block_sizes(conn: &Connection, report: &mut Report) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT cb.blockid, cb.frame_count, length(sb.samples), sb.sampleformat
         FROM capture_blocks cb JOIN sampleblocks sb ON sb.blockid = cb.blockid
         ORDER BY cb.blockid",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, Option<i64>>(2)?,
            r.get::<_, Option<i64>>(3)?,
        ))
    })?;

    for row in rows {
        let (blockid, frames, bytes, format) = row?;
        let Some(format) = format else {
            report.findings.push(Finding {
                code: "missing-format",
                detail: format!("block {blockid} has no sampleformat"),
            });
            continue;
        };
        let Some(storage) = StorageFormat::from_code(format as u32) else {
            report.findings.push(Finding {
                code: "unknown-format",
                detail: format!("block {blockid} has sampleformat 0x{format:08X}, which is not a format we write or import"),
            });
            continue;
        };
        let expected = frames * storage.bytes_per_sample() as i64;
        match bytes {
            None => report.findings.push(Finding {
                code: "missing-samples",
                detail: format!("block {blockid} declares {frames} frames and stores no samples"),
            }),
            Some(actual) if actual != expected => report.findings.push(Finding {
                code: "size-mismatch",
                detail: format!(
                    "block {blockid} declares {frames} frames of {storage:?} ({expected} bytes) \
                     but stores {actual}"
                ),
            }),
            Some(_) => {}
        }
    }
    Ok(())
}

/// Blocks must tile each channel's timeline without gap or overlap.
///
/// Sequence numbers alone do not prove that: what matters is that each block
/// starts where the previous one ended. Recovery depends on it, because the only
/// thing a reconstructed session has to go on is these rows.
fn check_timeline(conn: &Connection, report: &mut Report) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT capture_id, channel, sequence, start_frame, frame_count
         FROM capture_blocks ORDER BY capture_id, channel, sequence",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, i64>(4)?,
        ))
    })?;

    let mut previous: Option<(i64, i64, i64)> = None; // capture, channel, next expected frame
    for row in rows {
        let (capture, channel, sequence, start, frames) = row?;
        let expected = match previous {
            Some((c, ch, next)) if c == capture && ch == channel => next,
            _ => 0,
        };
        if start != expected {
            report.findings.push(Finding {
                code: "timeline-gap",
                detail: format!(
                    "capture {capture} channel {channel} sequence {sequence} starts at frame \
                     {start}, expected {expected}"
                ),
            });
        }
        if frames <= 0 {
            report.findings.push(Finding {
                code: "empty-block",
                detail: format!(
                    "capture {capture} channel {channel} sequence {sequence} declares {frames} frames"
                ),
            });
        }
        previous = Some((capture, channel, start + frames.max(0)));
    }
    Ok(())
}

fn check_captures(conn: &Connection, report: &mut Report) -> rusqlite::Result<()> {
    const STATES: [&str; 3] = ["recording", "finalised", "interrupted"];

    let mut stmt = conn.prepare(
        "SELECT capture_id, sample_rate, channels, storage_format, state, started_at, finished_at
         FROM captures ORDER BY capture_id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, i64>(2)?,
            r.get::<_, i64>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, i64>(5)?,
            r.get::<_, Option<i64>>(6)?,
        ))
    })?;

    for row in rows {
        let (id, rate, channels, format, state, started, finished) = row?;
        if rate <= 0 {
            report.findings.push(Finding {
                code: "bad-rate",
                detail: format!("capture {id} has sample_rate {rate}"),
            });
        }
        if channels <= 0 {
            report.findings.push(Finding {
                code: "bad-channels",
                detail: format!("capture {id} has {channels} channels"),
            });
        }
        if StorageFormat::from_code(format as u32).is_none() {
            report.findings.push(Finding {
                code: "unknown-format",
                detail: format!("capture {id} has storage_format 0x{format:08X}"),
            });
        }
        if !STATES.contains(&state.as_str()) {
            report.findings.push(Finding {
                code: "unknown-state",
                detail: format!("capture {id} is in state {state:?}"),
            });
        }
        if let Some(finished) = finished
            && finished < started
        {
            report.findings.push(Finding {
                code: "time-travel",
                detail: format!(
                    "capture {id} finished at {finished}, before it started at {started}"
                ),
            });
        }
    }
    Ok(())
}

fn check_checksums(conn: &Connection, report: &mut Report) -> rusqlite::Result<()> {
    let mut stmt = conn.prepare(
        "SELECT cb.blockid, cb.checksum, sb.samples
         FROM capture_blocks cb JOIN sampleblocks sb ON sb.blockid = cb.blockid
         ORDER BY cb.blockid",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, Option<Vec<u8>>>(2)?,
        ))
    })?;

    for row in rows {
        let (blockid, stored, samples) = row?;
        let Some(samples) = samples else { continue }; // already reported as missing-samples
        let actual = block_checksum(&samples);
        if actual as i64 != stored {
            report.findings.push(Finding {
                code: "checksum-mismatch",
                detail: format!(
                    "block {blockid} stores checksum 0x{stored:08X}, samples hash to 0x{actual:08X}"
                ),
            });
        }
    }
    Ok(())
}
