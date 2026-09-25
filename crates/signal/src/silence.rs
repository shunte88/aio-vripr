//! Silence and inter-track gap detection (§22).
//!
//! Filled by WP-11, ported from VRipr's RMS and spectral-flatness detectors. The port
//! splits decode from analyse: VRipr's detectors take a file path, and ours have to
//! run on a live stream of feature frames.
