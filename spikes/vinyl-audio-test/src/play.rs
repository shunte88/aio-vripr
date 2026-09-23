//! REQUIREMENTS §47.8 — play the capture back out of SQLite.
//!
//! This is the other half of the bit-perfect claim. Capture proves the bytes
//! went in unaltered; playback proves they can come back out and drive a device
//! at the same format. Where the output device will not accept the stored
//! format we convert — but we say so loudly, because a silent conversion here
//! is the same defect as a silent conversion on the way in.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{SampleFormat, StreamConfig};
use rusqlite::Connection;
use serde::Serialize;

use crate::devices;

#[derive(Debug, Serialize)]
pub struct Report {
    pub device: String,
    pub stored_rate: u32,
    pub stored_channels: u16,
    pub stored_bytes_per_sample: usize,
    pub stored_layout: String,
    pub blocks: u64,
    pub duration_secs: f64,
    pub output_format: String,
    pub converted: bool,
    pub conversion_note: Option<String>,
    pub frames_played: u64,
    /// Device-reported stream errors. With the whole capture resident in memory
    /// there is no software path to an underrun, so anything here came from the
    /// device or the driver — see the note on `underruns` in the module docs.
    pub device_errors: Vec<String>,
    /// Callbacks the source could not fill completely. Exactly one is expected
    /// — the last one, at the end of the capture.
    pub short_callbacks: u64,
    pub elapsed_secs: f64,
    pub verdict: String,
}

struct Stored {
    rate: u32,
    channels: u16,
    bps: usize,
    per_channel: bool,
}

fn read_meta(conn: &Connection) -> Result<Stored> {
    let (rate, channels, bps, layout): (u32, u16, i64, String) = conn
        .query_row(
            "SELECT sample_rate, channels, bytes_per_sample, layout
               FROM captures WHERE capture_id = 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .context("no capture row — is this a capture database?")?;
    Ok(Stored {
        rate,
        channels,
        bps: bps as usize,
        per_channel: layout.eq_ignore_ascii_case("perchannel"),
    })
}

/// Rebuild the interleaved stream for one block sequence.
///
/// Per-channel storage (the AUP4-compatible layout) holds one row per channel,
/// so playback has to re-interleave. Sample *values* are untouched either way —
/// that equivalence is what let S2 treat layout as a measurement rather than an
/// argument.
fn interleave(rows: &[(i64, Vec<u8>)], channels: u16, bps: usize) -> Vec<u8> {
    if rows.len() == 1 {
        return rows[0].1.clone();
    }
    let frames = rows.iter().map(|(_, b)| b.len() / bps).min().unwrap_or(0);
    let mut out = vec![0u8; frames * channels as usize * bps];
    for (channel, bytes) in rows {
        let ch = *channel as usize;
        if ch >= channels as usize {
            continue;
        }
        for f in 0..frames {
            let src = f * bps;
            let dst = (f * channels as usize + ch) * bps;
            out[dst..dst + bps].copy_from_slice(&bytes[src..src + bps]);
        }
    }
    out
}

pub fn run(db_path: &str, device_query: Option<&str>, max_secs: Option<u64>) -> Result<Report> {
    let conn = Connection::open_with_flags(db_path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("opening {db_path}"))?;
    let meta = read_meta(&conn)?;

    let (device, device_name) = match device_query {
        Some(q) => devices::find(q, false)?,
        None => devices::default_output()?,
    };

    // Prefer handing the device exactly the bytes we stored.
    let stored_format = match meta.bps {
        2 => SampleFormat::I16,
        4 => SampleFormat::I32,
        _ => bail!("unsupported stored sample width: {} bytes", meta.bps),
    };
    let supported: Vec<_> = device.supported_output_configs()?.collect();
    let exact = supported.iter().any(|r| {
        r.channels() == meta.channels
            && r.sample_format() == stored_format
            && r.min_sample_rate().0 <= meta.rate
            && r.max_sample_rate().0 >= meta.rate
    });
    let fallback = supported.iter().find(|r| {
        r.channels() == meta.channels
            && r.sample_format() == SampleFormat::F32
            && r.min_sample_rate().0 <= meta.rate
            && r.max_sample_rate().0 >= meta.rate
    });

    let (out_format, converted, note) = if exact {
        (stored_format, false, None)
    } else if fallback.is_some() {
        (
            SampleFormat::F32,
            true,
            Some(format!(
                "{device_name:?} will not take {stored_format:?} at {} Hz; converting to F32. \
                 Playback is therefore NOT bit-identical to the stored capture.",
                meta.rate
            )),
        )
    } else {
        bail!(
            "{device_name:?} supports neither {stored_format:?} nor F32 at {} Hz / {} channels",
            meta.rate,
            meta.channels
        );
    };

    // Pull the whole capture into memory. Fine for a spike measured in
    // minutes; the real player streams, which is WP-09's problem, not this
    // tool's, and pretending otherwise here would add risk without evidence.
    let mut stmt = conn.prepare(
        "SELECT c.sequence, c.channel, s.samples
           FROM capture_blocks c JOIN sampleblocks s ON s.blockid = c.blockid
          ORDER BY c.sequence, c.channel",
    )?;
    let mut pcm: Vec<u8> = Vec::new();
    let mut blocks = 0u64;
    let mut current_seq: i64 = -1;
    let mut current: Vec<(i64, Vec<u8>)> = Vec::new();
    let mut rows = stmt.query([])?;
    while let Some(r) = rows.next()? {
        let seq: i64 = r.get(0)?;
        let channel: i64 = r.get(1)?;
        let samples: Vec<u8> = r.get(2)?;
        if seq != current_seq && !current.is_empty() {
            pcm.extend_from_slice(&interleave(&current, meta.channels, meta.bps));
            blocks += 1;
            current.clear();
        }
        current_seq = seq;
        current.push((channel, samples));
    }
    if !current.is_empty() {
        pcm.extend_from_slice(&interleave(&current, meta.channels, meta.bps));
        blocks += 1;
    }
    let _ = meta.per_channel;

    let frame_bytes = meta.bps * meta.channels as usize;
    let total_frames = pcm.len() / frame_bytes;
    let duration_secs = total_frames as f64 / meta.rate as f64;
    if total_frames == 0 {
        bail!("capture contains no audio");
    }

    let config = StreamConfig {
        channels: meta.channels,
        sample_rate: cpal::SampleRate(meta.rate),
        buffer_size: cpal::BufferSize::Default,
    };

    let played = Arc::new(AtomicU64::new(0));
    let short_callbacks = Arc::new(AtomicU64::new(0));
    let finished = Arc::new(AtomicBool::new(false));
    let errors = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));

    let stream = {
        let played = Arc::clone(&played);
        let short_callbacks = Arc::clone(&short_callbacks);
        let finished = Arc::clone(&finished);
        let errors_cb = Arc::clone(&errors);
        let src = pcm;
        let bps = meta.bps;
        let mut cursor = 0usize;
        device.build_output_stream_raw(
            &config,
            out_format,
            move |data, _info| {
                let out = data.bytes_mut();
                // Past the end of the capture the device keeps asking until we
                // tear the stream down. Feeding it silence is correct; counting
                // those calls as underruns is not — that was measuring our own
                // shutdown latency and reporting it as a fault.
                if cursor >= src.len() {
                    out.fill(0);
                    finished.store(true, Ordering::Relaxed);
                    return;
                }
                let want_frames = out.len() / (out_format.sample_size() * config.channels as usize);
                let avail_frames = (src.len() - cursor) / frame_bytes;
                let n = want_frames.min(avail_frames);
                if n < want_frames {
                    // The final short callback is the end of the file, not a
                    // starvation event. There is no other way to run short here:
                    // the whole capture is resident before the stream starts,
                    // so a genuine underrun can only come from the device and
                    // arrives through the error callback instead.
                    short_callbacks.fetch_add(1, Ordering::Relaxed);
                }
                let src_bytes = &src[cursor..cursor + n * frame_bytes];
                if converted {
                    convert_to_f32(src_bytes, out, bps);
                } else {
                    out[..src_bytes.len()].copy_from_slice(src_bytes);
                    out[src_bytes.len()..].fill(0);
                }
                cursor += n * frame_bytes;
                played.fetch_add(n as u64, Ordering::Relaxed);
                if cursor >= src.len() {
                    finished.store(true, Ordering::Relaxed);
                }
            },
            move |err| {
                if let Ok(mut v) = errors_cb.lock() {
                    v.push(err.to_string());
                }
            },
            None,
        )?
    };

    let started = Instant::now();
    stream.play()?;
    let limit = max_secs.map(Duration::from_secs);
    while !finished.load(Ordering::Relaxed) {
        if let Some(l) = limit
            && started.elapsed() >= l
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Let the device drain what is already queued.
    std::thread::sleep(Duration::from_millis(200));
    let elapsed = started.elapsed().as_secs_f64();
    drop(stream);

    let frames_played = played.load(Ordering::Relaxed);
    let short = short_callbacks.load(Ordering::Relaxed);
    let device_errors = errors.lock().map(|v| v.clone()).unwrap_or_default();
    let expected_frames = total_frames as u64;
    let verdict = if !device_errors.is_empty() {
        format!("FAIL: {} device errors", device_errors.len())
    } else if max_secs.is_none() && frames_played != expected_frames {
        format!("FAIL: played {frames_played} of {expected_frames} frames")
    } else if converted {
        "PASS (audible, but format-converted — see conversion_note)".into()
    } else {
        "PASS (played back in the stored format, no conversion)".into()
    };

    Ok(Report {
        device: device_name,
        stored_rate: meta.rate,
        stored_channels: meta.channels,
        stored_bytes_per_sample: meta.bps,
        stored_layout: if meta.per_channel {
            "PerChannel"
        } else {
            "Interleaved"
        }
        .into(),
        blocks,
        duration_secs,
        output_format: format!("{out_format:?}"),
        converted,
        conversion_note: note,
        frames_played,
        device_errors,
        short_callbacks: short,
        elapsed_secs: elapsed,
        verdict,
    })
}

fn convert_to_f32(src: &[u8], out: &mut [u8], bps: usize) {
    let scale = match bps {
        2 => 1.0 / -(i16::MIN as f32),
        4 => 1.0 / -(i32::MIN as f32),
        _ => 0.0,
    };
    let count = src.len() / bps;
    for i in 0..count {
        let o = i * bps;
        let v = match bps {
            2 => i16::from_le_bytes([src[o], src[o + 1]]) as f32,
            4 => i32::from_le_bytes([src[o], src[o + 1], src[o + 2], src[o + 3]]) as f32,
            _ => 0.0,
        } * scale;
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
    }
    out[count * 4..].fill(0);
}
