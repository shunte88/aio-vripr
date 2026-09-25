//! Turning evidence into an identified release.
//!
//! Requirements: §45 and the identification scope behind gate G3. Phase 2.
//!
//! The split here is the design: evidence is *collected* without being judged,
//! candidates are scored against it, and the resolver commits to one - so a wrong
//! identification can be traced back to the evidence that caused it rather than
//! disappearing into a single opaque score.

pub mod candidate;
pub mod confidence;
pub mod evidence;
pub mod resolver;
