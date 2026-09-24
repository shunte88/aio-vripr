//! REQUIREMENTS §47.11 - requested versus negotiated format, with evidence.
//!
//! CPAL will tell you what config it built. It cannot tell you whether ALSA
//! quietly inserted a conversion behind it, and that is precisely the question
//! §8 cares about. The kernel will: while a PCM is open, ALSA publishes the
//! format it actually negotiated with the hardware under /proc/asound. If that
//! disagrees with what we asked for, the capture is not bit-perfect no matter
//! what the CPAL-level config says.
//!
//! Linux only. On other platforms this returns `None` and the report says so
//! rather than implying a check happened.

use std::path::PathBuf;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct HwParams {
    pub path: String,
    pub format: Option<String>,
    pub rate: Option<u32>,
    pub channels: Option<u16>,
    pub period_size: Option<u32>,
    pub buffer_size: Option<u32>,
    pub access: Option<String>,
    pub raw: String,
}

/// Read every open capture PCM's negotiated parameters.
///
/// We scan rather than resolve a specific card because CPAL's device name does
/// not carry the card/device index on every backend. With one stream open
/// there is normally exactly one hit, and reporting all of them is honest about
/// the ambiguity when there is not.
#[cfg(target_os = "linux")]
pub fn open_capture_streams() -> Vec<HwParams> {
    let mut found = Vec::new();
    let Ok(cards) = std::fs::read_dir("/proc/asound") else {
        return found;
    };
    let mut card_dirs: Vec<PathBuf> = cards
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("card"))
        })
        .collect();
    card_dirs.sort();
    for card in card_dirs {
        let Ok(pcms) = std::fs::read_dir(&card) else {
            continue;
        };
        let mut pcm_dirs: Vec<PathBuf> = pcms
            .flatten()
            .map(|e| e.path())
            // `pcm0c` is capture, `pcm0p` playback. We only want capture here.
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("pcm") && n.ends_with('c'))
            })
            .collect();
        pcm_dirs.sort();
        for pcm in pcm_dirs {
            let Ok(subs) = std::fs::read_dir(&pcm) else {
                continue;
            };
            for sub in subs.flatten() {
                let hw = sub.path().join("hw_params");
                let Ok(raw) = std::fs::read_to_string(&hw) else {
                    continue;
                };
                // A closed substream reads back literally "closed".
                if raw.trim() == "closed" || raw.trim().is_empty() {
                    continue;
                }
                found.push(parse(hw.display().to_string(), &raw));
            }
        }
    }
    found
}

#[cfg(not(target_os = "linux"))]
pub fn open_capture_streams() -> Vec<HwParams> {
    Vec::new()
}

fn parse(path: String, raw: &str) -> HwParams {
    let field = |key: &str| -> Option<String> {
        raw.lines()
            .find_map(|l| l.strip_prefix(key)?.strip_prefix(": "))
            .map(|v| v.trim().to_string())
    };
    HwParams {
        format: field("format"),
        rate: field("rate").and_then(|v| {
            // "rate: 192000 (192000/1)" - the leading integer is the one that matters.
            v.split_whitespace().next()?.parse().ok()
        }),
        channels: field("channels").and_then(|v| v.parse().ok()),
        period_size: field("period_size").and_then(|v| v.parse().ok()),
        buffer_size: field("buffer_size").and_then(|v| v.parse().ok()),
        access: field("access"),
        raw: raw.trim().to_string(),
        path,
    }
}

/// ALSA's format names against CPAL's. `None` where we have no mapping and so
/// cannot claim agreement either way.
pub fn alsa_format_matches(alsa: &str, cpal_format: cpal::SampleFormat) -> Option<bool> {
    use cpal::SampleFormat as F;
    let expected: &[&str] = match cpal_format {
        F::I16 => &["S16_LE"],
        F::I24 => &["S24_LE", "S24_3LE"],
        F::I32 => &["S32_LE"],
        F::U8 => &["U8"],
        F::F32 => &["FLOAT_LE"],
        F::F64 => &["FLOAT64_LE"],
        _ => return None,
    };
    Some(expected.contains(&alsa))
}
