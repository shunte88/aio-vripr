//! Cutting committed blocks into per-track streams using the edit instructions.
//!
//! Filled by WP-14. Block-aligned where it can be and sample-accurate where it must
//! be; a track boundary rarely lands on a block boundary.
