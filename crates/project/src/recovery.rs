//! Detecting and reconstructing an unfinished session (§15).
//!
//! Filled by WP-06. S2 established the floor this cannot beat: loss is commit
//! granularity plus the driver buffer, rounded to a block boundary. Ring size is
//! irrelevant to it. Anything claiming better is measuring wrong.
