//! Validation has to find damage, not just pass on healthy files.
//!
//! Each test breaks one specific thing and asserts the specific finding. A
//! validator tested only against sound projects is a function that returns
//! "clean".

use vcw_project::{Options, Project, validate};
use vcw_types::StorageFormat;

mod common;
use common::{Blocks, insert_capture};

/// A project with one clean stereo capture, ready to be damaged.
fn project(dir: &tempfile::TempDir) -> Project {
    let path = dir.path().join("side-a.vcw");
    let mut project = Project::create(&path).unwrap();
    insert_capture(
        &mut project,
        Blocks::new(StorageFormat::Int24Packed, 2, 4, 12_000),
    );
    project
}

fn findings(project: &Project) -> Vec<&'static str> {
    validate(
        project,
        Options {
            verify_checksums: true,
        },
    )
    .unwrap()
    .findings
    .iter()
    .map(|f| f.code)
    .collect()
}

#[test]
fn the_undamaged_project_is_clean() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(findings(&project(&dir)), Vec::<&str>::new());
}

#[test]
fn a_timeline_entry_with_no_samples_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(&dir);
    // Foreign keys would normally stop this; a project touched by a tool that did
    // not enable them can carry it, and it is the worst shape of damage there is -
    // the timeline claiming audio that does not exist.
    p.conn().pragma_update(None, "foreign_keys", false).unwrap();
    p.conn()
        .execute("DELETE FROM sampleblocks WHERE blockid = 3", [])
        .unwrap();
    assert!(findings(&p).contains(&"dangling-block"));
}

#[test]
fn samples_nothing_refers_to_are_found() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(&dir);
    p.conn()
        .execute(
            "INSERT INTO sampleblocks (sampleformat, samples) VALUES (196610, x'0011')",
            [],
        )
        .unwrap();
    assert!(findings(&p).contains(&"orphan-block"));
}

#[test]
fn a_declared_frame_count_that_does_not_match_the_bytes_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(&dir);
    p.conn()
        .execute(
            "UPDATE capture_blocks SET frame_count = 11999 WHERE blockid = 2",
            [],
        )
        .unwrap();
    let codes = findings(&p);
    assert!(codes.contains(&"size-mismatch"), "{codes:?}");
    // The shortened block also breaks contiguity, which is a separate fault and
    // should be reported as one.
    assert!(codes.contains(&"timeline-gap"), "{codes:?}");
}

#[test]
fn a_gap_in_the_timeline_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(&dir);
    p.conn()
        .execute(
            "UPDATE capture_blocks SET start_frame = start_frame + 1 WHERE sequence >= 2",
            [],
        )
        .unwrap();
    assert!(findings(&p).contains(&"timeline-gap"));
}

#[test]
fn a_format_code_we_do_not_know_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(&dir);
    p.conn()
        .execute(
            "UPDATE sampleblocks SET sampleformat = 123456 WHERE blockid = 1",
            [],
        )
        .unwrap();
    assert!(findings(&p).contains(&"unknown-format"));
}

#[test]
fn corrupted_samples_are_found_only_when_checksums_are_verified() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(&dir);
    // Same length, different bytes: the cheap checks cannot see this, which is
    // exactly why the expensive one exists.
    p.conn()
        .execute(
            "UPDATE sampleblocks SET samples = zeroblob(length(samples)) WHERE blockid = 4",
            [],
        )
        .unwrap();

    let cheap = validate(&p, Options::default()).unwrap();
    assert!(
        cheap.is_clean(),
        "structure is intact: {:?}",
        cheap.findings
    );
    assert!(!cheap.checksums_verified);

    let thorough = validate(
        &p,
        Options {
            verify_checksums: true,
        },
    )
    .unwrap();
    assert!(thorough.has("checksum-mismatch"), "{:?}", thorough.findings);
}

#[test]
fn a_capture_that_finished_before_it_started_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(&dir);
    p.conn()
        .execute("UPDATE captures SET finished_at = started_at - 1", [])
        .unwrap();
    assert!(findings(&p).contains(&"time-travel"));
}

#[test]
fn a_capture_in_an_unknown_state_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(&dir);
    p.conn()
        .execute("UPDATE captures SET state = 'confused'", [])
        .unwrap();
    assert!(findings(&p).contains(&"unknown-state"));
}

#[test]
fn missing_version_metadata_is_found() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(&dir);
    p.conn()
        .execute("DELETE FROM meta WHERE key = 'created.at'", [])
        .unwrap();
    assert!(findings(&p).contains(&"missing-meta"));
}

#[test]
fn a_missing_table_stops_the_run_rather_than_cascading() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(&dir);
    p.conn()
        .execute_batch("DROP TABLE capture_diagnostics")
        .unwrap();
    let report = validate(&p, Options::default()).unwrap();
    assert_eq!(report.findings.len(), 1, "{:?}", report.findings);
    assert!(report.has("missing-table"));
}

#[test]
fn every_finding_names_the_rows_involved() {
    let dir = tempfile::tempdir().unwrap();
    let p = project(&dir);
    p.conn().pragma_update(None, "foreign_keys", false).unwrap();
    p.conn()
        .execute("DELETE FROM sampleblocks WHERE blockid = 3", [])
        .unwrap();
    p.conn()
        .execute("UPDATE captures SET state = 'confused'", [])
        .unwrap();

    let report = validate(&p, Options::default()).unwrap();
    assert!(!report.is_clean());
    for finding in &report.findings {
        assert!(
            finding.detail.chars().any(|c| c.is_ascii_digit()),
            "finding {:?} names no row: {}",
            finding.code,
            finding.detail
        );
    }
}
