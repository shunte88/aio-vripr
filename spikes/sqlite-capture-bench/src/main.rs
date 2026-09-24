//! S2 - SQLite capture benchmark (REQUIREMENTS.md §48).
//!
//! Proves, or refuses to prove, the load-bearing claim of the architecture:
//!
//!     synthetic RT source -> bounded lock-free ring -> batched SQLite BLOB writes
//!                                                   -> simultaneous analysis reads
//!
//! Capture must never wait for the database. If the ring fills, frames are
//! counted as lost and the run fails - exactly as a real overrun would.

use capture_core::{config, db, metrics, producer, readers, verify, writer};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use clap::{Parser, Subcommand};
use serde::Serialize;

use config::{Layout, Params, Sync as SyncMode};
use metrics::{LatencySummary, human_bytes};

#[derive(Parser)]
#[command(
    name = "sqlite-capture-bench",
    about = "S2 spike: sustained capture into SQLite under concurrent analysis reads"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run a single capture and report.
    Run {
        #[arg(long, default_value = "/tmp/vcw-bench.vcw")]
        db: String,
        #[arg(long)]
        json: bool,
        #[arg(long)]
        keep: bool,
        #[command(flatten)]
        params: Params,
    },
    /// Sweep the §48 parameter matrix and emit one JSON object per run.
    Sweep {
        #[arg(long, default_value = "/tmp/vcw-sweep")]
        dir: String,
        /// Seconds per combination.
        #[arg(long, default_value_t = 20)]
        duration: u64,
        #[arg(long, value_delimiter = ',', default_values_t = [250u32, 500, 1000, 2000, 4000])]
        block_ms: Vec<u32>,
        #[arg(long, value_delimiter = ',', default_values_t = [1usize, 4, 16])]
        batch: Vec<usize>,
        #[arg(long, value_delimiter = ',', default_values_t = [4096u32, 16384, 32768])]
        page_size: Vec<u32>,
        #[arg(long, value_delimiter = ',')]
        layouts: Vec<Layout>,
        #[command(flatten)]
        params: Params,
    },
    /// Validate a project: integrity, checksums, deterministic pattern, sequence gaps.
    Verify {
        db: String,
        #[arg(long)]
        json: bool,
    },
    /// Run a capture and SIGKILL it mid-flight, then verify what survived.
    CrashTest {
        #[arg(long, default_value = "/tmp/vcw-crash.vcw")]
        db: String,
        /// Seconds to run before the kill.
        #[arg(long, default_value_t = 15)]
        kill_after: u64,
        /// Number of kill/verify cycles.
        #[arg(long, default_value_t = 3)]
        cycles: u32,
        #[command(flatten)]
        params: Params,
    },
}

#[derive(Serialize)]
struct RunReport {
    rate: u32,
    channels: u16,
    bytes_per_sample: usize,
    block_ms: u32,
    batch_blocks: usize,
    layout: String,
    journal: String,
    sync: String,
    checkpoint: String,
    summaries: String,
    page_size: u32,
    readers: usize,
    rate_multiplier: f64,

    wall_secs: f64,
    frames_produced: u64,
    frames_dropped: u64,
    drop_ppm: f64,
    overrun_events: u64,
    worst_producer_late_us: u64,

    blocks: u64,
    rows: u64,
    audio_bytes: u64,
    db_bytes: u64,
    peak_wal_bytes: u64,
    write_amplification: f64,
    throughput_mib_s: f64,
    realtime_factor: f64,

    commit: LatencySummary,
    checkpoint_stall: LatencySummary,
    summary_build: LatencySummary,
    block_budget_us: u64,
    commit_p99_over_budget: bool,

    reader_detail: Vec<String>,
    reader_queries: u64,
    reader_blocks: u64,
    reader_checksum_failures: u64,
    reader_latency: LatencySummary,

    rss_bytes: u64,
    verdict: &'static str,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Run {
            db,
            json,
            keep,
            params,
        } => {
            let report = single_run(&db, &params)?;
            emit(&report, json);
            if !keep {
                cleanup(&db);
            }
            if report.verdict != "PASS" {
                std::process::exit(2);
            }
        }
        Cmd::Sweep {
            dir,
            duration,
            block_ms,
            batch,
            page_size,
            layouts,
            mut params,
        } => {
            std::fs::create_dir_all(&dir)?;
            params.duration = duration;
            let layouts = if layouts.is_empty() {
                vec![Layout::Interleaved, Layout::PerChannel]
            } else {
                layouts
            };
            let total = block_ms.len() * batch.len() * page_size.len() * layouts.len();
            eprintln!("sweep: {total} combinations x {duration}s\n");
            let mut n = 0;
            for &bm in &block_ms {
                for &ba in &batch {
                    for &ps in &page_size {
                        for &ly in &layouts {
                            n += 1;
                            let mut p = params.clone();
                            p.block_ms = bm;
                            p.batch_blocks = ba;
                            p.page_size = ps;
                            p.layout = ly;
                            let path = format!("{dir}/run-{n:03}.vcw");
                            cleanup(&path);
                            eprintln!(
                                "[{n}/{total}] block={bm}ms batch={ba} page={ps} layout={ly:?}"
                            );
                            match single_run(&path, &p) {
                                Ok(r) => {
                                    println!("{}", serde_json::to_string(&r)?);
                                    eprintln!(
                                        "        {} drop={} commit_p99={}us wal_peak={}\n",
                                        r.verdict,
                                        r.frames_dropped,
                                        r.commit.p99_us,
                                        human_bytes(r.peak_wal_bytes)
                                    );
                                }
                                Err(e) => eprintln!("        ERROR: {e}\n"),
                            }
                            cleanup(&path);
                        }
                    }
                }
            }
        }
        Cmd::Verify { db, json } => {
            let r = verify::verify(&db)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&r)?);
            } else {
                print_verify(&r);
            }
            if !r.integrity_ok || r.checksum_failures > 0 || r.pattern_failures > 0 {
                std::process::exit(2);
            }
        }
        Cmd::CrashTest {
            db,
            kill_after,
            cycles,
            params,
        } => {
            crash_test(&db, kill_after, cycles, &params)?;
        }
    }
    Ok(())
}

fn single_run(db_path: &str, p: &Params) -> Result<RunReport> {
    if p.bytes_per_sample == 0 || p.bytes_per_sample > 4 {
        bail!("bytes_per_sample must be 1..=4");
    }
    cleanup(db_path);

    let ring_bytes = p
        .ring_bytes()
        .max(p.frame_bytes() * p.callback_frames as usize * 4);
    let (prod, cons) = rtrb::RingBuffer::<u8>::new(ring_bytes);

    let stop = Arc::new(AtomicBool::new(false));
    let blocks_committed = Arc::new(AtomicU64::new(0));
    let pstats = producer::ProducerStats::new();

    let started = Instant::now();

    let writer_handle = {
        let p = p.clone();
        let path = db_path.to_string();
        let stop = stop.clone();
        let bc = blocks_committed.clone();
        std::thread::Builder::new()
            .name("capture-writer".into())
            .spawn(move || writer::run(p, path, cons, stop, bc))?
    };

    // Give the writer a moment to create the schema before readers attach.
    std::thread::sleep(Duration::from_millis(150));

    let mut reader_handles = Vec::new();
    for i in 0..p.readers {
        let kind = if i % 2 == 0 {
            "waveform"
        } else {
            "fingerprint"
        };
        let path = db_path.to_string();
        let stop = stop.clone();
        let bc = blocks_committed.clone();
        reader_handles.push(
            std::thread::Builder::new()
                .name(format!("reader-{kind}-{i}"))
                .spawn(move || readers::run(kind, path, stop, bc))?,
        );
    }

    let producer_handle = {
        let p = p.clone();
        let stop = stop.clone();
        let stats = pstats.clone();
        std::thread::Builder::new()
            .name("rt-source".into())
            .spawn(move || producer::run(p, prod, stats, stop))?
    };

    std::thread::sleep(Duration::from_secs(p.duration));
    stop.store(true, Ordering::Relaxed);

    let _ = producer_handle.join();
    let mut wout = match writer_handle.join() {
        Ok(r) => r?,
        Err(_) => bail!("writer thread panicked"),
    };
    let mut router = Vec::new();
    for h in reader_handles {
        if let Ok(r) = h.join() {
            router.push(r);
        }
    }

    let wall = started.elapsed().as_secs_f64();
    let rss = metrics::rss_bytes();

    let frames_produced = pstats.frames_produced.load(Ordering::Relaxed);
    let frames_dropped = pstats.frames_dropped.load(Ordering::Relaxed);
    let db_bytes = metrics::file_len(std::path::Path::new(db_path));

    let commit = wout.commit_latency.summary();
    let checkpoint_stall = wout.checkpoint_latency.summary();
    let summary_build = wout.summary_latency.summary();
    let block_budget_us = p.block_ms as u64 * 1000 * p.batch_blocks as u64;

    let reader_detail: Vec<String> = router
        .iter()
        .map(|r| {
            format!(
                "{}: {} queries, {} blocks",
                r.kind, r.queries, r.blocks_read
            )
        })
        .collect();
    let reader_queries: u64 = router.iter().map(|r| r.queries).sum();
    let reader_blocks: u64 = router.iter().map(|r| r.blocks_read).sum();
    let reader_failures: u64 = router.iter().map(|r| r.checksum_failures).sum();
    let mut reader_lat = metrics::Latencies::default();
    for r in &mut router {
        let s = r.latency.summary();
        if s.count > 0 {
            reader_lat.record(s.p99_us);
        }
    }

    let commit_over = commit.p99_us > block_budget_us;
    let verdict = if frames_dropped == 0 && reader_failures == 0 && !commit_over {
        "PASS"
    } else {
        "FAIL"
    };

    Ok(RunReport {
        rate: p.rate,
        channels: p.channels,
        bytes_per_sample: p.bytes_per_sample,
        block_ms: p.block_ms,
        batch_blocks: p.batch_blocks,
        layout: format!("{:?}", p.layout),
        journal: format!("{:?}", p.journal),
        sync: db::sync_setting(p.sync).to_string(),
        checkpoint: format!("{:?}", p.checkpoint),
        summaries: format!("{:?}", p.summaries),
        page_size: p.page_size,
        readers: p.readers,
        rate_multiplier: p.rate_multiplier,

        wall_secs: wall,
        frames_produced,
        frames_dropped,
        drop_ppm: if frames_produced + frames_dropped > 0 {
            frames_dropped as f64 * 1e6 / (frames_produced + frames_dropped) as f64
        } else {
            0.0
        },
        overrun_events: pstats.overrun_events.load(Ordering::Relaxed),
        worst_producer_late_us: pstats.worst_late_us.load(Ordering::Relaxed),

        blocks: wout.blocks,
        rows: wout.rows,
        audio_bytes: wout.bytes,
        db_bytes,
        peak_wal_bytes: wout.peak_wal_bytes,
        write_amplification: if wout.bytes > 0 {
            db_bytes as f64 / wout.bytes as f64
        } else {
            0.0
        },
        throughput_mib_s: wout.bytes as f64 / wall / (1024.0 * 1024.0),
        realtime_factor: (wout.bytes as f64 / wall) / p.byte_rate(),

        commit,
        checkpoint_stall,
        summary_build,
        block_budget_us,
        commit_p99_over_budget: commit_over,

        reader_detail,
        reader_queries,
        reader_blocks,
        reader_checksum_failures: reader_failures,
        reader_latency: reader_lat.summary(),

        rss_bytes: rss,
        verdict,
    })
}

fn crash_test(db: &str, kill_after: u64, cycles: u32, p: &Params) -> Result<()> {
    let exe = std::env::current_exe()?;
    let mut all_ok = true;

    for c in 1..=cycles {
        cleanup(db);
        println!("── cycle {c}/{cycles}: capturing, SIGKILL after {kill_after}s ──");
        let mut child = std::process::Command::new(&exe)
            .arg("run")
            .arg("--db")
            .arg(db)
            .arg("--keep")
            .arg("--json")
            .arg("--rate")
            .arg(p.rate.to_string())
            .arg("--channels")
            .arg(p.channels.to_string())
            .arg("--bytes-per-sample")
            .arg(p.bytes_per_sample.to_string())
            .arg("--block-ms")
            .arg(p.block_ms.to_string())
            .arg("--batch-blocks")
            .arg(p.batch_blocks.to_string())
            .arg("--layout")
            .arg(match p.layout {
                Layout::Interleaved => "interleaved",
                Layout::PerChannel => "per-channel",
            })
            .arg("--readers")
            .arg(p.readers.to_string())
            .arg("--duration")
            .arg((kill_after + 600).to_string())
            .stdout(std::process::Stdio::null())
            .spawn()?;

        std::thread::sleep(Duration::from_secs(kill_after));
        child.kill()?;
        let status = child.wait()?;
        println!("   killed ({status})");

        let r = verify::verify(db)?;
        print_verify(&r);

        let clean = r.integrity_ok
            && r.checksum_failures == 0
            && r.pattern_failures == 0
            && r.sequence_gaps == 0;
        println!(
            "   → {}\n",
            if clean {
                format!("RECOVERED {:.1}s of audio, intact", r.duration_secs)
            } else {
                "DAMAGE DETECTED".to_string()
            }
        );
        all_ok &= clean;
    }

    cleanup(db);
    println!(
        "{}",
        if all_ok {
            "crash-test: PASS"
        } else {
            "crash-test: FAIL"
        }
    );
    if !all_ok {
        std::process::exit(2);
    }
    Ok(())
}

fn print_verify(r: &verify::VerifyReport) {
    println!(
        "   integrity_check   : {}",
        if r.integrity_ok {
            "ok".into()
        } else {
            r.integrity_detail.clone()
        }
    );
    println!("   blocks / rows     : {} / {}", r.blocks, r.rows);
    println!(
        "   audio recovered   : {:.2}s ({} frames)",
        r.duration_secs, r.frames
    );
    println!("   checksum failures : {}", r.checksum_failures);
    println!("   pattern failures  : {}", r.pattern_failures);
    println!("   sequence gaps     : {}", r.sequence_gaps);
    println!("   last good sequence: {}", r.last_good_sequence);
    println!("   database size     : {}", human_bytes(r.db_bytes));
}

fn emit(r: &RunReport, json: bool) {
    if json {
        println!("{}", serde_json::to_string_pretty(r).unwrap());
        return;
    }
    println!("\n╭─ capture ──────────────────────────────────────────────");
    println!(
        "│ {} Hz · {} ch · {}-bit container · {:?} layout",
        r.rate,
        r.channels,
        r.bytes_per_sample * 8,
        r.layout
    );
    println!(
        "│ block {}ms · batch {} · page {}B · {} journal · sync {} · ckpt {}",
        r.block_ms, r.batch_blocks, r.page_size, r.journal, r.sync, r.checkpoint
    );
    println!(
        "│ summaries {} · {} reader(s) · rate x{}",
        r.summaries, r.readers, r.rate_multiplier
    );
    println!("├─ result ───────────────────────────────────────────────");
    println!("│ wall               {:.1}s", r.wall_secs);
    println!("│ frames produced    {}", r.frames_produced);
    println!(
        "│ frames DROPPED     {}  ({:.1} ppm, {} overrun events)",
        r.frames_dropped, r.drop_ppm, r.overrun_events
    );
    println!("│ blocks / rows      {} / {}", r.blocks, r.rows);
    println!(
        "│ audio written      {}  ({:.1} MiB/s, {:.2}x realtime)",
        human_bytes(r.audio_bytes),
        r.throughput_mib_s,
        r.realtime_factor
    );
    println!(
        "│ database           {}  (write amp {:.2}x)",
        human_bytes(r.db_bytes),
        r.write_amplification
    );
    println!("│ peak WAL           {}", human_bytes(r.peak_wal_bytes));
    println!("│ RSS                {}", human_bytes(r.rss_bytes));
    println!("├─ latency (µs) ─────────────────────────────────────────");
    println!(
        "│ commit   p50 {:>7}  p95 {:>7}  p99 {:>7}  max {:>8}  (budget {})",
        r.commit.p50_us, r.commit.p95_us, r.commit.p99_us, r.commit.max_us, r.block_budget_us
    );
    println!(
        "│ summary  p50 {:>7}  p95 {:>7}  p99 {:>7}  max {:>8}",
        r.summary_build.p50_us,
        r.summary_build.p95_us,
        r.summary_build.p99_us,
        r.summary_build.max_us
    );
    println!(
        "│ ckpt     p50 {:>7}  p95 {:>7}  p99 {:>7}  max {:>8}  (n={})",
        r.checkpoint_stall.p50_us,
        r.checkpoint_stall.p95_us,
        r.checkpoint_stall.p99_us,
        r.checkpoint_stall.max_us,
        r.checkpoint_stall.count
    );
    println!("├─ concurrent readers ───────────────────────────────────");
    for d in &r.reader_detail {
        println!("│ {d}");
    }
    println!(
        "│ total {} queries · {} blocks · {} checksum failures",
        r.reader_queries, r.reader_blocks, r.reader_checksum_failures
    );
    println!(
        "╰─ VERDICT: {} ─────────────────────────────────────────\n",
        r.verdict
    );
}

fn cleanup(db: &str) {
    for suffix in ["", "-wal", "-shm", "-journal"] {
        let _ = std::fs::remove_file(format!("{db}{suffix}"));
    }
}

#[allow(dead_code)]
fn unused(_: SyncMode) {}
