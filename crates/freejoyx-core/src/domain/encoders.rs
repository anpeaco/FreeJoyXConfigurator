//! Encoder domain types.
//!
//! `EncoderMode` mirrors the `encoder_t` enum from
//! `vendored/common_types.h` (1x / 2x / 4x decoding). Both the
//! `encoders[16]` soft-encoder slots and `fast_encoders[2].mode`
//! carry this value.

use crate::domain::ButtonType;
use crate::wire::config::{Button, MAX_BUTTONS_NUM, MAX_ENCODERS_NUM, MAX_FAST_ENCODER_NUM};

/// Resolved soft-encoder A/B pairing for one `encoders_state[]` slot.
/// `a_button` and `b_button` are indices into `dev_config_t.buttons[]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SoftEncoderPair {
    pub a_button: u8,
    pub b_button: u8,
}

/// Derive the firmware's soft-encoder pairing from the buttons table.
///
/// Mirrors `EncodersInit` in `FreeJoyX/application/Src/encoders.c`: the
/// firmware scans `buttons[]` in index order, matching each
/// `ENCODER_INPUT_A` with the next `ENCODER_INPUT_B` that appears at a
/// higher slot index than the previous pair's B. Each successful pair
/// fills the next `encoders_state[]` slot starting at
/// `MAX_FAST_ENCODER_NUM` (slots 0 and 1 are reserved for the fast /
/// hardware-quadrature encoders regardless of whether they're enabled).
///
/// Returns one entry per `encoders_state[]` slot. Indices 0 and 1 are
/// always `None` here — fast encoders aren't formed from `buttons[]`.
/// Indices 2..[`MAX_ENCODERS_NUM`] hold the soft pairs in scan order.
#[must_use]
pub fn pair_soft_encoders(
    buttons: &[Button; MAX_BUTTONS_NUM],
) -> [Option<SoftEncoderPair>; MAX_ENCODERS_NUM] {
    let mut out: [Option<SoftEncoderPair>; MAX_ENCODERS_NUM] = [None; MAX_ENCODERS_NUM];
    let mut pos = MAX_FAST_ENCODER_NUM;
    let mut prev_a: i32 = -1;
    let mut prev_b: i32 = -1;
    let a_raw = ButtonType::EncoderInputA.to_u8();
    let b_raw = ButtonType::EncoderInputB.to_u8();
    for (i, btn_a) in buttons.iter().enumerate() {
        if pos >= MAX_ENCODERS_NUM {
            break;
        }
        if btn_a.button_type != a_raw || (i as i32) <= prev_a {
            continue;
        }
        for (j, btn_b) in buttons.iter().enumerate() {
            if btn_b.button_type == b_raw && (j as i32) > prev_b {
                #[allow(clippy::cast_possible_truncation)]
                let pair = SoftEncoderPair {
                    a_button: i as u8,
                    b_button: j as u8,
                };
                out[pos] = Some(pair);
                prev_a = i as i32;
                prev_b = j as i32;
                pos += 1;
                break;
            }
        }
    }
    out
}

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

    fn empty_buttons() -> [Button; MAX_BUTTONS_NUM] {
        use crate::wire::config::{DeviceConfig, DEV_CONFIG_SIZE};
        DeviceConfig::decode(&[0u8; DEV_CONFIG_SIZE])
            .unwrap()
            .buttons
    }

    fn set_type(b: &mut Button, t: ButtonType) {
        b.button_type = t.to_u8();
    }

    #[test]
    fn fast_slots_are_never_paired_from_buttons() {
        let buttons = empty_buttons();
        let pairs = pair_soft_encoders(&buttons);
        assert!(pairs[0].is_none());
        assert!(pairs[1].is_none());
    }

    #[test]
    fn pairs_fill_soft_slots_in_scan_order() {
        let mut buttons = empty_buttons();
        set_type(&mut buttons[12], ButtonType::EncoderInputA);
        set_type(&mut buttons[30], ButtonType::EncoderInputB);
        set_type(&mut buttons[40], ButtonType::EncoderInputA);
        set_type(&mut buttons[55], ButtonType::EncoderInputB);
        let pairs = pair_soft_encoders(&buttons);
        assert_eq!(
            pairs[2],
            Some(SoftEncoderPair {
                a_button: 12,
                b_button: 30
            })
        );
        assert_eq!(
            pairs[3],
            Some(SoftEncoderPair {
                a_button: 40,
                b_button: 55
            })
        );
        assert!(pairs[4].is_none());
    }

    #[test]
    fn unpaired_a_without_following_b_is_dropped() {
        let mut buttons = empty_buttons();
        set_type(&mut buttons[10], ButtonType::EncoderInputA);
        set_type(&mut buttons[20], ButtonType::EncoderInputB);
        set_type(&mut buttons[30], ButtonType::EncoderInputA); // no later B
        let pairs = pair_soft_encoders(&buttons);
        assert_eq!(
            pairs[2],
            Some(SoftEncoderPair {
                a_button: 10,
                b_button: 20
            })
        );
        assert!(pairs[3].is_none());
    }

    #[test]
    fn b_must_advance_past_previous_b() {
        // First pair: A@50 takes the earliest available B (10), since
        // prev_b starts at -1. Second pair: A@60's inner scan rejects
        // j=10 (not > prev_b=10) and lands on j=20 — i.e. the *next*
        // available B, not necessarily one after the A.
        let mut buttons = empty_buttons();
        set_type(&mut buttons[10], ButtonType::EncoderInputB);
        set_type(&mut buttons[20], ButtonType::EncoderInputB);
        set_type(&mut buttons[50], ButtonType::EncoderInputA);
        set_type(&mut buttons[60], ButtonType::EncoderInputA);
        let pairs = pair_soft_encoders(&buttons);
        assert_eq!(
            pairs[2],
            Some(SoftEncoderPair {
                a_button: 50,
                b_button: 10
            })
        );
        assert_eq!(
            pairs[3],
            Some(SoftEncoderPair {
                a_button: 60,
                b_button: 20
            })
        );
    }
}
