//! The HMM that turns per-frame features into track boundaries (§23, §24).
//!
//! Filled by WP-11. Boundaries carry provenance and confidence out of this module:
//! §24 requires the UI to distinguish a detected boundary from a user-placed one,
//! and a locked boundary must survive re-analysis untouched.
