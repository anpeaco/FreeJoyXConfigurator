//! Slice 8 glue: encoder + shift-register models + their callbacks.
//!
//! Same shape as [`crate::buttons`]: model refresh helpers + a single
//! `wire_callbacks` entry point. Each edit funnels through a small
//! mutation closure, refreshes the touched row, and calls
//! [`crate::buttons::mark_dirty`] so the toolbar surfaces the unsaved
//! change.

use std::cell::RefCell;
use std::rc::Rc;

use freejoyx_core::domain::{pair_soft_encoders, EncoderMode, ShiftRegType, SoftEncoderPair};
use freejoyx_core::wire::{
    DeviceConfig, ParamsReport, MAX_ENCODERS_NUM, MAX_FAST_ENCODER_NUM, MAX_SHIFT_REG_NUM,
};
use slint::{ComponentHandle, Model, SharedString, VecModel};

use crate::tabs::buttons::mark_dirty;
use crate::{AppWindow, EncoderRow, FastEncoderRow, ShiftRegRow};

const FAST_ENCODER_HINTS: [&str; MAX_FAST_ENCODER_NUM] = [
    "silicon-locked to PA8 (A) / PA9 (B) via TIM1",
    "silicon-locked to PB6 (A) / PB7 (B) via TIM4",
];

/// Does any soft or fast encoder slot carry a non-default value? Drives
/// the Encoders tab strip indicator dot.
#[must_use]
pub fn encoders_has_content(cfg: &DeviceConfig) -> bool {
    cfg.encoders.iter().any(|&v| v != 0)
        || cfg.fast_encoders.iter().any(|fe| fe.enabled != 0)
}

/// Does any shift-register slot carry a non-default reg-type? Drives
/// the Shift Registers tab strip indicator dot.
#[must_use]
pub fn shift_regs_has_content(cfg: &DeviceConfig) -> bool {
    cfg.shift_registers.iter().any(|sr| sr.reg_type != 0)
}

/// Number of soft-encoder rows the UI surfaces — slots
/// `MAX_FAST_ENCODER_NUM..MAX_ENCODERS_NUM` of `encoders[]`. The first
/// two are fast-encoder territory (Fast 1/2 use `fast_encoders[i].mode`
/// directly, so the matching `encoders[0..1]` cells are vestigial in
/// the firmware) and aren't rendered here — they'd just be dead rows.
/// Matches `MAX_ENCODERS_NUM - MAX_FAST_ENCODER_NUM` in the Qt
/// configurator's `EncodersConfig::EncodersConfig` ctor.
pub const SOFT_ENCODER_ROWS: usize = MAX_ENCODERS_NUM - MAX_FAST_ENCODER_NUM;

pub fn refresh_soft_encoder_model(
    model: &Rc<VecModel<EncoderRow>>,
    cfg: &DeviceConfig,
    params: Option<&ParamsReport>,
) {
    let pairs = pair_soft_encoders(&cfg.buttons);
    if model.row_count() != SOFT_ENCODER_ROWS {
        while model.row_count() > 0 {
            model.remove(0);
        }
        for row in 0..SOFT_ENCODER_ROWS {
            let wire_slot = row + MAX_FAST_ENCODER_NUM;
            model.push(build_soft_encoder_row(
                row,
                wire_slot,
                cfg.encoders[wire_slot],
                pairs[wire_slot],
                params,
            ));
        }
        return;
    }
    for row in 0..SOFT_ENCODER_ROWS {
        let wire_slot = row + MAX_FAST_ENCODER_NUM;
        model.set_row_data(
            row,
            build_soft_encoder_row(
                row,
                wire_slot,
                cfg.encoders[wire_slot],
                pairs[wire_slot],
                params,
            ),
        );
    }
}

pub(crate) fn build_soft_encoder_row(
    row: usize,
    wire_slot: usize,
    raw_mode: u8,
    pair: Option<SoftEncoderPair>,
    params: Option<&ParamsReport>,
) -> EncoderRow {
    let mode_label = EncoderMode::from_u8(raw_mode)
        .map_or_else(|| format!("?{raw_mode}"), |m| m.label().to_string());
    let (paired, pair_label, cw_active, ccw_active) = match pair {
        Some(p) => {
            let label = format!("btn {} \u{2194} btn {}", p.a_button + 1, p.b_button + 1);
            let (cw, ccw) = params.map_or((false, false), |pr| {
                (
                    log_button_bit(pr, p.a_button as usize),
                    log_button_bit(pr, p.b_button as usize),
                )
            });
            (true, label, cw, ccw)
        }
        None => (false, String::new(), false, false),
    };
    EncoderRow {
        label: SharedString::from(format!("Encoder {}", row + 1)),
        mode_label: SharedString::from(mode_label),
        mode_index: i32::from(raw_mode),
        wire_slot: i32::try_from(wire_slot).unwrap_or(0),
        paired,
        pair_label: SharedString::from(pair_label),
        cw_active,
        ccw_active,
    }
}

/// Read the logical-button bit for `slot` out of the params report.
/// Encoder CW/CCW pulses are firmware-emitted virtual presses on the
/// A/B button slots (held for `encoder_press_time_ms`).
fn log_button_bit(p: &ParamsReport, slot: usize) -> bool {
    let byte = slot / 8;
    let bit = slot % 8;
    let mask = 1u8 << bit;
    p.log_button_data.get(byte).copied().unwrap_or(0) & mask != 0
}

pub fn refresh_fast_encoder_model(model: &Rc<VecModel<FastEncoderRow>>, cfg: &DeviceConfig) {
    if model.row_count() != MAX_FAST_ENCODER_NUM {
        while model.row_count() > 0 {
            model.remove(0);
        }
        for slot in 0..MAX_FAST_ENCODER_NUM {
            model.push(build_fast_encoder_row(slot, &cfg.fast_encoders[slot]));
        }
        return;
    }
    for slot in 0..MAX_FAST_ENCODER_NUM {
        model.set_row_data(slot, build_fast_encoder_row(slot, &cfg.fast_encoders[slot]));
    }
}

pub(crate) fn build_fast_encoder_row(slot: usize, fe: &freejoyx_core::wire::FastEncoder) -> FastEncoderRow {
    let mode_label = EncoderMode::from_u8(fe.mode)
        .map_or_else(|| format!("?{}", fe.mode), |m| m.label().to_string());
    FastEncoderRow {
        label: SharedString::from(format!("Fast {}", slot + 1)),
        enabled: fe.enabled != 0,
        mode_label: SharedString::from(mode_label),
        mode_index: i32::from(fe.mode),
        hint: SharedString::from(FAST_ENCODER_HINTS[slot]),
    }
}

pub fn refresh_shift_reg_model(
    model: &Rc<VecModel<ShiftRegRow>>,
    cfg: &DeviceConfig,
    chip_size: [u8; MAX_SHIFT_REG_NUM],
) {
    if model.row_count() != MAX_SHIFT_REG_NUM {
        while model.row_count() > 0 {
            model.remove(0);
        }
        for slot in 0..MAX_SHIFT_REG_NUM {
            model.push(build_shift_reg_row(slot, cfg, chip_size[slot]));
        }
        return;
    }
    for slot in 0..MAX_SHIFT_REG_NUM {
        model.set_row_data(slot, build_shift_reg_row(slot, cfg, chip_size[slot]));
    }
}

pub(crate) fn build_shift_reg_row(
    slot: usize,
    cfg: &DeviceConfig,
    chip_size: u8,
) -> ShiftRegRow {
    let sr = &cfg.shift_registers[slot];
    let type_label = ShiftRegType::from_u8(sr.reg_type)
        .map_or_else(|| format!("?{}", sr.reg_type), |t| t.label().to_string());
    // Back-to-pin is enabled when the firmware-side ordering produces
    // a Data pin for this SR slot — i.e. the Nth ShiftRegData pin
    // exists in `pins[]`. Pin assignments live in the Pins tab.
    let back_to_pin_enabled = crate::tabs::pins::shift_reg_back_to_pin(cfg, slot).is_some();
    let bpc = chip_size.max(1);
    let num_chips = sr.button_cnt / bpc;
    ShiftRegRow {
        label: SharedString::from(format!("SR {}", slot + 1)),
        type_label: SharedString::from(type_label),
        type_index: i32::from(sr.reg_type),
        button_count: i32::from(sr.button_cnt),
        buttons_per_chip: i32::from(bpc),
        num_chips: i32::from(num_chips),
        back_to_pin_enabled,
    }
}

#[allow(clippy::too_many_lines)]
pub fn wire_callbacks(
    window: &AppWindow,
    state: &Rc<RefCell<crate::app::State>>,
    soft_model: &Rc<VecModel<EncoderRow>>,
    fast_model: &Rc<VecModel<FastEncoderRow>>,
    button_model: &Rc<VecModel<crate::ButtonRow>>,
    shift_reg_model: &Rc<VecModel<ShiftRegRow>>,
) {
    // (Soft encoder mode picks dispatched through `app::wire_dropdown_callbacks`.)

    // Swap the physical-input numbers on the paired A/B slots so the
    // encoder's CW/CCW direction inverts without rewiring the hardware.
    // The slot roles (A stays A, B stays B) and the encoder's wire-slot
    // index are unchanged — only which physical pin each role reads.
    {
        let s = state.clone();
        let soft = soft_model.clone();
        let buttons = button_model.clone();
        let w = window.as_weak();
        window.on_encoder_swap_pair_clicked(move |wire_slot| {
            let Ok(wire_slot) = usize::try_from(wire_slot) else {
                return;
            };
            if wire_slot >= MAX_ENCODERS_NUM {
                return;
            }
            {
                let mut st = s.borrow_mut();
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                let pairs = pair_soft_encoders(&cfg.buttons);
                let Some(pair) = pairs[wire_slot] else {
                    return;
                };
                let a_idx = pair.a_button as usize;
                let b_idx = pair.b_button as usize;
                let a_phys = cfg.buttons[a_idx].physical_num;
                cfg.buttons[a_idx].physical_num = cfg.buttons[b_idx].physical_num;
                cfg.buttons[b_idx].physical_num = a_phys;
            }
            // Refresh both the encoder and the button models so the
            // type column on the Buttons tab tracks the change.
            let st = s.borrow();
            if let Some(cfg) = st.last_config.as_ref() {
                let pairs = pair_soft_encoders(&cfg.buttons);
                for row in 0..SOFT_ENCODER_ROWS {
                    let ws = row + MAX_FAST_ENCODER_NUM;
                    soft.set_row_data(
                        row,
                        build_soft_encoder_row(
                            row,
                            ws,
                            cfg.encoders[ws],
                            pairs[ws],
                            st.last_params.as_ref(),
                        ),
                    );
                }
                let filter = crate::app::build_button_filter(&st);
                crate::tabs::buttons::refresh_button_model(
                    &buttons,
                    cfg,
                    st.last_params.as_ref(),
                    &filter,
                    &st.button_capture,
                );
            }
            drop(st);
            mark_dirty(&w);
        });
    }

    // Fast encoder enabled toggle.
    {
        let s = state.clone();
        let m = fast_model.clone();
        let w = window.as_weak();
        window.on_fast_encoder_enabled_toggled(move |slot| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            if slot >= MAX_FAST_ENCODER_NUM {
                return;
            }
            {
                let mut st = s.borrow_mut();
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                let fe = &mut cfg.fast_encoders[slot];
                fe.enabled = u8::from(fe.enabled == 0);
            }
            if let Some(cfg) = s.borrow().last_config.as_ref() {
                m.set_row_data(slot, build_fast_encoder_row(slot, &cfg.fast_encoders[slot]));
            }
            mark_dirty(&w);
        });
    }

    // (Fast encoder mode + shift-reg type picks dispatched through
    // `app::wire_dropdown_callbacks`.)

    // Soft encoder → Buttons jump. Flashes the paired A/B button slots
    // on the Buttons tab and scrolls the A slot into view. Mirrors the
    // pin-jump pattern (see `app::wire_pin_jump_callback`). Force-shows
    // the two slots so the user's current filter doesn't hide them.
    {
        let s = state.clone();
        let buttons = button_model.clone();
        let w = window.as_weak();
        window.on_encoder_jump_to_buttons_clicked(move |wire_slot| {
            let Ok(wire_slot) = usize::try_from(wire_slot) else {
                return;
            };
            if wire_slot >= MAX_ENCODERS_NUM {
                return;
            }
            let pair = {
                let st = s.borrow();
                let Some(cfg) = st.last_config.as_ref() else {
                    return;
                };
                pair_soft_encoders(&cfg.buttons)[wire_slot]
            };
            let Some(pair) = pair else { return };
            let a_slot = pair.a_button as usize;
            let b_slot = pair.b_button as usize;
            s.borrow_mut().btn_force_shown.insert(a_slot);
            s.borrow_mut().btn_force_shown.insert(b_slot);
            crate::tabs::buttons::rebuild_filtered(&s, &buttons, &w);
            let row_index = (0..buttons.row_count()).find(|i| {
                buttons
                    .row_data(*i)
                    .is_some_and(|r| usize::try_from(r.slot).unwrap_or(usize::MAX) == a_slot)
            });
            let Some(win) = w.upgrade() else { return };
            win.set_active_tab(2);
            win.set_flash_axis_slot(-1);
            win.set_flash_shift_reg_slot(-1);
            win.set_flash_pin_slot(-1);
            win.set_flash_button_slot_a(i32::try_from(a_slot).unwrap_or(-1));
            win.set_flash_button_slot_b(i32::try_from(b_slot).unwrap_or(-1));
            if let Some(idx) = row_index {
                #[allow(clippy::cast_precision_loss)]
                let y = (idx as f32) * 38.0;
                win.set_buttons_jump_y(y);
                win.set_buttons_jump_tick(win.get_buttons_jump_tick().wrapping_add(1));
            }
            crate::app::schedule_flash_clear(&s, &w);
        });
    }

    // Shift register button count edit. Snaps the typed total up to
    // the next multiple of the row's `buttons_per_chip` so the three
    // spin cells stay consistent (bpc × nc = total). nc is implicit —
    // recomputed on display from `button_cnt / bpc`.
    {
        let s = state.clone();
        let m = shift_reg_model.clone();
        let w = window.as_weak();
        window.on_shift_reg_count_edited(move |slot, v| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            if slot >= MAX_SHIFT_REG_NUM {
                return;
            }
            let typed = u8::try_from(v.clamp(0, i32::from(u8::MAX))).unwrap_or(0);
            {
                let mut st = s.borrow_mut();
                let bpc = st.shift_reg_chip_size[slot].max(1);
                let snapped = snap_total_to_chip_multiple(typed, bpc);
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                cfg.shift_registers[slot].button_cnt = snapped;
            }
            refresh_one_shift_reg_row(&s, &m, slot);
            mark_dirty(&w);
        });
    }

    // "Per chip" spin: change buttons-per-chip, keep the displayed
    // num_chips constant, recompute total = bpc * nc. If the new bpc
    // would push total above u8::MAX, num_chips gets clamped down.
    {
        let s = state.clone();
        let m = shift_reg_model.clone();
        let w = window.as_weak();
        window.on_shift_reg_chip_size_edited(move |slot, v| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            if slot >= MAX_SHIFT_REG_NUM {
                return;
            }
            let new_bpc = u8::try_from(v.clamp(1, 32)).unwrap_or(1).max(1);
            {
                let mut st = s.borrow_mut();
                let old_bpc = st.shift_reg_chip_size[slot].max(1);
                let cfg = match st.last_config.as_mut() {
                    Some(c) => c,
                    None => {
                        st.shift_reg_chip_size[slot] = new_bpc;
                        drop(st);
                        refresh_one_shift_reg_row(&s, &m, slot);
                        return;
                    }
                };
                let nc = cfg.shift_registers[slot].button_cnt / old_bpc;
                let new_total = u16::from(new_bpc) * u16::from(nc);
                cfg.shift_registers[slot].button_cnt =
                    u8::try_from(new_total).unwrap_or(u8::MAX);
                st.shift_reg_chip_size[slot] = new_bpc;
            }
            refresh_one_shift_reg_row(&s, &m, slot);
            mark_dirty(&w);
        });
    }

    // "Chips" spin: change num_chips, keep buttons-per-chip constant,
    // recompute total = bpc * nc.
    {
        let s = state.clone();
        let m = shift_reg_model.clone();
        let w = window.as_weak();
        window.on_shift_reg_chip_count_edited(move |slot, v| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            if slot >= MAX_SHIFT_REG_NUM {
                return;
            }
            let new_nc = u8::try_from(v.clamp(0, i32::from(u8::MAX))).unwrap_or(0);
            {
                let mut st = s.borrow_mut();
                let bpc = st.shift_reg_chip_size[slot].max(1);
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                let new_total = u16::from(bpc) * u16::from(new_nc);
                cfg.shift_registers[slot].button_cnt =
                    u8::try_from(new_total).unwrap_or(u8::MAX);
            }
            refresh_one_shift_reg_row(&s, &m, slot);
            mark_dirty(&w);
        });
    }
}

/// Snap a typed total up to the nearest multiple of `bpc`. The three
/// shift-register spin cells display `bpc × nc = total`; on a direct
/// total edit we walk up so the user's "I want at least N buttons"
/// intent is preserved (typing 10 with bpc=8 → 16, not 8).
fn snap_total_to_chip_multiple(total: u8, bpc: u8) -> u8 {
    let bpc = bpc.max(1);
    if total == 0 {
        return 0;
    }
    let chips = total.div_ceil(bpc);
    u8::try_from(u16::from(chips) * u16::from(bpc)).unwrap_or(u8::MAX)
}

fn refresh_one_shift_reg_row(
    state: &Rc<RefCell<crate::app::State>>,
    model: &Rc<VecModel<ShiftRegRow>>,
    slot: usize,
) {
    let st = state.borrow();
    let Some(cfg) = st.last_config.as_ref() else {
        return;
    };
    let bpc = st.shift_reg_chip_size[slot];
    model.set_row_data(slot, build_shift_reg_row(slot, cfg, bpc));
}
