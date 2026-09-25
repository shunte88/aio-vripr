//! The `.vcw` project: a single self-contained SQLite file.
//!
//! Requirements: §12 (format), §13, §14 (block capture and transactions), §15
//! (recovery), §16, §29, §31, §49.
//!
//! D1 fixes the shape: the schema is a deliberate *superset* of Audacity's AUP4 -
//! `sampleblocks` column-for-column identical, same summary pyramids, blocks never
//! updated once written - plus the tables Audacity has no equivalent for. The file
//! self-identifies through SQLite's `application_id` and `user_version`, so the
//! reader never has to trust the extension.
//!
//! Superset, not clone: §8 requires 32-bit integer capture and Audacity has no
//! sample-format code for it (S5). Compatibility is architectural and one-way -
//! we import `.aup3` and `.aup4`, we do not write them.

pub mod disc;
pub mod persistence;
pub mod recovery;
pub mod session;
pub mod side;
pub mod sqlite;
pub mod track;
