//! S4 — chromaprint-next streaming spike.
//!
//! The plan's acceptance is narrow: feed live-shaped PCM chunks through
//! `Fingerprinter::feed()` and assert the fingerprint equals the offline
//! fingerprint of the same region. That is `chunk`. The rest of the subcommands
//! exist because REQUIREMENTS §25 asks for *progressive regions*, which needs two
//! more things to be true: the fingerprint of a region must not depend on how the
//! chunks were cut (`chunk`), and it must be robust to the detector getting the
//! region boundary slightly wrong (`align`). §46 then wants the fingerprint worker
//! provably unable to starve capture (`throughput`).

mod fp;
mod wav;

use anyhow::{Context, Result, bail};
use chromaprint::Algorithm;
use clap::{Parser, Subcommand};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Default region: two minutes, which is what AcoustID submits and what fpcalc
/// defaults to, starting 60 s in so we are inside the music rather than the lead-in.
const DEFAULT_START_SECS: f64 = 60.0;
const DEFAULT_LEN_SECS: f64 = 120.0;

#[derive(Parser)]
#[command(
    name = "fingerprint-stream",
    about = "S4: chromaprint-next streaming equivalence"
)]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
    /// Emit machine-readable JSON in addition to the human summary.
    #[arg(long, global = true)]
    json: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Print header, duration and the region this spike would fingerprint.
    Info { wav: PathBuf },
    /// T1 — chunk-shape invariance. The plan's acceptance criterion.
    Chunk {
        wav: PathBuf,
        #[arg(long, default_value_t = DEFAULT_START_SECS)]
        start: f64,
        #[arg(long, default_value_t = DEFAULT_LEN_SECS)]
        len: f64,
    },
    /// T2 — region boundary sensitivity: how wrong can the detector's start be?
    Align {
        wav: PathBuf,
        #[arg(long, default_value_t = DEFAULT_START_SECS)]
        start: f64,
        #[arg(long, default_value_t = DEFAULT_LEN_SECS)]
        len: f64,
    },
    /// T4 — throughput and headroom against real-time capture.
    Throughput {
        wav: PathBuf,
        #[arg(long, default_value_t = DEFAULT_START_SECS)]
        start: f64,
        #[arg(long, default_value_t = DEFAULT_LEN_SECS)]
        len: f64,
    },
    /// T5 — cross-check against the C reference (`fpcalc`) on byte-identical input.
    Reference {
        wav: PathBuf,
        #[arg(long, default_value_t = DEFAULT_START_SECS)]
        start: f64,
        #[arg(long, default_value_t = DEFAULT_LEN_SECS)]
        len: f64,
    },
    /// T7 — stream a whole side off disk in capture-sized blocks, holding one
    /// block at a time, the way the real fingerprint worker would.
    Live {
        wav: PathBuf,
        #[arg(long, default_value_t = 0.25)]
        block_secs: f64,
        /// Stop after this many seconds of audio. 0 means the whole file.
        #[arg(long, default_value_t = 0.0)]
        len: f64,
        /// Run this many independent Fingerprinters over the same blocks, the way
        /// several candidate regions would be open at once. All must agree.
        #[arg(long, default_value_t = 1)]
        instances: usize,
    },
    /// T6 — calibrate the BER axis: what do real-world perturbations cost, and
    /// what does unrelated audio score? Every other number is read against this.
    Scale {
        wav: PathBuf,
        /// A second, musically unrelated recording, for the upper baseline.
        other: PathBuf,
        #[arg(long, default_value_t = DEFAULT_START_SECS)]
        start: f64,
        #[arg(long, default_value_t = DEFAULT_LEN_SECS)]
        len: f64,
    },
    /// T3 — does the capture sample rate change the fingerprint? Needs a 192 kHz
    /// source and `sox`.
    Rate {
        wav: PathBuf,
        #[arg(long, default_value_t = DEFAULT_START_SECS)]
        start: f64,
        #[arg(long, default_value_t = DEFAULT_LEN_SECS)]
        len: f64,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match &cli.cmd {
        Cmd::Info { wav } => cmd_info(wav),
        Cmd::Chunk { wav, start, len } => cmd_chunk(wav, *start, *len, cli.json),
        Cmd::Align { wav, start, len } => cmd_align(wav, *start, *len, cli.json),
        Cmd::Throughput { wav, start, len } => cmd_throughput(wav, *start, *len, cli.json),
        Cmd::Reference { wav, start, len } => cmd_reference(wav, *start, *len, cli.json),
        Cmd::Rate { wav, start, len } => cmd_rate(wav, *start, *len, cli.json),
        Cmd::Scale {
            wav,
            other,
            start,
            len,
        } => cmd_scale(wav, other, *start, *len, cli.json),
        Cmd::Live {
            wav,
            block_secs,
            len,
            instances,
        } => cmd_live(wav, *block_secs, *len, *instances, cli.json),
    }
}

// ---------------------------------------------------------------- shared helpers

struct Region {
    samples: Vec<i16>,
    rate: u32,
    channels: u16,
    frames: u64,
}

fn load_region(wav: &Path, start: f64, len: f64) -> Result<Region> {
    let info = wav::probe(wav)?;
    let start_frame = (start * info.sample_rate as f64) as u64;
    let frames = (len * info.sample_rate as f64) as u64;
    if start_frame + frames > info.frames() {
        bail!(
            "{}: region {start}s+{len}s runs past the end ({:.1} s of audio)",
            wav.display(),
            info.duration_secs()
        );
    }
    let samples = wav::read_i16(wav, &info, start_frame, frames)?;
    Ok(Region {
        samples,
        rate: info.sample_rate,
        channels: info.channels,
        frames,
    })
}

fn peak_rss_kib() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmHWM:"))
                .and_then(|l| l.split_whitespace().nth(1)?.parse().ok())
        })
        .unwrap_or(0)
}

fn head(fp: &[u32]) -> String {
    fp.iter()
        .take(4)
        .map(|v| format!("{v:08x}"))
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------- info

fn cmd_info(wav: &Path) -> Result<()> {
    let i = wav::probe(wav)?;
    println!("{}", wav.display());
    println!(
        "  format        {} Hz  {} ch  {}-bit PCM",
        i.sample_rate, i.channels, i.bits
    );
    println!(
        "  data          offset {}  len {} bytes",
        i.data_offset, i.data_len
    );
    println!(
        "  frames        {} ({:.2} s)",
        i.frames(),
        i.duration_secs()
    );
    println!(
        "  i16 downmix   {} bytes for a 120 s stereo region",
        120 * i.sample_rate * 4
    );
    Ok(())
}

// ------------------------------------------------------------------- T1 chunk

#[derive(Serialize)]
struct ChunkRow {
    shape: String,
    chunk_frames: Option<usize>,
    items: usize,
    identical: bool,
    ber: f64,
    elapsed_ms: f64,
}

#[derive(Serialize)]
struct ChunkReport {
    test: &'static str,
    source: String,
    rate: u32,
    channels: u16,
    region_secs: f64,
    offline_items: usize,
    offline_head: String,
    rows: Vec<ChunkRow>,
    pass: bool,
}

fn cmd_chunk(wav: &Path, start: f64, len: f64, json: bool) -> Result<()> {
    let r = load_region(wav, start, len)?;
    let algo = Algorithm::default();

    let t = Instant::now();
    let offline = fp::offline(&r.samples, r.rate, r.channels, algo)?;
    let offline_ms = t.elapsed().as_secs_f64() * 1000.0;

    println!("T1 — chunk-shape invariance");
    println!("  source      {}", wav.display());
    println!(
        "  region      {:.1} s from {:.1} s ({} frames @ {} Hz, {} ch)",
        len, start, r.frames, r.rate, r.channels
    );
    println!(
        "  offline     {} sub-fingerprints in {offline_ms:.0} ms  [{}]",
        offline.len(),
        head(&offline)
    );
    println!();
    println!(
        "  {:<34} {:>7} {:>10} {:>9} {:>10}",
        "chunk shape", "items", "identical", "BER", "elapsed"
    );

    // The shapes that matter: the ALSA period and buffer sizes S1 measured
    // (16384 / 32768 frames), the 250 ms commit block S2 settled on, the
    // internal 32768-sample mono buffer boundary, a pathologically small drain,
    // and a ragged sequence standing in for uneven ring drains.
    let block_250ms = (r.rate as f64 * 0.25) as usize;
    let mut shapes: Vec<(String, Option<usize>)> = vec![
        ("1 frame".into(), Some(1)),
        ("64 frames".into(), Some(64)),
        ("256 frames".into(), Some(256)),
        ("997 frames (prime)".into(), Some(997)),
        ("4096 frames".into(), Some(4096)),
        ("16384 frames (ALSA period)".into(), Some(16384)),
        ("32768 frames (ALSA buffer)".into(), Some(32768)),
        ("32769 frames (buffer + 1)".into(), Some(32769)),
        (
            format!("{block_250ms} frames (250 ms block)"),
            Some(block_250ms),
        ),
        ("ragged 1..8192 frames".into(), None),
    ];
    // Feeding a 120 s region one frame at a time is ~5.7 M calls at 48 kHz. It is
    // the most adversarial shape there is, so it stays, but only for short regions.
    if len > 30.0 {
        shapes.retain(|(s, _)| s != "1 frame");
        println!(
            "  {:<34} {:>7} {:>10} {:>9} {:>10}",
            "1 frame", "-", "skipped", "-", "len > 30 s"
        );
    }

    let mut rows = Vec::new();
    let mut pass = true;
    for (shape, cf) in shapes {
        let t = Instant::now();
        let got = match cf {
            Some(n) => fp::streamed(&r.samples, r.rate, r.channels, algo, std::iter::repeat(n))?,
            None => fp::streamed(
                &r.samples,
                r.rate,
                r.channels,
                algo,
                fp::Ragged::new(0x5EED, 8192),
            )?,
        };
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        let identical = got == offline;
        let b = fp::ber(&offline, &got);
        pass &= identical;
        println!(
            "  {shape:<34} {:>7} {:>10} {:>9.6} {:>8.0} ms",
            got.len(),
            if identical { "yes" } else { "NO" },
            b,
            ms
        );
        rows.push(ChunkRow {
            shape,
            chunk_frames: cf,
            items: got.len(),
            identical,
            ber: b,
            elapsed_ms: ms,
        });
    }

    println!();
    println!(
        "  {}",
        if pass {
            "PASS: every chunk shape reproduces the offline fingerprint exactly"
        } else {
            "FAIL: at least one chunk shape changed the fingerprint"
        }
    );

    if json {
        let rep = ChunkReport {
            test: "T1-chunk-invariance",
            source: wav.display().to_string(),
            rate: r.rate,
            channels: r.channels,
            region_secs: len,
            offline_items: offline.len(),
            offline_head: head(&offline),
            rows,
            pass,
        };
        println!("{}", serde_json::to_string_pretty(&rep)?);
    }
    if !pass {
        bail!("chunk invariance failed");
    }
    Ok(())
}

// ------------------------------------------------------------------- T2 align

#[derive(Serialize)]
struct AlignRow {
    offset_frames: i64,
    offset_ms: f64,
    items: usize,
    ber_unshifted: f64,
    ber_best: f64,
    best_shift: i64,
}

fn cmd_align(wav: &Path, start: f64, len: f64, json: bool) -> Result<()> {
    let info = wav::probe(wav)?;
    let algo = Algorithm::default();
    let rate = info.sample_rate;
    let ch = info.channels;
    let start_frame = (start * rate as f64) as u64;
    let frames = (len * rate as f64) as u64;

    // The reference: the region exactly as the detector would ideally cut it.
    let base_samples = wav::read_i16(wav, &info, start_frame, frames)?;
    let base = fp::offline(&base_samples, rate, ch, algo)?;

    println!("T2 — region boundary sensitivity");
    println!("  source      {}", wav.display());
    println!(
        "  region      {:.1} s from {:.1} s, {} sub-fingerprints",
        len,
        start,
        base.len()
    );
    println!(
        "  resolution  {:.4} s per sub-fingerprint ({:.2} items/s)",
        1.0 / fp::ITEMS_PER_SEC,
        fp::ITEMS_PER_SEC
    );
    println!();
    println!(
        "  {:>14} {:>10} {:>7} {:>13} {:>10} {:>6}",
        "start offset", "= ms", "items", "BER unshifted", "BER best", "shift"
    );

    // Offsets chosen to straddle the sub-fingerprint step: a fraction of one
    // step, exactly one step, and multiples a silence detector could plausibly
    // be out by. Negative offsets included because a detector can fire early.
    let step_frames = (rate as f64 * 1365.0 / 11025.0) as i64; // one sub-fingerprint
    let offsets: Vec<i64> = vec![
        1,
        64,
        step_frames / 8,
        step_frames / 2,
        step_frames,
        step_frames * 8,
        rate as i64,     // 1 s
        rate as i64 * 5, // 5 s
        -step_frames / 2,
        -step_frames,
        -(rate as i64),
    ];

    let mut rows = Vec::new();
    for off in offsets {
        let sf = start_frame as i64 + off;
        if sf < 0 {
            continue;
        }
        let s = wav::read_i16(wav, &info, sf as u64, frames)?;
        let got = fp::offline(&s, rate, ch, algo)?;
        let bu = fp::ber(&base, &got);
        let (bb, shift) = fp::ber_best_shift(&base, &got, 64, base.len() / 2);
        println!(
            "  {off:>14} {:>10.1} {:>7} {bu:>13.6} {bb:>10.6} {shift:>6}",
            off as f64 * 1000.0 / rate as f64,
            got.len()
        );
        rows.push(AlignRow {
            offset_frames: off,
            offset_ms: off as f64 * 1000.0 / rate as f64,
            items: got.len(),
            ber_unshifted: bu,
            ber_best: bb,
            best_shift: shift,
        });
    }

    println!();
    println!("  Read: BER unshifted is what a naive comparison sees. BER best is what");
    println!("  AcoustID sees, because its matcher aligns before scoring. The gap between");
    println!("  them is the part of a boundary error that alignment absorbs.");

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "test": "T2-region-alignment",
                "source": wav.display().to_string(),
                "rate": rate,
                "region_secs": len,
                "base_items": base.len(),
                "sub_fingerprint_secs": 1.0 / fp::ITEMS_PER_SEC,
                "rows": rows,
            }))?
        );
    }
    Ok(())
}

// -------------------------------------------------------------- T4 throughput

fn cmd_throughput(wav: &Path, start: f64, len: f64, json: bool) -> Result<()> {
    let r = load_region(wav, start, len)?;
    let algo = Algorithm::default();
    let audio_secs = r.frames as f64 / r.rate as f64;

    println!("T4 — throughput and real-time headroom");
    println!(
        "  source      {} @ {} Hz {} ch",
        wav.display(),
        r.rate,
        r.channels
    );
    println!(
        "  region      {:.1} s of audio, {} frames",
        audio_secs, r.frames
    );
    println!();
    println!(
        "  {:<30} {:>10} {:>12} {:>10} {:>10}",
        "shape", "wall", "Melem/s", "RT factor", "headroom"
    );

    let block_250ms = (r.rate as f64 * 0.25) as usize;
    let shapes: Vec<(String, Option<usize>)> = vec![
        ("one shot".into(), Some(usize::MAX / 4)),
        (
            format!("{block_250ms} frames (250 ms block)"),
            Some(block_250ms),
        ),
        ("4096 frames".into(), Some(4096)),
        ("ragged 1..8192 frames".into(), None),
    ];

    let mut rows = Vec::new();
    for (shape, cf) in shapes {
        let t = Instant::now();
        let got = match cf {
            Some(n) => fp::streamed(&r.samples, r.rate, r.channels, algo, std::iter::repeat(n))?,
            None => fp::streamed(
                &r.samples,
                r.rate,
                r.channels,
                algo,
                fp::Ragged::new(0x5EED, 8192),
            )?,
        };
        let wall = t.elapsed().as_secs_f64();
        // Melem/s counts input frames, matching how the crate's own benchmarks report.
        let melem = r.frames as f64 / wall / 1e6;
        let rt = audio_secs / wall;
        println!(
            "  {shape:<30} {:>9.3} s {melem:>12.1} {rt:>9.1}x {:>9.2}%",
            wall,
            100.0 / rt
        );
        rows.push(serde_json::json!({
            "shape": shape, "wall_secs": wall, "melem_per_sec": melem,
            "rt_factor": rt, "cpu_pct_of_one_core": 100.0 / rt, "items": got.len(),
        }));
    }

    let rss = peak_rss_kib();
    println!();
    println!(
        "  peak RSS    {:.1} MiB (whole process, region held in memory)",
        rss as f64 / 1024.0
    );
    println!(
        "  region cost {:.1} MiB of i16 for {:.0} s stereo @ {} Hz",
        (r.samples.len() * 2) as f64 / 1048576.0,
        audio_secs,
        r.rate
    );
    println!();
    println!("  Headroom is CPU of one core to fingerprint one stream in real time.");

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "test": "T4-throughput", "source": wav.display().to_string(),
                "rate": r.rate, "channels": r.channels, "audio_secs": audio_secs,
                "peak_rss_kib": rss, "rows": rows,
            }))?
        );
    }
    Ok(())
}

// --------------------------------------------------------------- T5 reference

fn run_fpcalc(raw: &Path, rate: u32, channels: u16, secs: f64) -> Result<Vec<u32>> {
    let out = std::process::Command::new("fpcalc")
        .args(["-format", "s16le", "-rate"])
        .arg(rate.to_string())
        .arg("-channels")
        .arg(channels.to_string())
        .arg("-length")
        .arg(format!("{:.0}", secs.ceil() + 1.0))
        .args(["-raw", "-plain"])
        .arg(raw)
        .output()
        .context("running fpcalc (is chromaprint-tools installed?)")?;
    if !out.status.success() {
        bail!(
            "fpcalc failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let s = String::from_utf8_lossy(&out.stdout);
    let line = s
        .lines()
        .find(|l| l.contains(','))
        .context("no fingerprint in fpcalc output")?;
    line.trim()
        .split(',')
        .map(|v| v.trim().parse::<u32>().map_err(anyhow::Error::from))
        .collect()
}

fn cmd_reference(wav: &Path, start: f64, len: f64, json: bool) -> Result<()> {
    let r = load_region(wav, start, len)?;
    let algo = Algorithm::default();
    let dir = std::env::temp_dir();

    println!("T5 — cross-check against the C reference (fpcalc)");
    println!(
        "  {}",
        String::from_utf8_lossy(
            &std::process::Command::new("fpcalc")
                .arg("-version")
                .output()?
                .stdout
        )
        .trim()
    );
    println!(
        "  source      {} @ {} Hz {} ch, {:.1} s from {:.1} s",
        wav.display(),
        r.rate,
        r.channels,
        len,
        start
    );
    println!();

    let mut rows = Vec::new();

    // Case A: native capture rate. Both implementations resample internally, so a
    // difference here is a resampler difference, not a fingerprint difference.
    // Case B: pre-resampled to 11025 Hz mono by neither implementation's code, so
    // both bypass their resampler entirely and only the FFT/chroma/classifier
    // stages are under test. B is the load-bearing comparison.
    let raw_native = dir.join("vcw_s4_native.raw");
    wav::write_raw(&raw_native, &r.samples)?;
    let mine_native = fp::offline(&r.samples, r.rate, r.channels, algo)?;
    let theirs_native = run_fpcalc(&raw_native, r.rate, r.channels, len)?;

    // Resample to 11025 mono with sox so neither library does it.
    let src = dir.join("vcw_s4_src.wav");
    let dst = dir.join("vcw_s4_11025.wav");
    wav::write_i16(&src, &r.samples, r.rate, r.channels)?;
    let sox = std::process::Command::new("sox")
        .arg(&src)
        .args(["-r", "11025", "-c", "1"])
        .arg(&dst)
        .output();
    let case_b = match sox {
        Ok(o) if o.status.success() => {
            let i = wav::probe(&dst)?;
            let s = wav::read_i16(&dst, &i, 0, i.frames())?;
            let raw_b = dir.join("vcw_s4_11025.raw");
            wav::write_raw(&raw_b, &s)?;
            let mine = fp::offline(&s, 11025, 1, algo)?;
            let theirs = run_fpcalc(&raw_b, 11025, 1, len)?;
            Some((mine, theirs))
        }
        _ => {
            println!("  (sox unavailable — skipping the no-resample comparison)");
            None
        }
    };

    println!(
        "  {:<44} {:>7} {:>7} {:>10} {:>9} {:>7} {:>6}",
        "case", "mine", "fpcalc", "identical", "BER", "items\u{2260}", "bits\u{2260}"
    );
    for (label, mine, theirs) in [
        Some((
            "A: native rate, both libraries resample",
            &mine_native,
            &theirs_native,
        )),
        case_b
            .as_ref()
            .map(|(m, t)| ("B: pre-resampled to 11025 Hz mono, neither does", m, t)),
    ]
    .into_iter()
    .flatten()
    {
        let identical = mine == theirs;
        let b = fp::ber(mine, theirs);
        let n = mine.len().min(theirs.len());
        let diff_idx: Vec<usize> = (0..n).filter(|&i| mine[i] != theirs[i]).collect();
        let diff_bits: u32 = diff_idx
            .iter()
            .map(|&i| (mine[i] ^ theirs[i]).count_ones())
            .sum();
        println!(
            "  {label:<44} {:>7} {:>7} {:>10} {b:>9.6} {:>7} {:>6}",
            mine.len(),
            theirs.len(),
            if identical { "yes" } else { "NO" },
            diff_idx.len(),
            diff_bits
        );
        if !diff_idx.is_empty() {
            let show: Vec<String> = diff_idx
                .iter()
                .take(6)
                .map(|&i| format!("#{i} {:08x}^{:08x}", mine[i], theirs[i]))
                .collect();
            println!("  {:<44} {}", "", show.join("  "));
        }
        rows.push(serde_json::json!({
            "case": label, "mine_items": mine.len(), "fpcalc_items": theirs.len(),
            "identical": identical, "ber": b,
            "items_differing": diff_idx.len(), "bits_differing": diff_bits,
            "first_differing": diff_idx.iter().take(16).collect::<Vec<_>>(),
        }));
    }

    // Case C tests the hypothesis that case A's stray bits are purely the
    // resampler. fpcalc 1.6.0 reports "SwR6.1.100" in its banner, i.e. it is
    // linked against FFmpeg's swresample, whereas chromaprint-next ports
    // FFmpeg's older av_resample. If we let swresample do the downmix and
    // resample and then fingerprint that with chromaprint-next, and the result
    // equals fpcalc's own native-rate answer, the resampler is the whole story.
    let dstc = dir.join("vcw_s4_swr.wav");
    let ff = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(&src)
        .args(["-ac", "1", "-ar", "11025", "-c:a", "pcm_s16le"])
        .arg(&dstc)
        .output();
    match ff {
        Ok(o) if o.status.success() => {
            let i = wav::probe(&dstc)?;
            let s = wav::read_i16(&dstc, &i, 0, i.frames())?;
            let mine = fp::offline(&s, 11025, 1, algo)?;
            let n = mine.len().min(theirs_native.len());
            let de: Vec<usize> = (0..n).filter(|&i| mine[i] != theirs_native[i]).collect();
            let bits: u32 = de
                .iter()
                .map(|&i| (mine[i] ^ theirs_native[i]).count_ones())
                .sum();
            println!(
                "  {:<44} {:>7} {:>7} {:>10} {:>9.6} {:>7} {:>6}",
                "C: swresample does the resample, vs fpcalc(A)",
                mine.len(),
                theirs_native.len(),
                if de.is_empty() { "yes" } else { "NO" },
                fp::ber(&mine, &theirs_native),
                de.len(),
                bits
            );
            rows.push(serde_json::json!({
                "case": "C: swresample resample, mine vs fpcalc native",
                "mine_items": mine.len(), "fpcalc_items": theirs_native.len(),
                "identical": de.is_empty(), "items_differing": de.len(), "bits_differing": bits,
            }));
        }
        _ => println!("  (ffmpeg unavailable — skipping the swresample hypothesis test)"),
    }

    for p in [raw_native, src, dst, dstc, dir.join("vcw_s4_11025.raw")] {
        let _ = std::fs::remove_file(p);
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "test": "T5-c-reference", "source": wav.display().to_string(), "rows": rows,
            }))?
        );
    }
    Ok(())
}

// -------------------------------------------------------------------- T3 rate

fn cmd_rate(wav: &Path, start: f64, len: f64, json: bool) -> Result<()> {
    let r = load_region(wav, start, len)?;
    let algo = Algorithm::default();
    let dir = std::env::temp_dir();

    println!("T3 — does the capture sample rate change the fingerprint?");
    println!(
        "  source      {} @ {} Hz {} ch",
        wav.display(),
        r.rate,
        r.channels
    );
    println!("  region      {:.1} s from {:.1} s", len, start);
    println!();

    let base = fp::offline(&r.samples, r.rate, r.channels, algo)?;
    let src = dir.join("vcw_s4_rate_src.wav");
    wav::write_i16(&src, &r.samples, r.rate, r.channels)?;

    println!(
        "  {:>10} {:>7} {:>13} {:>10} {:>6}",
        "rate", "items", "BER vs native", "BER best", "shift"
    );
    println!(
        "  {:>10} {:>7} {:>13} {:>10} {:>6}",
        r.rate,
        base.len(),
        "-",
        "-",
        "-"
    );

    let mut rows = Vec::new();
    for target in [96000u32, 48000, 44100] {
        if target >= r.rate {
            continue;
        }
        let dst = dir.join(format!("vcw_s4_rate_{target}.wav"));
        let o = std::process::Command::new("sox")
            .arg(&src)
            .args(["-r", &target.to_string()])
            .arg(&dst)
            .output()
            .context("running sox")?;
        if !o.status.success() {
            bail!(
                "sox to {target} Hz failed: {}",
                String::from_utf8_lossy(&o.stderr).trim()
            );
        }
        let i = wav::probe(&dst)?;
        let s = wav::read_i16(&dst, &i, 0, i.frames())?;
        let got = fp::offline(&s, i.sample_rate, i.channels, algo)?;
        let b = fp::ber(&base, &got);
        let (bb, shift) = fp::ber_best_shift(&base, &got, 16, base.len() / 2);
        println!(
            "  {target:>10} {:>7} {b:>13.6} {bb:>10.6} {shift:>6}",
            got.len()
        );
        rows.push(serde_json::json!({
            "rate": target, "items": got.len(), "ber": b, "ber_best": bb, "shift": shift,
        }));
        let _ = std::fs::remove_file(&dst);
    }
    let _ = std::fs::remove_file(&src);

    println!();
    println!("  A high BER here would mean the rate we capture at has to match the rate");
    println!("  the AcoustID submission was made at, which we cannot control. A low one");
    println!("  means we can fingerprint straight off the capture stream.");

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "test": "T3-rate-path", "source": wav.display().to_string(),
                "native_rate": r.rate, "native_items": base.len(), "rows": rows,
            }))?
        );
    }
    Ok(())
}

// ------------------------------------------------------------------- T6 scale

/// Run `ffmpeg` on `src`, with `args` inserted before the output path, and read
/// the result back as interleaved i16 at its own rate.
fn ffmpeg_roundtrip(src: &Path, out: &Path, args: &[&str]) -> Result<(Vec<i16>, u32, u16)> {
    let o = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-y", "-i"])
        .arg(src)
        .args(args)
        .arg(out)
        .output()
        .context("running ffmpeg")?;
    if !o.status.success() {
        bail!(
            "ffmpeg {args:?} failed: {}",
            String::from_utf8_lossy(&o.stderr).trim()
        );
    }
    let i = wav::probe(out)?;
    let s = wav::read_i16(out, &i, 0, i.frames())?;
    Ok((s, i.sample_rate, i.channels))
}

fn cmd_scale(wav: &Path, other: &Path, start: f64, len: f64, json: bool) -> Result<()> {
    let algo = Algorithm::default();
    let dir = std::env::temp_dir();
    let info = wav::probe(wav)?;
    let sf = (start * info.sample_rate as f64) as u64;
    let nf = (len * info.sample_rate as f64) as u64;

    let base_samples = wav::read_i16_narrow(wav, &info, sf, nf, wav::Narrow::Truncate)?;
    let base = fp::offline(&base_samples, info.sample_rate, info.channels, algo)?;

    println!("T6 — BER scale calibration");
    println!(
        "  source      {} @ {} Hz {} ch",
        wav.display(),
        info.sample_rate,
        info.channels
    );
    println!(
        "  region      {:.1} s from {:.1} s, {} sub-fingerprints",
        len,
        start,
        base.len()
    );
    println!("  reference   24-bit truncated to i16 (plain shift), fed at the native rate");
    println!();
    println!(
        "  {:<46} {:>12} {:>10} {:>6}",
        "perturbation", "BER", "BER best", "shift"
    );

    let src = dir.join("vcw_s4_scale_src.wav");
    wav::write_i16(&src, &base_samples, info.sample_rate, info.channels)?;
    let mut tmp = vec![src.clone()];
    let mut rows = Vec::new();

    let report = |label: &str, got: &[u32], rows: &mut Vec<serde_json::Value>| {
        let b = fp::ber(&base, got);
        let (bb, sh) = fp::ber_best_shift(&base, got, 64, base.len() / 2);
        println!("  {label:<46} {b:>12.6} {bb:>10.6} {sh:>6}");
        rows.push(
            serde_json::json!({ "perturbation": label, "ber": b, "ber_best": bb, "shift": sh }),
        );
    };

    // The choice VCW actually owns: how to narrow the captured sample word.
    let rounded = wav::read_i16_narrow(wav, &info, sf, nf, wav::Narrow::Round)?;
    let fp_round = fp::offline(&rounded, info.sample_rate, info.channels, algo)?;
    report("round to nearest instead of truncate", &fp_round, &mut rows);

    // Level differences between two plays of the same record.
    for (label, af) in [
        ("gain -6 dB", "volume=-6dB"),
        ("gain -20 dB", "volume=-20dB"),
        ("gain +3 dB", "volume=+3dB"),
    ] {
        let out = dir.join(format!(
            "vcw_s4_scale_{}.wav",
            af.replace(['=', '+', '-'], "_")
        ));
        let (s, r, c) = ffmpeg_roundtrip(&src, &out, &["-af", af, "-c:a", "pcm_s16le"])?;
        report(label, &fp::offline(&s, r, c, algo)?, &mut rows);
        tmp.push(out);
    }

    // Lossy round-trips: the shape of the audio AcoustID's database was mostly
    // built from, and the shape VCW's own MP3 export would produce.
    for (label, bitrate) in [
        ("MP3 320k round-trip", "320k"),
        ("MP3 128k round-trip", "128k"),
    ] {
        let mp3 = dir.join(format!("vcw_s4_scale_{bitrate}.mp3"));
        let back = dir.join(format!("vcw_s4_scale_{bitrate}.wav"));
        match ffmpeg_roundtrip(&src, &mp3, &["-c:a", "libmp3lame", "-b:a", bitrate]) {
            Ok(_) => {}
            Err(_) => {
                // ffmpeg_roundtrip tries to read the mp3 as a WAV and fails; the
                // encode itself is what mattered and it has already happened.
            }
        }
        if mp3.exists() {
            let (s, r, c) = ffmpeg_roundtrip(&mp3, &back, &["-c:a", "pcm_s16le"])?;
            report(label, &fp::offline(&s, r, c, algo)?, &mut rows);
            tmp.push(mp3);
            tmp.push(back);
        } else {
            println!("  {label:<46} {:>12}", "(libmp3lame unavailable)");
        }
    }

    // The upper baselines: a different part of the same record, then a different
    // record entirely. Unrelated audio should sit near 0.5.
    let far = sf + (300.0 * info.sample_rate as f64) as u64;
    if far + nf <= info.frames() {
        let s = wav::read_i16(wav, &info, far, nf)?;
        report(
            "same record, region 300 s later",
            &fp::offline(&s, info.sample_rate, info.channels, algo)?,
            &mut rows,
        );
    }
    let oi = wav::probe(other)?;
    let os = wav::read_i16(
        other,
        &oi,
        (start * oi.sample_rate as f64) as u64,
        (len * oi.sample_rate as f64) as u64,
    )?;
    report(
        &format!(
            "unrelated record ({})",
            other.file_name().unwrap_or_default().to_string_lossy()
        ),
        &fp::offline(&os, oi.sample_rate, oi.channels, algo)?,
        &mut rows,
    );

    for p in tmp {
        let _ = std::fs::remove_file(p);
    }

    println!();
    println!("  0.5 is a coin flip. Anything an ordinary perturbation costs sets the");
    println!("  scale that the other tests' numbers have to be judged against.");

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "test": "T6-ber-scale", "source": wav.display().to_string(),
                "rate": info.sample_rate, "region_secs": len, "base_items": base.len(), "rows": rows,
            }))?
        );
    }
    Ok(())
}

// -------------------------------------------------------------------- T7 live

fn cmd_live(wav: &Path, block_secs: f64, len: f64, instances: usize, json: bool) -> Result<()> {
    use chromaprint::Fingerprinter;

    let info = wav::probe(wav)?;
    let algo = Algorithm::default();
    let block_frames = (info.sample_rate as f64 * block_secs) as u64;
    let total = if len > 0.0 {
        ((len * info.sample_rate as f64) as u64).min(info.frames())
    } else {
        info.frames()
    };
    let audio_secs = total as f64 / info.sample_rate as f64;

    println!("T7 — whole-side streaming, one block resident");
    println!(
        "  source      {} @ {} Hz {} ch",
        wav.display(),
        info.sample_rate,
        info.channels
    );
    println!("  audio       {:.1} s ({} frames)", audio_secs, total);
    println!(
        "  block       {:.0} ms = {} frames = {:.2} MiB of i16",
        block_secs * 1000.0,
        block_frames,
        (block_frames * info.channels as u64 * 2) as f64 / 1048576.0
    );
    println!();

    let instances = instances.max(1);
    println!("  instances   {instances}");
    println!();

    let rss_before = peak_rss_kib();
    let mut fps: Vec<Fingerprinter> = (0..instances).map(|_| Fingerprinter::new(algo)).collect();
    for fp in &mut fps {
        fp.start(info.sample_rate, info.channels)
            .map_err(|e| anyhow::anyhow!("start: {e}"))?;
    }

    let mut feed_us: Vec<u64> = Vec::with_capacity((total / block_frames + 2) as usize);
    let mut read_us: Vec<u64> = Vec::with_capacity(feed_us.capacity());
    let mut done = 0u64;
    let wall = Instant::now();
    while done < total {
        let n = block_frames.min(total - done);
        let t = Instant::now();
        let block = wav::read_i16(wav, &info, done, n)?;
        read_us.push(t.elapsed().as_micros() as u64);
        let t = Instant::now();
        for fp in &mut fps {
            fp.feed(&block).map_err(|e| anyhow::anyhow!("feed: {e}"))?;
        }
        feed_us.push(t.elapsed().as_micros() as u64);
        done += n;
    }
    for fp in &mut fps {
        fp.finish().map_err(|e| anyhow::anyhow!("finish: {e}"))?;
    }
    let elapsed = wall.elapsed().as_secs_f64();
    let items = fps[0].fingerprint().len();
    let agree = fps.iter().all(|f| f.fingerprint() == fps[0].fingerprint());
    let rss_after = peak_rss_kib();

    let pct = |v: &mut Vec<u64>, p: f64| -> f64 {
        v.sort_unstable();
        let i = (((v.len() - 1) as f64) * p).round() as usize;
        v[i] as f64 / 1000.0
    };

    println!("  blocks      {}", feed_us.len());
    println!(
        "  items       {items} sub-fingerprints ({:.0} KiB)",
        items as f64 * 4.0 / 1024.0
    );
    println!(
        "  wall        {elapsed:.2} s  ({:.0}x real time, {:.3}% of one core)",
        audio_secs / elapsed,
        100.0 * elapsed / audio_secs
    );
    println!(
        "  feed time   p50 {:.2} ms · p99 {:.2} ms · max {:.2} ms  (all instances; budget {:.0} ms/block)",
        pct(&mut feed_us, 0.50),
        pct(&mut feed_us, 0.99),
        pct(&mut feed_us, 1.0),
        block_secs * 1000.0
    );
    println!(
        "  disk read   p50 {:.2} ms · p99 {:.2} ms · max {:.2} ms",
        pct(&mut read_us, 0.50),
        pct(&mut read_us, 0.99),
        pct(&mut read_us, 1.0)
    );
    println!(
        "  peak RSS    {:.1} MiB (was {:.1} MiB before the stream; {:.2} MiB per instance)",
        rss_after as f64 / 1024.0,
        rss_before as f64 / 1024.0,
        (rss_after.saturating_sub(rss_before)) as f64 / 1024.0 / instances as f64
    );
    if instances > 1 {
        println!(
            "  agreement   {}",
            if agree {
                "all instances produced the identical fingerprint"
            } else {
                "MISMATCH — instances are not independent"
            }
        );
    }

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "test": "T7-live-stream", "source": wav.display().to_string(),
                "rate": info.sample_rate, "channels": info.channels,
                "audio_secs": audio_secs, "block_secs": block_secs, "blocks": feed_us.len(),
                "items": items, "wall_secs": elapsed, "rt_factor": audio_secs / elapsed,
                "feed_p50_ms": pct(&mut feed_us, 0.50), "feed_p99_ms": pct(&mut feed_us, 0.99),
                "feed_max_ms": pct(&mut feed_us, 1.0),
                "instances": instances, "instances_agree": agree,
                "peak_rss_kib": rss_after, "rss_before_kib": rss_before,
            }))?
        );
    }
    Ok(())
}
