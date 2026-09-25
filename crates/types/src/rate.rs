//! Sample rates.

use serde::{Deserialize, Serialize};

/// A sample rate in Hz.
///
/// Deliberately a newtype and deliberately an integer. S5 found Audacity storing
/// rates as `f64` in two places that disagree, and an importer that picks the wrong
/// one plays a 48 kHz rip at 4x speed; keeping our own rate an integer makes that
/// class of mistake harder to repeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SampleRate(pub u32);

impl SampleRate {
    /// The rate in Hz.
    pub const fn hz(self) -> u32 {
        self.0
    }

    /// Whether this is one of the rates §8 requires support for.
    pub fn is_standard(self) -> bool {
        STANDARD_RATES.contains(&self)
    }
}

impl std::fmt::Display for SampleRate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} Hz", self.0)
    }
}

/// The hardware sample rates §8 requires support for, ascending.
pub const STANDARD_RATES: [SampleRate; 6] = [
    SampleRate(44_100),
    SampleRate(48_000),
    SampleRate(88_200),
    SampleRate(96_000),
    SampleRate(176_400),
    SampleRate(192_000),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_rates_are_ascending_and_complete() {
        assert!(STANDARD_RATES.windows(2).all(|w| w[0] < w[1]));
        assert!(SampleRate(192_000).is_standard());
        assert!(!SampleRate(8_000).is_standard());
    }
}
