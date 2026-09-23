//! REQUIREMENTS §47.1 and §47.2 — what devices exist, and what will they accept.
//!
//! Enumeration is deliberately host-by-host rather than default-host-only. On
//! Linux the same physical converter appears under ALSA as `hw:`, `plughw:` and
//! whatever PipeWire or PulseAudio publishes, and those are *not* equivalent:
//! only the `hw:` path can be bit-perfect. A user picking a device from a list
//! (§5) is really picking a path through the stack, so the list has to show it.

use anyhow::Result;
use cpal::traits::{DeviceTrait, HostTrait};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ConfigRange {
    pub channels: u16,
    pub min_rate: u32,
    pub max_rate: u32,
    pub sample_format: String,
    pub bytes_per_sample: usize,
    pub buffer_frames: Option<(u32, u32)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceInfo {
    pub host: String,
    pub name: String,
    pub is_default_input: bool,
    pub is_default_output: bool,
    /// True for an ALSA `hw:`/`plughw:` name, the only route that can bypass
    /// the resampler. Informational on other platforms.
    pub direct_hardware: bool,
    pub input_configs: Vec<ConfigRange>,
    pub output_configs: Vec<ConfigRange>,
    pub default_input: Option<ConfigRange>,
    pub default_output: Option<ConfigRange>,
    pub errors: Vec<String>,
}

fn range(r: &cpal::SupportedStreamConfigRange) -> ConfigRange {
    ConfigRange {
        channels: r.channels(),
        min_rate: r.min_sample_rate().0,
        max_rate: r.max_sample_rate().0,
        sample_format: format!("{:?}", r.sample_format()),
        bytes_per_sample: r.sample_format().sample_size(),
        buffer_frames: match r.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => Some((*min, *max)),
            cpal::SupportedBufferSize::Unknown => None,
        },
    }
}

fn exact(c: &cpal::SupportedStreamConfig) -> ConfigRange {
    ConfigRange {
        channels: c.channels(),
        min_rate: c.sample_rate().0,
        max_rate: c.sample_rate().0,
        sample_format: format!("{:?}", c.sample_format()),
        bytes_per_sample: c.sample_format().sample_size(),
        buffer_frames: match c.buffer_size() {
            cpal::SupportedBufferSize::Range { min, max } => Some((*min, *max)),
            cpal::SupportedBufferSize::Unknown => None,
        },
    }
}

/// Every device on every host CPAL compiled support for.
pub fn enumerate() -> Result<Vec<DeviceInfo>> {
    let mut out = Vec::new();
    for host_id in cpal::available_hosts() {
        let host = match cpal::host_from_id(host_id) {
            Ok(h) => h,
            Err(e) => {
                out.push(DeviceInfo {
                    host: format!("{host_id:?}"),
                    name: "<host unavailable>".into(),
                    is_default_input: false,
                    is_default_output: false,
                    direct_hardware: false,
                    input_configs: vec![],
                    output_configs: vec![],
                    default_input: None,
                    default_output: None,
                    errors: vec![e.to_string()],
                });
                continue;
            }
        };
        let def_in = host.default_input_device().and_then(|d| d.name().ok());
        let def_out = host.default_output_device().and_then(|d| d.name().ok());

        for device in host.devices()? {
            let mut errors = Vec::new();
            let name = device.name().unwrap_or_else(|e| {
                errors.push(format!("name: {e}"));
                "<unnamed>".into()
            });
            // Querying configs opens the PCM; a device in use by something else
            // fails here rather than at stream build. That is worth reporting,
            // not swallowing — "no formats" and "busy" are different diagnoses.
            let input_configs = match device.supported_input_configs() {
                Ok(it) => it.map(|r| range(&r)).collect(),
                Err(e) => {
                    errors.push(format!("input configs: {e}"));
                    vec![]
                }
            };
            let output_configs = match device.supported_output_configs() {
                Ok(it) => it.map(|r| range(&r)).collect(),
                Err(e) => {
                    errors.push(format!("output configs: {e}"));
                    vec![]
                }
            };
            out.push(DeviceInfo {
                host: format!("{host_id:?}"),
                is_default_input: Some(&name) == def_in.as_ref(),
                is_default_output: Some(&name) == def_out.as_ref(),
                direct_hardware: name.starts_with("hw:") || name.starts_with("plughw:"),
                input_configs,
                output_configs,
                default_input: device.default_input_config().ok().map(|c| exact(&c)),
                default_output: device.default_output_config().ok().map(|c| exact(&c)),
                errors,
                name,
            });
        }
    }
    Ok(out)
}

/// Resolve a device by exact name, then by substring, across all hosts.
/// Ambiguity is an error: silently taking the first match of "USB" when three
/// devices contain it is how you record from the wrong converter.
pub fn find(query: &str, want_input: bool) -> Result<(cpal::Device, String)> {
    let mut exact_hits = Vec::new();
    let mut fuzzy_hits = Vec::new();
    for host_id in cpal::available_hosts() {
        let Ok(host) = cpal::host_from_id(host_id) else {
            continue;
        };
        for device in host.devices()? {
            let Ok(name) = device.name() else { continue };
            let usable = if want_input {
                device.supported_input_configs().is_ok()
            } else {
                device.supported_output_configs().is_ok()
            };
            if !usable {
                continue;
            }
            if name == query {
                exact_hits.push((device, name));
            } else if name.contains(query) {
                fuzzy_hits.push((device, name));
            }
        }
    }
    if exact_hits.len() == 1 {
        return Ok(exact_hits.pop().unwrap());
    }
    if exact_hits.is_empty() && fuzzy_hits.len() == 1 {
        return Ok(fuzzy_hits.pop().unwrap());
    }
    let candidates: Vec<&str> = exact_hits
        .iter()
        .chain(fuzzy_hits.iter())
        .map(|(_, n)| n.as_str())
        .collect();
    if candidates.is_empty() {
        anyhow::bail!(
            "no {} device matching {query:?}; run `devices` to list them",
            if want_input { "input" } else { "output" }
        );
    }
    anyhow::bail!("{query:?} is ambiguous, matches: {candidates:?}");
}

/// The default input, for when the user does not name one.
pub fn default_input() -> Result<(cpal::Device, String)> {
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or_else(|| anyhow::anyhow!("no default input device on host {:?}", host.id()))?;
    let name = device.name().unwrap_or_else(|_| "<unnamed>".into());
    Ok((device, name))
}

/// The default output, for playback.
pub fn default_output() -> Result<(cpal::Device, String)> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::anyhow!("no default output device on host {:?}", host.id()))?;
    let name = device.name().unwrap_or_else(|_| "<unnamed>".into());
    Ok((device, name))
}
