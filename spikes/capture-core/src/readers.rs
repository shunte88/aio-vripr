//! Analysis readers running against the same database while capture continues —
//! the contention case §48 asks about. One imitates the waveform worker (reads
//! summaries), one the fingerprint worker (reads whole blocks and checksums them).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::db;
use crate::metrics::Latencies;

pub struct ReaderOutcome {
    pub kind: &'static str,
    pub queries: u64,
    pub blocks_read: u64,
    pub checksum_failures: u64,
    pub latency: Latencies,
}

pub fn run(
    kind: &'static str,
    db_path: String,
    stop: Arc<AtomicBool>,
    blocks_committed: Arc<AtomicU64>,
) -> ReaderOutcome {
    let mut out = ReaderOutcome {
        kind,
        queries: 0,
        blocks_read: 0,
        checksum_failures: 0,
        latency: Latencies::default(),
    };

    let conn = match db::open_reader(&db_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("reader {kind}: cannot open: {e}");
            return out;
        }
    };

    while !stop.load(Ordering::Relaxed) {
        if blocks_committed.load(Ordering::Relaxed) == 0 {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }
        let t = Instant::now();
        let result = if kind == "waveform" {
            read_summaries(&conn)
        } else {
            read_and_verify(&conn)
        };
        out.latency.record(t.elapsed().as_micros() as u64);
        out.queries += 1;
        match result {
            Ok((n, bad)) => {
                out.blocks_read += n;
                out.checksum_failures += bad;
            }
            Err(e) => {
                // Contention shows up here; a busy database is a finding, not a panic.
                eprintln!("reader {kind}: {e}");
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    out
}

fn read_summaries(conn: &rusqlite::Connection) -> anyhow::Result<(u64, u64)> {
    let mut stmt = conn.prepare_cached(
        "SELECT summin, summax, sumrms, length(summary256)
           FROM sampleblocks ORDER BY blockid DESC LIMIT 32",
    )?;
    let mut rows = stmt.query([])?;
    let mut n = 0;
    while let Some(r) = rows.next()? {
        let _: f64 = r.get(0).unwrap_or(0.0);
        let _: f64 = r.get(1).unwrap_or(0.0);
        let _: f64 = r.get(2).unwrap_or(0.0);
        let _: i64 = r.get(3).unwrap_or(0);
        n += 1;
    }
    Ok((n, 0))
}

fn read_and_verify(conn: &rusqlite::Connection) -> anyhow::Result<(u64, u64)> {
    let mut stmt = conn.prepare_cached(
        "SELECT s.samples, c.checksum
           FROM sampleblocks s JOIN capture_blocks c ON c.blockid = s.blockid
          ORDER BY s.blockid DESC LIMIT 8",
    )?;
    let mut rows = stmt.query([])?;
    let (mut n, mut bad) = (0u64, 0u64);
    while let Some(r) = rows.next()? {
        let samples: Vec<u8> = r.get(0)?;
        let expected: i64 = r.get(1)?;
        if crc32fast::hash(&samples) as i64 != expected {
            bad += 1;
        }
        n += 1;
    }
    Ok((n, bad))
}
