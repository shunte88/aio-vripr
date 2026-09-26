/*
 *  chunks.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  The playback queue: fixed buffers, epoch-tagged, recycled rather than allocated.
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

//! The playback queue: fixed buffers, epoch-tagged, recycled rather than allocated.
//!
//! Capture's ring ([`crate::buffers`]) is a byte ring, and that is right for
//! capture: bytes arrive, bytes are written, and nothing ever needs to know
//! *which* bytes are in the ring. Playback is not symmetrical with that, for one
//! reason.
//!
//! **A seek has to discard what is already buffered.** A byte ring cannot do it.
//! The producer cannot clear a ring it does not consume, so a seek would leave
//! the whole ring's worth of audio from the *old* position queued in front of the
//! new - up to a second of it at capture's ring size. That is not a gap, which
//! would at least be obvious; it is a second of the wrong music, and then a jump.
//!
//! So the unit here is a chunk, not a byte, and every chunk carries the epoch it
//! was filled in and the frame it starts at:
//!
//! - **Seeking** bumps the epoch. The callback discards any chunk that does not
//!   match, immediately and without playing it, so the stale audio never reaches
//!   the converter however much of it was queued.
//! - **Position** is exact rather than inferred. The callback knows the frame
//!   number of the audio it is rendering *now*, so a playhead does not have to
//!   be corrected for the depth of a buffer it cannot see.
//!
//! # Where the memory comes from
//!
//! Every buffer is allocated once, at [`queue`], and then circulates: the
//! callback hands a spent chunk back through a second queue and the feeder fills
//! it again. Chunks are never created, never dropped and never resized while
//! audio is running, which is what lets the callback's whole body be a memcpy -
//! the same real-time contract capture's [`crate::capture::Sink`] holds, and
//! `tests/rt_safety.rs` asserts it the same way.
//!
//! Both queues are `rtrb`, and both are used strictly single-producer
//! single-consumer: full chunks travel feeder → callback, spent chunks travel
//! callback → feeder. Neither direction ever blocks or allocates.

use rtrb::{Consumer, Producer, RingBuffer};

/// How much audio one chunk holds.
///
/// This is the seek granularity as well as the buffering unit: after an epoch
/// bump the callback has nothing valid to play until the feeder produces a whole
/// chunk, so a shorter chunk means a shorter join. 20 ms is short enough that
/// the join is under one device period on every configuration measured, and long
/// enough that the feeder wakes a manageable fifty times a second per stream.
pub const CHUNK_MILLIS: u32 = 20;

/// How many chunks circulate, and therefore how far ahead the feeder may run.
///
/// Eight chunks is 160 ms of slack against a feeder that has to read SQLite -
/// generous next to the 3.6 ms a warm 250 ms block read measures, and cheap:
/// 160 ms of 24/192 stereo is 184 KB.
pub const QUEUE_CHUNKS: usize = 8;

/// Bytes in a chunk at a given frame size and rate.
///
/// Never zero, and never smaller than one frame: a chunk that cannot hold a
/// frame would make every callback an underrun.
#[must_use]
pub fn chunk_bytes(frame_bytes: usize, rate: u32) -> usize {
    let frame_bytes = frame_bytes.max(1);
    let frames = (u64::from(rate) * u64::from(CHUNK_MILLIS))
        .div_ceil(1_000)
        .max(1);
    usize::try_from(frames.saturating_mul(frame_bytes as u64))
        .unwrap_or(usize::MAX)
        .max(frame_bytes)
}

/// One buffer of audio, on its way to the device or back for refilling.
///
/// Holds audio in the *device's* format, not the project's: conversion happens
/// on the feeder thread, so the callback never does arithmetic on a sample. See
/// [`crate::playback::Source`].
pub struct Chunk {
    epoch: u64,
    start_frame: u64,
    buffer: Box<[u8]>,
    filled: usize,
}

impl Chunk {
    /// An empty chunk of a fixed size.
    #[must_use]
    pub fn new(bytes: usize) -> Self {
        Self {
            epoch: 0,
            start_frame: 0,
            buffer: vec![0u8; bytes.max(1)].into_boxed_slice(),
            filled: 0,
        }
    }

    /// The whole buffer, for the feeder to fill.
    pub fn spare_mut(&mut self) -> &mut [u8] {
        &mut self.buffer
    }

    /// Records what was filled, and where it belongs.
    ///
    /// `filled` is clamped to the buffer, so a feeder that miscounts produces
    /// short audio rather than a panic on the thread reading the project.
    pub fn mark(&mut self, epoch: u64, start_frame: u64, filled: usize) {
        self.epoch = epoch;
        self.start_frame = start_frame;
        self.filled = filled.min(self.buffer.len());
    }

    /// Marks the chunk as holding nothing, without changing its buffer.
    pub fn clear(&mut self) {
        self.filled = 0;
    }

    /// The audio in it.
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.buffer[..self.filled]
    }

    /// How many bytes it could hold.
    #[must_use]
    pub const fn capacity(&self) -> usize {
        self.buffer.len()
    }

    /// The epoch it was filled in.
    #[must_use]
    pub const fn epoch(&self) -> u64 {
        self.epoch
    }

    /// The capture frame its first frame is.
    #[must_use]
    pub const fn start_frame(&self) -> u64 {
        self.start_frame
    }

    /// Whether it holds nothing.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.filled == 0
    }
}

impl std::fmt::Debug for Chunk {
    /// Without the audio, which is neither readable nor interesting.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Chunk")
            .field("epoch", &self.epoch)
            .field("start_frame", &self.start_frame)
            .field("filled", &self.filled)
            .field("capacity", &self.buffer.len())
            .finish()
    }
}

/// Builds a queue of `chunks` buffers of `bytes` each, and returns its two ends.
///
/// Every buffer is allocated here and nowhere else. The feeder starts holding
/// all of them, because none of them have anything in yet.
#[must_use]
pub fn queue(bytes: usize, chunks: usize) -> (Feeder, Drain) {
    let chunks = chunks.max(2);
    let (full_in, full_out) = RingBuffer::<Chunk>::new(chunks);
    let (spent_in, spent_out) = RingBuffer::<Chunk>::new(chunks);
    let mut feeder = Feeder {
        full: full_in,
        spent: spent_out,
        spare: Vec::with_capacity(chunks),
        chunk_bytes: bytes.max(1),
        reserve: 0,
        released: false,
    };
    for _ in 0..chunks {
        feeder.spare.push(Chunk::new(bytes));
    }
    (
        feeder,
        Drain {
            full: full_out,
            spent: spent_in,
        },
    )
}

/// The feeder's end: takes empty chunks, fills them, sends them on.
///
/// Lives on an ordinary thread that reads the project, so it may allocate, block
/// and take as long as it likes. It just has to keep up on average.
pub struct Feeder {
    full: Producer<Chunk>,
    spent: Consumer<Chunk>,
    /// Chunks in hand, not yet filled. Recovered from `spent` as the callback
    /// finishes with them.
    spare: Vec<Chunk>,
    chunk_bytes: usize,
    /// Chunks never spent on ordinary filling, so that a seek has something to
    /// fill. See [`Feeder::hold_back`].
    reserve: usize,
    /// Whether the reserve is currently spendable, set by
    /// [`Feeder::release_reserve`] and cleared as soon as the pool recovers.
    released: bool,
}

impl Feeder {
    /// An empty chunk to fill, or `None` if the callback holds them all.
    ///
    /// `None` is the ordinary "the queue is full, wait" signal, not an error:
    /// the feeder has run as far ahead as the queue allows. A held-back reserve
    /// counts as held by the callback until [`Feeder::release_reserve`] says
    /// otherwise.
    pub fn take(&mut self) -> Option<Chunk> {
        self.collect_spent();
        if self.spare.len() > self.reserve {
            // The pool has recovered on its own, so the reserve is intact
            // again whether or not anything spent it.
            self.released = false;
        } else if !self.released {
            return None;
        }
        self.spare.pop()
    }

    /// Keeps `reserve` chunks back from ordinary filling.
    ///
    /// The reserve is what makes a seek gapless. Without it the feeder runs the
    /// queue full, and a seek then invalidates every chunk in it: the callback
    /// discards the lot in one pass, finds nothing behind them, and plays a
    /// buffer of silence before the feeder can get a word in - it holds no
    /// empty chunk to fill, and cannot take one back out of an SPSC queue it is
    /// the producer of. Holding one callback's worth back means the feeder can
    /// queue the new position *behind* the audio the seek invalidated, so the
    /// callback discards the stale chunks and keeps reading in the same pass.
    ///
    /// Capped so that at least two chunks stay spendable; a queue that can
    /// never be filled is worse than a gap.
    pub fn hold_back(&mut self, reserve: usize) {
        self.reserve = reserve.min(self.spare.len().saturating_sub(2));
    }

    /// Lets the next fills come out of the reserve.
    ///
    /// Called when the feeder notices the epoch has changed, which is the one
    /// moment the reserve is for. It lapses as soon as the callback has handed
    /// enough chunks back.
    pub fn release_reserve(&mut self) {
        self.released = true;
    }

    /// How many chunks are held back from ordinary filling.
    #[must_use]
    pub const fn reserve(&self) -> usize {
        self.reserve
    }

    /// Hands a filled chunk to the callback.
    ///
    /// Returns the chunk on failure, which can only happen if the callback has
    /// gone away, because a chunk only exists if it was taken from this queue.
    pub fn send(&mut self, chunk: Chunk) -> Result<(), Chunk> {
        self.full.push(chunk).map_err(|rtrb::PushError::Full(c)| c)
    }

    /// Takes back a chunk without filling it, so an exiting feeder does not
    /// shrink the pool.
    pub fn give_back(&mut self, mut chunk: Chunk) {
        chunk.clear();
        self.spare.push(chunk);
    }

    /// How many filled chunks are waiting to be played.
    ///
    /// `rtrb` reports a producer's *free* slots, so the depth of the queue is
    /// what is left over. This is the number a starvation diagnosis turns on:
    /// a feeder that is keeping up leaves it near the queue size.
    #[must_use]
    pub fn queued(&self) -> usize {
        self.full
            .buffer()
            .capacity()
            .saturating_sub(self.full.slots())
    }

    /// How many empty chunks are in hand right now.
    #[must_use]
    pub fn spare(&self) -> usize {
        self.spare.len()
    }

    /// Bytes in a chunk from this queue.
    #[must_use]
    pub const fn chunk_bytes(&self) -> usize {
        self.chunk_bytes
    }

    /// Whether the callback has gone.
    #[must_use]
    pub fn is_abandoned(&self) -> bool {
        self.full.is_abandoned()
    }

    /// Recovers everything the callback has finished with.
    fn collect_spent(&mut self) {
        while let Ok(mut chunk) = self.spent.pop() {
            chunk.clear();
            self.spare.push(chunk);
        }
    }
}

/// The callback's end: takes filled chunks, hands the spent ones back.
///
/// Every method here is wait-free and allocation-free, because this is the end
/// that runs on the audio thread.
pub struct Drain {
    full: Consumer<Chunk>,
    spent: Producer<Chunk>,
}

impl Drain {
    /// The next filled chunk, if there is one.
    pub fn next_chunk(&mut self) -> Option<Chunk> {
        self.full.pop().ok()
    }

    /// Returns a chunk for refilling.
    ///
    /// A chunk that cannot be returned is dropped, which shrinks the pool by
    /// one. Unreachable by construction - the spent queue is as large as the
    /// number of chunks - and preferable to blocking the audio thread over.
    pub fn recycle(&mut self, chunk: Chunk) {
        let _ = self.spent.push(chunk);
    }

    /// How many filled chunks are waiting.
    #[must_use]
    pub fn queued(&self) -> usize {
        self.full.slots()
    }

    /// Whether the feeder has gone.
    #[must_use]
    pub fn is_abandoned(&self) -> bool {
        self.full.is_abandoned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reserve_is_what_a_seek_fills_and_ordinary_running_cannot_touch() {
        // The measured failure this pins: five seeks on a real device cost
        // five underruns and 34,560 frames of silence, one buffer per seek,
        // because the feeder had run the queue full and held no empty chunk to
        // put the new position in.
        let (mut feeder, mut drain) = queue(64, 9);
        feeder.hold_back(4);
        assert_eq!(feeder.reserve(), 4);

        // Ordinary running stops at the reserve, whatever the callback does.
        let mut sent = 0;
        while let Some(mut chunk) = feeder.take() {
            chunk.mark(0, sent as u64, 64);
            feeder.send(chunk).expect("the drain is alive");
            sent += 1;
        }
        assert_eq!(sent, 5, "ordinary filling spent the reserve");
        assert_eq!(feeder.spare(), 4, "the reserve is not in hand");

        // A seek. Everything queued is now stale, and nothing has come back.
        feeder.release_reserve();
        let mut fresh = 0;
        while let Some(mut chunk) = feeder.take() {
            chunk.mark(1, fresh as u64, 64);
            feeder.send(chunk).expect("the drain is alive");
            fresh += 1;
        }
        assert_eq!(fresh, 4, "the seek could not fill a callback's worth");

        // And the callback finds the new epoch behind the old one rather than
        // finding nothing, which is the whole point.
        let mut played = 0;
        while let Some(chunk) = drain.next_chunk() {
            if chunk.epoch() == 1 {
                played += 1;
            }
            drain.recycle(chunk);
        }
        assert_eq!(played, 4);

        // Once the callback has handed them back the reserve is intact again,
        // without anybody having to say so.
        assert!(feeder.take().is_some());
        assert_eq!(feeder.spare(), 8);
    }

    #[test]
    fn a_reserve_can_never_swallow_the_whole_queue() {
        let (mut feeder, _drain) = queue(64, 4);
        feeder.hold_back(99);
        assert_eq!(feeder.reserve(), 2, "two chunks always stay spendable");
        assert!(feeder.take().is_some());
        assert!(feeder.take().is_some());
        assert!(feeder.take().is_none());
    }

    #[test]
    fn a_chunk_is_twenty_milliseconds_of_whatever_it_is_carrying() {
        // 8 bytes per frame is stereo at the 32-bit width capture stores 24-bit
        // in, which is the widest thing playback will be handed.
        assert_eq!(chunk_bytes(8, 48_000), 960 * 8);
        assert_eq!(chunk_bytes(8, 192_000), 3_840 * 8);
        // Nonsense in, something usable out: a chunk that cannot hold a frame
        // would turn every callback into an underrun.
        assert_eq!(chunk_bytes(8, 0), 8);
        assert_eq!(chunk_bytes(0, 48_000), 960);
    }

    #[test]
    fn every_buffer_is_allocated_once_and_then_circulates() {
        // The property the real-time contract rests on. Chunks are conserved:
        // the feeder starts with all of them, and after any amount of traffic
        // the total is still the number `queue` made.
        let (mut feeder, mut drain) = queue(64, 4);
        assert_eq!(feeder.spare(), 4);
        assert_eq!(drain.queued(), 0);

        for round in 0..100u64 {
            while let Some(mut chunk) = feeder.take() {
                chunk.mark(round, round * 8, 64);
                feeder.send(chunk).expect("send");
            }
            assert_eq!(drain.queued(), 4);
            while let Some(chunk) = drain.next_chunk() {
                drain.recycle(chunk);
            }
        }
        feeder.take().expect("a chunk came back");
        assert_eq!(feeder.spare() + 1, 4, "a chunk was lost in circulation");
    }

    #[test]
    fn a_chunk_carries_where_it_came_from() {
        let (mut feeder, mut drain) = queue(32, 2);
        let mut chunk = feeder.take().expect("take");
        assert_eq!(chunk.capacity(), 32);
        chunk.spare_mut()[..4].copy_from_slice(&[1, 2, 3, 4]);
        chunk.mark(7, 48_000, 4);
        feeder.send(chunk).expect("send");

        let played = drain.next_chunk().expect("chunk");
        assert_eq!(played.epoch(), 7);
        assert_eq!(played.start_frame(), 48_000);
        assert_eq!(played.bytes(), &[1, 2, 3, 4]);
        assert!(!played.is_empty());
    }

    #[test]
    fn a_recycled_chunk_comes_back_empty() {
        // Otherwise a feeder that fills a chunk short would leave the tail of
        // the previous fill behind it, and play it.
        let (mut feeder, mut drain) = queue(32, 2);
        let mut chunk = feeder.take().expect("take");
        chunk.spare_mut().fill(0xAB);
        chunk.mark(1, 0, 32);
        feeder.send(chunk).expect("send");
        let played = drain.next_chunk().expect("chunk");
        drain.recycle(played);

        let back = feeder.take().expect("recovered");
        assert!(back.is_empty());
        assert_eq!(back.bytes().len(), 0);
    }

    #[test]
    fn a_feeder_that_overfills_is_clamped_rather_than_trusted() {
        let (mut feeder, _drain) = queue(16, 2);
        let mut chunk = feeder.take().expect("take");
        chunk.mark(1, 0, 9_999);
        assert_eq!(chunk.bytes().len(), 16);
    }

    #[test]
    fn a_queue_cannot_be_built_too_small_to_work() {
        // One chunk means the feeder and the callback cannot both hold one, so
        // the stream would stall on the first callback.
        let (feeder, _drain) = queue(16, 0);
        assert_eq!(feeder.spare(), 2);
    }

    #[test]
    fn each_end_notices_when_the_other_goes_away() {
        let (feeder, drain) = queue(16, 2);
        drop(drain);
        assert!(feeder.is_abandoned());

        let (feeder, drain) = queue(16, 2);
        drop(feeder);
        assert!(drain.is_abandoned());
    }
}
