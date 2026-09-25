//! Acoustic fingerprinting and AcoustID lookup.
//!
//! Requirements: §25 (fingerprinting), §26, §27 (lookup). Phase 2 - the crate exists
//! now so the dependency direction is fixed before there is code to misplace.
//!
//! S4 settled how fingerprinting attaches to capture: it runs off the capture stream
//! at the capture rate with plain `>> 16` narrowing to `i16`. Rate, gain and
//! narrowing all proved free, so there is no staging file and no pre-decimation.

pub mod acoustid;
pub mod chromaprint;
