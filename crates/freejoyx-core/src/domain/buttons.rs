//! Button domain types.
//!
//! Mirrors `button_type_t` from `vendored/common_types.h` (the full enum,
//! including the gesture types appended in v1.7.x). The wire codec stores
//! `button_type` as a raw `u8` to keep round-trips bit-exact even for
//! values outside the enum range; this module supplies the typed view
//! for the UI plus the coexistence rule from Step 4 of the firmware
//! plan (`F103_GESTURE_PLAN.md`).
//!
//! Gesture-coexistence rule (Port.md §5 Slice 7, `F103_GESTURE_PLAN.md`):
//! a single physical input may host slots only from
//! `{NORMAL, TAP, DOUBLE_TAP}`. Mixing any of those with
//! TOGGLE / TOGGLE_SWITCH* / POV* / ENCODER_* / RADIO* / SEQUENTIAL* /
//! LOGIC is blocked by the configurator before the user can pick a
//! conflicting type.

use crate::domain::logic::BUTTON_TYPE_LOGIC;
use crate::wire::config::{Button, MAX_BUTTONS_NUM};

/// Wire values for `button_type_t`. Variants follow the enum order in
/// `vendored/common_types.h::button_t.type` exactly so `to_u8()` /
/// `from_u8()` are zero-cost casts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ButtonType {
    Normal = 0,
    Toggle = 1,
    ToggleSwitch = 2,
    ToggleSwitchOn = 3,
    ToggleSwitchOff = 4,

    Pov1Up = 5,
    Pov1Right = 6,
    Pov1Down = 7,
    Pov1Left = 8,
    Pov1Center = 9,

    Pov2Up = 10,
    Pov2Right = 11,
    Pov2Down = 12,
    Pov2Left = 13,
    Pov2Center = 14,

    Pov3Up = 15,
    Pov3Right = 16,
    Pov3Down = 17,
    Pov3Left = 18,

    Pov4Up = 19,
    Pov4Right = 20,
    Pov4Down = 21,
    Pov4Left = 22,

    EncoderInputA = 23,
    EncoderInputB = 24,

    Radio1 = 25,
    Radio2 = 26,
    Radio3 = 27,
    Radio4 = 28,

    SequentialToggle = 29,
    SequentialButton = 30,

    /// Appended after Sequential* so adding it didn't shift earlier values.
    Pov3Center = 31,
    Pov4Center = 32,

    Logic = 33,

    /// Renamed from `LONG_PRESS` in v1.7.x; integer value preserved.
    Tap = 34,
    DoubleTap = 35,
}

impl ButtonType {
    /// Map a raw wire byte. Returns `None` for unknown values; the wire
    /// layer keeps the raw byte so an unknown variant round-trips.
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        use ButtonType::{
            DoubleTap, EncoderInputA, EncoderInputB, Logic, Normal, Pov1Center, Pov1Down, Pov1Left,
            Pov1Right, Pov1Up, Pov2Center, Pov2Down, Pov2Left, Pov2Right, Pov2Up, Pov3Center,
            Pov3Down, Pov3Left, Pov3Right, Pov3Up, Pov4Center, Pov4Down, Pov4Left, Pov4Right,
            Pov4Up, Radio1, Radio2, Radio3, Radio4, SequentialButton, SequentialToggle, Tap,
            Toggle, ToggleSwitch, ToggleSwitchOff, ToggleSwitchOn,
        };
        Some(match v {
            0 => Normal,
            1 => Toggle,
            2 => ToggleSwitch,
            3 => ToggleSwitchOn,
            4 => ToggleSwitchOff,
            5 => Pov1Up,
            6 => Pov1Right,
            7 => Pov1Down,
            8 => Pov1Left,
            9 => Pov1Center,
            10 => Pov2Up,
            11 => Pov2Right,
            12 => Pov2Down,
            13 => Pov2Left,
            14 => Pov2Center,
            15 => Pov3Up,
            16 => Pov3Right,
            17 => Pov3Down,
            18 => Pov3Left,
            19 => Pov4Up,
            20 => Pov4Right,
            21 => Pov4Down,
            22 => Pov4Left,
            23 => EncoderInputA,
            24 => EncoderInputB,
            25 => Radio1,
            26 => Radio2,
            27 => Radio3,
            28 => Radio4,
            29 => SequentialToggle,
            30 => SequentialButton,
            31 => Pov3Center,
            32 => Pov4Center,
            33 => Logic,
            34 => Tap,
            35 => DoubleTap,
            _ => return None,
        })
    }

    #[must_use]
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// UI label. Matches the corresponding entries from the Qt
    /// configurator's button-type dropdown for parity at a glance.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Normal => "Normal",
            Self::Toggle => "Toggle",
            Self::ToggleSwitch => "Toggle Switch",
            Self::ToggleSwitchOn => "Toggle Switch On",
            Self::ToggleSwitchOff => "Toggle Switch Off",
            Self::Pov1Up => "POV 1 Up",
            Self::Pov1Right => "POV 1 Right",
            Self::Pov1Down => "POV 1 Down",
            Self::Pov1Left => "POV 1 Left",
            Self::Pov1Center => "POV 1 Center",
            Self::Pov2Up => "POV 2 Up",
            Self::Pov2Right => "POV 2 Right",
            Self::Pov2Down => "POV 2 Down",
            Self::Pov2Left => "POV 2 Left",
            Self::Pov2Center => "POV 2 Center",
            Self::Pov3Up => "POV 3 Up",
            Self::Pov3Right => "POV 3 Right",
            Self::Pov3Down => "POV 3 Down",
            Self::Pov3Left => "POV 3 Left",
            Self::Pov3Center => "POV 3 Center",
            Self::Pov4Up => "POV 4 Up",
            Self::Pov4Right => "POV 4 Right",
            Self::Pov4Down => "POV 4 Down",
            Self::Pov4Left => "POV 4 Left",
            Self::Pov4Center => "POV 4 Center",
            Self::EncoderInputA => "Encoder A",
            Self::EncoderInputB => "Encoder B",
            Self::Radio1 => "Radio 1",
            Self::Radio2 => "Radio 2",
            Self::Radio3 => "Radio 3",
            Self::Radio4 => "Radio 4",
            Self::SequentialToggle => "Sequential Toggle",
            Self::SequentialButton => "Sequential Button",
            Self::Logic => "Logic",
            Self::Tap => "Tap",
            Self::DoubleTap => "Double Tap",
        }
    }

    /// True if this type is one of the gesture-compatible types that
    /// may share a physical input with the others in
    /// `{NORMAL, TAP, DOUBLE_TAP}`. Used by the per-physical
    /// coexistence filter.
    #[must_use]
    pub fn is_gesture_compatible(self) -> bool {
        matches!(self, Self::Normal | Self::Tap | Self::DoubleTap)
    }

    /// Every variant in wire order. Drives the type dropdown.
    pub fn all() -> impl Iterator<Item = Self> {
        (0u8..=35).filter_map(Self::from_u8)
    }

    /// Category the type belongs to in the category-grouped picker.
    /// Ordering follows the picker's display order (Basic / Toggle
    /// switches / POV 1..4 / Encoder / Radio / Sequential) and groups
    /// POV3 Center back with POV3 even though its wire byte was
    /// appended late.
    #[must_use]
    pub fn category(self) -> ButtonTypeCategory {
        use ButtonTypeCategory::{
            Basic, Encoder, Pov1, Pov2, Pov3, Pov4, Radio, Sequential, ToggleSwitches,
        };
        match self {
            // Basic absorbs Tap / DoubleTap (formerly Gestures) and
            // Logic (formerly its own group) — all everyday cockpit
            // switch types, so surfacing them together cuts picker
            // traversal vs. the old three-header split.
            Self::Normal | Self::Tap | Self::DoubleTap | Self::Toggle | Self::Logic => Basic,
            Self::ToggleSwitch | Self::ToggleSwitchOn | Self::ToggleSwitchOff => ToggleSwitches,
            Self::Pov1Up | Self::Pov1Right | Self::Pov1Down | Self::Pov1Left | Self::Pov1Center => {
                Pov1
            }
            Self::Pov2Up | Self::Pov2Right | Self::Pov2Down | Self::Pov2Left | Self::Pov2Center => {
                Pov2
            }
            Self::Pov3Up
            | Self::Pov3Right
            | Self::Pov3Down
            | Self::Pov3Left
            | Self::Pov3Center => Pov3,
            Self::Pov4Up
            | Self::Pov4Right
            | Self::Pov4Down
            | Self::Pov4Left
            | Self::Pov4Center => Pov4,
            Self::EncoderInputA | Self::EncoderInputB => Encoder,
            Self::Radio1 | Self::Radio2 | Self::Radio3 | Self::Radio4 => Radio,
            Self::SequentialToggle | Self::SequentialButton => Sequential,
        }
    }
}

/// Visual grouping driving the category-grouped function picker.
/// Display order matches the iteration order of [`ButtonTypeCategory::all`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ButtonTypeCategory {
    Basic,
    ToggleSwitches,
    Pov1,
    Pov2,
    Pov3,
    Pov4,
    Encoder,
    Radio,
    Sequential,
}

impl ButtonTypeCategory {
    /// Header label shown above the category's entries in the picker.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Basic => "Basic",
            Self::ToggleSwitches => "Toggle switches",
            Self::Pov1 => "POV 1",
            Self::Pov2 => "POV 2",
            Self::Pov3 => "POV 3",
            Self::Pov4 => "POV 4",
            Self::Encoder => "Encoder",
            Self::Radio => "Radio",
            Self::Sequential => "Sequential",
        }
    }

    /// Iterate categories in display order.
    pub fn all() -> impl Iterator<Item = Self> {
        [
            Self::Basic,
            Self::ToggleSwitches,
            Self::Pov1,
            Self::Pov2,
            Self::Pov3,
            Self::Pov4,
            Self::Encoder,
            Self::Radio,
            Self::Sequential,
        ]
        .into_iter()
    }

    /// All button types in the category, in display order. POV centres
    /// that were appended late on the wire are surfaced adjacent to the
    /// rest of their POV group rather than at the wire-byte position.
    #[must_use]
    pub fn entries(self) -> &'static [ButtonType] {
        match self {
            Self::Basic => &[
                ButtonType::Normal,
                ButtonType::Tap,
                ButtonType::DoubleTap,
                ButtonType::Toggle,
                ButtonType::Logic,
            ],
            Self::ToggleSwitches => &[
                ButtonType::ToggleSwitch,
                ButtonType::ToggleSwitchOn,
                ButtonType::ToggleSwitchOff,
            ],
            Self::Pov1 => &[
                ButtonType::Pov1Up,
                ButtonType::Pov1Right,
                ButtonType::Pov1Down,
                ButtonType::Pov1Left,
                ButtonType::Pov1Center,
            ],
            Self::Pov2 => &[
                ButtonType::Pov2Up,
                ButtonType::Pov2Right,
                ButtonType::Pov2Down,
                ButtonType::Pov2Left,
                ButtonType::Pov2Center,
            ],
            Self::Pov3 => &[
                ButtonType::Pov3Up,
                ButtonType::Pov3Right,
                ButtonType::Pov3Down,
                ButtonType::Pov3Left,
                ButtonType::Pov3Center,
            ],
            Self::Pov4 => &[
                ButtonType::Pov4Up,
                ButtonType::Pov4Right,
                ButtonType::Pov4Down,
                ButtonType::Pov4Left,
                ButtonType::Pov4Center,
            ],
            Self::Encoder => &[ButtonType::EncoderInputA, ButtonType::EncoderInputB],
            Self::Radio => &[
                ButtonType::Radio1,
                ButtonType::Radio2,
                ButtonType::Radio3,
                ButtonType::Radio4,
            ],
            Self::Sequential => &[
                ButtonType::SequentialToggle,
                ButtonType::SequentialButton,
            ],
        }
    }
}

/// Outcome of `physical_assignment_blocked`. Either the candidate type
/// is OK, or it's blocked because another slot already claimed the
/// same physical with an incompatible type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoexistenceCheck {
    Ok,
    /// `other_slot` already uses `other_type` on the same physical.
    /// The two types can't coexist.
    Blocked {
        other_slot: usize,
        other_type: u8,
    },
}

/// Apply the F103_GESTURE_PLAN.md coexistence rule: a single physical
/// input may host slots only from `{NORMAL, TAP, DOUBLE_TAP}`.
///
/// `slot` is the slot the user is about to change; `physical_num` is
/// the physical input that slot points at (or will point at);
/// `candidate` is the [`ButtonType`] the user is trying to assign.
///
/// Returns [`CoexistenceCheck::Ok`] if every other slot on the same
/// physical is gesture-compatible and `candidate` is gesture-compatible
/// too — or if no other slot uses that physical. Returns
/// [`CoexistenceCheck::Blocked`] with the first offending slot
/// otherwise.
///
/// **LOGIC slots are exempt** — their `physical_num` is the Source A
/// *button index*, not a physical GPIO, so the value never collides
/// with a real physical pin. LOGIC and NORMAL (or any other type)
/// always coexist freely.
///
/// Physical -1 (`unassigned`) is always allowed: an unassigned slot
/// doesn't collide with anything.
#[must_use]
pub fn physical_assignment_blocked(
    buttons: &[Button; MAX_BUTTONS_NUM],
    slot: usize,
    physical_num: i8,
    candidate: ButtonType,
) -> CoexistenceCheck {
    if physical_num < 0 || candidate == ButtonType::Logic {
        return CoexistenceCheck::Ok;
    }
    for (i, other) in buttons.iter().enumerate() {
        if i == slot || other.physical_num != physical_num {
            continue;
        }
        // Other slots that are LOGIC use their `physical_num` field as
        // a Source A button index, not as a GPIO — skip them.
        if other.button_type == BUTTON_TYPE_LOGIC {
            continue;
        }
        let other_typed = ButtonType::from_u8(other.button_type);
        let other_compat = other_typed.is_some_and(ButtonType::is_gesture_compatible);
        let candidate_compat = candidate.is_gesture_compatible();
        if !(other_compat && candidate_compat) {
            return CoexistenceCheck::Blocked {
                other_slot: i,
                other_type: other.button_type,
            };
        }
    }
    CoexistenceCheck::Ok
}

// =============================================================================
// Button bitfield setters (paired with the getters on Button itself).
// =============================================================================

impl Button {
    /// Set the 4-bit `shift_modificator` (0 = none, 1..=8 = shift slot).
    pub fn set_shift_modificator(&mut self, v: u8) {
        set_bits(&mut self.flags_a, 0, 0x0f, v);
    }
    pub fn set_is_inverted(&mut self, v: bool) {
        set_bit(&mut self.flags_a, 0x10, v);
    }
    pub fn set_is_disabled(&mut self, v: bool) {
        set_bit(&mut self.flags_a, 0x20, v);
    }
    /// Set the 3-bit `op` field (only meaningful when `button_type == LOGIC`).
    pub fn set_op(&mut self, v: u8) {
        set_bits(&mut self.flags_b, 0, 0x07, v);
    }
    /// Set the 3-bit `delay_timer` field (also serves as the
    /// LOGIC debounce-timer picker).
    pub fn set_delay_timer(&mut self, v: u8) {
        set_bits(&mut self.flags_c, 0, 0x07, v);
    }
    /// Set the 3-bit `press_timer` field.
    pub fn set_press_timer(&mut self, v: u8) {
        set_bits(&mut self.flags_c, 3, 0x07, v);
    }
}

fn set_bit(byte: &mut u8, mask: u8, v: bool) {
    if v {
        *byte |= mask;
    } else {
        *byte &= !mask;
    }
}

fn set_bits(byte: &mut u8, shift: u32, mask: u8, v: u8) {
    let cleared = *byte & !(mask << shift);
    *byte = cleared | ((v & mask) << shift);
}

/// `BUTTON_TYPE_LOGIC` re-exported here so callers picking a button
/// type don't need to reach across to [`crate::domain::logic`].
pub const LOGIC: u8 = BUTTON_TYPE_LOGIC;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::config::DEV_CONFIG_SIZE;

    #[test]
    fn every_button_type_has_a_category() {
        // Closed-enum match — a new variant added without a category arm
        // fails to compile. Belt-and-braces this also asserts each variant
        // shows up exactly once across `ButtonTypeCategory::entries()`.
        let mut seen = std::collections::HashSet::new();
        for cat in ButtonTypeCategory::all() {
            for &bt in cat.entries() {
                assert!(seen.insert(bt), "{bt:?} appears in two categories");
                assert_eq!(bt.category(), cat);
            }
        }
        // Every wire-known type made it into a category's entries list.
        for bt in ButtonType::all() {
            assert!(seen.contains(&bt), "{bt:?} missing from category entries");
        }
    }

    #[test]
    fn all_yields_every_known_variant_in_order() {
        let xs: Vec<_> = ButtonType::all().collect();
        assert_eq!(xs.len(), 36);
        for (i, t) in xs.iter().enumerate() {
            assert_eq!(t.to_u8() as usize, i);
        }
    }

    #[test]
    fn from_u8_round_trips_to_u8() {
        for v in 0u8..=35 {
            assert_eq!(ButtonType::from_u8(v).map(ButtonType::to_u8), Some(v));
        }
    }

    #[test]
    fn from_u8_returns_none_for_unknown() {
        assert!(ButtonType::from_u8(36).is_none());
        assert!(ButtonType::from_u8(0xff).is_none());
    }

    #[test]
    fn gesture_compatible_matches_spec() {
        assert!(ButtonType::Normal.is_gesture_compatible());
        assert!(ButtonType::Tap.is_gesture_compatible());
        assert!(ButtonType::DoubleTap.is_gesture_compatible());
        for t in [
            ButtonType::Toggle,
            ButtonType::ToggleSwitch,
            ButtonType::Pov1Up,
            ButtonType::EncoderInputA,
            ButtonType::Radio1,
            ButtonType::SequentialToggle,
            ButtonType::Logic,
        ] {
            assert!(!t.is_gesture_compatible(), "{t:?} should not be compatible");
        }
    }

    fn empty_buttons() -> [Button; MAX_BUTTONS_NUM] {
        use crate::wire::config::DeviceConfig;
        DeviceConfig::decode(&[0u8; DEV_CONFIG_SIZE])
            .unwrap()
            .buttons
    }

    #[test]
    fn coexistence_ok_for_unassigned_physical() {
        let buttons = empty_buttons();
        assert_eq!(
            physical_assignment_blocked(&buttons, 0, -1, ButtonType::Logic),
            CoexistenceCheck::Ok
        );
    }

    #[test]
    fn coexistence_ok_for_two_gesture_slots() {
        let mut buttons = empty_buttons();
        buttons[0].physical_num = 5;
        buttons[0].button_type = ButtonType::Tap.to_u8();
        assert_eq!(
            physical_assignment_blocked(&buttons, 1, 5, ButtonType::DoubleTap),
            CoexistenceCheck::Ok
        );
    }

    #[test]
    fn coexistence_allows_logic_against_existing_normal() {
        // LOGIC's `physical_num` is a Source A button index, not a GPIO,
        // so the value `7` here means "Source A = button slot 7" — it
        // doesn't conflict with another slot wired to physical pin 7.
        let mut buttons = empty_buttons();
        buttons[0].physical_num = 7;
        buttons[0].button_type = ButtonType::Normal.to_u8();
        assert_eq!(
            physical_assignment_blocked(&buttons, 2, 7, ButtonType::Logic),
            CoexistenceCheck::Ok,
        );
    }

    #[test]
    fn coexistence_allows_normal_against_existing_logic() {
        // Mirror of the above. The existing LOGIC slot's
        // `physical_num` is a button-index field, not a real pin
        // assignment, so a NORMAL slot can claim physical 4 freely.
        let mut buttons = empty_buttons();
        buttons[3].physical_num = 4;
        buttons[3].button_type = ButtonType::Logic.to_u8();
        assert_eq!(
            physical_assignment_blocked(&buttons, 9, 4, ButtonType::Normal),
            CoexistenceCheck::Ok,
        );
    }

    #[test]
    fn coexistence_ignores_current_slot() {
        let mut buttons = empty_buttons();
        buttons[5].physical_num = 8;
        buttons[5].button_type = ButtonType::Tap.to_u8();
        // Editing slot 5 itself shouldn't see itself as a conflict.
        assert_eq!(
            physical_assignment_blocked(&buttons, 5, 8, ButtonType::DoubleTap),
            CoexistenceCheck::Ok
        );
    }

    #[test]
    fn button_setters_round_trip() {
        let mut b = Button {
            physical_num: 0,
            button_type: 0,
            src_b: 0,
            flags_a: 0,
            flags_b: 0,
            flags_c: 0,
        };
        b.set_shift_modificator(5);
        b.set_is_inverted(true);
        b.set_is_disabled(true);
        b.set_op(3);
        b.set_delay_timer(2);
        b.set_press_timer(4);

        assert_eq!(b.shift_modificator(), 5);
        assert!(b.is_inverted());
        assert!(b.is_disabled());
        assert_eq!(b.op(), 3);
        assert_eq!(b.delay_timer(), 2);
        assert_eq!(b.press_timer(), 4);
    }

    #[test]
    fn button_setters_truncate_oversize() {
        let mut b = Button {
            physical_num: 0,
            button_type: 0,
            src_b: 0,
            flags_a: 0,
            flags_b: 0,
            flags_c: 0,
        };
        b.set_shift_modificator(0xff);
        assert_eq!(b.shift_modificator(), 0x0f);
        b.set_op(0xff);
        assert_eq!(b.op(), 0x07);
        b.set_delay_timer(0xff);
        b.set_press_timer(0xff);
        assert_eq!(b.delay_timer(), 0x07);
        assert_eq!(b.press_timer(), 0x07);
    }
}
