//! Stands in for the CPAL callback: hands over a fixed number of frames every
//! callback period and never blocks. If the ring is full it drops the chunk and
//! counts it, exactly as a real overrun would — capture is not allowed to wait
//! for the database (§10).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use rtrb::Producer;

use crate::config::Params;

pub struct ProducerStats {
    pub frames_produced: AtomicU64,
    pub frames_dropped: AtomicU64,
    pub overrun_events: AtomicU64,
    pub worst_late_us: AtomicU64,
}

impl ProducerStats {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            frames_produced: AtomicU64::new(0),
            frames_dropped: AtomicU64::new(0),
            overrun_events: AtomicU64::new(0),
            worst_late_us: AtomicU64::new(0),
        })
    }
}

pub fn run(p: Params, mut ring: Producer<u8>, stats: Arc<ProducerStats>, stop: Arc<AtomicBool>) {
    let frame_bytes = p.frame_bytes();
    let chunk_frames = p.callback_frames as u64;
    let chunk_bytes = chunk_frames as usize * frame_bytes;
    let mut scratch = vec![0u8; chunk_bytes];

    let period = Duration::from_secs_f64(chunk_frames as f64 / (p.rate as f64 * p.rate_multiplier));

    let start = Instant::now();
    let mut frame: u64 = 0;
    let mut next_deadline = start;

    while !stop.load(Ordering::Relaxed) {
        // Fill the callback buffer with the deterministic pattern.
        let mut off = 0usize;
        for f in 0..chunk_frames {
            for ch in 0..p.channels {
                let v = Params::expected_sample(frame + f, ch);
                Params::write_sample(&mut scratch[off..], v, p.bytes_per_sample);
                off += p.bytes_per_sample;
            }
        }

        if ring.slots() >= chunk_bytes {
            match ring.write_chunk_uninit(chunk_bytes) {
                Ok(chunk) => {
                    chunk.fill_from_iter(scratch.iter().copied());
                    stats
                        .frames_produced
                        .fetch_add(chunk_frames, Ordering::Relaxed);
                }
                Err(_) => {
                    stats
                        .frames_dropped
                        .fetch_add(chunk_frames, Ordering::Relaxed);
                    stats.overrun_events.fetch_add(1, Ordering::Relaxed);
                }
            }
        } else {
            stats
                .frames_dropped
                .fetch_add(chunk_frames, Ordering::Relaxed);
            stats.overrun_events.fetch_add(1, Ordering::Relaxed);
        }

        frame += chunk_frames;
        next_deadline += period;
        let now = Instant::now();
        if next_deadline > now {
            std::thread::sleep(next_deadline - now);
        } else {
            let late = (now - next_deadline).as_micros() as u64;
            stats.worst_late_us.fetch_max(late, Ordering::Relaxed);
        }
    }
}
