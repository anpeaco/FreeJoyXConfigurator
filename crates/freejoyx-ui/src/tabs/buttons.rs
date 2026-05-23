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
    physical_assignment_blocked, validate_logic_buttons, ButtonCapture, ButtonType,
    ButtonTypeCategory, CaptureTarget, CoexistenceCheck, LogicError, LogicOp, BUTTON_TYPE_LOGIC,
};
use freejoyx_core::wire::{Button, DeviceConfig, ParamsReport, MAX_BUTTONS_NUM, MAX_SHIFTS_NUM};
use slint::{ComponentHandle, Model, SharedString, VecModel};

use crate::{AppWindow, ButtonRow, DropdownEntry, ShiftSlot, TimerField};

/// Does any button slot have a meaningful assignment? A slot counts as
/// configured when it carries a physical mapping OR a non-NORMAL type
/// (LOGIC, TOGGLE, POV, etc.) — `physical_num >= 0` covers the everyday
/// "I've assigned this button" case; `button_type != 0` catches LOGIC
/// slots that may not have a physical yet but are still in-progress.
#[must_use]
pub fn has_content(cfg: &DeviceConfig) -> bool {
    cfg.buttons
        .iter()
        .any(|b| b.physical_num >= 0 || b.button_type != 0)
}

/// Does the Shifts & Timers tab have any non-default configuration?
/// Today: any of the 5 shift modificators on any button is non-zero,
/// or any timer field is non-default (zero on the wire).
#[must_use]
pub fn shifts_has_content(cfg: &DeviceConfig) -> bool {
    cfg.buttons.iter().any(|b| b.shift_modificator() != 0)
        || cfg.button_timer1_ms != 0
        || cfg.button_timer2_ms != 0
        || cfg.button_timer3_ms != 0
        || cfg.button_debounce_ms != 0
        || cfg.encoder_press_time_ms != 0
        || cfg.a2b_debounce_ms != 0
}

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

/// Buttons-tab filter inputs. Cheaper to pass as a small struct than as
/// 4 ad-hoc args. `hide_unused` hides slots whose `physical_num` is unset
/// (`< 0`) unless the slot is force-shown via [`Self::force_shown`].
/// `filter_physical` (`Some(n)`) restricts to slots that map to a
/// physical input. `filter_category` (`Some(c)`) restricts to slots
/// whose `ButtonType` belongs to category `c`.
#[derive(Debug, Clone)]
pub struct ButtonFilter<'a> {
    pub hide_unused: bool,
    pub filter_physical: Option<i8>,
    pub filter_category: Option<ButtonTypeCategory>,
    pub force_shown: &'a std::collections::BTreeSet<usize>,
}

fn slot_visible(filter: &ButtonFilter<'_>, slot: usize, btn: &Button) -> bool {
    // "+ Add"-promoted slots stay visible regardless of the filter.
    if filter.force_shown.contains(&slot) {
        return true;
    }
    if filter.hide_unused && btn.physical_num < 0 {
        return false;
    }
    if let Some(want_phy) = filter.filter_physical {
        if btn.physical_num != want_phy {
            return false;
        }
    }
    if let Some(want_cat) = filter.filter_category {
        let bt = ButtonType::from_u8(btn.button_type);
        match bt {
            Some(t) if t.category() == want_cat => {}
            _ => return false,
        }
    }
    true
}

/// Rebuild the button-row model honoring the current filter. Updates
/// in-place when the visible-slot set hasn't changed (preserves Slint
/// element identity — critical, otherwise the per-tick rebuild from
/// `ParamsTick` destroys/recreates each row's `TextInput` and steals
/// focus from any cell the user is editing or arming for capture).
/// Falls back to a full wipe-and-push only when filters / `+Add`
/// promotion change which slots are visible.
pub fn refresh_button_model(
    model: &Rc<VecModel<ButtonRow>>,
    cfg: &DeviceConfig,
    params: Option<&ParamsReport>,
    filter: &ButtonFilter<'_>,
    capture: &ButtonCapture,
) {
    let logic_errors = validate_logic_buttons(cfg);
    let visible_slots: Vec<usize> = (0..MAX_BUTTONS_NUM)
        .filter(|&s| slot_visible(filter, s, &cfg.buttons[s]))
        .collect();

    let unchanged_layout = visible_slots.len() == model.row_count()
        && visible_slots.iter().enumerate().all(|(i, &slot)| {
            model
                .row_data(i)
                .is_some_and(|r| usize::try_from(r.slot).unwrap_or(usize::MAX) == slot)
        });

    if unchanged_layout {
        // Dedup: only push a row update when the rebuilt struct
        // actually differs. Setting row data unconditionally re-fires
        // the `for ... in model:` binding chain, which re-sets the
        // focused TextInput's `text` binding — and Slint 1.13 drops
        // focus from a TextInput whose `text` is reassigned. Without
        // this guard, the 10 Hz ParamsTick rebuild silently steals
        // focus from any cell the user just clicked.
        for (i, &slot) in visible_slots.iter().enumerate() {
            let new_row = build_button_row(
                slot,
                &cfg.buttons[slot],
                params,
                &logic_errors,
                capture.disarm_ticks(slot),
            );
            if model.row_data(i).is_some_and(|existing| existing == new_row) {
                continue;
            }
            model.set_row_data(i, new_row);
        }
        return;
    }

    while model.row_count() > 0 {
        model.remove(0);
    }
    for slot in visible_slots {
        model.push(build_button_row(
            slot,
            &cfg.buttons[slot],
            params,
            &logic_errors,
            capture.disarm_ticks(slot),
        ));
    }
}

pub fn build_button_row(
    slot: usize,
    btn: &Button,
    params: Option<&ParamsReport>,
    logic_errors: &[LogicError],
    disarm_ticks: (i32, i32),
) -> ButtonRow {
    let typed = ButtonType::from_u8(btn.button_type);
    let type_label = typed.map(ButtonType::label).map_or_else(
        || format!("? ({})", btn.button_type),
        std::string::ToString::to_string,
    );
    let is_logic = btn.button_type == BUTTON_TYPE_LOGIC;
    let op_typed = LogicOp::from_u8(btn.op());
    let op_label = op_typed.map_or_else(|| format!("?{}", btn.op()), |o| o.label().to_string());
    let op_tooltip = if is_logic {
        op_typed.map_or("", LogicOp::truth_summary)
    } else {
        ""
    };
    let delay_label = timer_picker_label(btn.delay_timer());
    let press_label = timer_picker_label(btn.press_timer());
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
    let (phy, log) = pressed_bits(params, slot, btn.physical_num);
    let delay_enabled = typed.is_some_and(button_type_uses_delay_timer);
    let press_enabled = typed.is_some_and(button_type_uses_press_timer);
    ButtonRow {
        slot: i32::try_from(slot).unwrap_or(0),
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
        op_enabled: is_logic,
        src_b_enabled: is_logic,
        op_tooltip: SharedString::from(op_tooltip),
        delay_label: SharedString::from(delay_label),
        delay_index: i32::from(btn.delay_timer()),
        delay_enabled,
        press_label: SharedString::from(press_label),
        press_index: i32::from(btn.press_timer()),
        press_enabled,
        logic_error: SharedString::from(logic_error),
        phy_pressed: phy,
        log_pressed: log,
        physical_disarm_tick: disarm_ticks.0,
        src_b_disarm_tick: disarm_ticks.1,
    }
}

/// Whether the per-button Delay timer field is meaningful for this
/// button type. Used to grey out the picker on rows where it has no
/// effect (POV / Encoder inputs).
fn button_type_uses_delay_timer(t: ButtonType) -> bool {
    use ButtonType::{
        EncoderInputA, EncoderInputB, Pov1Center, Pov1Down, Pov1Left, Pov1Right, Pov1Up,
        Pov2Center, Pov2Down, Pov2Left, Pov2Right, Pov2Up, Pov3Center, Pov3Down, Pov3Left,
        Pov3Right, Pov3Up, Pov4Center, Pov4Down, Pov4Left, Pov4Right, Pov4Up,
    };
    !matches!(
        t,
        EncoderInputA
            | EncoderInputB
            | Pov1Up
            | Pov1Right
            | Pov1Down
            | Pov1Left
            | Pov1Center
            | Pov2Up
            | Pov2Right
            | Pov2Down
            | Pov2Left
            | Pov2Center
            | Pov3Up
            | Pov3Right
            | Pov3Down
            | Pov3Left
            | Pov3Center
            | Pov4Up
            | Pov4Right
            | Pov4Down
            | Pov4Left
            | Pov4Center
    )
}

/// Whether the per-button Press timer field is meaningful. Used by the
/// Tap / `DoubleTap` gestures and the basic press-and-hold paths;
/// greyed out elsewhere (POV / Encoder / Sequential / Radio / Toggle
/// Switches don't consume it).
fn button_type_uses_press_timer(t: ButtonType) -> bool {
    use ButtonType::{DoubleTap, Logic, Normal, Tap, Toggle};
    matches!(t, Normal | Toggle | Tap | DoubleTap | Logic)
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

fn pressed_bits(
    params: Option<&ParamsReport>,
    slot: usize,
    physical_num: i8,
) -> (bool, bool) {
    let Some(p) = params else {
        return (false, false);
    };
    // The amber `phy` dot reflects the device's raw physical-input
    // bitmap, indexed by the slot's configured `physical_num` — NOT by
    // the slot index. Wire format (params.rs): `phy_button_data` =
    // 128 bits, one per physical button; `log_button_data` = 128 bits,
    // one per logical slot. Off-by-one bugs here surfaced as "press
    // physical 20 → amber lights on row 21" because we previously
    // read `phy_button_data[slot]` instead of `[physical_num]`.
    let phy = if physical_num >= 0 {
        let phy_idx = physical_num as usize;
        let byte = phy_idx / 8;
        let bit = phy_idx % 8;
        let mask = 1u8 << bit;
        p.phy_button_data.get(byte).copied().unwrap_or(0) & mask != 0
    } else {
        false
    };
    let byte = slot / 8;
    let bit = slot % 8;
    let mask = 1u8 << bit;
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
/// Filters mean the model row count is no longer 128, so we look up
/// the visible row whose `slot` matches `wire_slot`. If the row isn't
/// in the visible model (slot was filtered out) we do nothing.
pub fn refresh_button_row(
    model: &Rc<VecModel<ButtonRow>>,
    wire_slot: usize,
    cfg: &DeviceConfig,
    params: Option<&ParamsReport>,
    disarm_ticks: (i32, i32),
) {
    let logic_errors = validate_logic_buttons(cfg);
    let row = build_button_row(
        wire_slot,
        &cfg.buttons[wire_slot],
        params,
        &logic_errors,
        disarm_ticks,
    );
    let wire_slot_i32 = i32::try_from(wire_slot).unwrap_or(0);
    for visible_idx in 0..model.row_count() {
        if let Some(existing) = model.row_data(visible_idx) {
            if existing.slot == wire_slot_i32 {
                model.set_row_data(visible_idx, row);
                return;
            }
        }
    }
}

/// Build the category-grouped Type-picker entries for the Buttons tab
/// (issue #15). Returns a flat list of [`DropdownEntry`] rows: one
/// `is_header: true` row per [`ButtonTypeCategory`] followed by its
/// entries in display order.
///
/// `blocked_context` (if `Some`) sets the per-physical coexistence
/// blocked flags on each entry for the supplied slot — used when a
/// Type cell opens so the user sees *why* a candidate is rejected.
/// Pass `None` for the initial startup build (all entries unblocked).
#[must_use]
pub fn build_button_type_entries(
    blocked_context: Option<(&[Button; MAX_BUTTONS_NUM], usize, i8)>,
) -> Vec<DropdownEntry> {
    let mut out = Vec::with_capacity(64);
    for cat in ButtonTypeCategory::all() {
        out.push(DropdownEntry {
            is_header: true,
            label: SharedString::from(cat.label()),
            value: -1,
            blocked: false,
            blocked_reason: SharedString::default(),
        });
        for bt in cat.entries() {
            let (blocked, reason) = match blocked_context {
                Some((buttons, slot, phy)) => {
                    match physical_assignment_blocked(buttons, slot, phy, *bt) {
                        CoexistenceCheck::Ok => (false, SharedString::default()),
                        CoexistenceCheck::Blocked { other_slot, other_type } => {
                            let other_label = ButtonType::from_u8(other_type).map_or_else(
                                || format!("? ({other_type})"),
                                |t| t.label().to_string(),
                            );
                            (
                                true,
                                SharedString::from(format!(
                                    "slot {} uses {other_label}",
                                    other_slot + 1
                                )),
                            )
                        }
                    }
                }
                None => (false, SharedString::default()),
            };
            out.push(DropdownEntry {
                is_header: false,
                label: SharedString::from(bt.label()),
                value: i32::from(bt.to_u8()),
                blocked,
                blocked_reason: reason,
            });
        }
    }
    out
}

/// Flat dropdown entries for the Buttons-tab Shift column. `0` means
/// "no shift modifier"; entries 1..=8 map to the eight shift slots.
#[must_use]
pub fn build_button_shift_entries() -> Vec<DropdownEntry> {
    let mut out = Vec::with_capacity(9);
    out.push(flat_entry("—", 0));
    for i in 1..=8 {
        out.push(flat_entry(&format!("Shift {i}"), i));
    }
    out
}

/// Flat dropdown entries for the LOGIC Op column. Labels come from
/// [`LogicOp::label`] so the picker and the selected-value cell
/// (`build_button_row::op_label`) stay in lockstep.
#[must_use]
pub fn build_button_op_entries() -> Vec<DropdownEntry> {
    [
        LogicOp::And,
        LogicOp::Or,
        LogicOp::Not,
        LogicOp::Nor,
        LogicOp::Nand,
        LogicOp::Xor,
        LogicOp::AAndNotB,
        LogicOp::Xnor,
    ]
    .iter()
    .map(|op| flat_entry(op.label(), i32::from(*op as u8)))
    .collect()
}

/// Flat dropdown entries for the per-button Delay timer and Press
/// timer columns. Mirrors the `timer_picker_label` shape used in the
/// row's value cell.
#[must_use]
pub fn build_button_timer_entries() -> Vec<DropdownEntry> {
    let mut out = Vec::with_capacity(4);
    out.push(flat_entry("—", 0));
    for i in 1..=3 {
        out.push(flat_entry(&format!("Timer {i}"), i));
    }
    out
}

/// Flat dropdown entries for the Buttons-tab filter strip's Type
/// category picker. Value -1 = "All", otherwise the
/// [`ButtonTypeCategory`] index inside [`ButtonTypeCategory::all`].
#[must_use]
pub fn build_filter_category_entries() -> Vec<DropdownEntry> {
    let mut out = Vec::with_capacity(12);
    out.push(flat_entry("All", -1));
    for (i, cat) in ButtonTypeCategory::all().enumerate() {
        out.push(flat_entry(
            cat.label(),
            i32::try_from(i).unwrap_or(0),
        ));
    }
    out
}

fn flat_entry(label: &str, value: i32) -> DropdownEntry {
    DropdownEntry {
        is_header: false,
        label: SharedString::from(label),
        value,
        blocked: false,
        blocked_reason: SharedString::default(),
    }
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

    wire_button_capture_callbacks(window, state, button_model);

    // Issue #5 filter callbacks. Each mutates State + rebuilds the
    // button model + pushes the UI mirrors so the strip's checkboxes /
    // labels reflect the current filter. (The Type filter cell goes
    // through the dropdown dispatch in `app::wire_dropdown_callbacks`.)
    wire_filter_callbacks(window, state, button_model);

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
        refresh_button_row(
            button_model,
            slot,
            cfg,
            s.last_params.as_ref(),
            s.button_capture.disarm_ticks(slot),
        );
    }
}

fn clamp_i8(v: i32) -> i8 {
    i8::try_from(v.clamp(i32::from(i8::MIN), i32::from(i8::MAX))).unwrap_or(0)
}

/// Rebuild the button model from current state + push the visible-count
/// mirror to the UI. Used by every filter callback below.
pub(crate) fn rebuild_filtered(
    state: &Rc<RefCell<crate::app::State>>,
    button_model: &Rc<VecModel<ButtonRow>>,
    window: &slint::Weak<AppWindow>,
) {
    let s = state.borrow();
    let Some(cfg) = s.last_config.as_ref() else {
        return;
    };
    let filter = crate::app::build_button_filter(&s);
    refresh_button_model(
        button_model,
        cfg,
        s.last_params.as_ref(),
        &filter,
        &s.button_capture,
    );
    let visible = i32::try_from(button_model.row_count()).unwrap_or(0);
    if let Some(w) = window.upgrade() {
        w.set_buttons_visible_count(visible);
        w.set_buttons_hide_unused(s.btn_hide_unused);
        w.set_buttons_filter_physical(s.btn_filter_physical.map_or(-1, i32::from));
        let label = s
            .btn_filter_category
            .and_then(|i| ButtonTypeCategory::all().nth(i))
            .map_or("All", ButtonTypeCategory::label);
        w.set_buttons_filter_category_label(SharedString::from(label));
    }
}

/// Wire the armed-changed callbacks on the Buttons-tab `physical_num`
/// and LOGIC `src_b` cells. Arming (click) snapshots the current
/// physical bitmap as a baseline so a button held at click time doesn't
/// auto-latch, and stashes a [`CaptureTarget`] in
/// `state.button_capture`. Disarming (second click, 5 s timeout, or
/// successful capture) clears it. The actual physical → field write
/// happens on the next `ParamsTick` via
/// [`ButtonCapture::on_params_tick`].
fn wire_button_capture_callbacks(
    window: &AppWindow,
    state: &Rc<RefCell<crate::app::State>>,
    button_model: &Rc<VecModel<ButtonRow>>,
) {
    {
        let s = state.clone();
        let m = button_model.clone();
        let w = window.as_weak();
        window.on_button_physical_armed_changed(move |slot, armed| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            // Arming a per-row Physical cell takes over capture; the
            // tab-level "press to filter" arm has to step aside so the
            // next press writes to the cell, not the filter.
            if armed {
                disarm_buttons_filter(&s, &w);
            }
            handle_arm_change(&s, &m, armed, CaptureTarget::Physical(slot));
        });
    }
    {
        let s = state.clone();
        let m = button_model.clone();
        window.on_button_src_b_armed_changed(move |slot, armed| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            handle_arm_change(&s, &m, armed, CaptureTarget::SrcB(slot));
        });
    }
}

/// Drop the buttons-tab press-to-filter arm. Idempotent — safe to call
/// when nothing is armed. Used by the tab-switch hook in app.slint and
/// by the per-row Physical cell arming path.
fn disarm_buttons_filter(
    state: &Rc<RefCell<crate::app::State>>,
    window: &slint::Weak<AppWindow>,
) {
    let was_armed = {
        let mut st = state.borrow_mut();
        let prev = st.btn_filter_arm;
        st.btn_filter_arm = false;
        prev
    };
    if was_armed {
        if let Some(win) = window.upgrade() {
            win.set_buttons_filter_arm(false);
        }
    }
}

/// Apply an arm or disarm signal from a `NumberCell`. Arming snapshots
/// the bitmap baseline (so a held button doesn't auto-latch) and
/// installs `cap` as the active capture target — `ButtonCapture::arm`
/// bumps the previous target's disarm tick so its cell drops out of
/// capture mode (rule c: "user selects another phys button row box").
/// Disarming clears the capture only if it still matches `cap`, so a
/// stale disarm from a timed-out cell doesn't stomp a freshly-armed
/// neighbour. Uses `clear()` (no bump) on the disarm path because the
/// cell already knows it's disarming.
fn handle_arm_change(
    state: &Rc<RefCell<crate::app::State>>,
    button_model: &Rc<VecModel<ButtonRow>>,
    armed: bool,
    cap: CaptureTarget,
) {
    let previous_slot = {
        let mut st = state.borrow_mut();
        if armed {
            if let Some(p) = st.last_params.as_ref() {
                st.last_phy_button_data = p.phy_button_data;
            }
            let prev = st.button_capture.armed();
            st.button_capture.arm(cap);
            match prev {
                Some(prev_cap) if prev_cap != cap => Some(prev_cap.slot()),
                _ => None,
            }
        } else {
            if st.button_capture.armed() == Some(cap) {
                st.button_capture.clear();
            }
            None
        }
    };
    if let Some(slot_to_refresh) = previous_slot {
        let s = state.borrow();
        if let Some(cfg) = s.last_config.as_ref() {
            refresh_button_row(
                button_model,
                slot_to_refresh,
                cfg,
                s.last_params.as_ref(),
                s.button_capture.disarm_ticks(slot_to_refresh),
            );
        }
    }
}

fn wire_filter_callbacks(
    window: &AppWindow,
    state: &Rc<RefCell<crate::app::State>>,
    button_model: &Rc<VecModel<ButtonRow>>,
) {
    {
        let s = state.clone();
        let m = button_model.clone();
        let w = window.as_weak();
        window.on_buttons_hide_unused_toggled(move || {
            {
                let mut st = s.borrow_mut();
                st.btn_hide_unused = !st.btn_hide_unused;
            }
            rebuild_filtered(&s, &m, &w);
        });
    }
    {
        let s = state.clone();
        let m = button_model.clone();
        let w = window.as_weak();
        window.on_buttons_filter_physical_edited(move |v| {
            {
                let mut st = s.borrow_mut();
                st.btn_filter_physical = if v < 0 {
                    None
                } else {
                    Some(clamp_i8(v))
                };
            }
            rebuild_filtered(&s, &m, &w);
        });
    }
    {
        let s = state.clone();
        let m = button_model.clone();
        let w = window.as_weak();
        window.on_buttons_filter_physical_cleared(move || {
            {
                let mut st = s.borrow_mut();
                st.btn_filter_physical = None;
            }
            rebuild_filtered(&s, &m, &w);
        });
    }
    {
        let s = state.clone();
        let w = window.as_weak();
        window.on_buttons_filter_arm_toggled(move || {
            let new_arm = {
                let mut st = s.borrow_mut();
                st.btn_filter_arm = !st.btn_filter_arm;
                st.btn_filter_arm
            };
            if let Some(win) = w.upgrade() {
                win.set_buttons_filter_arm(new_arm);
            }
        });
    }
    {
        let s = state.clone();
        let w = window.as_weak();
        window.on_buttons_filter_disarm(move || {
            disarm_buttons_filter(&s, &w);
        });
    }
    {
        let s = state.clone();
        let m = button_model.clone();
        let w = window.as_weak();
        window.on_buttons_add_clicked(move || {
            // Find the first wire slot not already shown (either via
            // assigned physical or already in force_shown). Promote it
            // to the visible list.
            let promoted = {
                let st = s.borrow();
                let Some(cfg) = st.last_config.as_ref() else {
                    return;
                };
                (0..MAX_BUTTONS_NUM).find(|i| {
                    !st.btn_force_shown.contains(i) && cfg.buttons[*i].physical_num < 0
                })
            };
            if let Some(idx) = promoted {
                s.borrow_mut().btn_force_shown.insert(idx);
                rebuild_filtered(&s, &m, &w);
            }
        });
    }
}
