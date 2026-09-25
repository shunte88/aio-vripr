/*
 *  format.rs
 *
 *  VCW - The Vinyl Capture Workstation
 *  (c) 2026 Stue Hunter
 *
 *  PCM sample representation and the honesty rules that attach to it.
 *
 * MIT License
 *
 * Copyright (c) 2026 Stue Hunter
 *
 * Permission is hereby granted, free of charge, to any person obtaining a copy
 * of this software and associated documentation files (the "Software"), to deal
 * in the Software without restriction, including without limitation the rights
 * to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
 * copies of the Software, and to permit persons to whom the Software is
 * furnished to do so, subject to the following conditions:
 *
 * The above copyright notice and this permission notice shall be included in all
 * copies or substantial portions of the Software.
 *
 * THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
 * IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
 * FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
 * AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
 * LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
 * OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
 * SOFTWARE.
 *
 */

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

/// How a block of samples is laid out on disk, and the `sampleformat` code that
/// records it.
///
/// Distinct from [`SampleFormat`], which says what the samples *are*. The same
/// logical format can be stored two ways: Audacity pads 24-bit to four bytes, and
/// we do not. A block therefore carries a storage code, not a sample format, and
/// the codes are a superset of Audacity's three.
///
/// The encoding is Audacity's - `(bytes_per_sample << 16) | type_code` - kept so
/// that imported blocks need no rewriting and stay byte-identical to the source
/// project. Type code 1 is integer and 15 is float in Audacity's space; the two
/// formats it has no code for take type code 2, which Audacity never emits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StorageFormat {
    /// 16-bit integer, 2 bytes. Audacity's code, identical meaning.
    Int16,
    /// 24-bit integer packed into 3 bytes. Ours: this is what the device hands us
    /// and D4 stores it verbatim, where Audacity would pad it.
    Int24Packed,
    /// 24-bit integer padded into 4 bytes. Audacity's code and layout, produced
    /// only by import.
    Int24Padded,
    /// 32-bit integer, 4 bytes. Ours; §8 requires it and Audacity has no code for
    /// it, which is the concrete reason D1 is a superset rather than a clone.
    Int32,
    /// 32-bit float, 4 bytes. Audacity's code, identical meaning.
    Float32,
}

impl StorageFormat {
    /// The `sampleformat` code written into a block row.
    pub const fn code(self) -> u32 {
        match self {
            Self::Int16 => 0x0002_0001,
            Self::Int24Packed => 0x0003_0002,
            Self::Int24Padded => 0x0004_0001,
            Self::Int32 => 0x0004_0002,
            Self::Float32 => 0x0004_000F,
        }
    }

    /// Reads a `sampleformat` code from a `.vcw` or an imported Audacity project.
    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0x0002_0001 => Some(Self::Int16),
            0x0003_0002 => Some(Self::Int24Packed),
            0x0004_0001 => Some(Self::Int24Padded),
            0x0004_0002 => Some(Self::Int32),
            0x0004_000F => Some(Self::Float32),
            _ => None,
        }
    }

    /// Bytes one sample of one channel occupies in the stored blob.
    pub const fn bytes_per_sample(self) -> usize {
        ((self.code() >> 16) & 0xFFFF) as usize
    }

    /// What the samples are, independent of how they are laid out.
    pub const fn sample_format(self) -> SampleFormat {
        match self {
            Self::Int16 => SampleFormat::S16,
            Self::Int24Packed | Self::Int24Padded => SampleFormat::S24,
            Self::Int32 => SampleFormat::S32,
            Self::Float32 => SampleFormat::F32,
        }
    }

    /// Whether Audacity writes this code, i.e. whether a block carrying it could
    /// have arrived by import.
    pub const fn is_audacity(self) -> bool {
        matches!(self, Self::Int16 | Self::Int24Padded | Self::Float32)
    }

    /// How VCW stores a freshly captured sample format: verbatim, no padding (D4).
    pub const fn native_for(format: SampleFormat) -> Self {
        match format {
            SampleFormat::S16 => Self::Int16,
            SampleFormat::S24 => Self::Int24Packed,
            SampleFormat::S32 => Self::Int32,
            SampleFormat::F32 => Self::Float32,
        }
    }

    /// Every storage format, for exhaustive tests and schema documentation.
    pub const ALL: [Self; 5] = [
        Self::Int16,
        Self::Int24Packed,
        Self::Int24Padded,
        Self::Int32,
        Self::Float32,
    ];
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

    #[test]
    fn storage_codes_round_trip_and_are_distinct() {
        let mut seen = std::collections::HashSet::new();
        for s in StorageFormat::ALL {
            assert!(seen.insert(s.code()), "duplicate code for {s:?}");
            assert_eq!(StorageFormat::from_code(s.code()), Some(s));
        }
    }

    #[test]
    fn audacitys_three_codes_are_ours_verbatim() {
        for f in [SampleFormat::S16, SampleFormat::S24, SampleFormat::F32] {
            let code = f.audacity_code().expect("representable");
            let stored = StorageFormat::from_code(code).expect("we read it too");
            assert!(stored.is_audacity());
            assert_eq!(stored.sample_format(), f);
            assert_eq!(Some(stored.bytes_per_sample()), f.audacity_bytes());
        }
    }

    #[test]
    fn our_two_extra_codes_are_outside_audacitys() {
        for s in [StorageFormat::Int24Packed, StorageFormat::Int32] {
            assert!(!s.is_audacity());
            assert_eq!(SampleFormat::from_audacity_code(s.code()), None);
        }
    }

    #[test]
    fn capture_stores_verbatim_and_never_pads() {
        for f in [
            SampleFormat::S16,
            SampleFormat::S24,
            SampleFormat::S32,
            SampleFormat::F32,
        ] {
            let stored = StorageFormat::native_for(f);
            assert_eq!(stored.sample_format(), f);
            assert_eq!(stored.bytes_per_sample(), f.bytes_per_sample());
        }
        assert_eq!(
            StorageFormat::native_for(SampleFormat::S24).bytes_per_sample(),
            3
        );
    }

    #[test]
    fn the_width_lives_in_the_high_half_of_the_code() {
        for s in StorageFormat::ALL {
            assert_eq!(s.bytes_per_sample(), (s.code() >> 16) as usize);
        }
    }
}
