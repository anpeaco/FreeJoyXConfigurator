//! Pre-write configuration validator.
//!
//! Single point the UI calls before pushing a `dev_config_t` to the
//! device. Aggregates the existing per-area validators ([`validate_pins`],
//! [`validate_logic_buttons`]) and adds the missing whole-config
//! coexistence pass for button types (today only enforced per-edit by
//! [`physical_assignment_blocked`]).
//!
//! Design choice: the validator does **not** replace the existing
//! per-area validators or the dropdown-time guards. Those still drive
//! inline error chips and disabled menu entries — they're the
//! point-of-edit signal. `validate_for_write` is the *gate* the Write
//! Device button trusts so a half-completed LOGIC slot or a sneaky
//! cross-slot conflict can't reach the firmware.
//!
//! Each [`ConfigError`] knows its own human-readable summary and which
//! tab the user needs to visit to fix it, so the UI layer doesn't need
//! to dispatch on error variants — it just renders the strings.
//!
//! ## What this validator catches today
//!
//! - Pin conflicts (duplicate singleton functions, unknown function codes)
//! - Incomplete LOGIC button slots (Source A unset, Source B unset for a
//!   binary op, op-code out of range)
//! - Button-type coexistence violations across the whole 128-slot table
//!   (the per-physical `{NORMAL, LONG_PRESS, DOUBLE_TAP}`-only rule from
//!   F103_GESTURE_PLAN.md)
//!
//! ## What it does **not** catch yet
//!
//! Board-specific timer / silicon conflicts (PA8 PWM ↔ PA10 RGB, PB6/7
//! FAST_ENCODER ↔ PB6 TLE5011_GEN on F103) belong to later slices that
//! also expose those toggles in the UI. They drop into this validator
//! when their domain rules land.

use crate::domain::buttons::ButtonType;
use crate::domain::logic::{validate_logic_buttons, LogicError, LogicOp, BUTTON_TYPE_LOGIC};
use crate::domain::pins::{validate_pins, PinConflict, PinConflictKind};
use crate::wire::config::{DeviceConfig, MAX_BUTTONS_NUM};

/// One problem found in a `DeviceConfig`. Carries enough structure that
/// the UI can choose to highlight a specific row, but its
/// [`Self::human_summary`] + [`Self::tab_hint`] cover the common case
/// of "render a list of strings in a toast".
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigError {
    /// A pin slot reports a duplicate-singleton or unknown-function
    /// conflict — same as [`validate_pins`] surfaces inline.
    Pin(PinConflict),
    /// A LOGIC button slot is incomplete — same as
    /// [`validate_logic_buttons`] surfaces inline.
    Logic(LogicError),
    /// Two button slots share the same `physical_num` with types that
    /// can't coexist per the F103_GESTURE_PLAN.md rule. Stored once per
    /// offending pair, with `slot_a < slot_b` so the UI sees stable
    /// ordering and no duplicates.
    ButtonCoexistence {
        slot_a: usize,
        type_a: u8,
        slot_b: usize,
        type_b: u8,
        physical_num: i8,
    },
}

impl ConfigError {
    /// Short, user-facing string suitable for a toast or list row.
    /// 1-based slot numbers everywhere — matches what the user sees in
    /// the UI (slot #1, not slot 0).
    #[must_use]
    pub fn human_summary(&self) -> String {
        match self {
            Self::Pin(c) => match &c.kind {
                PinConflictKind::DuplicateSingleton(func) => {
                    format!("Pin slot {}: duplicate {func}", c.slot + 1)
                }
                PinConflictKind::UnknownFunction(raw) => {
                    format!("Pin slot {}: unknown function code 0x{raw:02x}", c.slot + 1)
                }
            },
            Self::Logic(e) => match e {
                LogicError::SourceAUnset { button_index } => {
                    format!("Button #{}: LOGIC slot needs Source A", button_index + 1)
                }
                LogicError::SourceBUnsetForBinaryOp { button_index, op } => {
                    format!(
                        "Button #{}: LOGIC op {} needs Source B",
                        button_index + 1,
                        logic_op_label(*op),
                    )
                }
                LogicError::OpOutOfRange {
                    button_index,
                    op_raw,
                } => format!(
                    "Button #{}: LOGIC op code 0x{op_raw:02x} is out of range",
                    button_index + 1
                ),
            },
            Self::ButtonCoexistence {
                slot_a,
                type_a,
                slot_b,
                type_b,
                physical_num,
            } => format!(
                "Buttons #{} ({}) and #{} ({}) both use physical {} but can't coexist",
                slot_a + 1,
                button_type_label(*type_a),
                slot_b + 1,
                button_type_label(*type_b),
                physical_num,
            ),
        }
    }

    /// Which tab the user needs to visit to fix this. Used by the UI to
    /// hint at where to look, not to switch tabs automatically.
    #[must_use]
    pub fn tab_hint(&self) -> &'static str {
        match self {
            Self::Pin(_) => "Pins",
            Self::Logic(_) | Self::ButtonCoexistence { .. } => "Buttons",
        }
    }
}

/// Aggregate every domain rule into a single yes/no for the Write
/// button. An empty `Vec` means the config is shippable; a non-empty
/// `Vec` means block the write and surface every problem.
///
/// Order: pin conflicts first, then logic-button completeness, then
/// cross-slot coexistence. This is the order the UI reads naturally
/// (left tabs first) and keeps the list stable across runs.
#[must_use]
pub fn validate_for_write(config: &DeviceConfig) -> Vec<ConfigError> {
    let mut out = Vec::new();
    out.extend(
        validate_pins(&config.pins)
            .into_iter()
            .map(ConfigError::Pin),
    );
    out.extend(
        validate_logic_buttons(config)
            .into_iter()
            .map(ConfigError::Logic),
    );
    out.extend(validate_button_coexistence(config));
    out
}

/// Walk every pair of button slots that share a `physical_num` and
/// flag pairs whose types violate the gesture-coexistence rule.
///
/// Produces one error per offending **pair**, not per slot, with
/// `slot_a < slot_b`. Pairs whose physical is `-1` (unassigned) are
/// always allowed — an unassigned slot doesn't collide with anything.
///
/// **LOGIC slots are exempt** from this check: their `physical_num`
/// field stores a Source A button index, not a GPIO, so it can't
/// collide with a real physical pin. Either side of a pair being LOGIC
/// skips the comparison.
fn validate_button_coexistence(config: &DeviceConfig) -> Vec<ConfigError> {
    let mut out = Vec::new();
    let buttons = &config.buttons;
    for a in 0..MAX_BUTTONS_NUM {
        if buttons[a].button_type == BUTTON_TYPE_LOGIC {
            continue;
        }
        let phy = buttons[a].physical_num;
        if phy < 0 {
            continue;
        }
        let type_a = ButtonType::from_u8(buttons[a].button_type);
        let a_compat = type_a.is_some_and(ButtonType::is_gesture_compatible);
        for b in (a + 1)..MAX_BUTTONS_NUM {
            if buttons[b].physical_num != phy {
                continue;
            }
            if buttons[b].button_type == BUTTON_TYPE_LOGIC {
                continue;
            }
            let type_b = ButtonType::from_u8(buttons[b].button_type);
            let b_compat = type_b.is_some_and(ButtonType::is_gesture_compatible);
            if a_compat && b_compat {
                continue;
            }
            out.push(ConfigError::ButtonCoexistence {
                slot_a: a,
                type_a: buttons[a].button_type,
                slot_b: b,
                type_b: buttons[b].button_type,
                physical_num: phy,
            });
        }
    }
    out
}

fn button_type_label(raw: u8) -> &'static str {
    match ButtonType::from_u8(raw) {
        Some(t) => t.label(),
        None => "unknown",
    }
}

fn logic_op_label(op: LogicOp) -> &'static str {
    op.label()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::logic::BUTTON_TYPE_LOGIC;
    use crate::domain::pins::PinFunction;
    use crate::wire::config::{Button, DeviceConfig, DEV_CONFIG_SIZE};

    fn empty_config() -> DeviceConfig {
        DeviceConfig::decode(&[0u8; DEV_CONFIG_SIZE]).unwrap()
    }

    fn set_button(cfg: &mut DeviceConfig, slot: usize, physical_num: i8, button_type: u8) {
        cfg.buttons[slot] = Button {
            physical_num,
            button_type,
            ..cfg.buttons[slot]
        };
    }

    #[test]
    fn empty_config_validates() {
        assert!(validate_for_write(&empty_config()).is_empty());
    }

    #[test]
    fn surfaces_pin_singleton_conflict() {
        let mut cfg = empty_config();
        cfg.pins[5] = PinFunction::SpiSck.to_i8();
        cfg.pins[12] = PinFunction::SpiSck.to_i8();
        let errors = validate_for_write(&cfg);
        assert!(errors
            .iter()
            .any(|e| matches!(e, ConfigError::Pin(c) if c.slot == 5)));
        assert!(errors
            .iter()
            .any(|e| matches!(e, ConfigError::Pin(c) if c.slot == 12)));
    }

    #[test]
    fn surfaces_incomplete_logic_button() {
        let mut cfg = empty_config();
        // LOGIC slot with physical -1 (Source A unset).
        cfg.buttons[3] = Button {
            physical_num: -1,
            button_type: BUTTON_TYPE_LOGIC,
            src_b: 7,
            flags_a: 0,
            flags_b: LogicOp::And as u8, // binary op
            flags_c: 0,
        };
        let errors = validate_for_write(&cfg);
        assert!(errors.iter().any(|e| matches!(
            e,
            ConfigError::Logic(LogicError::SourceAUnset { button_index: 3 })
        )));
    }

    #[test]
    fn allows_coexistent_gesture_types_on_same_physical() {
        // NORMAL + TAP + DOUBLE_TAP on physical 5 — all three are
        // gesture-compatible per F103_GESTURE_PLAN.md. (The wire calls
        // the long-press variant `Tap`; same gesture, different name.)
        let mut cfg = empty_config();
        set_button(&mut cfg, 0, 5, ButtonType::Normal as u8);
        set_button(&mut cfg, 1, 5, ButtonType::Tap as u8);
        set_button(&mut cfg, 2, 5, ButtonType::DoubleTap as u8);
        let errors = validate_for_write(&cfg);
        assert!(errors
            .iter()
            .all(|e| !matches!(e, ConfigError::ButtonCoexistence { .. })));
    }

    #[test]
    fn flags_coexistence_violation_across_slots() {
        // NORMAL + TOGGLE on the same physical — incompatible.
        let mut cfg = empty_config();
        set_button(&mut cfg, 4, 7, ButtonType::Normal as u8);
        set_button(&mut cfg, 9, 7, ButtonType::Toggle as u8);
        let errors = validate_for_write(&cfg);
        let coex: Vec<_> = errors
            .iter()
            .filter_map(|e| match e {
                ConfigError::ButtonCoexistence {
                    slot_a,
                    slot_b,
                    physical_num,
                    ..
                } => Some((*slot_a, *slot_b, *physical_num)),
                _ => None,
            })
            .collect();
        assert_eq!(coex, vec![(4, 9, 7)]);
    }

    #[test]
    fn unassigned_physical_never_blocks() {
        // Two buttons both unassigned with incompatible types — fine,
        // they aren't actually competing for any physical input.
        let mut cfg = empty_config();
        set_button(&mut cfg, 0, -1, ButtonType::Normal as u8);
        set_button(&mut cfg, 1, -1, ButtonType::Toggle as u8);
        let errors = validate_for_write(&cfg);
        assert!(errors
            .iter()
            .all(|e| !matches!(e, ConfigError::ButtonCoexistence { .. })));
    }

    #[test]
    fn human_summary_uses_one_based_indices() {
        let err = ConfigError::Logic(LogicError::SourceAUnset { button_index: 41 });
        assert!(err.human_summary().contains("#42"));
    }

    #[test]
    fn tab_hint_routes_to_pins_or_buttons() {
        let pin_err = ConfigError::Pin(PinConflict {
            slot: 0,
            kind: PinConflictKind::UnknownFunction(99),
        });
        assert_eq!(pin_err.tab_hint(), "Pins");
        let logic_err = ConfigError::Logic(LogicError::SourceAUnset { button_index: 0 });
        assert_eq!(logic_err.tab_hint(), "Buttons");
    }
}
