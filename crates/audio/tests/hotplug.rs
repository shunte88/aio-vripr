/*
 *  hotplug.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Hot-plug and hot-unplug detection (§7).
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

//! Hot-plug and hot-unplug detection (§7).
//!
//! CPAL exposes no device-change notification on any platform we target, so the
//! only portable mechanism is to re-enumerate and compare. That makes the
//! comparison the thing worth testing, and it is pure - no hardware, no cable, no
//! spare hand.

mod common;

use common::{device, input, range, snapshot, three_paths_to_one_card};
use vcw_audio::devices::{Change, DeviceKey, Direction, Transport};
use vcw_types::SampleFormat;

#[test]
fn nothing_changing_reports_nothing() {
    let before = three_paths_to_one_card();
    let after = three_paths_to_one_card();
    assert!(after.diff(&before).is_empty());
}

#[test]
fn a_converter_being_plugged_in_is_noticed() {
    let before = three_paths_to_one_card();
    let mut after = three_paths_to_one_card();
    after
        .devices
        .push(device("hw:CARD=3,DEV=0", "Tascam DA-3000"));

    let changes = after.diff(&before);
    assert_eq!(changes.len(), 1);
    assert!(matches!(&changes[0], Change::Appeared { name, .. } if name == "Tascam DA-3000"));
    assert_eq!(changes[0].key(), &DeviceKey::new("alsa", "hw:CARD=3,DEV=0"));
}

#[test]
fn a_converter_being_unplugged_is_noticed_by_id() {
    let mut before = three_paths_to_one_card();
    before
        .devices
        .push(device("hw:CARD=3,DEV=0", "Tascam DA-3000"));
    let after = three_paths_to_one_card();

    let changes = after.diff(&before);
    assert_eq!(changes.len(), 1);
    match &changes[0] {
        Change::Disappeared { key, name } => {
            assert_eq!(key.id(), "hw:CARD=3,DEV=0");
            assert_eq!(name, "Tascam DA-3000");
        }
        other => panic!("expected Disappeared, got {other:?}"),
    }
}

/// The case that makes name-based selection unsafe. Unplugging the card removes
/// `hw:` and `plughw:` as two separate devices, not one - and a list keyed on
/// names could not tell which of them had gone.
#[test]
fn unplugging_one_card_removes_every_path_to_it_separately() {
    let before = three_paths_to_one_card();
    let mut after = three_paths_to_one_card();
    after.devices.retain(|d| !d.key.id().contains("CARD=0"));

    let changes = after.diff(&before);
    assert_eq!(changes.len(), 2);
    let gone: Vec<&str> = changes.iter().map(|c| c.key().id()).collect();
    assert!(gone.contains(&"hw:CARD=0,DEV=0"));
    assert!(gone.contains(&"plughw:CARD=0,DEV=0"));
    assert!(
        changes
            .iter()
            .all(|c| matches!(c, Change::Disappeared { .. }))
    );
}

/// A converter switched to a different rate by its own front panel. The device
/// never left, so this is neither an appearance nor a disappearance, and a
/// capture armed against the old rate is about to fail.
#[test]
fn a_device_that_changes_what_it_offers_is_reported_as_reconfigured() {
    let before = three_paths_to_one_card();
    let mut after = three_paths_to_one_card();
    after.devices[0].input = input(vec![range(2, 44_100, 48_000, SampleFormat::S24)]);

    let changes = after.diff(&before);
    assert_eq!(changes.len(), 1);
    match &changes[0] {
        Change::Reconfigured { key, details, .. } => {
            assert_eq!(key.id(), "hw:CARD=0,DEV=0");
            assert!(
                details.iter().any(|d| d.contains("configurations")),
                "{details:?}"
            );
        }
        other => panic!("expected Reconfigured, got {other:?}"),
    }
}

#[test]
fn a_renamed_or_reclassified_device_is_reported_with_what_moved() {
    let before = three_paths_to_one_card();
    let mut after = three_paths_to_one_card();
    after.devices[0].name = "HDA Intel PCH (rear)".to_owned();
    after.devices[0].transport = Transport::Converting;
    after.devices[2].is_default_input = false;

    let changes = after.diff(&before);
    assert_eq!(changes.len(), 2);
    let reconfigured = changes
        .iter()
        .find(|c| c.key().id() == "hw:CARD=0,DEV=0")
        .expect("the renamed device");
    let Change::Reconfigured { details, .. } = reconfigured else {
        panic!("expected Reconfigured, got {reconfigured:?}")
    };
    assert_eq!(details.len(), 2, "{details:?}");
    assert!(details.iter().any(|d| d.contains("name")), "{details:?}");
    assert!(
        details.iter().any(|d| d.contains("transport")),
        "{details:?}"
    );
}

/// A device held open by another application reports an error on the sweep it was
/// busy for. That is not a change to the device, and treating it as one would make
/// every JACK session look like a hardware fault.
#[test]
fn a_transiently_busy_device_is_not_a_change() {
    let before = three_paths_to_one_card();
    let mut after = three_paths_to_one_card();
    after.devices[0]
        .problems
        .push("input configs: Device or resource busy".to_owned());

    assert!(after.diff(&before).is_empty());
}

#[test]
fn everything_disappears_when_the_host_does() {
    let before = three_paths_to_one_card();
    let after = snapshot(vec![]);
    let changes = after.diff(&before);
    assert_eq!(changes.len(), 3);
    assert!(
        changes
            .iter()
            .all(|c| matches!(c, Change::Disappeared { .. }))
    );
}

#[test]
fn selection_by_id_survives_a_name_collision_that_selection_by_name_cannot() {
    let snapshot = three_paths_to_one_card();

    // Three devices, two of them sharing a name. The name is unusable.
    let by_name = snapshot.find("HDA Intel PCH", Direction::Input);
    assert!(
        by_name.is_err(),
        "the shared name should be refused, not resolved"
    );

    // The id is not.
    let direct = snapshot.find("hw:CARD=0,DEV=0", Direction::Input).unwrap();
    assert_eq!(direct.transport, Transport::DirectHardware);
    assert_eq!(direct.transport.can_be_bit_perfect(), Some(true));

    let plug = snapshot
        .find("alsa:plughw:CARD=0,DEV=0", Direction::Input)
        .unwrap();
    assert_eq!(plug.transport, Transport::Converting);
    assert_eq!(plug.transport.can_be_bit_perfect(), Some(false));
}

#[test]
fn a_query_that_matches_nothing_says_how_much_it_searched() {
    let snapshot = three_paths_to_one_card();
    let err = snapshot.find("Tascam", Direction::Input).unwrap_err();
    let message = err.to_string();
    assert!(message.contains("Tascam"), "{message}");
    assert!(message.contains('3'), "{message}");
}

#[test]
fn output_only_queries_do_not_match_input_devices() {
    let snapshot = three_paths_to_one_card();
    assert!(snapshot.find("hw:CARD=0,DEV=0", Direction::Output).is_err());
    assert_eq!(snapshot.in_direction(Direction::Output).count(), 0);
    assert_eq!(snapshot.in_direction(Direction::Input).count(), 3);
}

#[test]
fn the_default_is_findable_and_is_the_one_s1_warned_about() {
    let snapshot = three_paths_to_one_card();
    let default = snapshot
        .default_for(Direction::Input)
        .expect("a default input");
    assert_eq!(default.transport, Transport::Virtual);
    assert_eq!(default.transport.can_be_bit_perfect(), None);
}
