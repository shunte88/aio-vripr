//! Tracks, boundaries and the non-destructive edit model (§29, §31).
//!
//! Filled by WP-13. Every edit is an instruction over immutable blocks; §4.1 and §31
//! mean a split, merge or move never rewrites captured audio. WP-13's exit criterion
//! asserts exactly that in a test.
