/*
 *  edits_are_nondestructive.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  WP-13's first exit criterion: an edit cannot reach the audio.
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

//! WP-13's first exit criterion: an edit cannot reach the audio.
//!
//! §4.1 says captured samples are never modified, and §31's editing model is built
//! so that promise holds by construction: every verb writes to `sides`,
//! `track_boundaries` or `tracks`, and the audio lives in `sampleblocks` with
//! `capture_blocks` as its timeline. That is an argument, not evidence. This is the
//! evidence.
//!
//! The method is a fingerprint over both audio tables - every block's id, its
//! declared shape, its stored checksum and a hash of its actual bytes - taken
//! fingerprint over both audio tables - every block's id, its format code, its
//! stored summaries and a hash of its actual bytes - taken before and after each
//! edit and compared. Hashing the bytes rather than trusting the summaries matters:
//! a verb that rewrote a block *and* recomputed its summaries would pass a summary
//! comparison and fail this one. The first assertion in the test proves the
//! fingerprint notices a single altered sample byte, so a green run means
//! something.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use vcw_project::track::{NewBoundary, Update};
use vcw_project::{Options, Project, side, track, validate};
use vcw_types::StorageFormat;
use vcw_types::observation::{Edge, Provenance};
use vcw_types::vinyl::Side;

mod common;
use common::{Blocks, insert_capture};

/// Every audio row in the project, as one number.
///
/// Covers `sampleblocks` (the bytes, the format code and all four summaries) and
/// `capture_blocks` (which frames of which capture they are). An
/// edit that changed any of it changes this.
fn audio_fingerprint(project: &Project) -> u64 {
    let mut hasher = DefaultHasher::new();
    let conn = project.conn();

    let mut stmt = conn
        .prepare(
            "SELECT blockid, sampleformat, summin, summax, sumrms,
                    samples, summary256, summary64k
               FROM sampleblocks ORDER BY blockid",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                // The three summaries are nullable floats, so hash their bits: a
                // summary recomputed from altered samples has to show up here
                // too, and a NULL is itself a value worth noticing a change to.
                r.get::<_, Option<f64>>(2)?.map(f64::to_bits),
                r.get::<_, Option<f64>>(3)?.map(f64::to_bits),
                r.get::<_, Option<f64>>(4)?.map(f64::to_bits),
                r.get::<_, Option<Vec<u8>>>(5)?,
                r.get::<_, Option<Vec<u8>>>(6)?,
                r.get::<_, Option<Vec<u8>>>(7)?,
            ))
        })
        .unwrap();
    for row in rows {
        row.unwrap().hash(&mut hasher);
    }

    let mut stmt = conn
        .prepare(
            "SELECT capture_id, channel, sequence, start_frame, frame_count, blockid
               FROM capture_blocks ORDER BY capture_id, channel, sequence",
        )
        .unwrap();
    let rows = stmt
        .query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, i64>(2)?,
                r.get::<_, i64>(3)?,
                r.get::<_, i64>(4)?,
                r.get::<_, i64>(5)?,
            ))
        })
        .unwrap();
    for row in rows {
        row.unwrap().hash(&mut hasher);
    }
    hasher.finish()
}

/// A project holding one capture, with side A pointing at it.
fn recorded(dir: &tempfile::TempDir) -> (Project, i64) {
    let mut project = Project::create(dir.path().join("edits.vcw")).unwrap();
    insert_capture(
        &mut project,
        Blocks::new(StorageFormat::Int24Packed, 2, 6, 48_000),
    );
    let capture: i64 = project
        .conn()
        .query_row("SELECT MIN(capture_id) FROM captures", [], |r| r.get(0))
        .unwrap();
    side::attach(&mut project, Side::A, capture).unwrap();
    (project, capture)
}

/// Runs one edit and asserts it left the audio alone.
fn unchanged(project: &mut Project, what: &str, edit: impl FnOnce(&mut Project)) {
    let before = audio_fingerprint(project);
    edit(project);
    assert_eq!(
        audio_fingerprint(project),
        before,
        "{what} changed the captured audio"
    );
}

#[test]
fn no_editing_verb_touches_a_committed_block() {
    let dir = tempfile::tempdir().unwrap();
    let (mut project, capture) = recorded(&dir);

    let mut first = 0;
    let mut second = 0;
    let mut detected = 0;

    unchanged(&mut project, "add_track", |p| {
        first = track::add_track(p, Side::A, 0, 96_000).unwrap();
        second = track::add_track(p, Side::A, 96_000, 192_000).unwrap();
    });
    unchanged(&mut project, "add_boundary", |p| {
        detected = track::add_boundary(
            p,
            Side::A,
            &NewBoundary::detected(240_000, Edge::Start, 0.8, Provenance::Silence),
        )
        .unwrap();
    });
    unchanged(&mut project, "split", |p| {
        track::split(p, first, 48_000).unwrap();
    });
    unchanged(&mut project, "update", |p| {
        track::update(p, first, &Update::title("Sister Ray")).unwrap();
    });
    unchanged(&mut project, "set_lock", |p| {
        track::set_lock(p, detected, true).unwrap();
        track::set_lock(p, detected, false).unwrap();
    });
    unchanged(&mut project, "move_boundary", |p| {
        track::move_boundary(p, detected, 288_000).unwrap();
    });
    unchanged(&mut project, "move_boundary_forced", |p| {
        let at = track::track(p.conn(), second)
            .unwrap()
            .unwrap()
            .end_boundary;
        track::move_boundary_forced(p, at, 200_000).unwrap();
    });
    unchanged(&mut project, "delete_boundary", |p| {
        track::delete_boundary(p, detected).unwrap();
    });
    unchanged(&mut project, "renumber", |p| {
        track::renumber(p, Side::A).unwrap();
    });
    unchanged(&mut project, "merge", |p| {
        let on_side = track::tracks(p.conn(), Side::A).unwrap();
        let (left, right) = (&on_side[0], &on_side[1]);
        track::set_lock(p, left.end_boundary, false).unwrap();
        track::set_lock(p, right.start_boundary, false).unwrap();
        track::merge(p, left.id, right.id).unwrap();
    });
    unchanged(&mut project, "move_to_side", |p| {
        side::attach(p, Side::from_letter('B').unwrap(), capture).unwrap();
        let moving = track::tracks(p.conn(), Side::A).unwrap()[0].id;
        track::move_to_side(p, moving, Side::from_letter('B').unwrap()).unwrap();
    });
    unchanged(&mut project, "remove", |p| {
        let going = track::tracks(p.conn(), Side::from_letter('B').unwrap()).unwrap()[0].id;
        track::remove(p, going).unwrap();
    });
    unchanged(&mut project, "side::relabel", |p| {
        side::relabel(p, Side::A, Side::from_letter('C').unwrap()).unwrap();
    });
    unchanged(&mut project, "side::detach", |p| {
        side::detach(p, Side::from_letter('B').unwrap()).unwrap();
    });
    unchanged(&mut project, "side::remove", |p| {
        // `track::remove` left side B's user boundaries behind, on purpose, so
        // clear them before the side will go.
        for boundary in track::boundaries(p.conn(), Side::from_letter('B').unwrap()).unwrap() {
            track::set_lock(p, boundary.id, false).unwrap();
            track::delete_boundary(p, boundary.id).unwrap();
        }
        side::remove(p, Side::from_letter('B').unwrap()).unwrap();
    });

    // And the project is still a project afterwards, not merely one whose audio
    // survived: a green fingerprint over a corrupt timeline would prove nothing.
    let report = validate(
        &project,
        Options {
            verify_checksums: true,
        },
    )
    .unwrap();
    assert!(report.is_clean(), "{:?}", report.findings);
    assert!(report.checksums_verified);
}

#[test]
fn deleting_every_track_leaves_every_sample() {
    // The strongest form of §4.1: there is no sequence of edits that loses audio,
    // including the one that ends with an empty side.
    let dir = tempfile::tempdir().unwrap();
    let (mut project, _) = recorded(&dir);
    let before = audio_fingerprint(&project);
    let blocks: i64 = project
        .conn()
        .query_row("SELECT COUNT(*) FROM sampleblocks", [], |r| r.get(0))
        .unwrap();

    for (start, end) in [(0, 48_000), (48_000, 96_000), (96_000, 144_000)] {
        track::add_track(&mut project, Side::A, start, end).unwrap();
    }
    for record in track::tracks(project.conn(), Side::A).unwrap() {
        track::remove(&mut project, record.id).unwrap();
    }
    for boundary in track::boundaries(project.conn(), Side::A).unwrap() {
        track::set_lock(&mut project, boundary.id, false).unwrap();
        track::delete_boundary(&mut project, boundary.id).unwrap();
    }
    side::detach(&mut project, Side::A).unwrap();
    side::remove(&mut project, Side::A).unwrap();

    assert!(side::list(project.conn()).unwrap().is_empty());
    assert_eq!(audio_fingerprint(&project), before);
    assert_eq!(
        project
            .conn()
            .query_row("SELECT COUNT(*) FROM sampleblocks", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        blocks,
        "an empty project still holds every block it recorded"
    );
}

/// The fingerprint has to be sensitive before it can be evidence.
///
/// Without this, every assertion in this file could be satisfied by a function
/// that returns a constant.
#[test]
fn the_fingerprint_notices_altered_sample_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let (project, _) = recorded(&dir);
    let before = audio_fingerprint(&project);

    project
        .conn()
        .execute(
            // `substr` over a blob returns text, which the fingerprint would
            // reject rather than compare, so overwrite with a blob of the same
            // length: same shape, different bytes, which is the sneakiest
            // change an edit could make.
            "UPDATE sampleblocks SET samples = zeroblob(length(samples)) WHERE blockid = 1",
            [],
        )
        .unwrap();
    assert_ne!(
        audio_fingerprint(&project),
        before,
        "the fingerprint is blind to the thing it exists to see"
    );
}
