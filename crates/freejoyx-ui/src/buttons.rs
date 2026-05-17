//! Slice 7 glue: button model + shifts/timers model + their callbacks.
//!
//! Sliced out of [`crate::app`] because the bulk of the surface is here
//! and `app::run` was already at the clippy `too_many_lines` cap after
//! Slice 6. The shape mirrors [`crate::app::wire_axis_callbacks`]: each
//! callback funnels through a `mutate_button` / `mutate_shift` /
//! `mutate_timer` that takes the held config under `RefCell::borrow_mut`,
//! applies the edit, refreshes only the touched row, and marks the
//! config dirty (`can_write = connected`, `can_save = true`).

use std::cell::RefCell;
use std::rc::Rc;

use freejoyx_core::domain::{
    physical_assignment_blocked, validate_logic_buttons, ButtonType, CoexistenceCheck, LogicError,
    LogicOp, BUTTON_TYPE_LOGIC,
};
use freejoyx_core::wire::{Button, DeviceConfig, ParamsReport, MAX_BUTTONS_NUM, MAX_SHIFTS_NUM};
use slint::{ComponentHandle, Model, SharedString, VecModel};

use crate::{AppWindow, ButtonRow, ShiftSlot, TimerField};

/// Number of editable timer fields surfaced on the Shifts & Timers tab.
/// Indices map to `TIMER_FIELDS` below.
pub const TIMER_FIELD_COUNT: usize = 6;

/// `(label, hint)` for each editable timer field, in the order the UI
/// renders them. The same indices feed `apply_timer_edit`.
const TIMER_FIELDS: [(&str, &str); TIMER_FIELD_COUNT] = [
    (
        "Button Timer 1",
        "shared timer used by per-button delay/press picks",
    ),
    (
        "Button Timer 2",
        "shared timer used by per-button delay/press picks",
    ),
    (
        "Button Timer 3",
        "shared timer used by per-button delay/press picks",
    ),
    (
        "Button Debounce",
        "rising/falling edge filter for every physical input",
    ),
    (
        "Tap cutoff",
        "TAP fires only if released within this window",
    ),
    (
        "Double-tap window",
        "second tap must arrive within this window",
    ),
];

fn timer_value_at(cfg: &DeviceConfig, index: usize) -> u16 {
    match index {
        0 => cfg.button_timer1_ms,
        1 => cfg.button_timer2_ms,
        2 => cfg.button_timer3_ms,
        3 => cfg.button_debounce_ms,
        4 => cfg.tap_cutoff_ms,
        5 => cfg.double_tap_window_ms,
        _ => 0,
    }
}

fn set_timer_value(cfg: &mut DeviceConfig, index: usize, v: u16) {
    match index {
        0 => cfg.button_timer1_ms = v,
        1 => cfg.button_timer2_ms = v,
        2 => cfg.button_timer3_ms = v,
        3 => cfg.button_debounce_ms = v,
        4 => cfg.tap_cutoff_ms = v,
        5 => cfg.double_tap_window_ms = v,
        _ => {}
    }
}

/// Rebuild every button row from `cfg`. The model is rebuilt wholesale
/// the first time (or after a load/read) and per-row otherwise so a
/// `TextInput` in mid-edit keeps focus.
pub fn refresh_button_model(
    model: &Rc<VecModel<ButtonRow>>,
    cfg: &DeviceConfig,
    params: Option<&ParamsReport>,
) {
    let logic_errors = validate_logic_buttons(cfg);

    if model.row_count() != MAX_BUTTONS_NUM {
        while model.row_count() > 0 {
            model.remove(0);
        }
        for slot in 0..MAX_BUTTONS_NUM {
            model.push(build_button_row(
                slot,
                &cfg.buttons[slot],
                params,
                &logic_errors,
            ));
        }
        return;
    }
    for slot in 0..MAX_BUTTONS_NUM {
        let row = build_button_row(slot, &cfg.buttons[slot], params, &logic_errors);
        model.set_row_data(slot, row);
    }
}

fn build_button_row(
    slot: usize,
    btn: &Button,
    params: Option<&ParamsReport>,
    logic_errors: &[LogicError],
) -> ButtonRow {
    let typed = ButtonType::from_u8(btn.button_type);
    let type_label = typed.map(ButtonType::label).map_or_else(
        || format!("? ({})", btn.button_type),
        std::string::ToString::to_string,
    );
    let is_logic = btn.button_type == BUTTON_TYPE_LOGIC;
    let op_typed = LogicOp::from_u8(btn.op());
    let op_label = op_typed.map_or_else(|| format!("?{}", btn.op()), |o| format!("{o:?}"));
    let debounce_label = timer_picker_label(btn.delay_timer());
    let logic_error = logic_errors
        .iter()
        .find(|e| {
            matches!(e,
                LogicError::SourceAUnset { button_index } |
                LogicError::SourceBUnsetForBinaryOp { button_index, .. } |
                LogicError::OpOutOfRange { button_index, .. }
                if *button_index == slot
            )
        })
        .map(short_logic_error_label)
        .unwrap_or_default();
    let (phy, log) = pressed_bits(params, slot);
    ButtonRow {
        physical_num: i32::from(btn.physical_num),
        type_label: SharedString::from(type_label),
        type_index: typed.map(ButtonType::to_u8).map_or(-1, i32::from),
        is_logic,
        shift_modificator: i32::from(btn.shift_modificator()),
        is_inverted: btn.is_inverted(),
        is_disabled: btn.is_disabled(),
        src_b: i32::from(btn.src_b),
        op_label: SharedString::from(op_label),
        op_index: i32::from(btn.op()),
        debounce_label: SharedString::from(debounce_label),
        debounce_index: i32::from(btn.delay_timer()),
        logic_error: SharedString::from(logic_error),
        phy_pressed: phy,
        log_pressed: log,
    }
}

fn timer_picker_label(v: u8) -> String {
    match v {
        0 => "off".to_string(),
        1..=3 => format!("Timer {v}"),
        _ => format!("?{v}"),
    }
}

fn short_logic_error_label(e: &LogicError) -> String {
    match e {
        LogicError::SourceAUnset { .. } => "Source A unset".to_string(),
        LogicError::SourceBUnsetForBinaryOp { op, .. } => format!("Source B unset for {op:?}"),
        LogicError::OpOutOfRange { op_raw, .. } => format!("op {op_raw} out of range"),
    }
}

fn pressed_bits(params: Option<&ParamsReport>, slot: usize) -> (bool, bool) {
    let Some(p) = params else {
        return (false, false);
    };
    let byte = slot / 8;
    let bit = slot % 8;
    let mask = 1u8 << bit;
    let phy = p.phy_button_data.get(byte).copied().unwrap_or(0) & mask != 0;
    let log = p.log_button_data.get(byte).copied().unwrap_or(0) & mask != 0;
    (phy, log)
}

pub fn refresh_shift_model(model: &Rc<VecModel<ShiftSlot>>, cfg: &DeviceConfig) {
    if model.row_count() != MAX_SHIFTS_NUM {
        while model.row_count() > 0 {
            model.remove(0);
        }
        for slot in 0..MAX_SHIFTS_NUM {
            model.push(ShiftSlot {
                label: SharedString::from(format!("Shift {}", slot + 1)),
                button_index: i32::from(cfg.shift_config[slot]),
            });
        }
        return;
    }
    for slot in 0..MAX_SHIFTS_NUM {
        model.set_row_data(
            slot,
            ShiftSlot {
                label: SharedString::from(format!("Shift {}", slot + 1)),
                button_index: i32::from(cfg.shift_config[slot]),
            },
        );
    }
}

pub fn refresh_timer_model(model: &Rc<VecModel<TimerField>>, cfg: &DeviceConfig) {
    if model.row_count() != TIMER_FIELD_COUNT {
        while model.row_count() > 0 {
            model.remove(0);
        }
        for (i, (label, hint)) in TIMER_FIELDS.iter().enumerate() {
            model.push(TimerField {
                label: SharedString::from(*label),
                hint: SharedString::from(*hint),
                value_ms: i32::from(timer_value_at(cfg, i)),
            });
        }
        return;
    }
    for (i, (label, hint)) in TIMER_FIELDS.iter().enumerate() {
        model.set_row_data(
            i,
            TimerField {
                label: SharedString::from(*label),
                hint: SharedString::from(*hint),
                value_ms: i32::from(timer_value_at(cfg, i)),
            },
        );
    }
}

/// Shared dirty-marker for every button/shift/timer edit. Sets
/// `can_write` (gated on connection) + `can_save` so the toolbar
/// surfaces that the held config has unsaved edits.
pub fn mark_dirty(window: &slint::Weak<AppWindow>) {
    if let Some(w) = window.upgrade() {
        w.set_can_write(w.get_connected());
        w.set_can_save(true);
    }
}

/// Refresh one button row in the model after a per-slot mutation.
/// Re-runs `validate_logic_buttons` only across that slot's neighbours
/// — actually, simpler: re-run it over the full config and pull the
/// matching error. The validator is O(128) and runs only on edit,
/// negligible.
pub fn refresh_button_row(
    model: &Rc<VecModel<ButtonRow>>,
    slot: usize,
    cfg: &DeviceConfig,
    params: Option<&ParamsReport>,
) {
    let logic_errors = validate_logic_buttons(cfg);
    let row = build_button_row(slot, &cfg.buttons[slot], params, &logic_errors);
    model.set_row_data(slot, row);
}

/// Step the type cycle, skipping variants blocked by the per-physical
/// coexistence rule. Returns the new wire byte the caller should write
/// into `button.button_type`.
///
/// If `current` doesn't decode to a known type we start at `Normal`
/// rather than try to interpret garbage. If *every* variant is blocked
/// (shouldn't happen — Normal is always gesture-compatible) the
/// function returns the current value unchanged.
#[must_use]
pub fn next_compatible_type(
    buttons: &[Button; MAX_BUTTONS_NUM],
    slot: usize,
    physical_num: i8,
    current: u8,
) -> u8 {
    let start = ButtonType::from_u8(current).map_or(0u8, ButtonType::to_u8);
    for step in 1u8..=36 {
        let candidate = (u16::from(start) + u16::from(step)) % 36;
        let candidate_u8 = u8::try_from(candidate).unwrap_or(0);
        let Some(typed) = ButtonType::from_u8(candidate_u8) else {
            continue;
        };
        match physical_assignment_blocked(buttons, slot, physical_num, typed) {
            CoexistenceCheck::Ok => return candidate_u8,
            CoexistenceCheck::Blocked { .. } => {}
        }
    }
    current
}

/// Mutate a button slot under `state.last_config`. Returns the most
/// recent `ParamsReport` so the caller can refresh the row with live
/// state. Returns `None` if no config is loaded.
pub fn with_button_slot<R>(
    state: &Rc<RefCell<crate::app::State>>,
    slot: usize,
    f: impl FnOnce(&mut Button, &[Button; MAX_BUTTONS_NUM]) -> R,
) -> Option<R> {
    if slot >= MAX_BUTTONS_NUM {
        return None;
    }
    let mut s = state.borrow_mut();
    let cfg = s.last_config.as_mut()?;
    let buttons_snapshot = cfg.buttons.clone();
    let r = f(&mut cfg.buttons[slot], &buttons_snapshot);
    Some(r)
}

/// Wire all button-row / shift-slot / timer callbacks onto `window`.
#[allow(clippy::too_many_lines)]
pub fn wire_callbacks(
    window: &AppWindow,
    state: &Rc<RefCell<crate::app::State>>,
    button_model: &Rc<VecModel<ButtonRow>>,
    shift_model: &Rc<VecModel<ShiftSlot>>,
    timer_model: &Rc<VecModel<TimerField>>,
) {
    let mk_btn_int = |cb: fn(&mut Button, i32)| {
        let s = state.clone();
        let m = button_model.clone();
        let w = window.as_weak();
        move |slot: i32, v: i32| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            let _ = with_button_slot(&s, slot, |b, _| cb(b, v));
            refresh_after_button_edit(&s, &m, slot);
            mark_dirty(&w);
        }
    };
    let mk_btn_toggle = |cb: fn(&mut Button)| {
        let s = state.clone();
        let m = button_model.clone();
        let w = window.as_weak();
        move |slot: i32| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            let _ = with_button_slot(&s, slot, |b, _| cb(b));
            refresh_after_button_edit(&s, &m, slot);
            mark_dirty(&w);
        }
    };

    window.on_button_physical_edited(mk_btn_int(|b, v| {
        b.physical_num = clamp_i8(v);
    }));
    window.on_button_src_b_edited(mk_btn_int(|b, v| {
        b.src_b = clamp_i8(v);
    }));
    window.on_button_inverted_toggled(mk_btn_toggle(|b| b.set_is_inverted(!b.is_inverted())));
    window.on_button_disabled_toggled(mk_btn_toggle(|b| b.set_is_disabled(!b.is_disabled())));
    window.on_button_shift_cycled(mk_btn_toggle(|b| {
        let next = (b.shift_modificator() + 1) % 9;
        b.set_shift_modificator(next);
    }));
    window.on_button_op_cycled(mk_btn_toggle(|b| {
        let next = (b.op() + 1) % 7;
        b.set_op(next);
    }));
    window.on_button_debounce_cycled(mk_btn_toggle(|b| {
        let next = (b.delay_timer() + 1) % 4;
        b.set_delay_timer(next);
    }));

    // Type cycle has to consult the coexistence rule, so it can't
    // use the simple toggle helper.
    {
        let s = state.clone();
        let m = button_model.clone();
        let w = window.as_weak();
        window.on_button_type_cycled(move |slot| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            let _ = with_button_slot(&s, slot, |b, all_buttons| {
                let next = next_compatible_type(all_buttons, slot, b.physical_num, b.button_type);
                b.button_type = next;
            });
            refresh_after_button_edit(&s, &m, slot);
            mark_dirty(&w);
        });
    }

    // Shift slot edit (i8 button index, -1 = unused).
    {
        let s = state.clone();
        let m = shift_model.clone();
        let w = window.as_weak();
        window.on_shift_edited(move |idx, v| {
            let Ok(idx) = usize::try_from(idx) else {
                return;
            };
            if idx >= MAX_SHIFTS_NUM {
                return;
            }
            {
                let mut st = s.borrow_mut();
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                cfg.shift_config[idx] = clamp_i8(v);
            }
            if let Some(cfg) = s.borrow().last_config.as_ref() {
                m.set_row_data(
                    idx,
                    ShiftSlot {
                        label: SharedString::from(format!("Shift {}", idx + 1)),
                        button_index: i32::from(cfg.shift_config[idx]),
                    },
                );
            }
            mark_dirty(&w);
        });
    }

    // Timer edit (u16 ms field at one of TIMER_FIELDS' indices).
    {
        let s = state.clone();
        let m = timer_model.clone();
        let w = window.as_weak();
        window.on_timer_edited(move |idx, v| {
            let Ok(idx) = usize::try_from(idx) else {
                return;
            };
            if idx >= TIMER_FIELD_COUNT {
                return;
            }
            let clamped = u16::try_from(v.clamp(0, i32::from(u16::MAX))).unwrap_or(0);
            {
                let mut st = s.borrow_mut();
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                set_timer_value(cfg, idx, clamped);
            }
            let (label, hint) = TIMER_FIELDS[idx];
            m.set_row_data(
                idx,
                TimerField {
                    label: SharedString::from(label),
                    hint: SharedString::from(hint),
                    value_ms: i32::from(clamped),
                },
            );
            mark_dirty(&w);
        });
    }
}

fn refresh_after_button_edit(
    state: &Rc<RefCell<crate::app::State>>,
    button_model: &Rc<VecModel<ButtonRow>>,
    slot: usize,
) {
    let s = state.borrow();
    if let Some(cfg) = s.last_config.as_ref() {
        refresh_button_row(button_model, slot, cfg, s.last_params.as_ref());
    }
}

fn clamp_i8(v: i32) -> i8 {
    i8::try_from(v.clamp(i32::from(i8::MIN), i32::from(i8::MAX))).unwrap_or(0)
}
