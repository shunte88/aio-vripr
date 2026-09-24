//! Schema and connection setup.
//!
//! `sampleblocks` deliberately mirrors Audacity's AUP4 table column for column
//! (D1: our native format is an AUP4 superset). `capture_blocks` is the superset
//! half - the provenance Audacity has nowhere to put.

use anyhow::Result;
use rusqlite::Connection;

use crate::config::{Journal, Params, Sync};

pub const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sampleblocks (
    blockid      INTEGER PRIMARY KEY AUTOINCREMENT,
    sampleformat INTEGER NOT NULL,
    summin       REAL,
    summax       REAL,
    sumrms       REAL,
    summary256   BLOB,
    summary64k   BLOB,
    samples      BLOB NOT NULL
);

CREATE TABLE IF NOT EXISTS capture_blocks (
    blockid      INTEGER PRIMARY KEY REFERENCES sampleblocks(blockid),
    capture_id   INTEGER NOT NULL,
    channel      INTEGER NOT NULL,
    sequence     INTEGER NOT NULL,
    start_frame  INTEGER NOT NULL,
    frame_count  INTEGER NOT NULL,
    sample_rate  INTEGER NOT NULL,
    checksum     INTEGER NOT NULL,
    committed_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_capture_seq
    ON capture_blocks(capture_id, sequence, channel);

CREATE TABLE IF NOT EXISTS captures (
    capture_id       INTEGER PRIMARY KEY,
    sample_rate      INTEGER NOT NULL,
    channels         INTEGER NOT NULL,
    bytes_per_sample INTEGER NOT NULL,
    layout           TEXT    NOT NULL,
    started_at       INTEGER NOT NULL,
    finished_at      INTEGER
);
"#;

/// Page size must be set before any table exists, so this runs on a fresh file.
pub fn open_writer(path: &str, p: &Params) -> Result<Connection> {
    let conn = Connection::open(path)?;
    conn.pragma_update(None, "page_size", p.page_size)?;
    let mode: String = conn.query_row(
        &format!("PRAGMA journal_mode={}", p.journal.as_pragma()),
        [],
        |r| r.get(0),
    )?;
    conn.pragma_update(None, "synchronous", p.sync.as_pragma())?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    conn.pragma_update(None, "cache_size", -32_000i64)?;
    if p.journal == Journal::Wal {
        match p.checkpoint {
            crate::config::Checkpoint::Auto => {}
            _ => {
                conn.pragma_update(None, "wal_autocheckpoint", 0i64)?;
            }
        }
    }
    conn.execute_batch(SCHEMA)?;
    if mode.to_uppercase() != p.journal.as_pragma() {
        eprintln!(
            "warning: requested journal_mode={} but SQLite reports {mode}",
            p.journal.as_pragma()
        );
    }
    Ok(conn)
}

pub fn open_reader(path: &str) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.pragma_update(None, "cache_size", -8_000i64)?;
    Ok(conn)
}

/// Sample format tag, mirroring Audacity's enum space loosely: we record the
/// byte width so a reader can reconstruct without guessing.
pub fn format_tag(bytes_per_sample: usize) -> i64 {
    match bytes_per_sample {
        2 => 16,
        3 => 24,
        4 => 32,
        n => (n * 8) as i64,
    }
}

pub fn sync_setting(s: Sync) -> &'static str {
    s.as_pragma()
}
