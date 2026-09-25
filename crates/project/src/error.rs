/*
 *  error.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  What can go wrong opening, creating or validating a project.
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

//! What can go wrong opening, creating or validating a project.

use std::path::PathBuf;

/// A project-layer failure.
///
/// Refusals are deliberately specific. §15 makes recovery a correctness
/// requirement, and a recovery tool that reports "could not open project" has
/// thrown away the information the user needs.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The file exists but is not a VCW project.
    #[error(
        "{path} is not a VCW project: application_id is 0x{found:08X}{}",
        describe_application_id(*found)
    )]
    NotAProject {
        /// The file we were asked to open.
        path: PathBuf,
        /// The `application_id` the file actually carries.
        found: u32,
    },

    /// The project was written by a newer VCW than this one.
    ///
    /// Refused rather than guessed at. §16 allows newer versions to upgrade older
    /// projects; it says nothing about older versions reading newer ones, and a
    /// partial read of a schema we do not know is how data gets lost.
    #[error(
        "{path} has schema version {found}, and this build understands up to {supported}. \
         Upgrade VCW to open it."
    )]
    SchemaTooNew {
        /// The file we were asked to open.
        path: PathBuf,
        /// The schema version in the file.
        found: u32,
        /// The newest schema version this build can read.
        supported: u32,
    },

    /// A migration failed, and was rolled back.
    #[error("migration to schema version {version} ({description}) failed and was rolled back")]
    Migration {
        /// The version being migrated to.
        version: u32,
        /// That migration's description.
        description: String,
        /// The underlying SQLite failure.
        #[source]
        source: rusqlite::Error,
    },

    /// The project opened, but its contents are not self-consistent.
    #[error("{path} failed validation: {} problem(s)", findings.len())]
    Invalid {
        /// The project that failed.
        path: PathBuf,
        /// Every problem found, not just the first.
        findings: Vec<String>,
    },

    /// A capture cannot be written as it is described.
    ///
    /// Only reachable through the library, never from a device: a stream with no
    /// channels has no frames, and a writer asked for zero-length blocks would
    /// spin forever rather than fail. Refusing at the door is cheaper than making
    /// every loop defend itself.
    #[error(
        "a capture with {channels} channel(s) at {frame_bytes} bytes per frame cannot be written"
    )]
    Unwritable {
        /// The channel count asked for.
        channels: u16,
        /// The frame width that implies.
        frame_bytes: usize,
    },

    /// The capture writer thread ended without reporting an outcome.
    ///
    /// Only reachable if it panicked. Its ordinary failure path closes the
    /// session as interrupted and returns the error, so this variant means
    /// something worse happened than a refused write, and the project should be
    /// treated as needing recovery rather than as merely short.
    #[error("the capture writer ended without reporting; the project needs recovery")]
    WriterLost,

    /// A write was attempted through a read-only handle.
    ///
    /// Not a programming slip worth a panic: recovery is routinely offered a
    /// project the caller opened read-only to inspect it, and "reopen it for
    /// writing" is a better answer than a crash.
    #[error("{path} is open read-only and cannot be written")]
    ReadOnly {
        /// The project in question.
        path: PathBuf,
    },

    /// Recovery found blocks it is not allowed to remove without being asked.
    ///
    /// Blocks are immutable and never deleted to tidy up (D4), so recovery
    /// refuses rather than quietly destroying captured audio. See
    /// [`crate::recovery::Plan::Repair`].
    #[error(
        "capture {capture_id} has {blocks} block(s) stranded past the recoverable end; \
         recovering would discard them, so it needs Plan::Repair"
    )]
    StrandedBlocks {
        /// The capture concerned.
        capture_id: i64,
        /// How many blocks would be discarded.
        blocks: usize,
    },

    /// SQLite said no.
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),

    /// The filesystem said no.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Adds a hint when the `application_id` is one we recognise.
fn describe_application_id(id: u32) -> &'static str {
    match id {
        0x4155_4459 => {
            " - that is an Audacity project. Import it instead; VCW reads .aup3 and .aup4."
        }
        0 => {
            " - the file has no application_id, so it is a plain SQLite database or not SQLite at all."
        }
        _ => "",
    }
}

/// A project-layer result.
pub type Result<T> = std::result::Result<T, Error>;
