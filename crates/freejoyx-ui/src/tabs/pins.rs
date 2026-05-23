//! Pins tab glue: view-model builder + pin-jump target resolver.
//!
//! Owns everything that depends on the 30-pin layout *as data* — i.e.
//! the functions that read `DeviceConfig::pins` and produce model rows
//! or jump targets. Slint callback wiring (`wire_pin_jump_callback`)
//! still lives in `app.rs` because it touches the global `State` and
//! tab-switch logic, but the model-build + target-resolve halves are
//! pure functions of (`DeviceConfig`, `Board`) and live here.
//!
//! Cross-tab helpers ([`axis_back_to_pin`], [`shift_reg_back_to_pin`])
//! live in this module too — they're the Axes / Shift Registers tabs'
//! reverse jumps *back to a pin*, so the pin layout owns them.

use std::rc::Rc;

use freejoyx_core::domain::{validate_pins, AxisSource, Board, PinConflict, PinFunction};
use freejoyx_core::wire::{DeviceConfig, MAX_AXIS_NUM, MAX_SHIFT_REG_NUM, USED_PINS_NUM};
use slint::{Model, SharedString, VecModel};

use crate::PinRow;

/// Does any pin slot carry a non-default function? Drives the Pins tab
/// strip indicator dot.
#[must_use]
pub fn has_content(cfg: &DeviceConfig) -> bool {
    cfg.pins.iter().any(|&p| p != 0)
}

/// Rebuild the Slint `[PinRow]` model from `cfg.pins` + the board's
/// physical layout. Drops every row first and walks the full 40-hole
/// layout so locked power/USB rows render with their fixed roles
/// alongside the configurable GPIO rows.
pub fn refresh_pin_model(model: &Rc<VecModel<PinRow>>, cfg: &DeviceConfig, board: Board) {
    let conflicts: Vec<PinConflict> = validate_pins(&cfg.pins);
    while model.row_count() > 0 {
        model.remove(0);
    }
    let func_list: Vec<PinFunction> = PinFunction::all().collect();
    for board_slot in board.layout() {
        let row = match board_slot.wire_slot {
            Some(wire_slot) => {
                let raw = cfg.pins[wire_slot];
                let function = PinFunction::from_i8(raw).unwrap_or(PinFunction::NotUsed);
                let function_index = func_list.iter().position(|f| *f == function).unwrap_or(0);
                let conflict_msg = conflicts
                    .iter()
                    .find(|c| c.slot == wire_slot)
                    .map(|c| c.kind.short_label())
                    .unwrap_or_default();
                let family = function.family();
                let eligible = function_jump_eligible(function);
                // `enabled` requires both eligibility and an actual
                // resolvable target on the destination tab. Unmapped
                // analog pins (no axis points at them) and overflow
                // shift-data pins (ordinal > MAX_SHIFT_REG_NUM) flag
                // eligible-but-disabled so the slot still renders the
                // arrow placeholder, just greyed.
                let enabled = eligible && pin_jump_target(cfg, wire_slot).is_some();
                PinRow {
                    pin_name: SharedString::from(board_slot.silk),
                    wire_slot: i32::try_from(wire_slot).unwrap_or(0),
                    is_locked: false,
                    locked_role: SharedString::default(),
                    function_label: SharedString::from(function.label()),
                    function_index: i32::try_from(function_index).unwrap_or(0),
                    function_family: i32::from(family.to_u8()),
                    family_short: SharedString::from(family.short_label()),
                    conflict_msg: SharedString::from(conflict_msg),
                    jump_eligible: eligible,
                    jump_enabled: enabled,
                }
            }
            None => PinRow {
                pin_name: SharedString::from(board_slot.silk),
                wire_slot: -1,
                is_locked: true,
                locked_role: SharedString::from(board_slot.role_label),
                function_label: SharedString::default(),
                function_index: 0,
                function_family: 0,
                family_short: SharedString::default(),
                conflict_msg: SharedString::default(),
                jump_eligible: false,
                jump_enabled: false,
            },
        };
        model.push(row);
    }
}

/// True when a pin function maps to a per-row destination on another
/// tab. `AxisAnalog` / `FastEncoder` go to the Axes tab; `ShiftRegData`
/// maps to a specific shift-register slot. `ShiftRegLatch` and
/// `ShiftRegClk` are shared across every chain (one of each, reused
/// by all enabled SRs), so they don't carry a unique target.
fn function_jump_eligible(f: PinFunction) -> bool {
    matches!(
        f,
        PinFunction::AxisAnalog | PinFunction::FastEncoder | PinFunction::ShiftRegData
    )
}

/// Pin-jump target resolved from a wire slot and the in-memory config.
/// Tab index uses `AppWindow`'s active-tab convention (1 = Axes,
/// 5 = Shift Registers).
#[derive(Debug, Clone, Copy)]
pub struct PinJumpTarget {
    pub tab: i32,
    pub slot: i32,
}

/// Resolve `pins[wire_slot]` → which tab and which row to flash /
/// scroll to. Returns None when the pin isn't currently mapped to
/// any row (e.g. an `AxisAnalog` pin that no axis has picked yet) —
/// the UI surfaces that as a disabled jump button.
///
/// - `AxisAnalog` → first axis whose `source_main` matches `wire_slot`.
///   No match → None (button greys out: pin assigned but unmapped).
/// - `FastEncoder` → first axis whose source is `Encoder N` where N
///   is the encoder slot the pin belongs to (PA8/PA9 → 0,
///   PB6/PB7 → 1). No match → None.
/// - `ShiftRegData` → the shift register slot determined by data-pin
///   ordinal (matches the firmware's `shift_registers.c` enumeration:
///   the 1st data pin in `pins[]` feeds SR 0, the 2nd feeds SR 1, …).
#[must_use]
pub fn pin_jump_target(cfg: &DeviceConfig, wire_slot: usize) -> Option<PinJumpTarget> {
    if wire_slot >= USED_PINS_NUM {
        return None;
    }
    let function = PinFunction::from_i8(cfg.pins[wire_slot])?;
    match function {
        PinFunction::AxisAnalog => {
            let pin_idx = u8::try_from(wire_slot).ok()?;
            let axis_slot = cfg
                .axis_config
                .iter()
                .position(|a| matches!(a.source(), AxisSource::Pin(p) if p == pin_idx))?;
            Some(PinJumpTarget {
                tab: 1,
                slot: i32::try_from(axis_slot).ok()?,
            })
        }
        PinFunction::FastEncoder => {
            let encoder_slot = encoder_slot_for_pin(wire_slot)?;
            let axis_slot = cfg
                .axis_config
                .iter()
                .position(|a| matches!(a.source(), AxisSource::Encoder(e) if e == encoder_slot))?;
            Some(PinJumpTarget {
                tab: 1,
                slot: i32::try_from(axis_slot).ok()?,
            })
        }
        PinFunction::ShiftRegData => {
            let sr_slot = shift_register_slot_for_data_pin(&cfg.pins, wire_slot)?;
            Some(PinJumpTarget {
                tab: 5,
                slot: i32::try_from(sr_slot).ok()?,
            })
        }
        _ => None,
    }
}

/// The firmware walks `pins[]` in array order and assigns each
/// `ShiftRegData` it encounters to the next shift-register slot
/// (`shift_registers.c:56-58`). Mirror that here: the ordinal of
/// `wire_slot` among `ShiftRegData` pins is the SR slot it drives.
/// Returns None if `wire_slot` isn't a Data pin or the ordinal
/// exceeds `MAX_SHIFT_REG_NUM`.
fn shift_register_slot_for_data_pin(pins: &[i8], wire_slot: usize) -> Option<usize> {
    if pins.get(wire_slot).copied().and_then(PinFunction::from_i8)
        != Some(PinFunction::ShiftRegData)
    {
        return None;
    }
    let ordinal = pins
        .iter()
        .take(wire_slot)
        .filter(|&&p| PinFunction::from_i8(p) == Some(PinFunction::ShiftRegData))
        .count();
    if ordinal < MAX_SHIFT_REG_NUM {
        Some(ordinal)
    } else {
        None
    }
}

/// Reverse of [`pin_jump_target`] for the Axes tab: given an axis
/// slot, return the wire slot of the pin that drives it (so the user
/// can hop from an Axis row back to its source pin on the Pins tab).
///
/// - `AxisSource::Pin(N)` → `Some(N)` if pin N currently carries
///   `AxisAnalog`; otherwise None (the source field outlived the pin
///   role).
/// - `AxisSource::Encoder(N)` → the first `FastEncoder`-assigned pin
///   in the encoder's pin pair (PA8 for slot 0, PB6 for slot 1).
/// - `AxisSource::None` / `AxisSource::I2C` → None.
#[must_use]
pub fn axis_back_to_pin(cfg: &DeviceConfig, axis_slot: usize) -> Option<usize> {
    if axis_slot >= MAX_AXIS_NUM {
        return None;
    }
    match cfg.axis_config[axis_slot].source() {
        AxisSource::Pin(idx) => {
            let slot = idx as usize;
            (cfg.pins.get(slot).copied().and_then(PinFunction::from_i8)
                == Some(PinFunction::AxisAnalog))
            .then_some(slot)
        }
        AxisSource::Encoder(slot) => {
            // Slot 0 = Enc 1 on PA8 (8) / PA9 (9).
            // Slot 1 = Enc 2 on PB6 (17) / PB7 (18).
            // Prefer the A-pin (first half) so the flash always lands
            // in a predictable spot, then fall back to the B-pin in
            // case the user has assigned only one half.
            let candidates: [usize; 2] = match slot {
                0 => [8, 9],
                1 => [17, 18],
                _ => return None,
            };
            candidates.into_iter().find(|&p| {
                cfg.pins.get(p).copied().and_then(PinFunction::from_i8)
                    == Some(PinFunction::FastEncoder)
            })
        }
        AxisSource::None | AxisSource::I2C => None,
    }
}

/// Reverse of [`pin_jump_target`] for the Shift Registers tab: given
/// an SR slot, return the wire slot of the Nth `ShiftRegData` pin in
/// `pins[]` — the pin the firmware will wire to that register.
#[must_use]
pub fn shift_reg_back_to_pin(cfg: &DeviceConfig, sr_slot: usize) -> Option<usize> {
    if sr_slot >= MAX_SHIFT_REG_NUM {
        return None;
    }
    cfg.pins
        .iter()
        .enumerate()
        .filter(|(_, &p)| PinFunction::from_i8(p) == Some(PinFunction::ShiftRegData))
        .nth(sr_slot)
        .map(|(i, _)| i)
}

/// Map a `FastEncoder` pin slot to the encoder slot it belongs to.
/// Slot 0 = Enc 1 (PA8 = wire slot 8, PA9 = slot 9).
/// Slot 1 = Enc 2 (PB6 = wire slot 17, PB7 = slot 18).
fn encoder_slot_for_pin(wire_slot: usize) -> Option<u8> {
    match wire_slot {
        8 | 9 => Some(0),
        17 | 18 => Some(1),
        _ => None,
    }
}
