/*
 *  doc.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Generating `docs/SCHEMA.md` from the schema, so the two cannot drift.
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

//! Generating `docs/SCHEMA.md` from the schema, so the two cannot drift.
//!
//! §49 promises the project format is openly documented. A schema document
//! maintained by hand keeps that promise for about one release, so this one is
//! generated: [`markdown`] parses [`SCHEMA_V1`] - comments included, since the
//! comments are the explanation - and `tests/schema_doc.rs` fails if the committed
//! file differs.
//!
//! The parser only has to handle DDL we wrote ourselves, so it is deliberately
//! strict and small. `tests/schema_doc.rs` cross-checks every table and column it
//! finds against `PRAGMA table_info` on a real database, so a parser that
//! misreads the schema fails the build rather than quietly documenting a fiction.

use std::fmt::Write as _;

use vcw_types::StorageFormat;

use crate::schema::{
    APPLICATION_ID, BLOCK_MILLIS, EXTENSION, FORMAT_VERSION, PAGE_SIZE, SCHEMA_V1, SCHEMA_VERSION,
    SUMMARY_64K_STRIDE, SUMMARY_256_STRIDE,
};

/// A column in a parsed `CREATE TABLE`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    /// Column name.
    pub name: String,
    /// Declared type and constraints, as written.
    pub declaration: String,
    /// The `--` comment lines immediately above it, joined.
    pub comment: String,
}

/// A parsed schema object.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Object {
    /// `TABLE` or `INDEX`.
    pub kind: String,
    /// Object name.
    pub name: String,
    /// The `--` comment block immediately above the statement, joined.
    pub comment: String,
    /// Columns, for a table. Empty for an index.
    pub columns: Vec<Column>,
    /// The statement, as written.
    pub sql: String,
}

/// Parses DDL into objects, carrying the comments across.
///
/// # Panics
///
/// On DDL shapes it was not written for. That is intentional: the only input is
/// our own schema, and a silent misparse would produce documentation that looks
/// authoritative and is wrong.
pub fn parse(ddl: &str) -> Vec<Object> {
    let mut objects = Vec::new();
    let mut comment = Vec::new();
    let mut statement = String::new();

    for line in ddl.lines() {
        let trimmed = line.trim();
        if statement.is_empty() {
            if let Some(text) = trimmed.strip_prefix("--") {
                comment.push(text.trim().to_owned());
                continue;
            }
            if trimmed.is_empty() {
                comment.clear();
                continue;
            }
        }
        statement.push_str(line);
        statement.push('\n');
        if trimmed.ends_with(';') {
            objects.push(parse_statement(statement.trim_end(), &comment.join(" ")));
            statement.clear();
            comment.clear();
        }
    }
    assert!(
        statement.trim().is_empty(),
        "unterminated statement: {statement}"
    );
    objects
}

fn parse_statement(sql: &str, comment: &str) -> Object {
    let head = sql.split_whitespace().take(3).collect::<Vec<_>>();
    assert_eq!(head.first(), Some(&"CREATE"), "unexpected statement: {sql}");
    let kind = head[1].to_owned();
    let name = head[2].trim_end_matches('(').to_owned();

    let mut columns = Vec::new();
    if kind == "TABLE" {
        let body = sql
            .split_once('(')
            .expect("CREATE TABLE has a body")
            .1
            .rsplit_once(')')
            .expect("CREATE TABLE closes its body")
            .0;
        let mut pending = Vec::new();
        for line in body.lines() {
            let line = line.trim().trim_end_matches(',').trim();
            if let Some(text) = line.strip_prefix("--") {
                pending.push(text.trim().to_owned());
                continue;
            }
            if line.is_empty() {
                continue;
            }
            // Table-level constraints are not columns.
            if line.starts_with("UNIQUE")
                || line.starts_with("PRIMARY KEY")
                || line.starts_with("FOREIGN KEY")
                || line.starts_with("CHECK")
            {
                pending.clear();
                continue;
            }
            let (col, declaration) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
            columns.push(Column {
                name: col.to_owned(),
                declaration: declaration.split_whitespace().collect::<Vec<_>>().join(" "),
                comment: pending.join(" "),
            });
            pending.clear();
        }
    }

    Object {
        kind,
        name,
        comment: comment.to_owned(),
        columns,
        sql: sql.to_owned(),
    }
}

/// Escapes a value for a markdown table cell. Schema comments contain shift
/// operators and bitwise ors, and an unescaped `|` silently splits the row.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// Renders the schema document.
pub fn markdown() -> String {
    let mut out = String::new();

    out.push_str(
        "<!-- Generated by `cargo test -p vcw-project`. Do not edit by hand:\n     \
         the source is crates/project/src/schema.rs, and tests/schema_doc.rs fails\n     \
         if this file and that one disagree. Re-generate with VCW_BLESS=1. -->\n\n",
    );
    out.push_str("# The `.vcw` project schema\n\n");
    out.push_str(
        "A VCW project is one SQLite file (§12): self-contained, movable, and\n\
         self-identifying. The schema is a deliberate superset of Audacity's AUP4 -\n\
         `sampleblocks` is reproduced column for column so that imported blocks need\n\
         no rewriting, and everything else is what Audacity has nowhere to put. See\n\
         [ADR-0001](adr/0001-project-format.md).\n\n",
    );

    out.push_str("## Identity\n\n");
    out.push_str("| | |\n|---|---|\n");
    let _ = writeln!(out, "| extension | `.{EXTENSION}` |");
    let _ = writeln!(
        out,
        "| `application_id` | `0x{APPLICATION_ID:08X}` (ASCII `VCW\\0`) - Audacity's is \
         `0x41554459`, `AUDY`, for *both* AUP3 and AUP4 |"
    );
    let _ = writeln!(
        out,
        "| `user_version` | {SCHEMA_VERSION} - the schema version, a plain ascending \
         integer |"
    );
    let _ = writeln!(
        out,
        "| format version | {FORMAT_VERSION} - the *meaning* of the schema, in `meta` |"
    );
    let _ = writeln!(out, "| page size | {PAGE_SIZE} bytes, set at creation |");
    let _ = writeln!(out, "| journal mode | WAL, `synchronous=FULL` (D3) |");
    out.push_str(
        "\nDispatch on `application_id` and `user_version`, never on the extension. \
         AUP3 and AUP4 share an `application_id` and differ only in `user_version`, \
         so a reader that trusts the file name is already guessing.\n\n",
    );

    out.push_str("## Capture parameters\n\n");
    let _ = writeln!(
        out,
        "Blocks are **{BLOCK_MILLIS} ms per channel** (D3, provisional from S2). The \
         figure is a recovery-granularity decision, not a throughput one: crash loss is \
         commit granularity plus the driver buffer, and this is the commit granularity.\n"
    );
    let _ = writeln!(
        out,
        "Summaries match Audacity exactly - `(min, max, rms)` f32 triplets over \
         {SUMMARY_256_STRIDE} and {SUMMARY_64K_STRIDE} samples. Measured overhead on a \
         774 MB project: 1.1%.\n"
    );

    out.push_str("## Sample format codes\n\n");
    out.push_str(
        "Audacity's encoding, `(bytes_per_sample << 16) | type_code`, extended with the \
         two formats it has no code for. Audacity never reads a `.vcw` file, so the \
         extension costs nothing; §8 requires 32-bit integer capture and D4 stores \
         24-bit verbatim rather than padded.\n\n",
    );
    out.push_str("| code | format | bytes | origin |\n|---|---|---|---|\n");
    for s in StorageFormat::ALL {
        let _ = writeln!(
            out,
            "| `0x{:08X}` | {:?} | {} | {} |",
            s.code(),
            s,
            s.bytes_per_sample(),
            if s.is_audacity() {
                "Audacity, identical meaning"
            } else {
                "VCW"
            }
        );
    }
    out.push('\n');

    out.push_str("## Tables\n\n");
    for object in parse(SCHEMA_V1) {
        if object.kind != "TABLE" {
            continue;
        }
        let _ = writeln!(out, "### `{}`\n", object.name);
        if !object.comment.is_empty() {
            let _ = writeln!(out, "{}\n", object.comment);
        }
        out.push_str("| column | declaration | notes |\n|---|---|---|\n");
        for c in &object.columns {
            let _ = writeln!(
                out,
                "| `{}` | `{}` | {} |",
                c.name,
                cell(&c.declaration),
                cell(&c.comment)
            );
        }
        out.push('\n');
    }

    out.push_str("## Indices\n\n");
    for object in parse(SCHEMA_V1) {
        if object.kind != "INDEX" {
            continue;
        }
        let _ = writeln!(out, "- `{}`", object.name);
        if !object.comment.is_empty() {
            let _ = writeln!(out, "  {}", object.comment);
        }
    }
    out.push('\n');

    out.push_str("## What is not here yet\n\n");
    out.push_str(
        "Schema v1 covers capture: blocks, sessions, diagnostics and versioning - \
         everything milestone M1, *it records*, depends on. The vinyl data model \
         (releases, discs, sides, tracks, boundaries), metadata, identification \
         evidence, artwork and export settings arrive as later migrations, at WP-13 \
         and beyond. That is what the migration machinery is for, and writing those \
         tables now would be guessing at shapes three work packages away.\n",
    );

    out
}
