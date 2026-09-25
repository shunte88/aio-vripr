/*
 *  known_levels.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  WP-08's exit criterion: the meter against synthesised signals of known level.
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

//! WP-08's exit criterion: the meter against synthesised signals of known level.
//!
//! Recorded material is no use here. A rip of a record has an unknown true
//! level, so a meter tested against one can only be compared with another
//! meter, and then a shared misunderstanding of full scale reads as agreement.
//! Everything below is synthesised, so the expected answer is arithmetic:
//! a sine of amplitude a has peak a and RMS a/sqrt(2), a square wave has both
//! equal to a, and direct current has both equal to a.
//!
//! Every level test runs through **all five storage formats**, because the one
//! thing most likely to be wrong is the scaling, and a bug there shows up as
//! one format disagreeing with the other four rather than as an implausible
//! number.

use std::f32::consts::TAU;
use std::time::Instant;

use vcw_signal::meter::{Config, Meter, SILENCE_DB, db};
use vcw_types::StorageFormat;

const RATE: u32 = 48_000;

/// Every format a block can be stored in, which is every format a meter can be
/// asked to read.
const FORMATS: [StorageFormat; 5] = StorageFormat::ALL;

/// A meter with a 100 ms RMS window: sixteen buckets of 300 frames at 48 kHz,
/// so the window is a whole number of cycles of every test tone below.
fn meter(channels: usize, format: StorageFormat) -> Meter {
    Meter::new(Config::new(channels, RATE, format).rms_window(100))
}

/// Interleaved sine, the same on every channel.
fn sine(amplitude: f32, hz: f32, channels: usize, frames: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(frames * channels);
    for frame in 0..frames {
        let value = amplitude * (TAU * hz * frame as f32 / RATE as f32).sin();
        for _ in 0..channels {
            out.push(value);
        }
    }
    out
}

/// Interleaved constant, which is both a DC test and the simplest square wave.
fn constant(value: f32, channels: usize, frames: usize) -> Vec<f32> {
    vec![value; frames * channels]
}

/// Quantises to the format's integer grid the way a converter would.
fn quantise(x: f32, bits: i32) -> i64 {
    let full = 2.0_f64.powi(bits);
    #[allow(clippy::cast_possible_truncation)]
    {
        (f64::from(x) * full).round().clamp(-full, full - 1.0) as i64
    }
}

/// Encodes interleaved samples the way a device and the writer would store them.
fn encode(format: StorageFormat, samples: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * format.bytes_per_sample());
    for &sample in samples {
        match format {
            StorageFormat::Int16 => {
                out.extend_from_slice(&(quantise(sample, 15) as i16).to_le_bytes());
            }
            StorageFormat::Int24Packed => {
                out.extend_from_slice(&(quantise(sample, 23) as i32).to_le_bytes()[..3]);
            }
            StorageFormat::Int24Padded => {
                out.extend_from_slice(&(quantise(sample, 23) as i32).to_le_bytes());
            }
            StorageFormat::Int32 => {
                out.extend_from_slice(&(quantise(sample, 31) as i32).to_le_bytes());
            }
            StorageFormat::Float32 => out.extend_from_slice(&sample.to_le_bytes()),
        }
    }
    out
}

/// Asserts two decibel values agree to within a tolerance, saying by how much
/// they did not when they do not.
#[track_caller]
fn close(got: f32, want: f32, tolerance: f32, what: &str) {
    assert!(
        (got - want).abs() <= tolerance,
        "{what}: got {got:.4} dB, expected {want:.4} dB, out by {:.4} dB",
        (got - want).abs()
    );
}

/// A sine at a known level reads that level, in every format.
#[test]
fn a_sine_of_known_amplitude_reads_its_amplitude() {
    // -0.5 dBFS rather than 0, so the integer formats are not sitting on their
    // clamp and the test measures scaling rather than saturation.
    for (amplitude, peak_db) in [(0.944_06_f32, -0.5), (0.501_187, -6.0), (0.1, -20.0)] {
        for format in FORMATS {
            let mut meter = meter(2, format);
            meter.feed(&encode(
                format,
                &sine(amplitude, 1_000.0, 2, RATE as usize / 2),
            ));
            let snapshot = meter.snapshot();

            for (channel, levels) in snapshot.channels.iter().enumerate() {
                close(
                    levels.peak_db(),
                    peak_db,
                    0.05,
                    &format!("{format:?} ch{channel} peak"),
                );
                // A sine's RMS is its amplitude over root two: 3.01 dB down.
                close(
                    levels.rms_db(),
                    peak_db - 3.010_3,
                    0.05,
                    &format!("{format:?} ch{channel} rms"),
                );
                assert!(!levels.clipped, "{format:?} clipped at {peak_db} dBFS");
            }
        }
    }
}

/// Direct current and a square wave have peak and RMS equal, which is the
/// crest-factor case a sine cannot catch.
#[test]
fn a_constant_reads_the_same_peak_and_rms() {
    for format in FORMATS {
        let mut meter = meter(1, format);
        meter.feed(&encode(format, &constant(0.5, 1, RATE as usize / 2)));
        let levels = meter.snapshot().channels[0];
        close(levels.peak_db(), -6.0206, 0.01, &format!("{format:?} peak"));
        close(levels.rms_db(), -6.0206, 0.01, &format!("{format:?} rms"));
    }
}

/// Silence is the floor, not minus infinity and not a NaN.
#[test]
fn silence_reads_the_floor_in_every_format() {
    for format in FORMATS {
        let mut meter = meter(2, format);
        meter.feed(&encode(format, &constant(0.0, 2, RATE as usize / 10)));
        let snapshot = meter.snapshot();
        for levels in &snapshot.channels {
            assert_eq!(levels.peak, 0.0);
            assert_eq!(levels.rms, 0.0);
            assert!((levels.peak_db() - SILENCE_DB).abs() < f32::EPSILON);
            assert!(levels.peak_db().is_finite(), "a UI cannot lay out -inf");
        }
    }
}

/// §17 says stereo meters. A loud left and a quiet right must not average.
#[test]
fn channels_are_metered_independently() {
    let format = StorageFormat::Int32;
    let mut meter = meter(2, format);

    let frames = RATE as usize / 2;
    let left = sine(0.5, 1_000.0, 1, frames);
    let right = sine(0.05, 1_000.0, 1, frames);
    let mut interleaved = Vec::with_capacity(frames * 2);
    for frame in 0..frames {
        interleaved.push(left[frame]);
        interleaved.push(right[frame]);
    }
    meter.feed(&encode(format, &interleaved));

    let snapshot = meter.snapshot();
    close(snapshot.channels[0].peak_db(), -6.0206, 0.05, "left peak");
    close(snapshot.channels[1].peak_db(), -26.0206, 0.05, "right peak");
    close(snapshot.peak(), 0.5, 0.001, "the loudest channel");
}

/// The RMS window is a property of the signal, not of the polling rate.
#[test]
fn rms_does_not_depend_on_how_often_the_ui_looks() {
    let format = StorageFormat::Float32;
    let signal = encode(format, &sine(0.5, 1_000.0, 2, RATE as usize / 2));

    // One caller feeds the lot and looks once; the other feeds it in 128-frame
    // pieces and looks after every one. A meter whose window depended on the
    // read rate would give two different answers.
    let mut rare = meter(2, format);
    rare.feed(&signal);
    let once = rare.snapshot().channels[0].rms_db();

    let mut often = meter(2, format);
    let chunk = 128 * 2 * format.bytes_per_sample();
    for piece in signal.chunks(chunk) {
        often.feed(piece);
        let _ = often.snapshot();
    }
    let repeatedly = often.snapshot().channels[0].rms_db();

    close(repeatedly, once, 0.2, "rms across two polling rates");
}

/// And it slides: loud audio leaves the window when it leaves the window.
#[test]
fn the_rms_window_forgets_what_has_left_it() {
    let format = StorageFormat::Float32;
    let mut meter = meter(1, format);

    meter.feed(&encode(format, &constant(0.5, 1, RATE as usize / 2)));
    close(
        meter.peek().channels[0].rms_db(),
        -6.0206,
        0.05,
        "while loud",
    );

    // Half a second of silence, against a 100 ms window: five windows' worth.
    meter.feed(&encode(format, &constant(0.0, 1, RATE as usize / 2)));
    assert_eq!(
        meter.peek().channels[0].rms,
        0.0,
        "the window still holds audio that left it half a second ago"
    );
}

/// §18: clipping latches, and stays latched until it is acknowledged.
#[test]
fn one_sample_at_full_scale_latches_and_stays_latched() {
    for format in FORMATS {
        let mut meter = meter(2, format);

        // Quiet audio with a single full-scale sample in the left channel.
        let frames = RATE as usize / 10;
        let mut samples = constant(0.1, 2, frames);
        samples[frames] = 1.0;
        meter.feed(&encode(format, &samples));

        let snapshot = meter.snapshot();
        assert!(
            snapshot.channels[0].clipped,
            "{format:?}: a sample at full scale did not latch"
        );
        assert_eq!(snapshot.channels[0].clipped_samples, 1, "{format:?}");
        assert!(
            !snapshot.channels[1].clipped,
            "{format:?}: the clip latched on the wrong channel too"
        );

        // A second of quiet audio must not clear it.
        meter.feed(&encode(format, &constant(0.1, 2, RATE as usize)));
        assert!(
            meter.snapshot().channels[0].clipped,
            "{format:?}: the latch let go on its own"
        );

        meter.clear_clip();
        assert!(!meter.snapshot().channels[0].clipped, "{format:?}");
    }
}

/// The asymmetry of two's complement, which is where a naive detector fails.
#[test]
fn the_top_integer_code_clips_although_it_is_not_one_point_zero() {
    // One code at a time, each into its own meter, so the latch cannot carry.
    let clips = |code: i16| {
        let mut one = meter(1, StorageFormat::Int16);
        one.feed(&code.to_le_bytes());
        one.snapshot().channels[0].clipped
    };

    // 32767/32768 is the loudest an i16 can be, and it is 0.99997, not 1.0.
    assert!(
        clips(32_767),
        "a detector comparing against 1.0 would never fire on integer input"
    );
    assert!(!clips(32_766), "one code below full scale is not a clip");
    // The negative end is exactly -1.0, and clips.
    assert!(clips(-32_768));
}

/// The broadcast convention, which is not the default but has to work.
#[test]
fn three_in_a_row_can_be_required_instead_of_one() {
    let format = StorageFormat::Float32;
    let config = Config::new(1, RATE, format).clip_after(3);

    let mut meter = Meter::new(config);
    meter.feed_samples(&[1.0, 0.1, 1.0, 1.0, 0.1]);
    assert!(
        !meter.snapshot().channels[0].clipped,
        "two full-scale samples, never three in a row"
    );
    assert_eq!(meter.peek().channels[0].clipped_samples, 3);

    let mut meter = Meter::new(config);
    meter.feed_samples(&[0.1, 1.0, 1.0, 1.0, 0.1]);
    assert!(meter.snapshot().channels[0].clipped);
}

/// The hold needle takes a new maximum at once, holds, then falls at its rate.
#[test]
fn the_hold_needle_holds_and_then_falls_at_the_rate_it_was_given() {
    let format = StorageFormat::Float32;
    // Hold for 200 ms, then fall at 20 dB a second.
    let mut meter = Meter::new(Config::new(1, RATE, format).rms_window(100).hold(200, 20.0));

    // The hold clock starts when the needle takes a new maximum, not when the
    // signal stops, which for a constant burst is its very first sample.
    // A 100 ms burst at -6 dBFS, then silence.
    meter.feed_samples(&constant(0.5, 1, RATE as usize / 10));
    close(meter.peek().channels[0].hold_db(), -6.0206, 0.01, "at once");

    // 50 ms of silence: 150 ms since the peak, still inside the 200 ms hold.
    meter.feed_samples(&constant(0.0, 1, RATE as usize / 20));
    close(
        meter.peek().channels[0].hold_db(),
        -6.0206,
        0.01,
        "during the hold",
    );

    // 550 ms more: 700 ms since the peak, so 500 ms of falling at 20 dB a
    // second, which is 10 dB down.
    meter.feed_samples(&constant(0.0, 1, (RATE as usize * 55) / 100));
    close(
        meter.peek().channels[0].hold_db(),
        -16.0206,
        0.3,
        "after the hold",
    );

    // And the difference between the two needles: the instantaneous peak is
    // still holding 0.5, because nothing has read it. One snapshot clears it
    // and the hold needle carries on regardless.
    let read = meter.snapshot().channels[0];
    assert_eq!(read.peak, 0.5, "the peak survives until it is read");
    let after = meter.peek().channels[0];
    assert_eq!(after.peak, 0.0, "reading the peak is what resets it");
    close(after.hold_db(), read.hold_db(), 0.01, "the hold needle");
}

/// Every format reads the same signal the same way, to within its own LSB.
#[test]
fn the_five_formats_agree_with_each_other() {
    let reference = sine(0.301, 997.0, 2, RATE as usize / 2);
    let mut levels = Vec::new();
    for format in FORMATS {
        let mut meter = meter(2, format);
        meter.feed(&encode(format, &reference));
        levels.push((format, meter.snapshot().channels[0]));
    }
    let (_, first) = levels[0];
    for (format, got) in &levels[1..] {
        close(
            got.peak_db(),
            first.peak_db(),
            0.01,
            &format!("{format:?} peak against {:?}", levels[0].0),
        );
        close(
            got.rms_db(),
            first.rms_db(),
            0.01,
            &format!("{format:?} rms against {:?}", levels[0].0),
        );
    }
}

/// §37 wants real-time-feeling meters, so metering must cost far less than the
/// audio it measures.
#[test]
fn the_meter_keeps_ahead_of_a_192k_stereo_stream() {
    let format = StorageFormat::Int32;
    let rate = 192_000;
    let mut meter = Meter::new(Config::new(2, rate, format));

    // Ten seconds of audio, fed in the 250 ms pieces D3 commits in.
    let piece = encode(format, &sine(0.5, 1_000.0, 2, rate as usize / 4));
    let started = Instant::now();
    for _ in 0..40 {
        meter.feed(&piece);
        let _ = meter.snapshot();
    }
    let taken = started.elapsed().as_secs_f64();

    assert_eq!(meter.frames(), u64::from(rate) * 10);
    // A generous bound: the point is the order of magnitude, and a debug build
    // on a loaded CI machine is not the place for a tight one.
    assert!(
        taken < 2.0,
        "10 s of 192 kHz stereo took {taken:.3} s to meter"
    );
    println!("192 kHz stereo: 10 s metered in {taken:.3} s");
}

/// dB and amplitude are inverses, and the floor is a floor rather than a cliff.
#[test]
fn decibels_round_trip() {
    for amplitude in [1.0_f32, 0.5, 0.1, 0.001] {
        let round_tripped = vcw_signal::meter::amplitude(db(amplitude));
        assert!(
            (round_tripped - amplitude).abs() < 1e-5,
            "{amplitude} -> {} -> {round_tripped}",
            db(amplitude)
        );
    }
    assert!((db(0.0) - SILENCE_DB).abs() < f32::EPSILON);
    assert_eq!(vcw_signal::meter::amplitude(SILENCE_DB), 0.0);
    assert!(
        (db(-0.5) - db(0.5)).abs() < f32::EPSILON,
        "sign is not level"
    );
}
