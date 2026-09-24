//! Minimal seekable reader for the 32-bit PCM WAV files in /data2/source_rips.
//!
//! Deliberately not a WAV library: the rips are multi-gigabyte, so the spike needs
//! to seek straight to a frame offset rather than stream from byte zero. It handles
//! canonical PCM and WAVE_FORMAT_EXTENSIBLE, 16/24/32-bit integer, and refuses
//! anything else loudly rather than guessing.

use anyhow::{Context, Result, bail};
use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct WavInfo {
    pub sample_rate: u32,
    pub channels: u16,
    pub bits: u16,
    pub data_offset: u64,
    pub data_len: u64,
}

impl WavInfo {
    pub fn bytes_per_frame(&self) -> u64 {
        (self.bits as u64 / 8) * self.channels as u64
    }
    pub fn frames(&self) -> u64 {
        self.data_len / self.bytes_per_frame()
    }
    pub fn duration_secs(&self) -> f64 {
        self.frames() as f64 / self.sample_rate as f64
    }
}

fn u16le(b: &[u8]) -> u16 {
    u16::from_le_bytes([b[0], b[1]])
}
fn u32le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

pub fn probe(path: &Path) -> Result<WavInfo> {
    let mut f = File::open(path).with_context(|| format!("open {}", path.display()))?;
    let mut hdr = [0u8; 12];
    f.read_exact(&mut hdr)?;
    if &hdr[0..4] != b"RIFF" || &hdr[8..12] != b"WAVE" {
        bail!("{}: not a RIFF/WAVE file", path.display());
    }

    let file_len = f.metadata()?.len();
    let mut pos: u64 = 12;
    let mut fmt: Option<(u16, u16, u32, u16)> = None; // (format_tag, channels, rate, bits)

    loop {
        if pos + 8 > file_len {
            bail!(
                "{}: ran out of file before finding a data chunk",
                path.display()
            );
        }
        f.seek(SeekFrom::Start(pos))?;
        let mut ch = [0u8; 8];
        f.read_exact(&mut ch)?;
        let id = [ch[0], ch[1], ch[2], ch[3]];
        let size = u32le(&ch[4..8]) as u64;
        let body = pos + 8;

        match &id {
            b"fmt " => {
                let mut b = vec![0u8; size.min(40) as usize];
                f.read_exact(&mut b)?;
                if b.len() < 16 {
                    bail!(
                        "{}: fmt chunk is {} bytes, need 16",
                        path.display(),
                        b.len()
                    );
                }
                let mut tag = u16le(&b[0..2]);
                // WAVE_FORMAT_EXTENSIBLE carries the real tag in the GUID's first two bytes.
                if tag == 0xFFFE && b.len() >= 26 {
                    tag = u16le(&b[24..26]);
                }
                fmt = Some((tag, u16le(&b[2..4]), u32le(&b[4..8]), u16le(&b[14..16])));
            }
            b"data" => {
                let (tag, channels, sample_rate, bits) =
                    fmt.context("data chunk appeared before fmt chunk")?;
                if tag != 1 {
                    bail!(
                        "{}: format tag {tag} is not integer PCM (float and compressed \
                         input are out of scope for this spike)",
                        path.display()
                    );
                }
                if !matches!(bits, 16 | 24 | 32) {
                    bail!("{}: {bits}-bit samples unsupported", path.display());
                }
                // A 4 GiB rip overflows the 32-bit size field; trust the file length.
                let avail = file_len - body;
                let data_len = if size == 0 || size == u32::MAX as u64 || size > avail {
                    avail
                } else {
                    size
                };
                return Ok(WavInfo {
                    sample_rate,
                    channels,
                    bits,
                    data_offset: body,
                    data_len,
                });
            }
            _ => {}
        }
        pos = body + size + (size & 1); // chunks are word-aligned
    }
}

/// How the 24/32-bit source samples are narrowed to the i16 that
/// `Fingerprinter::feed` requires. The capture path has to pick one, so the spike
/// measures whether the choice is visible in the fingerprint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Narrow {
    /// Arithmetic shift - drop the low bits. What a naive `>> 16` does.
    Truncate,
    /// Round to nearest, ties away from zero, saturating at i16::MAX.
    Round,
}

/// Read `frames` frames starting at `start_frame`, converting to interleaved i16 -
/// the only shape `Fingerprinter::feed` accepts. The conversion is a plain
/// arithmetic right shift of the most significant bits, which is what the capture
/// path will do: the rips are 24-bit samples left-aligned in a 32-bit container.
pub fn read_i16(path: &Path, info: &WavInfo, start_frame: u64, frames: u64) -> Result<Vec<i16>> {
    read_i16_narrow(path, info, start_frame, frames, Narrow::Truncate)
}

pub fn read_i16_narrow(
    path: &Path,
    info: &WavInfo,
    start_frame: u64,
    frames: u64,
    narrow: Narrow,
) -> Result<Vec<i16>> {
    let bpf = info.bytes_per_frame();
    let total = info.frames();
    if start_frame >= total {
        bail!(
            "start frame {start_frame} is past the end of {} ({total} frames)",
            path.display()
        );
    }
    let frames = frames.min(total - start_frame);
    let want_bytes = (frames * bpf) as usize;

    let f = File::open(path)?;
    let mut r = BufReader::with_capacity(1 << 20, f);
    r.seek(SeekFrom::Start(info.data_offset + start_frame * bpf))?;
    let mut raw = vec![0u8; want_bytes];
    r.read_exact(&mut raw)?;

    let n = frames as usize * info.channels as usize;
    let mut out = Vec::with_capacity(n);
    // `shift` bits are discarded; `Narrow::Round` adds half a least significant
    // output bit first, saturating rather than wrapping at the top of the range.
    let narrow_i32 = |v: i32, shift: u32| -> i16 {
        match narrow {
            Narrow::Truncate => (v >> shift) as i16,
            Narrow::Round => {
                let half = 1i64 << (shift - 1);
                let r = if v >= 0 {
                    (v as i64 + half) >> shift
                } else {
                    -((-(v as i64) + half) >> shift)
                };
                r.clamp(i16::MIN as i64, i16::MAX as i64) as i16
            }
        }
    };
    match info.bits {
        16 => {
            for c in raw.chunks_exact(2) {
                out.push(i16::from_le_bytes([c[0], c[1]]));
            }
        }
        24 => {
            for c in raw.chunks_exact(3) {
                let v = i32::from_le_bytes([0, c[0], c[1], c[2]]) >> 8; // sign-extend 24-bit
                out.push(narrow_i32(v, 8));
            }
        }
        32 => {
            for c in raw.chunks_exact(4) {
                out.push(narrow_i32(i32::from_le_bytes([c[0], c[1], c[2], c[3]]), 16));
            }
        }
        _ => unreachable!("probe() rejects other widths"),
    }
    Ok(out)
}

/// Write interleaved i16 as a canonical PCM WAV. Only used to hand identical
/// bytes to the C reference binary, so it stays minimal.
pub fn write_i16(path: &Path, samples: &[i16], rate: u32, channels: u16) -> Result<()> {
    use std::io::Write;
    let data_len = (samples.len() * 2) as u32;
    let byte_rate = rate * channels as u32 * 2;
    let mut f = std::io::BufWriter::new(File::create(path)?);
    f.write_all(b"RIFF")?;
    f.write_all(&(36 + data_len).to_le_bytes())?;
    f.write_all(b"WAVEfmt ")?;
    f.write_all(&16u32.to_le_bytes())?;
    f.write_all(&1u16.to_le_bytes())?; // PCM
    f.write_all(&channels.to_le_bytes())?;
    f.write_all(&rate.to_le_bytes())?;
    f.write_all(&byte_rate.to_le_bytes())?;
    f.write_all(&(channels * 2).to_le_bytes())?; // block align
    f.write_all(&16u16.to_le_bytes())?;
    f.write_all(b"data")?;
    f.write_all(&data_len.to_le_bytes())?;
    for s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()?;
    Ok(())
}

/// Write interleaved i16 as headerless little-endian PCM, for `fpcalc -format s16le`.
pub fn write_raw(path: &Path, samples: &[i16]) -> Result<()> {
    use std::io::Write;
    let mut f = std::io::BufWriter::new(File::create(path)?);
    for s in samples {
        f.write_all(&s.to_le_bytes())?;
    }
    f.flush()?;
    Ok(())
}
