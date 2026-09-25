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
//! # The fan-out, and where it sits
//!
//! §10 draws one distribution point feeding the writer, the meter, the waveform
//! and the detector. [`Tee`] is that point, and it sits **after** the ring
//! rather than inside the callback. The callback keeps its three atomics and one
//! memcpy; the writer thread, which has to touch every byte anyway, copies what
//! it read into each tap on its way past.
//!
//! Two reasons, one practical and one that decided it. The practical one: a tap
//! inside the callback would have to exist before the stream is built, so every
//! consumer would have to be known at open time. The deciding one: the
//! simulated source has no callback and no ring at all, and a fan-out that only
//! worked for real hardware would mean the UI could not be developed against it
//! - which is exactly what §4.5 asks to be possible.
//!
//! The cost is stated rather than hidden: a writer thread that stalls freezes
//! the meter with it. That is the right direction for the trade, because the
//! reverse - a slow consumer costing a sample - is what §10 forbids. **Taps are
//! lossy by design.** A tap that falls behind drops what it cannot hold and
//! counts the bytes; the writer's ring is the only one where loss means lost
//! audio.
//!
//! # Whole chunks only
//!
//! [`RingWriter::push`] writes all of a buffer or none of it. A partial write
//! would leave half a frame in the ring and silently shift every channel in
//! everything that followed - a fault that survives into the archive and is
//! nearly impossible to diagnose later. A dropped callback is a counted, visible
//! gap; a torn frame is corruption that looks like audio.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// A lossy second reader of the capture stream (§10).
///
/// Handed out by [`Tee::tap`] and read by a worker thread. Falling behind is
/// not an error and is not reported as one: the bytes are dropped, the count is
/// kept, and the capture never notices. A meter that misses a buffer shows a
/// stale needle for 20 ms; a writer that misses one has lost the record.
pub struct Tap {
    reader: RingReader,
    dropped: Arc<AtomicU64>,
}

impl Tap {
    /// Takes up to `dst.len()` bytes. Never blocks.
    pub fn read(&mut self, dst: &mut [u8]) -> usize {
        self.reader.read(dst)
    }

    /// Bytes waiting to be read.
    pub fn available(&self) -> usize {
        self.reader.available()
    }

    /// Bytes this tap was not able to keep, over its whole life.
    ///
    /// Worth logging and not worth alarming about. A non-zero count means the
    /// worker on this end is slower than the stream, which for a meter is
    /// cosmetic.
    pub fn dropped_bytes(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }

    /// Whether the [`Tee`] has gone, which is how a worker learns to stop.
    pub fn is_abandoned(&self) -> bool {
        self.reader.is_abandoned()
    }
}

/// The writing end of a tap, held by the [`Tee`].
struct TapWriter {
    ring: RingWriter,
    dropped: Arc<AtomicU64>,
}

impl TapWriter {
    /// Offers bytes to the tap, dropping them if they will not fit.
    fn offer(&mut self, bytes: &[u8]) {
        if !self.ring.push(bytes) {
            self.dropped
                .fetch_add(bytes.len() as u64, Ordering::Relaxed);
        }
    }
}

/// One PCM stream, many readers: §10's distribution point.
///
/// Wraps any [`PcmSource`](vcw_types::PcmSource) - the capture ring, the
/// simulated generator, a file in a test - and copies whatever is read into
/// each tap. The wrapped source stays the authority: a tap sees exactly the
/// bytes the writer saw, in the order it saw them, and nothing else.
pub struct Tee<S> {
    inner: S,
    taps: Vec<TapWriter>,
}

impl<S> Tee<S> {
    /// Wraps a source with no taps attached. Adding none costs nothing.
    pub const fn new(inner: S) -> Self {
        Self {
            inner,
            taps: Vec::new(),
        }
    }

    /// Adds a tap of this capacity in bytes and returns the reading end.
    ///
    /// Size it for the worker's latency, not for the recording: a meter reading
    /// every 20 ms needs a few times that, and a bigger ring only means a
    /// staler needle when it does fall behind.
    pub fn tap(&mut self, bytes: usize) -> Tap {
        let (ring, reader) = raw_ring(bytes.max(1));
        let dropped = Arc::new(AtomicU64::new(0));
        self.taps.push(TapWriter {
            ring,
            dropped: Arc::clone(&dropped),
        });
        Tap { reader, dropped }
    }

    /// How many taps are attached.
    pub fn taps(&self) -> usize {
        self.taps.len()
    }

    /// The wrapped source.
    pub const fn inner(&self) -> &S {
        &self.inner
    }
}

impl<S: vcw_types::PcmSource> vcw_types::PcmSource for Tee<S> {
    fn read(&mut self, dst: &mut [u8]) -> usize {
        let read = self.inner.read(dst);
        if read > 0 {
            for tap in &mut self.taps {
                tap.offer(&dst[..read]);
            }
        }
        read
    }

    fn is_finished(&self) -> bool {
        self.inner.is_finished()
    }
}

/// A ring of an exact byte capacity, with no duration or floor applied.
///
/// The [`MIN_MILLIS`] floor exists to stop the *capture* ring being sized into
/// dropped samples. A tap has no such stake - dropping is what it is for - so
/// it is sized in bytes by the worker that will read it.
fn raw_ring(capacity: usize) -> (RingWriter, RingReader) {
    let (producer, consumer) = RingBuffer::<u8>::new(capacity.max(1));
    (
        RingWriter {
            inner: producer,
            frame_bytes: 1,
        },
        RingReader {
            inner: consumer,
            frame_bytes: 1,
        },
    )
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

    /// A source that hands out a counting ramp and then stops.
    struct Ramp {
        next: u8,
        left: usize,
    }

    impl vcw_types::PcmSource for Ramp {
        fn read(&mut self, dst: &mut [u8]) -> usize {
            let take = dst.len().min(self.left);
            for slot in &mut dst[..take] {
                *slot = self.next;
                self.next = self.next.wrapping_add(1);
            }
            self.left -= take;
            take
        }

        fn is_finished(&self) -> bool {
            self.left == 0
        }
    }

    #[test]
    fn a_tap_sees_exactly_what_the_writer_saw() {
        use vcw_types::PcmSource as _;

        let mut tee = Tee::new(Ramp { next: 0, left: 300 });
        let mut tap = tee.tap(1024);
        assert_eq!(tee.taps(), 1);

        let mut writer = Vec::new();
        let mut buffer = [0u8; 64];
        loop {
            let read = tee.read(&mut buffer);
            if read == 0 {
                break;
            }
            writer.extend_from_slice(&buffer[..read]);
        }

        let mut seen = vec![0u8; writer.len()];
        let got = tap.read(&mut seen);
        assert_eq!(got, writer.len(), "nothing should have been dropped");
        assert_eq!(seen, writer, "the tap must see the same bytes, in order");
        assert_eq!(tap.dropped_bytes(), 0);
        assert!(tee.is_finished());
    }

    #[test]
    fn a_tap_that_falls_behind_drops_and_counts_but_costs_nothing() {
        use vcw_types::PcmSource as _;

        // A tap far too small for what is about to go past it. The capture
        // must not notice, which is the whole point of §10's fan-out.
        let mut tee = Tee::new(Ramp {
            next: 0,
            left: 1_000,
        });
        let mut tap = tee.tap(64);

        let mut total = 0;
        let mut buffer = [0u8; 64];
        loop {
            let read = tee.read(&mut buffer);
            if read == 0 {
                break;
            }
            total += read;
        }
        assert_eq!(total, 1_000, "the writer reads every byte regardless");
        assert!(
            tap.dropped_bytes() > 0,
            "a tap this small cannot have kept up"
        );

        // What it did keep is still whole chunks, not shredded ones.
        let mut seen = [0u8; 64];
        assert_eq!(tap.read(&mut seen), 64);
        assert_eq!(tap.available(), 0);
    }

    #[test]
    fn two_taps_each_get_the_whole_stream() {
        use vcw_types::PcmSource as _;

        let mut tee = Tee::new(Ramp { next: 9, left: 96 });
        let mut meter = tee.tap(256);
        let mut waveform = tee.tap(256);
        assert_eq!(tee.taps(), 2);

        let mut buffer = [0u8; 96];
        assert_eq!(tee.read(&mut buffer), 96);

        let mut a = [0u8; 96];
        let mut b = [0u8; 96];
        assert_eq!(meter.read(&mut a), 96);
        assert_eq!(waveform.read(&mut b), 96);
        assert_eq!(a, buffer);
        assert_eq!(b, buffer);
    }

    #[test]
    fn a_tap_learns_that_the_capture_has_gone() {
        let mut tee = Tee::new(Ramp { next: 0, left: 0 });
        let tap = tee.tap(16);
        assert!(!tap.is_abandoned());
        drop(tee);
        assert!(tap.is_abandoned());
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
