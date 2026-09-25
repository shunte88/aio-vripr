/*
 *  enumerate.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Live enumeration against whatever audio stack this machine has (§7).
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

//! Live enumeration against whatever audio stack this machine has (§7).
//!
//! These run on CI, where a runner may have no sound card at all - so they assert
//! invariants that hold for an empty list as readily as for a full one, and skip
//! the per-device checks when there is nothing to check. The point is that
//! enumeration cannot panic, cannot produce a device that cannot be re-opened, and
//! cannot invent a configuration outside §8.

mod common;

use vcw_audio::devices::{self, DeviceKey, Direction, Transport};
use vcw_audio::probe::{Matrix, Support};
use vcw_types::{STANDARD_RATES, SampleFormat};

#[test]
fn enumeration_never_fails_as_a_whole() {
    let snapshot = devices::enumerate();
    // A host that will not load is a problem recorded against the sweep, not an
    // error that hides the hosts that did load.
    for problem in &snapshot.problems {
        assert!(!problem.is_empty());
    }
    eprintln!(
        "{} device(s), {} host problem(s) on {:?}",
        snapshot.devices.len(),
        snapshot.problems.len(),
        devices::available_hosts()
    );
}

#[test]
fn every_enumerated_device_can_be_found_again_by_its_id() {
    let snapshot = devices::enumerate();
    for device in &snapshot.devices {
        let text = device.key.to_string();
        let parsed: DeviceKey = text
            .parse()
            .expect("a key must survive its own string form");
        assert_eq!(parsed, device.key, "{text}");
        assert!(
            snapshot.get(&device.key).is_some(),
            "{text} is not findable in its own snapshot"
        );
        assert!(!device.label().is_empty());
    }
}

#[test]
fn transport_never_contradicts_the_id() {
    for device in devices::enumerate().devices {
        let expected_hw = device.key.is_direct_hardware();
        if device.transport == Transport::DirectHardware {
            assert!(expected_hw, "{} claims direct hardware", device.label());
        }
        if expected_hw {
            // A `hw:` PCM is direct hardware unless the backend says it is virtual.
            assert!(
                matches!(
                    device.transport,
                    Transport::DirectHardware | Transport::Virtual
                ),
                "{} is a hw: PCM but classified {}",
                device.label(),
                device.transport
            );
        }
        // Only the direct path is even a candidate, and being a candidate is not
        // a claim: §9's answer comes from WP-04's verifier, against the OS.
        if device.transport.can_be_bit_perfect() == Some(true) {
            assert_eq!(
                device.transport,
                Transport::DirectHardware,
                "{}",
                device.label()
            );
        }
    }
}

#[test]
fn the_capability_matrix_never_leaves_section_8() {
    for device in devices::enumerate().devices {
        for direction in [Direction::Input, Direction::Output] {
            let matrix = Matrix::from_report(device.direction(direction), direction);
            assert_eq!(matrix.direction, direction);
            for entry in &matrix.entries {
                assert!(STANDARD_RATES.contains(&entry.rate), "{:?}", entry.rate);
                assert!(
                    matches!(
                        entry.format,
                        SampleFormat::S16
                            | SampleFormat::S24
                            | SampleFormat::S32
                            | SampleFormat::F32
                    ),
                    "{:?}",
                    entry.format
                );
                assert!(entry.channels > 0);
                // Nothing is confirmed until something opens the device.
                assert_eq!(entry.support, Support::Advertised);
            }
            if let Some(suggestion) = matrix.suggest() {
                assert!(matrix.rates().contains(&suggestion.rate));
                assert!(suggestion.is_usable());
            }
        }
    }
}

/// A device advertising nothing §8 wants must produce an empty matrix rather than
/// a tempting near-miss. S1 met exactly this: a webcam offering 8 kHz mono I16.
#[test]
fn devices_outside_section_8_yield_nothing_rather_than_a_near_miss() {
    for device in devices::enumerate().devices {
        let matrix = Matrix::from_report(&device.input, Direction::Input);
        if matrix.is_empty() {
            assert!(matrix.suggest().is_none(), "{}", device.label());
            assert!(matrix.rates().is_empty());
        }
    }
}

#[test]
fn an_id_from_another_machine_fails_with_the_host_named() {
    let key = DeviceKey::new("coreaudio", "AppleHDAEngineInput:1F,3,0,1,0:1");
    // On a machine where CoreAudio is not compiled in, this is the moved-settings
    // case and the message has to say which host is missing.
    let err = devices::open(&key, Direction::Input)
        .unwrap_err()
        .to_string();
    assert!(err.contains("coreaudio"), "{err}");
}

#[test]
fn an_unplugged_id_on_a_present_host_reports_the_device_not_the_host() {
    let host = devices::available_hosts()[0].to_ascii_lowercase();
    let key = DeviceKey::new(host, "hw:CARD=99,DEV=99");
    let err = devices::open(&key, Direction::Input)
        .unwrap_err()
        .to_string();
    assert!(err.contains("hw:CARD=99,DEV=99"), "{err}");
    assert!(err.contains("unplugged"), "{err}");
}

#[test]
fn finding_in_an_empty_snapshot_says_so() {
    let empty = common::snapshot(vec![]);
    let err = empty
        .find("anything", Direction::Input)
        .unwrap_err()
        .to_string();
    assert!(err.contains('0'), "{err}");
}
