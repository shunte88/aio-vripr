//! PCM sample representation and the honesty rules that attach to it.

use serde::{Deserialize, Serialize};

/// A PCM sample representation, stored verbatim as the device supplies it (D4).
///
/// §9 forbids format conversion on the capture path, so this is a tag travelling
/// beside the bytes rather than a conversion target. §8 requires all four variants
/// where the hardware permits them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SampleFormat {
    /// 16-bit signed integer, little-endian.
    S16,
    /// 24-bit signed integer packed into 3 bytes, little-endian.
    S24,
    /// 32-bit signed integer, little-endian. Has no Audacity equivalent - see
    /// [`SampleFormat::audacity_code`].
    S32,
    /// 32-bit IEEE 754 float, little-endian.
    F32,
}

impl SampleFormat {
    /// Bytes occupied by one sample of one channel at rest.
    ///
    /// Note that [`SampleFormat::S24`] is 3 bytes here but 4 in an Audacity
    /// `sampleblocks` row, which pads it - see [`SampleFormat::audacity_bytes`].
    pub const fn bytes_per_sample(self) -> usize {
        match self {
            Self::S16 => 2,
            Self::S24 => 3,
            Self::S32 | Self::F32 => 4,
        }
    }

    /// The `sampleformat` code Audacity writes into an AUP3/AUP4 `sequence` element,
    /// or `None` for a format Audacity cannot represent.
    ///
    /// Measured from the corpus rather than taken from Audacity's source (S5). The
    /// `None` arm is exactly why D1 makes the `.vcw` schema a *superset* of AUP4's:
    /// §8 requires 32-bit integer capture and Audacity has no code for it.
    pub const fn audacity_code(self) -> Option<u32> {
        match self {
            Self::S16 => Some(0x0002_0001),
            Self::S24 => Some(0x0004_0001),
            Self::F32 => Some(0x0004_000F),
            Self::S32 => None,
        }
    }

    /// Bytes per sample in an Audacity `sampleblocks` payload, or `None` for a format
    /// Audacity cannot represent. Audacity pads 24-bit to 4 bytes; we do not.
    pub const fn audacity_bytes(self) -> Option<usize> {
        match self {
            Self::S16 => Some(2),
            Self::S24 | Self::F32 => Some(4),
            Self::S32 => None,
        }
    }

    /// Reads an Audacity `sampleformat` code, for the AUP3/AUP4 importer (WP-20).
    pub const fn from_audacity_code(code: u32) -> Option<Self> {
        match code {
            0x0002_0001 => Some(Self::S16),
            0x0004_0001 => Some(Self::S24),
            0x0004_000F => Some(Self::F32),
            _ => None,
        }
    }
}

/// How the capture stream asks the host to open the device (§9).
///
/// The mode is a *request*. What the hardware actually did is a separate,
/// OS-confirmed fact: §9 forbids claiming bit-perfect operation on the strength of
/// the API's own report, and S1 found CPAL 0.16 reporting a silent 8 kHz -> 48 kHz
/// upsample as an honoured request. WP-04 carries the per-platform verifier that
/// settles it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CaptureMode {
    /// The OS mixer owns the device; conversion is likely and must be reported.
    Shared,
    /// The device's own default configuration, taken unmodified.
    Native,
    /// Exclusive hardware access where the platform offers it (WASAPI exclusive,
    /// ALSA `hw:`); the only mode in which bit-perfection is plausible.
    Exclusive,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audacity_codes_round_trip() {
        for f in [SampleFormat::S16, SampleFormat::S24, SampleFormat::F32] {
            let code = f.audacity_code().expect("representable in Audacity");
            assert_eq!(SampleFormat::from_audacity_code(code), Some(f));
        }
    }

    #[test]
    fn s32_is_the_superset_case() {
        assert_eq!(SampleFormat::S32.audacity_code(), None);
        assert_eq!(SampleFormat::S32.audacity_bytes(), None);
        assert_eq!(SampleFormat::S32.bytes_per_sample(), 4);
    }

    #[test]
    fn audacity_pads_24_bit_and_we_do_not() {
        assert_eq!(SampleFormat::S24.bytes_per_sample(), 3);
        assert_eq!(SampleFormat::S24.audacity_bytes(), Some(4));
    }
}
