//! Axis domain types.
//!
//! `AxisFilter` mirrors the 3-bit `filter` field at `AxisConfig::filter()` —
//! the firmware reads it as `FILTER_NO..FILTER_LEVEL_7` (see
//! `vendored/common_defines.h::filter_t`). The Qt configurator surfaces
//! it as a labelled slider; we expose the same labels for the Slint UI.
//!
//! `AxisSource` mirrors the Qt configurator's axis main-source picker
//! (`axes.h::Axes::{Encoder, I2C, None, A0..C15}`). On the wire,
//! `axis_config.source_main` is an `i8`: `-1`/`-2`/`-3` are sentinels for
//! None / I2C / Encoder respectively; `0..29` is a pin slot index into
//! `dev_config_t.pins[]`. For Encoder sources, `axis_config.channel`
//! selects the fast-encoder slot (0 = Enc 1 on PA8+PA9,
//! 1 = Enc 2 on PB6+PB7).

use crate::domain::pins::PinFunction;

/// Wire sentinel values for `axis_config_t.source_main`.
pub const AXIS_SOURCE_NONE: i8 = -1;
pub const AXIS_SOURCE_I2C: i8 = -2;
pub const AXIS_SOURCE_ENCODER: i8 = -3;

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

/// `function` 2-bit field from `axis_config.flags1`. Variants match
/// the C enum at `vendored/common_types.h::NO_FUNCTION..FUNCTION_EQUAL`.
/// When `function() != None`, the firmware combines this axis's raw
/// value with the axis indexed by `source_secondary()` using the
/// chosen operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisFunction {
    None,
    Plus,
    Minus,
    Equal,
}

impl AxisFunction {
    #[must_use]
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v & 0x03 {
            1 => Self::Plus,
            2 => Self::Minus,
            3 => Self::Equal,
            _ => Self::None,
        }
    }

    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::Plus => "Plus",
            Self::Minus => "Minus",
            Self::Equal => "Equal",
        }
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [Self::None, Self::Plus, Self::Minus, Self::Equal].into_iter()
    }
}

/// I2C device addresses surfaced by the Qt configurator's
/// `m_i2cPtrList` (axesextended.h). Only meaningful when the axis's
/// source is `AxisSource::I2C`; for all other sources the wire byte is
/// preserved untouched but the UI dropdown is greyed.
///
/// Wire is a raw u8 — these enum values *are* the on-wire values, not
/// indices into the variant list. Unknown bytes fold to `As5600` on
/// read (Qt's converter does the same via `EnumToIndex` returning -1
/// then defaulting to 0).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum I2cAddress {
    As5600 = 0x36,
    Ads1115_00 = 0x48,
    Ads1115_01 = 0x49,
    Ads1115_10 = 0x4A,
    Ads1115_11 = 0x4B,
}

impl I2cAddress {
    #[must_use]
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            0x36 => Self::As5600,
            0x48 => Self::Ads1115_00,
            0x49 => Self::Ads1115_01,
            0x4A => Self::Ads1115_10,
            0x4B => Self::Ads1115_11,
            // Garbage / uninitialised bytes (e.g. fresh-flashed devices)
            // fold to the first variant so the dropdown lands on
            // something the user can recognise.
            _ => Self::As5600,
        }
    }

    /// Display label matching the Qt configurator's `m_i2cPtrList`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::As5600 => "AS 5600",
            Self::Ads1115_00 => "ADS 1115_00",
            Self::Ads1115_01 => "ADS 1115_01",
            Self::Ads1115_10 => "ADS 1115_10",
            Self::Ads1115_11 => "ADS 1115_11",
        }
    }

    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::As5600,
            Self::Ads1115_00,
            Self::Ads1115_01,
            Self::Ads1115_10,
            Self::Ads1115_11,
        ]
        .into_iter()
    }
}

/// `AXIS_BUTTON_*` enum from `vendored/common_types.h`. Action a
/// momentary physical button can apply to its host axis when pressed.
/// Variants and ordering mirror the C enum literally.
///
/// Slot 2 (`button2_type`) is encoded as **2 bits**, so it can only hold
/// `FuncEn`, `PrescalerEn`, `Center`, `Reset` — not `Down` or `Up`.
/// Use [`Self::valid_for_slot2`] to filter the dropdown for that slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisButtonAction {
    FuncEn = 0,
    PrescalerEn = 1,
    Center = 2,
    Reset = 3,
    Down = 4,
    Up = 5,
}

impl AxisButtonAction {
    #[must_use]
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::PrescalerEn,
            2 => Self::Center,
            3 => Self::Reset,
            4 => Self::Down,
            5 => Self::Up,
            _ => Self::FuncEn,
        }
    }

    /// Display label matching the Qt configurator's `m_button_1_3_list`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::FuncEn => "Function enable",
            Self::PrescalerEn => "Prescale enable",
            Self::Center => "Center",
            Self::Reset => "Reset",
            Self::Down => "Down",
            Self::Up => "Up",
        }
    }

    /// All six action variants in wire order. Drives the dropdown
    /// for button slots 1 and 3 (3-bit fields).
    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::FuncEn,
            Self::PrescalerEn,
            Self::Center,
            Self::Reset,
            Self::Down,
            Self::Up,
        ]
        .into_iter()
    }

    /// True when this variant fits the 2-bit `button2_type` field. The
    /// Qt configurator filters its slot-2 dropdown to the first four
    /// entries; mirror that here.
    #[must_use]
    pub fn valid_for_slot2(self) -> bool {
        matches!(
            self,
            Self::FuncEn | Self::PrescalerEn | Self::Center | Self::Reset
        )
    }
}

/// Typed view of `(axis_config.source_main, axis_config.channel)`.
///
/// `Pin(slot)` carries the index into `dev_config_t.pins[]` (0..29).
/// `Encoder(slot)` carries the fast-encoder slot (0..1) — written
/// to `axis_config.channel` on encode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AxisSource {
    None,
    I2C,
    Encoder(u8),
    Pin(u8),
}

impl AxisSource {
    /// Resolve from the raw `(source_main, channel)` pair.
    ///
    /// Unknown sentinels fold to `None` so corrupt/uninitialised wire
    /// bytes don't crash the UI.
    #[must_use]
    pub fn from_wire(source_main: i8, channel: u8) -> Self {
        match source_main {
            AXIS_SOURCE_NONE => Self::None,
            AXIS_SOURCE_I2C => Self::I2C,
            AXIS_SOURCE_ENCODER => Self::Encoder(channel & 0x0f),
            n if n >= 0 => Self::Pin(n as u8),
            _ => Self::None,
        }
    }

    /// Encode back to `(source_main, channel)`. Caller writes channel
    /// only when the source actually carries one (we still return a
    /// value for the non-channel variants so the caller can zero it).
    #[must_use]
    pub fn to_wire(self) -> (i8, u8) {
        match self {
            Self::None => (AXIS_SOURCE_NONE, 0),
            Self::I2C => (AXIS_SOURCE_I2C, 0),
            Self::Encoder(slot) => (AXIS_SOURCE_ENCODER, slot & 0x0f),
            Self::Pin(idx) => (idx as i8, 0),
        }
    }

    /// Stable 32-bit handle the UI passes through Slint properties
    /// without giving up the variant + payload. High bits encode the
    /// variant tag; low bits the slot. `-1` is reserved for None so
    /// the existing `Option<i32>` patterns stay drop-in.
    #[must_use]
    pub fn to_handle(self) -> i32 {
        match self {
            Self::None => -1,
            Self::I2C => -2,
            Self::Encoder(slot) => -100 - i32::from(slot),
            Self::Pin(idx) => i32::from(idx),
        }
    }

    /// Inverse of `to_handle`.
    #[must_use]
    pub fn from_handle(h: i32) -> Self {
        if h == -1 {
            Self::None
        } else if h == -2 {
            Self::I2C
        } else if h <= -100 {
            let slot = ((-100 - h) & 0x0f) as u8;
            Self::Encoder(slot)
        } else if (0..30).contains(&h) {
            Self::Pin(h as u8)
        } else {
            Self::None
        }
    }
}

/// Walk `pins[]` and return the indices currently set to
/// `PinFunction::AxisAnalog`. Drives the per-axis source dropdown.
#[must_use]
pub fn analog_pin_slots(pins: &[i8]) -> Vec<u8> {
    pins.iter()
        .enumerate()
        .filter_map(|(i, raw)| {
            (PinFunction::from_i8(*raw)? == PinFunction::AxisAnalog).then_some(i as u8)
        })
        .collect()
}

/// Return the fast-encoder slot indices whose A and B pins are BOTH
/// currently assigned to `PinFunction::FastEncoder`. Mirrors the Qt
/// configurator's `AxesConfig::completedEncoderSlots()` — a
/// half-assigned encoder isn't usable as an axis source.
///
/// Slot 0 = Enc 1 on PA8 (slot 8) + PA9 (slot 9).
/// Slot 1 = Enc 2 on PB6 (slot 17) + PB7 (slot 18).
#[must_use]
pub fn completed_fast_encoder_slots(pins: &[i8]) -> Vec<u8> {
    let is_enc = |slot: usize| {
        pins.get(slot)
            .copied()
            .and_then(PinFunction::from_i8)
            .map(|f| f == PinFunction::FastEncoder)
            .unwrap_or(false)
    };
    let mut out = Vec::with_capacity(2);
    if is_enc(8) && is_enc(9) {
        out.push(0);
    }
    if is_enc(17) && is_enc(18) {
        out.push(1);
    }
    out
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

    #[test]
    fn axis_source_round_trips_wire() {
        let cases = [
            (AxisSource::None, (-1i8, 0u8)),
            (AxisSource::I2C, (-2, 0)),
            (AxisSource::Encoder(0), (-3, 0)),
            (AxisSource::Encoder(1), (-3, 1)),
            (AxisSource::Pin(0), (0, 0)),
            (AxisSource::Pin(29), (29, 0)),
        ];
        for (src, wire) in cases {
            assert_eq!(src.to_wire(), wire);
            assert_eq!(AxisSource::from_wire(wire.0, wire.1), src);
        }
    }

    #[test]
    fn axis_source_round_trips_handle() {
        let xs = [
            AxisSource::None,
            AxisSource::I2C,
            AxisSource::Encoder(0),
            AxisSource::Encoder(1),
            AxisSource::Pin(0),
            AxisSource::Pin(7),
            AxisSource::Pin(29),
        ];
        for s in xs {
            assert_eq!(AxisSource::from_handle(s.to_handle()), s);
        }
    }

    #[test]
    fn analog_pin_slots_finds_axis_analog_only() {
        let mut pins = [0i8; 30];
        pins[3] = PinFunction::AxisAnalog.to_i8();
        pins[7] = PinFunction::AxisAnalog.to_i8();
        pins[8] = PinFunction::FastEncoder.to_i8();
        assert_eq!(analog_pin_slots(&pins), vec![3, 7]);
    }

    #[test]
    fn i2c_address_round_trips() {
        for a in I2cAddress::all() {
            assert_eq!(I2cAddress::from_u8(a.to_u8()), a);
        }
        // Wire values match the C enum literally.
        assert_eq!(I2cAddress::As5600.to_u8(), 0x36);
        assert_eq!(I2cAddress::Ads1115_11.to_u8(), 0x4B);
        // Unknown bytes fold to AS5600.
        assert_eq!(I2cAddress::from_u8(0x00), I2cAddress::As5600);
        assert_eq!(I2cAddress::from_u8(0xff), I2cAddress::As5600);
    }

    #[test]
    fn axis_button_action_round_trips() {
        for a in AxisButtonAction::all() {
            assert_eq!(AxisButtonAction::from_u8(a.to_u8()), a);
        }
        // Out-of-range and zero fold to FuncEn (the C enum default).
        assert_eq!(AxisButtonAction::from_u8(0), AxisButtonAction::FuncEn);
        assert_eq!(AxisButtonAction::from_u8(99), AxisButtonAction::FuncEn);
    }

    #[test]
    fn axis_button_action_slot2_filter_matches_qt() {
        let slot2: Vec<_> = AxisButtonAction::all()
            .filter(|a| a.valid_for_slot2())
            .collect();
        assert_eq!(
            slot2,
            vec![
                AxisButtonAction::FuncEn,
                AxisButtonAction::PrescalerEn,
                AxisButtonAction::Center,
                AxisButtonAction::Reset,
            ]
        );
    }

    #[test]
    fn axis_function_round_trips() {
        for v in [
            AxisFunction::None,
            AxisFunction::Plus,
            AxisFunction::Minus,
            AxisFunction::Equal,
        ] {
            assert_eq!(AxisFunction::from_u8(v.to_u8()), v);
        }
        assert_eq!(AxisFunction::from_u8(0xff), AxisFunction::Equal);
        assert_eq!(AxisFunction::from_u8(0xfc), AxisFunction::None);
    }

    #[test]
    fn completed_encoder_slots_needs_both_pins() {
        let mut pins = [0i8; 30];
        // Only PA8 set: slot 0 not yet complete.
        pins[8] = PinFunction::FastEncoder.to_i8();
        assert!(completed_fast_encoder_slots(&pins).is_empty());
        // Now both PA8 + PA9: slot 0 complete.
        pins[9] = PinFunction::FastEncoder.to_i8();
        assert_eq!(completed_fast_encoder_slots(&pins), vec![0]);
        // Add PB6 + PB7: slot 1 also complete.
        pins[17] = PinFunction::FastEncoder.to_i8();
        pins[18] = PinFunction::FastEncoder.to_i8();
        assert_eq!(completed_fast_encoder_slots(&pins), vec![0, 1]);
    }
}
