//! The capture writer thread: batched transactions and checkpoint policy (§13, §14).
//!
//! Filled by WP-05. S2 provisionally settled the parameters (D3): per-channel blocks
//! of 250 ms, batch 1, WAL, `synchronous=FULL`, ring at least 500 ms. The rationale
//! inverted the starting hypothesis - throughput turned out to be a non-issue at
//! 24/192, so the budget buys recovery granularity instead of headroom.
