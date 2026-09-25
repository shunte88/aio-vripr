/*
 *  meta.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Project metadata: §16's four versions, and configuration that outgrows a
 *  column.
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

//! Project metadata: §16's four versions, and configuration that outgrows a column.
//!
//! Key/value, because the set grows with nearly every work package and a migration
//! per setting is a poor trade. Values are text; callers parse. The keys §16
//! requires are named constants here so a typo is a compile error rather than a
//! silently missing version.

use rusqlite::{Connection, OptionalExtension};

/// The project-format version at creation (§16, "creation version").
pub const CREATED_FORMAT_VERSION: &str = "created.format_version";
/// The build that created the project.
pub const CREATED_BY: &str = "created.by";
/// Unix seconds at creation.
pub const CREATED_AT: &str = "created.at";
/// The project-format version as last written (§16, "last-written version").
pub const FORMAT_VERSION: &str = "format_version";
/// The build that last wrote to the project.
pub const LAST_WRITTEN_BY: &str = "last_written.by";
/// Unix seconds at the last write.
pub const LAST_WRITTEN_AT: &str = "last_written.at";

/// Keys §16 requires every project to carry.
pub const REQUIRED_KEYS: [&str; 6] = [
    CREATED_AT,
    CREATED_BY,
    CREATED_FORMAT_VERSION,
    FORMAT_VERSION,
    LAST_WRITTEN_AT,
    LAST_WRITTEN_BY,
];

/// Reads a key, or `None` if it is not set.
pub fn get(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
    conn.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
        .optional()
}

/// Sets a key, replacing any existing value.
pub fn set(conn: &Connection, key: &str, value: &str) -> rusqlite::Result<()> {
    conn.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2)
         ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

/// Every key/value pair, ascending by key.
pub fn all(conn: &Connection) -> rusqlite::Result<Vec<(String, String)>> {
    let mut stmt = conn.prepare("SELECT key, value FROM meta ORDER BY key")?;
    let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    rows.collect()
}
