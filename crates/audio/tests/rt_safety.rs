/*
 *  rt_safety.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Proving the §10 real-time contract instead of asserting it in a comment.
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

//! Proving the §10 real-time contract instead of asserting it in a comment.
//!
//! §10 says the callback shall not allocate, lock, or do I/O. Every audio
//! codebase says that; most of them are wrong somewhere, because nothing checks.
//! This file checks.
//!
//! **Allocation is measured.** A counting global allocator is installed for this
//! test binary, armed only around [`Sink::on_data`], and the assertion is that
//! the count does not move. `the_harness_itself_can_see_an_allocation` is the
//! control: without it, a broken counter would silently "prove" every other test
//! in the file.
//!
//! **Blocking is measured.** The consumer thread is parked and never drains, so
//! the ring fills and stays full. A callback that waits on a stalled reader
//! would hang here; a wait-free one overruns, counts it, and returns. That is
//! the operational property §10 is really asking for.
//!
//! What is *not* proven here is lock-freedom as a formal property. There is no
//! lock in the path - `rtrb` is a wait-free SPSC ring and the counters are
//! relaxed atomics - but that is an argument from construction, and this file
//! only claims what it measures.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use vcw_audio::buffers;
use vcw_audio::capture::{Counters, Sink};

/// An allocator that counts, but only while this thread has armed it.
///
/// Both the switch and the tally are thread-local and const-initialised: arming
/// cannot itself allocate, which would make the measurement measure the
/// measurement, and one test's armed window cannot pollute another's. Cargo runs
/// these in parallel, and a shared counter made the starvation test fail about
/// one run in five with two allocations it never made.
struct Counting;

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note();
        unsafe { System.alloc_zeroed(layout) }
    }
}

fn note() {
    // `try_with` rather than `with`: during thread teardown the local is gone,
    // and panicking inside the allocator would be a poor way to find out.
    let _ = ARMED.try_with(|armed| {
        if armed.get() {
            let _ = ALLOCATIONS.try_with(|n| n.set(n.get() + 1));
        }
    });
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// Runs `body` with the allocator counting, and reports how many it saw.
fn allocations_during(body: impl FnOnce()) -> u64 {
    // Touch the local first: its first access on a thread is not something we
    // want inside the measured window.
    ARMED.with(|a| a.set(false));
    let before = ALLOCATIONS.with(Cell::get);
    ARMED.with(|a| a.set(true));
    body();
    ARMED.with(|a| a.set(false));
    ALLOCATIONS.with(Cell::get) - before
}

/// 2 ch of 32-bit: 8 bytes a frame.
const FRAME: usize = 8;
const RATE: u32 = 192_000;

fn sink(millis: u32) -> (Sink, buffers::RingReader, Arc<Counters>) {
    let counters = Arc::new(Counters::default());
    let (writer, reader) = buffers::ring(FRAME, RATE, millis);
    (
        Sink::new(writer, Arc::clone(&counters), FRAME),
        reader,
        counters,
    )
}

#[test]
fn the_harness_itself_can_see_an_allocation() {
    // The control. If this ever fails, every other assertion in this file is
    // vacuous and the real-time contract is unproven rather than proven.
    let seen = allocations_during(|| {
        let v: Vec<u8> = Vec::with_capacity(4096);
        std::hint::black_box(&v);
    });
    assert!(seen > 0, "the counting allocator is not counting");
}

#[test]
fn the_callback_allocates_nothing_on_the_ordinary_path() {
    let (mut s, mut r, c) = sink(buffers::MIN_MILLIS);
    let payload = vec![0x5Au8; FRAME * 480];
    let mut drain = vec![0u8; FRAME * 480];

    // Warm every path once outside the measured window.
    s.on_data(&payload);
    r.read(&mut drain);

    let seen = allocations_during(|| {
        for _ in 0..1_000 {
            s.on_data(&payload);
            r.read(&mut drain);
        }
    });
    assert_eq!(seen, 0, "§10: the callback allocated {seen} times");
    assert!(c.frames() > 0);
}

#[test]
fn the_callback_allocates_nothing_when_it_overruns() {
    // The failure path is the one that tempts an implementation into building an
    // error, formatting a message, or pushing to a log. None of that may happen
    // on the audio thread.
    let (mut s, _r, c) = sink(buffers::MIN_MILLIS);
    let payload = vec![0u8; FRAME * 480];
    while c.snapshot().overruns == 0 {
        s.on_data(&payload);
    }

    let seen = allocations_during(|| {
        for _ in 0..1_000 {
            s.on_data(&payload);
        }
    });
    assert_eq!(seen, 0, "an overrun allocated {seen} times");
    assert!(c.snapshot().overruns >= 1_000);
    assert_eq!(c.snapshot().dropped_frames % 480, 0);
}

#[test]
fn the_callback_allocates_nothing_when_the_device_starves() {
    let (mut s, _r, c) = sink(buffers::MIN_MILLIS);
    s.on_data(&[]);
    let seen = allocations_during(|| {
        for _ in 0..1_000 {
            s.on_data(&[]);
        }
    });
    assert_eq!(seen, 0, "an empty callback allocated {seen} times");
    assert_eq!(c.snapshot().underruns, 1_001);
}

#[test]
fn recording_a_stream_error_is_kept_off_the_callback_path() {
    // record_error formats and lowercases a string, so it certainly allocates.
    // That is fine - CPAL calls the error callback from its own thread, not the
    // audio one - but it must never be reachable from on_data. This pins the
    // split in place so a later refactor cannot quietly merge them.
    let c = Arc::new(Counters::default());
    let seen = allocations_during(|| c.record_error("ALSA xrun"));
    assert!(
        seen > 0,
        "if this ever stops allocating the test is no longer pinning anything"
    );

    let (mut s, _r, sink_counters) = sink(buffers::MIN_MILLIS);
    let payload = vec![0u8; FRAME * 64];
    s.on_data(&payload);
    let seen = allocations_during(|| s.on_data(&payload));
    assert_eq!(seen, 0);
    assert_eq!(sink_counters.snapshot().stream_errors, 0);
}

#[test]
fn a_stalled_reader_cannot_block_the_callback() {
    // The consumer is parked and never drains. With a lock-based queue and an
    // unlucky interleaving this is where an audio thread waits; with a wait-free
    // one the ring fills, callbacks are dropped and counted, and nothing stops.
    let (mut s, reader, c) = sink(buffers::MIN_MILLIS);
    let stop = Arc::new(AtomicBool::new(false));
    let parked = {
        let stop = Arc::clone(&stop);
        std::thread::spawn(move || {
            // Holds the reading end and deliberately does nothing with it.
            let _reader = reader;
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(1));
            }
        })
    };

    let payload = vec![0u8; FRAME * 480];
    let mut worst = Duration::ZERO;
    for _ in 0..2_000 {
        let at = Instant::now();
        s.on_data(&payload);
        worst = worst.max(at.elapsed());
    }
    stop.store(true, Ordering::Relaxed);
    parked.join().unwrap();

    // Generous by three orders of magnitude against a 2.5 ms callback period:
    // this is here to catch a callback that *waits*, not one that is slow.
    assert!(
        worst < Duration::from_millis(100),
        "a callback took {worst:?} with the reader stalled"
    );
    assert!(
        c.snapshot().overruns > 0,
        "the ring should have filled, which is the correct outcome"
    );
}
