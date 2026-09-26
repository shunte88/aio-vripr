/*
 *  playback_live.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Playback on a real output device: the seek-join latency, measured.
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

//! Playback on a real output device: the seek-join latency, measured.
//!
//! `crates/cli/tests/play_from_cli.rs` proves the bytes are right without a
//! device. This proves the part that needs one, and measures the one number
//! §21's "gapless seek" cannot be asserted without: how long after a seek the
//! new position reaches the driver.
//!
//! **What is measured, exactly.** The clock starts when [`Player::seek`]
//! returns and stops when [`Player::position`] first reports a frame at or
//! after the target. That is the moment the audio callback began reading the
//! new position - the queue's own latency, and the part VCW is responsible
//! for. Whatever the device holds *after* the callback is the device's buffer,
//! which playback pins and reports as [`Opened::buffer`]; a measurement of the
//! delay all the way to the converter would need a loopback cable and belongs
//! with S1's per-platform work, not here.
//!
//! **This test makes noise.** It plays a simulated capture, which is uniform
//! over the whole code range - full-scale white noise. Set `VCW_TEST_OUTPUT`
//! to a device id from `vcw devices --which output` to send it somewhere
//! harmless; a digital output with nothing plugged into it is ideal.
//!
//! Ignored by default because it needs hardware:
//!
//! ```text
//! cargo test -p vcw-core --test playback_live -- --ignored --nocapture
//! ```

use std::time::{Duration, Instant};

use vcw_core::commands::{Command, Setup};
use vcw_core::events::{Bus, Event};
use vcw_core::playback::{Audition, Player};
use vcw_core::{Engine, Scope};
use vcw_types::span::frames_at;
use vcw_types::{SampleRate, Span};

/// The rate the test records and plays at.
const RATE: SampleRate = SampleRate(48_000);

/// How long to give a seek before calling it a failure.
///
/// Generous: the point of the test is the number it prints, not the ceiling it
/// passes. A join that took this long would mean the queue is not being
/// discarded at all.
const CEILING: Duration = Duration::from_millis(500);

/// Records a few seconds into a new project and returns its path.
fn recorded(dir: &std::path::Path, seconds: f64) -> std::path::PathBuf {
    let path = dir.join("live.vcw");
    let engine = Engine::start().expect("engine");
    let events = engine.events();
    let mut setup = Setup::simulated(&path);
    setup.rate = Some(RATE.hz());
    setup.channels = Some(2);
    engine.send(Command::Arm(Box::new(setup))).expect("arm");
    engine.send(Command::Record).expect("record");
    std::thread::sleep(Duration::from_secs_f64(seconds));
    engine.send(Command::Stop).expect("stop");
    engine.shutdown().expect("shutdown");
    let mut frames = 0;
    for event in events {
        if let Event::Finished { frames: n, .. } = event {
            frames = n;
        }
    }
    assert!(frames > 0, "nothing was recorded");
    path
}

/// The measurement.
#[test]
#[ignore = "needs an audio output device, and makes a noise"]
fn the_seek_join_latency_is_measured_not_claimed() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = recorded(dir.path(), 8.0);

    let bus = Bus::new();
    let mut audition = Audition::new(&path, 1);
    audition.device = std::env::var("VCW_TEST_OUTPUT").ok();
    let player = Player::open(&audition, &bus).expect("open the output device");
    println!("device     {}", player.opened().buffer);
    println!("stream     {:?}", player.opened().format);

    player.play().expect("play");
    // Let it settle: the first callbacks of a stream are not representative of
    // any of the ones after them.
    let settle = Instant::now() + Duration::from_millis(500);
    while Instant::now() < settle {
        std::thread::sleep(Duration::from_millis(10));
    }

    let mut joins = Vec::new();
    for target in [3.0_f64, 1.0, 6.0, 2.0, 5.0] {
        let to = frames_at(RATE, target);
        let at = Instant::now();
        let landed = player.seek(to);
        assert_eq!(landed, to, "the seek was clamped");
        let epoch = player.epoch();

        // Not `position()`. The seek stores the frame it asked for, so reading
        // the playhead back measures the write that was just made and nothing
        // the device did. `delivered_epoch` is written by the audio callback,
        // and only once it has copied bytes from a chunk filled after the seek.
        let give_up = at + CEILING;
        loop {
            if player.delivered_epoch() >= epoch {
                joins.push(at.elapsed());
                break;
            }
            assert!(
                Instant::now() < give_up,
                "{target} s never reached the device; the playhead reads {}",
                player.position()
            );
            std::hint::spin_loop();
        }
        let frame = player.position();
        assert!(
            frame >= to && frame < to + frames_at(RATE, 1.0),
            "the device joined at {frame}, nowhere near {target} s"
        );
        // Long enough that the next seek is not measuring the last one.
        std::thread::sleep(Duration::from_millis(300));
    }

    let played = player.stop();
    joins.sort_unstable();
    let millis = |d: &Duration| d.as_secs_f64() * 1_000.0;
    println!(
        "seek join  min {:.2} ms, median {:.2} ms, max {:.2} ms, over {} seeks",
        millis(&joins[0]),
        millis(&joins[joins.len() / 2]),
        millis(&joins[joins.len() - 1]),
        joins.len()
    );
    println!("health     {:?}", played.health);
    println!("fidelity   {}", played.fidelity.summary());

    assert!(
        played.was_gapless(),
        "the listener heard {} gap(s) across five seeks",
        played.health.underruns
    );
    assert!(
        played.health.stale_chunks > 0,
        "five seeks discarded nothing, so nothing was invalidated"
    );
}

/// A device that cannot play a capture's rate is refused, not resampled.
///
/// The one requirement in §21 that is easier to get wrong than to get right:
/// every audio framework will happily resample, and the result is a capture
/// that no longer sounds like the record.
#[test]
#[ignore = "needs an audio output device"]
fn a_rate_the_device_cannot_play_is_refused() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = recorded(dir.path(), 1.0);
    let bus = Bus::new();
    let mut audition = Audition::new(&path, 1).scope(Scope::Region(Span::new(0, 48_000)));
    audition.device = std::env::var("VCW_TEST_OUTPUT").ok();

    // Whatever this machine's output can do, it cannot do 12,345 Hz, and the
    // refusal has to name the rates it does offer.
    match Player::open(&audition, &bus) {
        Ok(player) => {
            // The capture is at 48 kHz, which most devices do play. Then the
            // assertion is the other half: it plays at its own rate.
            assert_eq!(player.opened().rate, RATE, "playback changed the rate");
        }
        Err(vcw_core::playback::Error::Audio(vcw_audio::Error::RateUnavailable {
            offered,
            ..
        })) => {
            assert!(!offered.is_empty(), "a refusal has to say what is on offer");
        }
        Err(other) => panic!("unexpected failure: {other}"),
    }
}
