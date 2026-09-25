/*
 *  aup4_shape.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  D1's central claim, checked rather than asserted: our `sampleblocks` is
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

//! D1's central claim, checked rather than asserted: our `sampleblocks` is
//! Audacity's, column for column.
//!
//! The reference is DDL extracted verbatim from real projects in the corpus
//! (`fixtures/*.sql`, provenance recorded in their headers). If a future migration
//! adds a column to `sampleblocks`, or reorders one, or tightens a constraint, this
//! test fails - which is the point. The superset grows by adding *tables*, never by
//! editing Audacity's.

use rusqlite::Connection;

const AUP3: &str = include_str!("fixtures/aup3-schema.sql");
const AUP4: &str = include_str!("fixtures/aup4-schema.sql");

/// One row of `PRAGMA table_info`.
#[derive(Debug, PartialEq, Eq)]
struct ColumnInfo {
    cid: i64,
    name: String,
    ty: String,
    notnull: bool,
    default: Option<String>,
    pk: i64,
}

fn columns(conn: &Connection, table: &str) -> Vec<ColumnInfo> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap();
    stmt.query_map([], |r| {
        Ok(ColumnInfo {
            cid: r.get(0)?,
            name: r.get(1)?,
            ty: r.get(2)?,
            notnull: r.get::<_, i64>(3)? != 0,
            default: r.get(4)?,
            pk: r.get(5)?,
        })
    })
    .unwrap()
    .collect::<rusqlite::Result<Vec<_>>>()
    .unwrap()
}

fn create_sql(conn: &Connection, table: &str) -> String {
    conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
        [table],
        |r| r.get(0),
    )
    .unwrap()
}

fn db_from(ddl: &str) -> Connection {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(ddl).unwrap();
    conn
}

fn vcw() -> Connection {
    db_from(vcw_project::schema::SCHEMA_V1)
}

#[test]
fn our_sampleblocks_matches_aup4_column_for_column() {
    let ours = columns(&vcw(), "sampleblocks");
    let theirs = columns(&db_from(AUP4), "sampleblocks");
    assert_eq!(
        ours, theirs,
        "sampleblocks has diverged from AUP4. D1 makes this table Audacity's; \
         the superset grows by adding tables, not by editing this one."
    );
}

#[test]
fn our_sampleblocks_matches_aup3_too() {
    let ours = columns(&vcw(), "sampleblocks");
    let theirs = columns(&db_from(AUP3), "sampleblocks");
    assert_eq!(ours, theirs);
}

#[test]
fn audacity_did_not_change_sampleblocks_between_aup3_and_aup4() {
    // S5's finding, and the evidence D1 rests on. Kept as a test because if it
    // ever stops being true the importer has a real problem.
    assert_eq!(
        columns(&db_from(AUP3), "sampleblocks"),
        columns(&db_from(AUP4), "sampleblocks")
    );
}

#[test]
fn the_primary_key_is_autoincrement_like_audacitys() {
    // AUTOINCREMENT does not show up in table_info, and it matters: it is what
    // stops SQLite reusing a blockid after a delete, which in a format where
    // blocks may be shared would silently re-point a reference.
    for (label, conn) in [
        ("vcw", vcw()),
        ("aup3", db_from(AUP3)),
        ("aup4", db_from(AUP4)),
    ] {
        let sql = create_sql(&conn, "sampleblocks");
        assert!(
            sql.contains("AUTOINCREMENT"),
            "{label} sampleblocks is not AUTOINCREMENT"
        );
    }
}

#[test]
fn we_add_tables_audacity_does_not_have_and_omit_the_ones_we_cannot_use() {
    let ours = tables(&vcw());
    let theirs = tables(&db_from(AUP4));

    for added in [
        "captures",
        "capture_blocks",
        "capture_diagnostics",
        "meta",
        "schema_migrations",
    ] {
        assert!(
            ours.contains(&added.to_string()),
            "{added} missing from the schema"
        );
        assert!(
            !theirs.contains(&added.to_string()),
            "{added} unexpectedly present in AUP4"
        );
    }

    // Audacity's document blob and its save history are the parts we deliberately
    // do not reproduce: our project state lives in tables, so that recovery works
    // from committed rows with no document to parse.
    for declined in ["project", "autosave", "project_history"] {
        assert!(
            !ours.contains(&declined.to_string()),
            "{declined} should not be in the schema"
        );
    }
}

fn tables(conn: &Connection) -> Vec<String> {
    let mut stmt = conn
        .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name")
        .unwrap();
    stmt.query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap()
}
