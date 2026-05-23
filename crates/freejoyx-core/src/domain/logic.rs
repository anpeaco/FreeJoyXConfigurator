//! LOGIC button validator.
//!
//! Port of `ButtonLogical::isLogicConfigComplete()` from
//! `FreeJoyXConfiguratorQt/src/widgets/buttons/buttonlogical.cpp`. The
//! configurator does **not** evaluate the boolean expression (the firmware
//! does that, per Port.md §3) — it only checks that a LOGIC slot is
//! filled in to the point the firmware can run it.
//!
//! Two failure modes:
//!
//! - Source A (`physical_num`) is unset. Both unary and binary ops need
//!   Source A.
//! - The operator is binary (AND / OR / NAND / NOR / XOR / A_AND_NOT_B)
//!   but Source B (`src_b`) is unset.
//!
//! The "operator is unpicked" sentinel that the Qt UI carries (`op == -1`)
//! does not apply here — the wire format stores `op` as a 3-bit unsigned
//! field, so on the wire `op` is always picked (it defaults to AND = 0).
//! The Slice 7 UI layer is responsible for distinguishing "user explicitly
//! chose AND" from "user hasn't touched the dropdown yet"; this validator
//! only checks the wire-level invariants.

use crate::wire::config::DeviceConfig;

/// `LOGIC` value from the `button_type_t` enum in
/// `vendored/common_types.h` (33). Anchored by a test that decodes the
/// `wide_coverage` fixture and counts at least one LOGIC slot — if the
/// upstream enum reshuffles, the test fails loudly.
pub const BUTTON_TYPE_LOGIC: u8 = 33;

/// Logic operator codes (the 3-bit `op` field on `button_t`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum LogicOp {
    And = 0,
    Or = 1,
    Not = 2,
    Nor = 3,
    Nand = 4,
    Xor = 5,
    AAndNotB = 6,
    /// `LOGIC_OP_XNOR` — added in firmware 0x0020. Fits the 3-bit `op`
    /// field; `LOGIC_OP_COUNT` is now 8 so the field is at capacity.
    Xnor = 7,
}

impl LogicOp {
    /// Map the raw 3-bit op field. All eight values are now operator
    /// codes (firmware 0x0020 filled the last slot with `LOGIC_OP_XNOR`).
    #[must_use]
    pub fn from_u8(v: u8) -> Option<Self> {
        Some(match v {
            0 => Self::And,
            1 => Self::Or,
            2 => Self::Not,
            3 => Self::Nor,
            4 => Self::Nand,
            5 => Self::Xor,
            6 => Self::AAndNotB,
            7 => Self::Xnor,
            _ => return None,
        })
    }

    /// True if the op consumes both Source A and Source B. NOT is the
    /// only unary op in the MVP set (Port.md §1 / F103_LOGIC_PLAN.md).
    #[must_use]
    pub fn is_binary(self) -> bool {
        !matches!(self, Self::Not)
    }

    /// Display string for pickers and selected-value cells. Uppercase
    /// to match how users write boolean algebra and to read as an
    /// operator label, not as prose.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::And => "AND",
            Self::Or => "OR",
            Self::Not => "NOT",
            Self::Nor => "NOR",
            Self::Nand => "NAND",
            Self::Xor => "XOR",
            Self::AAndNotB => "A AND NOT B",
            Self::Xnor => "XNOR",
        }
    }

    /// One-line plain-English summary of when the virtual button fires.
    /// Surfaces on the LOGIC op cell as a hover tooltip so users
    /// learning the operator set don't need to keep a truth table in
    /// their head.
    #[must_use]
    pub fn truth_summary(self) -> &'static str {
        match self {
            Self::And => "Fires when Source A and Source B are both pressed.",
            Self::Or => "Fires when Source A or Source B is pressed.",
            Self::Not => "Fires when Source A is not pressed.",
            Self::Nor => "Fires when neither Source A nor Source B is pressed.",
            Self::Nand => "Fires unless both Source A and Source B are pressed.",
            Self::Xor => "Fires when exactly one of Source A or Source B is pressed.",
            Self::AAndNotB => "Fires when Source A is pressed and Source B is not.",
            Self::Xnor => "Fires when Source A and Source B match (both pressed or both released).",
        }
    }
}

/// One reason a LOGIC button slot is incomplete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LogicError {
    /// `physical_num` (Source A) is the `-1` sentinel.
    SourceAUnset { button_index: usize },
    /// Binary op picked but `src_b` is the `-1` sentinel.
    SourceBUnsetForBinaryOp { button_index: usize, op: LogicOp },
    /// `op` field holds a value not mapped to any [`LogicOp`]. With
    /// firmware 0x0020 every 3-bit value (0..=7) is a valid operator
    /// (XNOR fills slot 7), so this variant is unreachable today; kept
    /// for forward compatibility if the wire-format ever widens `op`.
    OpOutOfRange { button_index: usize, op_raw: u8 },
}

/// Scan a `DeviceConfig` for incomplete LOGIC button slots.
///
/// Returns an empty `Vec` if every LOGIC slot is well-formed. The caller
/// (e.g. the Buttons tab when it lands in Slice 7, or a pre-write guard
/// before pushing config to the device) decides how to surface the
/// errors.
#[must_use]
pub fn validate_logic_buttons(config: &DeviceConfig) -> Vec<LogicError> {
    let mut errs = Vec::new();
    for (i, btn) in config.buttons.iter().enumerate() {
        if btn.button_type != BUTTON_TYPE_LOGIC {
            continue;
        }
        // Source A
        if btn.physical_num < 0 {
            errs.push(LogicError::SourceAUnset { button_index: i });
        }
        // Operator + Source B
        match LogicOp::from_u8(btn.op()) {
            None => errs.push(LogicError::OpOutOfRange {
                button_index: i,
                op_raw: btn.op(),
            }),
            Some(op) if op.is_binary() && btn.src_b < 0 => {
                errs.push(LogicError::SourceBUnsetForBinaryOp {
                    button_index: i,
                    op,
                });
            }
            Some(_) => {}
        }
    }
    errs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::config::{Button, DeviceConfig, DEV_CONFIG_SIZE};

    fn empty_config() -> DeviceConfig {
        DeviceConfig::decode(&[0u8; DEV_CONFIG_SIZE]).unwrap()
    }

    fn make_logic_button(physical_num: i8, src_b: i8, op_bits: u8) -> Button {
        Button {
            physical_num,
            button_type: BUTTON_TYPE_LOGIC,
            src_b,
            flags_a: 0,
            flags_b: op_bits & 0x07,
            flags_c: 0,
        }
    }

    #[test]
    fn no_logic_buttons_means_no_errors() {
        let cfg = empty_config();
        assert!(validate_logic_buttons(&cfg).is_empty());
    }

    #[test]
    fn well_formed_binary_logic_passes() {
        let mut cfg = empty_config();
        cfg.buttons[0] = make_logic_button(3, 5, LogicOp::And as u8);
        cfg.buttons[1] = make_logic_button(2, 7, LogicOp::Xor as u8);
        assert!(validate_logic_buttons(&cfg).is_empty());
    }

    #[test]
    fn well_formed_unary_not_passes_with_unset_src_b() {
        let mut cfg = empty_config();
        cfg.buttons[5] = make_logic_button(3, -1, LogicOp::Not as u8);
        assert!(validate_logic_buttons(&cfg).is_empty());
    }

    #[test]
    fn binary_op_with_unset_src_b_flagged() {
        let mut cfg = empty_config();
        cfg.buttons[7] = make_logic_button(3, -1, LogicOp::Or as u8);
        let errs = validate_logic_buttons(&cfg);
        assert_eq!(
            errs,
            vec![LogicError::SourceBUnsetForBinaryOp {
                button_index: 7,
                op: LogicOp::Or,
            }]
        );
    }

    #[test]
    fn unset_source_a_flagged() {
        let mut cfg = empty_config();
        cfg.buttons[2] = make_logic_button(-1, 4, LogicOp::And as u8);
        let errs = validate_logic_buttons(&cfg);
        assert_eq!(errs, vec![LogicError::SourceAUnset { button_index: 2 }]);
    }

    #[test]
    fn xnor_is_a_valid_binary_op() {
        // Firmware 0x0020 added LOGIC_OP_XNOR at slot 7. With both
        // sources set the slot is well-formed; with src_b unset it
        // flags the same binary-op violation as the other binary ops.
        let mut cfg = empty_config();
        cfg.buttons[0] = make_logic_button(3, 4, LogicOp::Xnor as u8);
        assert!(validate_logic_buttons(&cfg).is_empty());

        let mut cfg = empty_config();
        cfg.buttons[0] = make_logic_button(3, -1, LogicOp::Xnor as u8);
        assert_eq!(
            validate_logic_buttons(&cfg),
            vec![LogicError::SourceBUnsetForBinaryOp {
                button_index: 0,
                op: LogicOp::Xnor,
            }]
        );
    }

    #[test]
    fn unset_source_a_and_b_both_flagged() {
        let mut cfg = empty_config();
        cfg.buttons[9] = make_logic_button(-1, -1, LogicOp::Nand as u8);
        let errs = validate_logic_buttons(&cfg);
        assert_eq!(errs.len(), 2);
        assert!(errs.contains(&LogicError::SourceAUnset { button_index: 9 }));
        assert!(errs.contains(&LogicError::SourceBUnsetForBinaryOp {
            button_index: 9,
            op: LogicOp::Nand,
        }));
    }

    #[test]
    fn non_logic_buttons_ignored() {
        let mut cfg = empty_config();
        // BUTTON_NORMAL with -1 src_b — looks malformed by LOGIC rules
        // but it's not a LOGIC slot, so the validator must skip it.
        cfg.buttons[3] = Button {
            physical_num: -1,
            button_type: 0, // BUTTON_NORMAL
            src_b: -1,
            flags_a: 0,
            flags_b: 0,
            flags_c: 0,
        };
        assert!(validate_logic_buttons(&cfg).is_empty());
    }
}
