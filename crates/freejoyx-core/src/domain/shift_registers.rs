//! Shift register domain types.
//!
//! `ShiftRegType` mirrors `shift_reg_config_type_t` from
//! `vendored/common_types.h`. The wire field is the `type` byte on
//! `shift_reg_config_t`; the variants encode chip family (HC165 vs
//! CD4021) and idle direction (PULL_DOWN vs PULL_UP).
//!
//! The latch / data / clock pin assignments live in the device's
//! `pins[30]` array as `PinFunction::ShiftRegLatch` / `ShiftRegData` /
//! `ShiftRegClk` (see [`super::pins`]); they're not part of
//! `shift_reg_config_t` and so are not surfaced by [`ShiftRegType`].

/// `shift_reg_config_type_t` from `vendored/common_types.h`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ShiftRegType {
    Hc165PullDown = 0,
    Cd4021PullDown = 1,
    Hc165PullUp = 2,
    Cd4021PullUp = 3,
}

impl ShiftRegType {
    #[must_use]
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Decode the wire byte. Returns `None` for values outside the
    /// 4-variant enum (the codec keeps the raw byte either way).
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Hc165PullDown),
            1 => Some(Self::Cd4021PullDown),
            2 => Some(Self::Hc165PullUp),
            3 => Some(Self::Cd4021PullUp),
            _ => None,
        }
    }

    /// Human label. Matches the Qt configurator's type-picker entries.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Hc165PullDown => "HC165 pull-down",
            Self::Cd4021PullDown => "CD4021 pull-down",
            Self::Hc165PullUp => "HC165 pull-up",
            Self::Cd4021PullUp => "CD4021 pull-up",
        }
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::Hc165PullDown,
            Self::Cd4021PullDown,
            Self::Hc165PullUp,
            Self::Cd4021PullUp,
        ]
        .into_iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_through_u8() {
        for v in 0u8..=3 {
            assert_eq!(ShiftRegType::from_u8(v).map(ShiftRegType::to_u8), Some(v));
        }
    }

    #[test]
    fn unknown_value_returns_none() {
        assert!(ShiftRegType::from_u8(4).is_none());
        assert!(ShiftRegType::from_u8(0xff).is_none());
    }

    #[test]
    fn all_yields_four_variants_in_wire_order() {
        let xs: Vec<_> = ShiftRegType::all().collect();
        assert_eq!(xs.len(), 4);
        for (i, t) in xs.iter().enumerate() {
            assert_eq!(t.to_u8() as usize, i);
        }
    }
}
