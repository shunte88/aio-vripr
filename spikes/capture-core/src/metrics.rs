/*
 *  metrics.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Measurement helpers. The acceptance question is not "is it fast on
 *  average"
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

//! Measurement helpers. The acceptance question is not "is it fast on average"
//! but "does the tail ever exceed one block duration" - so percentiles and max
//! matter far more than means.

use serde::Serialize;
use std::path::Path;

#[derive(Default)]
pub struct Latencies(Vec<u64>);

impl Latencies {
    pub fn record(&mut self, micros: u64) {
        self.0.push(micros);
    }

    pub fn summary(&mut self) -> LatencySummary {
        if self.0.is_empty() {
            return LatencySummary::default();
        }
        self.0.sort_unstable();
        let pick = |q: f64| -> u64 {
            let idx = ((self.0.len() as f64 - 1.0) * q).round() as usize;
            self.0[idx]
        };
        LatencySummary {
            count: self.0.len() as u64,
            p50_us: pick(0.50),
            p95_us: pick(0.95),
            p99_us: pick(0.99),
            max_us: *self.0.last().unwrap(),
            mean_us: self.0.iter().sum::<u64>() / self.0.len() as u64,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, Serialize)]
pub struct LatencySummary {
    pub count: u64,
    pub mean_us: u64,
    pub p50_us: u64,
    pub p95_us: u64,
    pub p99_us: u64,
    pub max_us: u64,
}

pub fn file_len(path: &Path) -> u64 {
    std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)
}

/// Resident set size in bytes, read from /proc. Returns 0 where unavailable.
pub fn rss_bytes() -> u64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/self/statm")
            && let Some(field) = s.split_whitespace().nth(1)
            && let Ok(pages) = field.parse::<u64>()
        {
            return pages * 4096;
        }
    }
    0
}

pub fn human_bytes(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u < UNITS.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    format!("{v:.1} {}", UNITS[u])
}
