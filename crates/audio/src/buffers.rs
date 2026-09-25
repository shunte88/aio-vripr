/*
 *  buffers.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Bounded lock-free PCM distribution from the callback to the workers (§10).
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

//! Bounded lock-free PCM distribution from the callback to the workers (§10).
//!
//! One SPSC ring of bytes, written by the audio callback and drained by the
//! writer thread. Bytes rather than samples because D4 stores what the device
//! gave us: the ring has no opinion about sample width, and a format it has
//! never heard of travels through it unharmed.
//!
//! # Why the capacity does not matter much
//!
//! S2 inverted the expected answer here. Crash loss is *commit granularity plus
//! the driver buffer*, and the ring contributes nothing to it - samples sitting
//! in the ring when the power goes are lost whether the ring holds 500 ms or ten
//! seconds. So capacity buys exactly one thing, jitter tolerance, and the
//! recovery budget is spent on commit granularity instead. [`MIN_MILLIS`] is
//! the floor S2 settled on; making it larger is not an improvement, it is a
//! larger window of samples to lose.
//!
//! # Whole chunks only
//!
//! [`RingWriter::push`] writes all of a buffer or none of it. A partial write
//! would leave half a frame in the ring and silently shift every channel in
//! everything that followed - a fault that survives into the archive and is
//! nearly impossible to diagnose later. A dropped callback is a counted, visible
//! gap; a torn frame is corruption that looks like audio.

use rtrb::{Consumer, Producer, RingBuffer};

/// The smallest ring S2 found tolerable at 24/192 (D3).
///
/// Below this, ordinary writer jitter starts costing samples. Above it, nothing
/// improves: see the module note.
pub const MIN_MILLIS: u32 = 500;

/// What [`RingWriter`] and [`RingReader`] are built with when nobody has a reason
/// to ask for something else.
pub const DEFAULT_MILLIS: u32 = 1_000;

/// Bytes needed to hold `millis` of audio at this frame size and rate.
///
/// Rounds up, and never returns zero: a ring of no bytes would turn every
/// callback into an overrun, which is a confusing way to report bad arithmetic.
#[must_use]
pub fn bytes_for(frame_bytes: usize, rate: u32, millis: u32) -> usize {
    let frames = (u64::from(rate) * u64::from(millis)).div_ceil(1_000);
    let bytes = frames.saturating_mul(frame_bytes as u64);
    usize::try_from(bytes)
        .unwrap_or(usize::MAX)
        .max(frame_bytes.max(1))
}

/// Creates a ring sized for a duration, and returns its two ends.
///
/// `millis` is raised to [`MIN_MILLIS`] if a caller asks for less. The floor is
/// not negotiable from the outside because the cost of being under it is dropped
/// samples and the benefit is nothing.
#[must_use]
pub fn ring(frame_bytes: usize, rate: u32, millis: u32) -> (RingWriter, RingReader) {
    let frame_bytes = frame_bytes.max(1);
    let capacity = bytes_for(frame_bytes, rate, millis.max(MIN_MILLIS));
    let (producer, consumer) = RingBuffer::<u8>::new(capacity);
    (
        RingWriter {
            inner: producer,
            frame_bytes,
        },
        RingReader {
            inner: consumer,
            frame_bytes,
        },
    )
}

/// The callback's end of the ring. Wait-free, and never blocks.
pub struct RingWriter {
    inner: Producer<u8>,
    frame_bytes: usize,
}

impl RingWriter {
    /// Writes the whole buffer, or nothing at all.
    ///
    /// `false` means the reader has fallen behind and this callback's samples are
    /// gone. The caller counts it; see the module note on why a partial write is
    /// not the kinder option.
    ///
    /// Allocation-free and lock-free: `tests/rt_safety.rs` asserts the first of
    /// those against a counting allocator rather than taking it on trust.
    pub fn push(&mut self, bytes: &[u8]) -> bool {
        if bytes.is_empty() {
            return true;
        }
        if self.inner.slots() < bytes.len() {
            return false;
        }
        match self.inner.write_chunk_uninit(bytes.len()) {
            Ok(chunk) => {
                chunk.fill_from_iter(bytes.iter().copied());
                true
            }
            Err(_) => false,
        }
    }

    /// Bytes that would be accepted right now.
    pub fn free_bytes(&self) -> usize {
        self.inner.slots()
    }

    /// Total size of the ring, in bytes.
    pub fn capacity(&self) -> usize {
        self.inner.buffer().capacity()
    }

    /// Bytes in one frame: one sample for every channel.
    pub const fn frame_bytes(&self) -> usize {
        self.frame_bytes
    }

    /// Whether the reader has gone away, which is how a stopped writer thread
    /// reaches the callback.
    pub fn is_abandoned(&self) -> bool {
        self.inner.is_abandoned()
    }
}

/// The writer thread's end of the ring.
pub struct RingReader {
    inner: Consumer<u8>,
    frame_bytes: usize,
}

impl RingReader {
    /// Bytes waiting to be drained.
    pub fn available(&self) -> usize {
        self.inner.slots()
    }

    /// Whole frames waiting to be drained.
    pub fn available_frames(&self) -> usize {
        self.available() / self.frame_bytes
    }

    /// Fills as much of `dst` as there is data for, and reports how many bytes
    /// that was.
    pub fn read(&mut self, dst: &mut [u8]) -> usize {
        let n = self.available().min(dst.len());
        if n == 0 {
            return 0;
        }
        match self.inner.read_chunk(n) {
            Ok(chunk) => {
                let (first, second) = chunk.as_slices();
                dst[..first.len()].copy_from_slice(first);
                dst[first.len()..n].copy_from_slice(&second[..n - first.len()]);
                chunk.commit_all();
                n
            }
            Err(_) => 0,
        }
    }

    /// Fills `dst` completely or takes nothing, so a caller assembling
    /// fixed-size blocks cannot end up with a short one it mistakes for a full
    /// one.
    pub fn read_exact(&mut self, dst: &mut [u8]) -> bool {
        if self.available() < dst.len() {
            return false;
        }
        self.read(dst) == dst.len()
    }

    /// Whether the callback has gone away and no more data will arrive.
    pub fn is_abandoned(&self) -> bool {
        self.inner.is_abandoned()
    }

    /// Bytes in one frame: one sample for every channel.
    pub const fn frame_bytes(&self) -> usize {
        self.frame_bytes
    }
}

/// The ring is what WP-05's writer drains.
///
/// The trait lives in `vcw-types` so that `vcw-project` can consume a ring
/// without depending on this crate, and so that the same writer can be driven by
/// a file or a generator in CI. `is_abandoned` is exactly the "producer has gone
/// for good" signal the trait asks for: `rtrb` sets it when the writing end is
/// dropped, which is what happens when the capture stream stops.
impl vcw_types::PcmSource for RingReader {
    fn read(&mut self, dst: &mut [u8]) -> usize {
        Self::read(self, dst)
    }

    fn is_finished(&self) -> bool {
        self.is_abandoned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2 ch of 24-bit packed, the archival case D4 cares most about.
    const FRAME: usize = 6;

    #[test]
    fn a_ring_holds_the_duration_it_was_asked_for() {
        // 500 ms of 2 ch 24-bit at 96 kHz: 48000 frames x 6 bytes.
        assert_eq!(bytes_for(FRAME, 96_000, 500), 48_000 * FRAME);
        // Rounding is upwards, so the ring is never a frame short of the ask.
        assert_eq!(bytes_for(FRAME, 44_100, 1), 45 * FRAME);
    }

    #[test]
    fn a_ring_is_never_zero_bytes() {
        assert!(bytes_for(FRAME, 0, 0) >= FRAME);
        let (w, _r) = ring(FRAME, 0, 0);
        assert!(w.capacity() >= FRAME);
    }

    #[test]
    fn asking_for_less_than_the_floor_gets_the_floor() {
        let (small, _) = ring(FRAME, 96_000, 10);
        let (floor, _) = ring(FRAME, 96_000, MIN_MILLIS);
        assert_eq!(small.capacity(), floor.capacity());
    }

    #[test]
    fn what_goes_in_comes_out_unaltered() {
        let (mut w, mut r) = ring(FRAME, 48_000, MIN_MILLIS);
        let data: Vec<u8> = (0..=255u8).cycle().take(FRAME * 100).collect();
        assert!(w.push(&data));
        let mut out = vec![0u8; data.len()];
        assert!(r.read_exact(&mut out));
        assert_eq!(out, data);
    }

    #[test]
    fn a_full_ring_refuses_the_whole_chunk_and_never_a_part_of_it() {
        let (mut w, mut r) = ring(FRAME, 1_000, MIN_MILLIS); // 500 frames
        let capacity = w.capacity();
        let chunk = vec![0xAAu8; capacity];
        assert!(w.push(&chunk), "an exactly-full write should fit");
        assert_eq!(w.free_bytes(), 0);

        // The overrun. Nothing of this may land.
        assert!(!w.push(&[0xBB; FRAME]));
        let mut out = vec![0u8; capacity];
        assert!(r.read_exact(&mut out));
        assert!(
            out.iter().all(|&b| b == 0xAA),
            "a refused push must leave no trace behind"
        );
    }

    #[test]
    fn frame_alignment_survives_an_overrun() {
        // The fault this whole design exists to prevent: after a dropped
        // callback the next frame must still start on a frame boundary.
        let (mut w, mut r) = ring(FRAME, 1_000, MIN_MILLIS);
        let capacity = w.capacity();
        assert!(w.push(&vec![1u8; capacity - FRAME]));
        assert!(!w.push(&[2u8; FRAME * 2]), "two frames cannot fit in one");
        assert!(w.push(&[3u8; FRAME]), "one frame still can");

        let mut out = vec![0u8; capacity];
        assert!(r.read_exact(&mut out));
        assert_eq!(out.len() % FRAME, 0);
        assert_eq!(&out[capacity - FRAME..], &[3u8; FRAME]);
    }

    #[test]
    fn a_partial_read_takes_what_is_there_and_an_exact_read_takes_nothing() {
        let (mut w, mut r) = ring(FRAME, 48_000, MIN_MILLIS);
        assert!(w.push(&[7u8; FRAME * 3]));

        let mut too_big = [0u8; FRAME * 10];
        assert!(!r.read_exact(&mut too_big), "not enough for an exact read");
        assert_eq!(
            r.available(),
            FRAME * 3,
            "a refused exact read consumes nothing"
        );
        assert_eq!(r.read(&mut too_big), FRAME * 3);
        assert_eq!(r.available(), 0);
    }

    #[test]
    fn a_wrapped_read_reassembles_both_halves_in_order() {
        // Drive the ring past its end so read_chunk returns two slices.
        let (mut w, mut r) = ring(FRAME, 1_000, MIN_MILLIS);
        let capacity = w.capacity();
        assert!(w.push(&vec![0u8; capacity - FRAME]));
        let mut drain = vec![0u8; capacity - FRAME];
        assert!(r.read_exact(&mut drain));

        let payload: Vec<u8> = (0..FRAME as u8 * 4).collect();
        assert!(w.push(&payload));
        let mut out = vec![0u8; payload.len()];
        assert!(r.read_exact(&mut out));
        assert_eq!(out, payload, "the two halves must come back in order");
    }

    #[test]
    fn counts_are_reported_in_frames_as_well_as_bytes() {
        let (mut w, r) = ring(FRAME, 48_000, MIN_MILLIS);
        assert!(w.push(&[0u8; FRAME * 5]));
        assert_eq!(r.available(), FRAME * 5);
        assert_eq!(r.available_frames(), 5);
        assert_eq!(r.frame_bytes(), FRAME);
        assert_eq!(w.frame_bytes(), FRAME);
    }

    #[test]
    fn an_empty_push_is_a_no_op_and_not_an_overrun() {
        let (mut w, r) = ring(FRAME, 48_000, MIN_MILLIS);
        assert!(w.push(&[]));
        assert_eq!(r.available(), 0);
    }

    #[test]
    fn each_end_notices_when_the_other_is_dropped() {
        let (w, r) = ring(FRAME, 48_000, MIN_MILLIS);
        assert!(!w.is_abandoned());
        drop(r);
        assert!(w.is_abandoned());

        let (w, r) = ring(FRAME, 48_000, MIN_MILLIS);
        assert!(!r.is_abandoned());
        drop(w);
        assert!(r.is_abandoned());
    }
}
