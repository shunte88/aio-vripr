//! Bounded lock-free PCM distribution from the callback to the workers (§10).
//!
//! Filled by WP-04. S2 settled the sizing question and inverted the expected answer:
//! ring capacity does not affect crash loss, which is bounded by commit granularity
//! plus the driver buffer. The ring is sized for jitter tolerance alone - at least
//! 500 ms - and the recovery budget is spent on commit granularity instead.
