//! Enumeration, capability probing and device selection (§7).
//!
//! Filled by WP-03. The shape it has to take is already constrained by S1: devices
//! are selected by PCM id through `HostTrait::device_by_id` rather than by name,
//! because names are not stable and, on ALSA before CPAL 0.18, were the plug layer's
//! invention rather than the hardware's.

/// The host APIs this build of CPAL can actually use on this platform.
///
/// A smoke check, not the §7 device matrix - that is WP-03. `vcw doctor` prints it
/// so a support question starts from what the binary can see rather than from what
/// the platform is supposed to offer.
pub fn available_hosts() -> Vec<&'static str> {
    cpal::available_hosts()
        .into_iter()
        .map(|h| h.name())
        .collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn at_least_one_host_is_compiled_in() {
        assert!(!super::available_hosts().is_empty());
    }
}
