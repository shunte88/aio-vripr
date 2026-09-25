/*
 *  schema.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  Schema v1, and the constants that identify a `.vcw` file.
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

//! Schema v1, and the constants that identify a `.vcw` file.
//!
//! The DDL lives here as text rather than being built by a query builder, because
//! it is the thing the format *is*. It is diffed against a real Audacity project in
//! CI (`tests/aup4_shape.rs`) and rendered into `docs/SCHEMA.md` by
//! [`crate::schema::markdown`], so there is one definition and two checks on it.

/// SQLite `application_id` for a `.vcw` project: ASCII `"VCW\0"`.
///
/// Distinct from Audacity's `0x41554459` (`"AUDY"`), which both AUP3 and AUP4 use.
pub const APPLICATION_ID: u32 = 0x5643_5700;

/// The newest schema version this build understands.
///
/// Stored in SQLite's `user_version`. A plain ascending integer, deliberately not
/// Audacity's packed dotted quad - we have one number to express and no reason to
/// pack four into it.
pub const SCHEMA_VERSION: u32 = 1;

/// The project-format version: the *meaning* of the schema, as opposed to its shape.
///
/// §16 asks for both. They move independently: adding a table bumps
/// [`SCHEMA_VERSION`] alone, while changing what an existing column means - a unit,
/// an encoding, an invariant - bumps this too, and that is the one that makes an
/// older reader unsafe rather than merely incomplete.
pub const FORMAT_VERSION: u32 = 1;

/// The file extension. One file, movable, self-contained (§12).
pub const EXTENSION: &str = "vcw";

/// Page size for a new project.
///
/// 64 KiB, which is what Audacity uses for full-length projects and what S2
/// benchmarked against. Sample blobs are ~1 MiB, so a large page keeps a block to a
/// short chain of overflow pages. Must be set before the first table exists, which
/// is why it is applied on a freshly created file and never on open.
///
/// S2's page-size sweep is still outstanding, so treat this as measured-adjacent
/// rather than tuned: it is Audacity's choice, validated by not being a problem
/// across a 90-minute soak.
pub const PAGE_SIZE: u32 = 65_536;

/// Samples per block, per channel, at 48 kHz - 250 ms (D3, provisional from S2).
///
/// The number is a recovery-granularity decision, not a throughput one. S2 found
/// throughput to be a non-issue at 24/192, so the budget buys a smaller worst-case
/// loss window instead: crash loss is commit granularity plus the driver buffer,
/// and this is the commit granularity.
pub const BLOCK_MILLIS: u32 = 250;

/// Samples per `summary256` triplet, matching Audacity exactly.
pub const SUMMARY_256_STRIDE: u32 = 256;

/// Samples per `summary64k` triplet, matching Audacity exactly.
pub const SUMMARY_64K_STRIDE: u32 = 65_536;

/// Schema v1.
///
/// Two halves. `sampleblocks` is Audacity's table, column for column and
/// constraint for constraint - including the absence of `NOT NULL`, so the diff
/// against a real `.aup4` is literal rather than approximate. Everything else is
/// the superset: the provenance, diagnostics and versioning Audacity has nowhere to
/// put, and which §10, §15 and §16 require.
pub const SCHEMA_V1: &str = r#"
-- Audacity's table, reproduced exactly. Column names, types, order, the
-- AUTOINCREMENT primary key and the absence of NOT NULL are all deliberate:
-- tests/aup4_shape.rs diffs this against DDL extracted from a real project.
-- Integrity is enforced next door in capture_blocks and by validate(), not by
-- constraints that would break the match.
CREATE TABLE sampleblocks (
    -- Never reused and never updated (D4). A block is written once; edits
    -- produce new blocks and leave the old ones for the undo history.
    blockid      INTEGER PRIMARY KEY AUTOINCREMENT,
    -- (bytes_per_sample << 16) | type_code. See the format table above.
    sampleformat INTEGER,
    -- Minimum sample value in the block, as f32. Audacity's whole-block summary.
    summin       REAL,
    -- Maximum sample value in the block, as f32.
    summax       REAL,
    -- RMS across the block, as f32.
    sumrms       REAL,
    -- (min, max, rms) f32 triplets, one per 256 samples. The waveform pyramid.
    summary256   BLOB,
    -- The same, one triplet per 65536 samples.
    summary64k   BLOB,
    -- The audio itself, native-endian, exactly as captured (D4: no conversion
    -- on the write path).
    samples      BLOB
);

-- One row per capture session (§13, §15). A session is unfinished exactly when
-- finished_at IS NULL, which is what recovery looks for on the next launch.
CREATE TABLE captures (
    -- Stable for the life of the project; referenced by every block.
    capture_id     INTEGER PRIMARY KEY AUTOINCREMENT,
    -- Hz, as negotiated with the device. Authoritative, unlike Audacity's
    -- project/@rate, which is only an editor preference.
    sample_rate    INTEGER NOT NULL,
    -- Channel count. Blocks are stored per channel, never interleaved.
    channels       INTEGER NOT NULL,
    -- StorageFormat code, matching sampleblocks.sampleformat for this capture.
    storage_format INTEGER NOT NULL,
    -- How the stream was opened: 'exclusive', 'native' or 'shared' (§9). These are
    -- CaptureMode's three spellings; a request, not a confirmed outcome.
    capture_mode   TEXT    NOT NULL,
    -- CPAL host, e.g. 'ALSA', 'WASAPI', 'CoreAudio'. Recorded for provenance.
    host_api       TEXT,
    -- Stable device identifier where the platform offers one.
    device_id      TEXT,
    -- Human-readable device name at the time of capture.
    device_name    TEXT,
    -- §9 and S1: bit-perfection is never claimed on CPAL's word. 0 until the
    -- platform verifier has confirmed the negotiated format against the OS.
    os_verified    INTEGER NOT NULL DEFAULT 0,
    -- What the OS reported, verbatim, so a later reader can audit the claim.
    os_report      TEXT,
    -- Unix seconds at stream start.
    started_at     INTEGER NOT NULL,
    -- Unix seconds at clean stop. NULL means interrupted: recovery's signal.
    finished_at    INTEGER,
    -- Frames committed per channel. Updated as blocks land, so it survives a crash.
    frames         INTEGER NOT NULL DEFAULT 0,
    -- 'recording', 'finalised' or 'interrupted'. validate() rejects anything else.
    state          TEXT    NOT NULL DEFAULT 'recording'
);

-- The superset half: where each sample block came from and how to find it again.
-- Audacity keeps this in its document blob; we keep it in a table, because
-- recovery has to work from committed rows alone with no document to parse.
CREATE TABLE capture_blocks (
    -- One row per sample block, sharing its key. Blocks Audacity wrote and we
    -- imported have no row here, which is how the two halves stay separable.
    blockid      INTEGER PRIMARY KEY REFERENCES sampleblocks(blockid),
    -- The session this block belongs to.
    capture_id   INTEGER NOT NULL REFERENCES captures(capture_id),
    -- Zero-based channel index.
    channel      INTEGER NOT NULL,
    -- Zero-based position within the channel. Contiguous, with no gaps.
    sequence     INTEGER NOT NULL,
    -- Frame offset from the start of the capture. Redundant against sequence and
    -- frame_count by design: validate() checks them against each other.
    start_frame  INTEGER NOT NULL,
    -- Frames in this block. The last block of a capture is usually short.
    frame_count  INTEGER NOT NULL,
    -- CRC32 of the samples blob, computed before the write. Detects bit rot that
    -- SQLite's own integrity check cannot see.
    checksum     INTEGER NOT NULL,
    -- Unix seconds at commit. Bounds how much a crash can have cost.
    committed_at INTEGER NOT NULL,
    UNIQUE (capture_id, channel, sequence)
);

-- Ordered playback and recovery both walk the timeline, and both do it per channel.
CREATE INDEX capture_blocks_timeline
    ON capture_blocks (capture_id, channel, start_frame);

-- §10 requires overruns, underruns, dropped frames and stream errors counted and
-- *persisted*: they belong to the recording, not to the process that made it.
CREATE TABLE capture_diagnostics (
    -- One row per capture, created with it.
    capture_id     INTEGER PRIMARY KEY REFERENCES captures(capture_id),
    -- Times the capture ring filled before the writer drained it.
    overruns       INTEGER NOT NULL DEFAULT 0,
    -- Times the device callback found no data ready.
    underruns      INTEGER NOT NULL DEFAULT 0,
    -- Frames known to be lost. Non-zero means the capture is not bit-perfect.
    dropped_frames INTEGER NOT NULL DEFAULT 0,
    -- Stream errors reported by the host.
    stream_errors  INTEGER NOT NULL DEFAULT 0,
    -- Unix seconds of the last update to this row.
    updated_at     INTEGER NOT NULL
);

-- §16's four versions, plus project configuration. Key/value because the set grows
-- with every work package and a migration per setting is not a good trade.
CREATE TABLE meta (
    -- Dotted key, e.g. 'created.by', 'format_version'. See meta.rs for the
    -- required set, which is checked on open.
    key   TEXT PRIMARY KEY,
    -- Always text. Callers parse; the schema does not pretend to types it
    -- cannot enforce.
    value TEXT NOT NULL
);

-- What has been applied, when, and by which build. §16 requires transactional
-- migrations; this is the record that makes a half-applied one detectable.
CREATE TABLE schema_migrations (
    -- Matches user_version once the run completes.
    version     INTEGER PRIMARY KEY,
    -- The migration's own description, copied at apply time.
    description TEXT    NOT NULL,
    -- Unix seconds.
    applied_at  INTEGER NOT NULL,
    -- Crate name and version of the build that applied it.
    applied_by  TEXT    NOT NULL
);
"#;

/// Tables schema v1 must contain. Checked on open, so a truncated or
/// partially-migrated file is refused rather than half-read.
pub const REQUIRED_TABLES: [&str; 6] = [
    "capture_blocks",
    "capture_diagnostics",
    "captures",
    "meta",
    "sampleblocks",
    "schema_migrations",
];
