//! Slice 8 glue: encoder + shift-register models + their callbacks.
//!
//! Same shape as [`crate::buttons`]: model refresh helpers + a single
//! `wire_callbacks` entry point. Each edit funnels through a small
//! mutation closure, refreshes the touched row, and calls
//! [`crate::buttons::mark_dirty`] so the toolbar surfaces the unsaved
//! change.

use std::cell::RefCell;
use std::rc::Rc;

use freejoyx_core::domain::{EncoderMode, ShiftRegType};
use freejoyx_core::wire::{
    DeviceConfig, MAX_ENCODERS_NUM, MAX_FAST_ENCODER_NUM, MAX_SHIFT_REG_NUM,
};
use slint::{ComponentHandle, Model, SharedString, VecModel};

use crate::buttons::mark_dirty;
use crate::{AppWindow, EncoderRow, FastEncoderRow, ShiftRegRow};

const FAST_ENCODER_HINTS: [&str; MAX_FAST_ENCODER_NUM] = [
    "silicon-locked to PA8 (A) / PA9 (B) via TIM1",
    "silicon-locked to PB6 (A) / PB7 (B) via TIM4",
];

pub fn refresh_soft_encoder_model(model: &Rc<VecModel<EncoderRow>>, cfg: &DeviceConfig) {
    if model.row_count() != MAX_ENCODERS_NUM {
        while model.row_count() > 0 {
            model.remove(0);
        }
        for slot in 0..MAX_ENCODERS_NUM {
            model.push(build_soft_encoder_row(slot, cfg.encoders[slot]));
        }
        return;
    }
    for slot in 0..MAX_ENCODERS_NUM {
        model.set_row_data(slot, build_soft_encoder_row(slot, cfg.encoders[slot]));
    }
}

fn build_soft_encoder_row(slot: usize, raw_mode: u8) -> EncoderRow {
    let mode_label = EncoderMode::from_u8(raw_mode)
        .map_or_else(|| format!("?{raw_mode}"), |m| m.label().to_string());
    EncoderRow {
        label: SharedString::from(format!("Encoder {}", slot + 1)),
        mode_label: SharedString::from(mode_label),
        mode_index: i32::from(raw_mode),
    }
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

fn build_fast_encoder_row(slot: usize, fe: &freejoyx_core::wire::FastEncoder) -> FastEncoderRow {
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

pub fn refresh_shift_reg_model(model: &Rc<VecModel<ShiftRegRow>>, cfg: &DeviceConfig) {
    if model.row_count() != MAX_SHIFT_REG_NUM {
        while model.row_count() > 0 {
            model.remove(0);
        }
        for slot in 0..MAX_SHIFT_REG_NUM {
            model.push(build_shift_reg_row(slot, &cfg.shift_registers[slot]));
        }
        return;
    }
    for slot in 0..MAX_SHIFT_REG_NUM {
        model.set_row_data(slot, build_shift_reg_row(slot, &cfg.shift_registers[slot]));
    }
}

fn build_shift_reg_row(slot: usize, sr: &freejoyx_core::wire::ShiftRegConfig) -> ShiftRegRow {
    let type_label = ShiftRegType::from_u8(sr.reg_type)
        .map_or_else(|| format!("?{}", sr.reg_type), |t| t.label().to_string());
    ShiftRegRow {
        label: SharedString::from(format!("SR {}", slot + 1)),
        type_label: SharedString::from(type_label),
        type_index: i32::from(sr.reg_type),
        button_count: i32::from(sr.button_cnt),
    }
}

#[allow(clippy::too_many_lines)]
pub fn wire_callbacks(
    window: &AppWindow,
    state: &Rc<RefCell<crate::app::State>>,
    soft_model: &Rc<VecModel<EncoderRow>>,
    fast_model: &Rc<VecModel<FastEncoderRow>>,
    shift_reg_model: &Rc<VecModel<ShiftRegRow>>,
) {
    // Soft encoder mode pick.
    {
        let s = state.clone();
        let m = soft_model.clone();
        let w = window.as_weak();
        window.on_soft_encoder_mode_picked(move |slot, value| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            if slot >= MAX_ENCODERS_NUM {
                return;
            }
            let Ok(mode) = u8::try_from(value.clamp(0, 2)) else {
                return;
            };
            {
                let mut st = s.borrow_mut();
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                cfg.encoders[slot] = mode;
            }
            if let Some(cfg) = s.borrow().last_config.as_ref() {
                m.set_row_data(slot, build_soft_encoder_row(slot, cfg.encoders[slot]));
            }
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

    // Fast encoder mode pick.
    {
        let s = state.clone();
        let m = fast_model.clone();
        let w = window.as_weak();
        window.on_fast_encoder_mode_picked(move |slot, value| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            if slot >= MAX_FAST_ENCODER_NUM {
                return;
            }
            let Ok(mode) = u8::try_from(value.clamp(0, 2)) else {
                return;
            };
            {
                let mut st = s.borrow_mut();
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                cfg.fast_encoders[slot].mode = mode;
            }
            if let Some(cfg) = s.borrow().last_config.as_ref() {
                m.set_row_data(slot, build_fast_encoder_row(slot, &cfg.fast_encoders[slot]));
            }
            mark_dirty(&w);
        });
    }

    // Shift register type pick.
    {
        let s = state.clone();
        let m = shift_reg_model.clone();
        let w = window.as_weak();
        window.on_shift_reg_type_picked(move |slot, value| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            if slot >= MAX_SHIFT_REG_NUM {
                return;
            }
            let Ok(reg_type) = u8::try_from(value.clamp(0, 3)) else {
                return;
            };
            {
                let mut st = s.borrow_mut();
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                cfg.shift_registers[slot].reg_type = reg_type;
            }
            if let Some(cfg) = s.borrow().last_config.as_ref() {
                m.set_row_data(slot, build_shift_reg_row(slot, &cfg.shift_registers[slot]));
            }
            mark_dirty(&w);
        });
    }

    // Shift register button count edit.
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
            let clamped = u8::try_from(v.clamp(0, i32::from(u8::MAX))).unwrap_or(0);
            {
                let mut st = s.borrow_mut();
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                cfg.shift_registers[slot].button_cnt = clamped;
            }
            if let Some(cfg) = s.borrow().last_config.as_ref() {
                m.set_row_data(slot, build_shift_reg_row(slot, &cfg.shift_registers[slot]));
            }
            mark_dirty(&w);
        });
    }
}
