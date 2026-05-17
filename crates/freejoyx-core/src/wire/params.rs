//! `params_report_t` wire codec.
//!
//! Layout (from `vendored/common_types.h` `params_report_t`, total = 72 bytes):
//!
//! ```text
//! offset  size  field
//!   0       2   firmware_version          u16 LE
//!   2       1   board_id                  u8
//!   3       1   reserved_layout           u8   (repurposed as FIRMWARE_BUILD_ID & 0xFF)
//!   4      16   raw_axis_data[8]          [i16 LE; 8]
//!  20      16   axis_data[8]              [i16 LE; 8]
//!  36      16   phy_button_data[16]       [u8; 16]   (128 bits, one per physical button)
//!  52      16   log_button_data[16]       [u8; 16]   (128 bits, one per logical button)
//!  68       1   shift_button_data         u8
//!  69       1   freejoyx_version_major    u8
//!  70       1   freejoyx_version_minor    u8
//!  71       1   freejoyx_version_patch    u8
//! ```
//!
//! Per the firmware (`FreeJoyX/application/Src/usb_app.c`), this 72-byte
//! struct is split across two 64-byte HID frames (62 bytes payload each).
//! The reassembly is done in `super::fragments::reassemble_two_fragment`.

use super::cursor::{Cursor, Writer};
use super::error::DecodeError;

/// `params_report_t` size in bytes, from `vendored/common_defines.h`.
pub const PARAMS_REPORT_SIZE: usize = 72;

/// Per-axis count from `vendored/common_defines.h::MAX_AXIS_NUM`.
pub const MAX_AXIS_NUM: usize = 8;

/// `MAX_BUTTONS_NUM / 8` from `vendored/common_defines.h::MAX_BUTTONS_NUM = 128`.
pub const BUTTON_BITMAP_BYTES: usize = 128 / 8;

/// Decoded `params_report_t`.
///
/// Field names match the C struct verbatim. Types are idiomatic Rust:
/// the C `int16_t` analog_data_t becomes `i16`, the C bit-packed
/// button arrays stay as `[u8; 16]` (consumers index by bit).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParamsReport {
    pub firmware_version: u16,
    pub board_id: u8,
    /// Repurposed as `FIRMWARE_BUILD_ID & 0xFF` per firmware
    /// `usb_app.c:391`. The field is named `reserved_layout` in the
    /// header for backwards-compat reasons; we expose it under that
    /// name and let callers interpret as a build id.
    pub reserved_layout: u8,
    pub raw_axis_data: [i16; MAX_AXIS_NUM],
    pub axis_data: [i16; MAX_AXIS_NUM],
    pub phy_button_data: [u8; BUTTON_BITMAP_BYTES],
    pub log_button_data: [u8; BUTTON_BITMAP_BYTES],
    pub shift_button_data: u8,
    pub freejoyx_version_major: u8,
    pub freejoyx_version_minor: u8,
    pub freejoyx_version_patch: u8,
}

impl ParamsReport {
    /// Decode a 72-byte assembled `params_report_t` payload.
    ///
    /// The caller is responsible for reassembling fragments first via
    /// `wire::fragments::reassemble_two_fragment`. This function takes
    /// the assembled bytes.
    pub fn decode(bytes: &[u8; PARAMS_REPORT_SIZE]) -> Result<Self, DecodeError> {
        let mut cur = Cursor::new(bytes);
        let firmware_version = cur.read_u16_le()?;
        let board_id = cur.read_u8()?;
        let reserved_layout = cur.read_u8()?;
        let raw_axis_data = read_i16_array::<MAX_AXIS_NUM>(&mut cur)?;
        let axis_data = read_i16_array::<MAX_AXIS_NUM>(&mut cur)?;
        let phy_button_data = cur.read_array::<BUTTON_BITMAP_BYTES>()?;
        let log_button_data = cur.read_array::<BUTTON_BITMAP_BYTES>()?;
        let shift_button_data = cur.read_u8()?;
        let freejoyx_version_major = cur.read_u8()?;
        let freejoyx_version_minor = cur.read_u8()?;
        let freejoyx_version_patch = cur.read_u8()?;
        debug_assert_eq!(cur.position(), PARAMS_REPORT_SIZE);
        Ok(Self {
            firmware_version,
            board_id,
            reserved_layout,
            raw_axis_data,
            axis_data,
            phy_button_data,
            log_button_data,
            shift_button_data,
            freejoyx_version_major,
            freejoyx_version_minor,
            freejoyx_version_patch,
        })
    }

    /// Encode this report back into 72 bytes (the assembled,
    /// pre-fragmentation form). Useful for tests and synthetic
    /// fixtures; the device-bound writer doesn't use this directly
    /// since the configurator only sends `REPORT_ID_PARAM` as a poll
    /// request, never as a payload.
    #[must_use]
    pub fn encode(&self) -> [u8; PARAMS_REPORT_SIZE] {
        let mut out = [0u8; PARAMS_REPORT_SIZE];
        let mut w = Writer::new(&mut out);
        w.write_u16_le(self.firmware_version);
        w.write_u8(self.board_id);
        w.write_u8(self.reserved_layout);
        for v in self.raw_axis_data {
            w.write_i16_le(v);
        }
        for v in self.axis_data {
            w.write_i16_le(v);
        }
        w.write_array(&self.phy_button_data);
        w.write_array(&self.log_button_data);
        w.write_u8(self.shift_button_data);
        w.write_u8(self.freejoyx_version_major);
        w.write_u8(self.freejoyx_version_minor);
        w.write_u8(self.freejoyx_version_patch);
        debug_assert_eq!(w.position(), PARAMS_REPORT_SIZE);
        out
    }
}

fn read_i16_array<const N: usize>(cur: &mut Cursor) -> Result<[i16; N], DecodeError> {
    let mut out = [0i16; N];
    for slot in &mut out {
        *slot = cur.read_i16_le()?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip with hand-built bytes proves encode/decode are
    /// inverses. Real fixture-driven tests live in
    /// `crates/freejoyx-core/tests/codec_params.rs`.
    #[test]
    fn roundtrip_zeros() {
        let zero = [0u8; PARAMS_REPORT_SIZE];
        let p = ParamsReport::decode(&zero).unwrap();
        assert_eq!(p.encode(), zero);
    }

    #[test]
    fn roundtrip_known_values() {
        let mut bytes = [0u8; PARAMS_REPORT_SIZE];
        // firmware_version = 0x0010 LE
        bytes[0] = 0x10;
        bytes[1] = 0x00;
        // board_id = 0x01
        bytes[2] = 0x01;
        // reserved_layout = 0x0f (build id 15)
        bytes[3] = 0x0f;
        // axis_data[0] = -1 (sign extension check)
        bytes[20] = 0xff;
        bytes[21] = 0xff;
        // phy_button_data[0] = 0x01 (button 0 pressed)
        bytes[36] = 0x01;
        // log_button_data[15] = 0x80 (button 127 logical-pressed)
        bytes[67] = 0x80;
        // shift_button_data = 0x04 (shift 2 active)
        bytes[68] = 0x04;
        // freejoyx_version = 0.1.2
        bytes[69] = 0;
        bytes[70] = 1;
        bytes[71] = 2;

        let p = ParamsReport::decode(&bytes).unwrap();
        assert_eq!(p.firmware_version, 0x0010);
        assert_eq!(p.board_id, 0x01);
        assert_eq!(p.reserved_layout, 0x0f);
        assert_eq!(p.axis_data[0], -1);
        assert_eq!(p.phy_button_data[0], 0x01);
        assert_eq!(p.log_button_data[15], 0x80);
        assert_eq!(p.shift_button_data, 0x04);
        assert_eq!(p.freejoyx_version_minor, 1);
        assert_eq!(p.freejoyx_version_patch, 2);

        assert_eq!(p.encode(), bytes);
    }
}
