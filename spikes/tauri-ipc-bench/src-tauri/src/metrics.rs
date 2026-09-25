/*
 *  metrics.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Histograms and process memory, kept deliberately small: the bench must not
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

//! Histograms and process memory, kept deliberately small: the bench must not
//! perturb what it measures.

use serde::Serialize;

/// Fixed-bucket microsecond histogram. No allocation after construction, so it
/// is safe to record from the producer thread on every send.
#[derive(Default)]
pub struct Hist {
    /// Bucket i holds counts for [2^i, 2^(i+1)) microseconds.
    buckets: [u64; 32],
    count: u64,
    sum: u64,
    max: u64,
}

impl Hist {
    pub fn record(&mut self, us: u64) {
        let i = if us == 0 {
            0
        } else {
            63 - us.leading_zeros() as usize
        };
        self.buckets[i.min(31)] += 1;
        self.count += 1;
        self.sum += us;
        self.max = self.max.max(us);
    }

    /// Upper edge of the bucket the given percentile falls in. Log buckets mean
    /// this is an over-estimate bounded by a factor of two, which is honest
    /// enough for "does it fit in 16 ms" and cheap enough to record inline.
    fn pct_us(&self, p: f64) -> u64 {
        if self.count == 0 {
            return 0;
        }
        let target = (self.count as f64 * p).ceil() as u64;
        let mut seen = 0u64;
        for (i, &c) in self.buckets.iter().enumerate() {
            seen += c;
            if seen >= target {
                return 1u64 << (i + 1);
            }
        }
        self.max
    }

    pub fn summary(&self) -> HistSummary {
        HistSummary {
            count: self.count,
            mean_us: if self.count == 0 {
                0.0
            } else {
                self.sum as f64 / self.count as f64
            },
            p50_us_upper: self.pct_us(0.50),
            p99_us_upper: self.pct_us(0.99),
            max_us: self.max,
        }
    }
}

#[derive(Serialize, Clone, Copy, Debug, Default)]
pub struct HistSummary {
    pub count: u64,
    pub mean_us: f64,
    /// Log-bucketed, so this is an upper bound within a factor of two.
    pub p50_us_upper: u64,
    pub p99_us_upper: u64,
    pub max_us: u64,
}

#[derive(Serialize, Clone, Copy, Debug, Default)]
pub struct Memory {
    pub self_rss_kib: u64,
    /// WebKitGTK runs the web content and network in separate processes, so the
    /// interesting number is the whole tree, not our own RSS.
    pub children_rss_kib: u64,
    pub total_rss_kib: u64,
    pub process_count: u32,
}

fn rss_kib(pid: u32) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    s.lines()
        .find(|l| l.starts_with("VmRSS:"))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

fn children_of(pid: u32) -> Vec<u32> {
    let mut out = Vec::new();
    // /proc/<pid>/task/<tid>/children is the cheap way to walk the tree; it is
    // Linux-specific, which is fine for a spike whose numbers are Linux-specific
    // anyway.
    if let Ok(tasks) = std::fs::read_dir(format!("/proc/{pid}/task")) {
        for t in tasks.flatten() {
            if let Ok(s) = std::fs::read_to_string(t.path().join("children")) {
                out.extend(s.split_whitespace().filter_map(|v| v.parse::<u32>().ok()));
            }
        }
    }
    out
}

pub fn memory() -> Memory {
    let me = std::process::id();
    let mut m = Memory {
        self_rss_kib: rss_kib(me).unwrap_or(0),
        ..Default::default()
    };
    m.process_count = 1;

    let mut stack = children_of(me);
    while let Some(pid) = stack.pop() {
        if let Some(k) = rss_kib(pid) {
            m.children_rss_kib += k;
            m.process_count += 1;
        }
        stack.extend(children_of(pid));
    }
    m.total_rss_kib = m.self_rss_kib + m.children_rss_kib;
    m
}
