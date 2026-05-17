//! Encoder domain types.
//!
//! `EncoderMode` mirrors the `encoder_t` enum from
//! `vendored/common_types.h` (1x / 2x / 4x decoding). Both the
//! `encoders[16]` soft-encoder slots and `fast_encoders[2].mode`
//! carry this value.

/// `encoder_t` from `vendored/common_types.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum EncoderMode {
    /// `ENCODER_CONF_1x` — one count per full quadrature cycle.
    X1 = 0,
    /// `ENCODER_CONF_2x` — two counts per full cycle.
    X2 = 1,
    /// `ENCODER_CONF_4x` — four counts per full cycle (every edge).
    X4 = 2,
}

impl EncoderMode {
    #[must_use]
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Decode the wire byte. Returns `None` for values outside the
    /// 3-variant enum — the codec keeps the raw byte either way so
    /// round-trip is preserved.
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::X1),
            1 => Some(Self::X2),
            2 => Some(Self::X4),
            _ => None,
        }
    }

    /// Human label. Matches the Qt configurator's mode-picker dropdown.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::X1 => "1x",
            Self::X2 => "2x",
            Self::X4 => "4x",
        }
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [Self::X1, Self::X2, Self::X4].into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_u8() {
        for v in 0u8..=2 {
            assert_eq!(EncoderMode::from_u8(v).map(EncoderMode::to_u8), Some(v));
        }
    }

    #[test]
    fn unknown_value_returns_none() {
        assert!(EncoderMode::from_u8(3).is_none());
        assert!(EncoderMode::from_u8(0xff).is_none());
    }
}
