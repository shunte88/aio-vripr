//! The capture stream: mode negotiation, the RT callback, and format verification.
//!
//! Filled by WP-04. Two obligations are already non-negotiable here. The callback
//! does no allocation, no locking and no I/O (§10). And the negotiated format is
//! cross-checked against the operating system - `/proc/asound/*/hw_params` on Linux,
//! the WASAPI exclusive-mode format on Windows - before anything reports bit-perfect
//! operation (§9, S1 finding 1).
