//! Axis domain types.
//!
//! `AxisFilter` mirrors the 3-bit `filter` field at `AxisConfig::filter()` —
//! the firmware reads it as `FILTER_NO..FILTER_LEVEL_7` (see
//! `vendored/common_defines.h::filter_t`). The Qt configurator surfaces
//! it as a labelled slider; we expose the same labels for the Slint UI.

/// `filter_t` from `vendored/common_defines.h`.
///
/// Ordering and labels match `axesextended.h::m_filterList` so the Slint
/// dropdown reads the same as the Qt slider tooltip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisFilter {
    Off,
    Level1,
    Level2,
    Level3,
    Level4,
    Level5,
    Level6,
    Level7,
}

impl AxisFilter {
    /// Wire value (the `filter` 3-bit field).
    #[must_use]
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Map a wire value back to the typed enum. Input is masked to
    /// 3 bits before lookup so every `u8` round-trips through the
    /// same eight variants the firmware enumerates.
    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x07 {
            1 => Self::Level1,
            2 => Self::Level2,
            3 => Self::Level3,
            4 => Self::Level4,
            5 => Self::Level5,
            6 => Self::Level6,
            7 => Self::Level7,
            _ => Self::Off,
        }
    }

    /// Human label, matching the Qt configurator's filter slider entries.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Level1 => "level 1",
            Self::Level2 => "level 2",
            Self::Level3 => "level 3",
            Self::Level4 => "level 4",
            Self::Level5 => "level 5",
            Self::Level6 => "level 6",
            Self::Level7 => "level 7",
        }
    }

    /// All filter variants in wire order. Drives the Slint dropdown.
    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::Off,
            Self::Level1,
            Self::Level2,
            Self::Level3,
            Self::Level4,
            Self::Level5,
            Self::Level6,
            Self::Level7,
        ]
        .into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filter_round_trips_through_u8() {
        for v in 0u8..=7 {
            assert_eq!(AxisFilter::from_u8(v).to_u8(), v);
        }
    }

    #[test]
    fn from_u8_masks_high_bits() {
        assert_eq!(AxisFilter::from_u8(0xf8), AxisFilter::Off);
        assert_eq!(AxisFilter::from_u8(0xff), AxisFilter::Level7);
    }

    #[test]
    fn all_yields_eight_unique_variants() {
        let xs: Vec<_> = AxisFilter::all().collect();
        assert_eq!(xs.len(), 8);
        for (i, f) in xs.iter().enumerate() {
            assert_eq!(f.to_u8() as usize, i);
        }
    }
}
