//! Slice 9 glue: Advanced Settings tab + toolbar device picker.
//!
//! Two surfaces share this module because they both rotate around the
//! `DeviceCandidate` list and the persisted-on-device identification
//! fields (`device_name`, `vid`, `pid`, `firmware_version`, `board_id`).

use std::cell::RefCell;
use std::rc::Rc;

use freejoyx_core::wire::DeviceConfig;
use freejoyx_device::{Command, DeviceCandidate};
use slint::{ComponentHandle, Model, SharedString, VecModel};

use crate::buttons::mark_dirty;
use crate::{AdvancedModel, AppWindow, DeviceOption};

/// Build the Slint advanced-tab model from the held config. Called on
/// every `ConfigReceived` / Load and after each edit.
#[must_use]
pub fn build_advanced_model(cfg: &DeviceConfig) -> AdvancedModel {
    AdvancedModel {
        device_name: SharedString::from(device_name_to_string(&cfg.device_name)),
        vid_hex: SharedString::from(format!("{:04x}", cfg.vid)),
        pid_hex: SharedString::from(format!("{:04x}", cfg.pid)),
        firmware_version_hex: SharedString::from(format!("0x{:04x}", cfg.firmware_version)),
        board_id: i32::from(cfg.board_id),
        freejoyx_version: SharedString::from(String::new()),
    }
}

/// Empty advanced model used until a config is loaded. Keeps the
/// `TextInput`s visibly blank rather than displaying stale defaults.
#[must_use]
pub fn empty_advanced_model() -> AdvancedModel {
    AdvancedModel {
        device_name: SharedString::from(""),
        vid_hex: SharedString::from(""),
        pid_hex: SharedString::from(""),
        firmware_version_hex: SharedString::from("(no config)"),
        board_id: 0,
        freejoyx_version: SharedString::from(""),
    }
}

/// Update only the `freejoyx_version` line on the current advanced
/// model. Called on `ParamsTick` so the user sees the build version
/// the device reports without waiting for a Read.
pub fn merge_params_into_advanced(
    current: &AdvancedModel,
    major: u8,
    minor: u8,
    patch: u8,
) -> AdvancedModel {
    AdvancedModel {
        device_name: current.device_name.clone(),
        vid_hex: current.vid_hex.clone(),
        pid_hex: current.pid_hex.clone(),
        firmware_version_hex: current.firmware_version_hex.clone(),
        board_id: current.board_id,
        freejoyx_version: SharedString::from(format!("{major}.{minor}.{patch}")),
    }
}

fn device_name_to_string(bytes: &[u8; 26]) -> String {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).into_owned()
}

/// Pack a user-typed name back into the 26-byte slot. Truncates at 25
/// chars to leave room for the NUL terminator; any byte past `end` is
/// zeroed so trailing garbage from a previous longer name doesn't
/// linger on the wire.
pub fn pack_device_name(input: &str) -> [u8; 26] {
    let mut out = [0u8; 26];
    let truncated = input.as_bytes();
    let len = truncated.len().min(25);
    out[..len].copy_from_slice(&truncated[..len]);
    out
}

/// Build the device-picker dropdown model from the worker's last
/// `Candidates` event + the currently-connected serial (so the row
/// renders an accent dot).
#[must_use]
pub fn build_candidates_model(
    candidates: &[DeviceCandidate],
    current_serial: Option<&str>,
) -> Vec<DeviceOption> {
    candidates
        .iter()
        .map(|c| {
            let serial = c.serial_number.clone().unwrap_or_default();
            let is_current = current_serial.is_some_and(|cur| cur.eq_ignore_ascii_case(&serial));
            DeviceOption {
                label: SharedString::from(c.display_summary()),
                serial: SharedString::from(serial),
                is_current,
            }
        })
        .collect()
}

pub fn refresh_candidates_model(
    model: &Rc<VecModel<DeviceOption>>,
    candidates: &[DeviceCandidate],
    current_serial: Option<&str>,
) {
    let rows = build_candidates_model(candidates, current_serial);
    while model.row_count() > 0 {
        model.remove(0);
    }
    for row in rows {
        model.push(row);
    }
}

/// Wire all Advanced-tab and device-picker callbacks. Edits to
/// `device_name` / `vid` / `pid` mutate the held config and mark it
/// dirty; the picker callbacks talk to the worker over the command
/// channel.
pub fn wire_callbacks(
    window: &AppWindow,
    state: &Rc<RefCell<crate::app::State>>,
    candidates_model: &Rc<VecModel<DeviceOption>>,
) {
    // Device name.
    {
        let s = state.clone();
        let w = window.as_weak();
        window.on_advanced_name_edited(move |new_name| {
            {
                let mut st = s.borrow_mut();
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                cfg.device_name = pack_device_name(new_name.as_str());
            }
            mark_dirty(&w);
        });
    }

    // VID.
    {
        let s = state.clone();
        let w = window.as_weak();
        window.on_advanced_vid_edited(move |new_vid| {
            if let Some(v) = parse_hex_u16(new_vid.as_str()) {
                let mut st = s.borrow_mut();
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                cfg.vid = v;
                mark_dirty(&w);
            }
        });
    }

    // PID.
    {
        let s = state.clone();
        let w = window.as_weak();
        window.on_advanced_pid_edited(move |new_pid| {
            if let Some(v) = parse_hex_u16(new_pid.as_str()) {
                let mut st = s.borrow_mut();
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                cfg.pid = v;
                mark_dirty(&w);
            }
        });
    }

    // Refresh devices.
    {
        let s = state.clone();
        window.on_refresh_devices(move || {
            if let Err(e) = s.borrow().handle_send(Command::Enumerate) {
                tracing::warn!("enumerate send failed: {e}");
            }
        });
    }

    // Pick device.
    {
        let s = state.clone();
        let m = candidates_model.clone();
        window.on_device_picked(move |serial| {
            let serial_owned = if serial.is_empty() {
                None
            } else {
                Some(serial.as_str().to_string())
            };
            let st = s.borrow();
            if let Err(e) = st.handle_send(Command::Reopen {
                serial: serial_owned.clone(),
            }) {
                tracing::warn!("reopen send failed: {e}");
                return;
            }
            // Optimistic re-render: mark the picked entry as current
            // so the dot moves immediately. The next `Candidates`
            // event will overwrite with authoritative state.
            let rows: Vec<DeviceOption> = (0..m.row_count())
                .filter_map(|i| m.row_data(i))
                .map(|opt| DeviceOption {
                    label: opt.label,
                    serial: opt.serial.clone(),
                    is_current: serial_owned.as_deref() == Some(opt.serial.as_str()),
                })
                .collect();
            while m.row_count() > 0 {
                m.remove(0);
            }
            for r in rows {
                m.push(r);
            }
        });
    }
}

fn parse_hex_u16(input: &str) -> Option<u16> {
    let trimmed = input
        .trim()
        .trim_start_matches("0x")
        .trim_start_matches("0X");
    if trimmed.is_empty() {
        return None;
    }
    u16::from_str_radix(trimmed, 16).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_device_name_truncates_and_zeroes_tail() {
        let bytes = pack_device_name("FreeJoyX 0.0.2");
        assert_eq!(&bytes[..14], b"FreeJoyX 0.0.2");
        assert_eq!(bytes[14], 0);

        // Overlong name truncates at 25 bytes (room for NUL).
        let long = "A".repeat(40);
        let packed = pack_device_name(&long);
        assert_eq!(&packed[..25], &[b'A'; 25]);
        assert_eq!(packed[25], 0);
    }

    #[test]
    fn parse_hex_u16_handles_common_forms() {
        assert_eq!(parse_hex_u16("0483"), Some(0x0483));
        assert_eq!(parse_hex_u16("0x0483"), Some(0x0483));
        assert_eq!(parse_hex_u16("FFFF"), Some(0xffff));
        assert_eq!(parse_hex_u16("  ABCD  "), Some(0xabcd));
        assert_eq!(parse_hex_u16(""), None);
        assert_eq!(parse_hex_u16("zzz"), None);
        assert_eq!(parse_hex_u16("10000"), None);
    }
}
