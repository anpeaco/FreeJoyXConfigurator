//! `params_report_t` wire codec.
//!
//! Base layout (from `vendored/common_types.h` `params_report_t`, the
//! legacy size = 72 bytes):
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
//! Firmware **v0.1.3** appended one field (params-report-only, prefix-
//! compatible — `dev_config_t` is untouched, so no factory reset), growing
//! the report to 88 bytes:
//!
//! ```text
//!  72      16   detect_axis_raw[8]        [i16 LE; 8]   (axis auto-detect)
//! ```
//!
//! Because the field is appended, a 0.1.3+ report is a strict superset of
//! the legacy one. We always decode the 72-byte prefix and only interpret
//! the extra 16 bytes when (a) they're present and (b) the reporting
//! firmware is >= 0.1.3 — older firmware leaves that region of the second
//! HID frame uninitialised, so reading it as a field would be garbage.
//!
//! Per the firmware (`FreeJoyX/application/Src/usb_app.c`), the struct is
//! split across two 64-byte HID frames (62 bytes payload each); both the
//! 72- and 88-byte forms fit in two frames. Reassembly is done in
//! `super::fragments::reassemble_two_fragment`.

use super::cursor::{Cursor, Writer};
use super::error::DecodeError;

/// Full `params_report_t` size in bytes (firmware >= 0.1.3), from
/// `vendored/common_defines.h::FREEJOY_PARAMS_REPORT_SIZE`.
pub const PARAMS_REPORT_SIZE: usize = 88;

/// Legacy `params_report_t` size (firmware < 0.1.3, before `detect_axis_raw`
/// was appended). Reports this short decode fine; `detect_axis_raw` is `None`.
pub const PARAMS_REPORT_LEGACY_SIZE: usize = 72;

/// Per-axis count from `vendored/common_defines.h::MAX_AXIS_NUM`.
pub const MAX_AXIS_NUM: usize = 8;

/// `MAX_BUTTONS_NUM / 8` from `vendored/common_defines.h::MAX_BUTTONS_NUM = 128`.
pub const BUTTON_BITMAP_BYTES: usize = 128 / 8;

/// `(major, minor, patch)` at/after which `detect_axis_raw` is present.
const DETECT_AXIS_SINCE: (u8, u8, u8) = (0, 1, 3);

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
    /// Per-axis raw readings for the axis auto-detect flow. `Some` only for
    /// firmware >= 0.1.3 that actually carries the appended field; `None` for
    /// legacy (72-byte) reports.
    pub detect_axis_raw: Option<[i16; MAX_AXIS_NUM]>,
}

impl ParamsReport {
    /// Decode an assembled `params_report_t` payload (72 or 88 bytes).
    ///
    /// The caller is responsible for reassembling fragments first via
    /// `wire::fragments::reassemble_two_fragment`. Any length >= the legacy
    /// 72 bytes is accepted; `detect_axis_raw` is read only when the buffer
    /// is full-length *and* the reporting firmware is >= 0.1.3.
    pub fn decode(bytes: &[u8]) -> Result<Self, DecodeError> {
        if bytes.len() < PARAMS_REPORT_LEGACY_SIZE {
            return Err(DecodeError::BufferTooShort {
                needed: PARAMS_REPORT_LEGACY_SIZE,
                got: bytes.len(),
            });
        }
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
        debug_assert_eq!(cur.position(), PARAMS_REPORT_LEGACY_SIZE);

        // Only interpret the appended field when it's both present and produced
        // by firmware new enough to actually populate it (see module docs).
        let carries_detect = version_at_least(
            (
                freejoyx_version_major,
                freejoyx_version_minor,
                freejoyx_version_patch,
            ),
            DETECT_AXIS_SINCE,
        ) && bytes.len() >= PARAMS_REPORT_SIZE;
        let detect_axis_raw = if carries_detect {
            Some(read_i16_array::<MAX_AXIS_NUM>(&mut cur)?)
        } else {
            None
        };

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
            detect_axis_raw,
        })
    }

    /// Encode this report back into its assembled (pre-fragmentation) bytes:
    /// 88 bytes when `detect_axis_raw` is `Some`, else the legacy 72. Useful
    /// for tests and synthetic fixtures; the device-bound writer doesn't use
    /// this directly since the configurator only sends `REPORT_ID_PARAM` as a
    /// poll request, never as a payload.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let size = if self.detect_axis_raw.is_some() {
            PARAMS_REPORT_SIZE
        } else {
            PARAMS_REPORT_LEGACY_SIZE
        };
        let mut out = vec![0u8; size];
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
        if let Some(raw) = self.detect_axis_raw {
            for v in raw {
                w.write_i16_le(v);
            }
        }
        debug_assert_eq!(w.position(), size);
        out
    }
}

/// `(major, minor, patch) >= want`, lexicographically.
fn version_at_least(v: (u8, u8, u8), want: (u8, u8, u8)) -> bool {
    v >= want
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
    fn roundtrip_zeros_legacy() {
        let zero = [0u8; PARAMS_REPORT_LEGACY_SIZE];
        let p = ParamsReport::decode(&zero).unwrap();
        assert_eq!(p.detect_axis_raw, None);
        assert_eq!(p.encode(), zero.to_vec());
    }

    #[test]
    fn roundtrip_known_values_legacy() {
        let mut bytes = [0u8; PARAMS_REPORT_LEGACY_SIZE];
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
        // freejoyx_version = 0.1.2 (below the detect_axis_raw gate)
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
        assert_eq!(p.detect_axis_raw, None);

        assert_eq!(p.encode(), bytes.to_vec());
    }

    #[test]
    fn detect_axis_raw_decoded_for_0_1_3_and_round_trips() {
        let mut bytes = [0u8; PARAMS_REPORT_SIZE];
        // freejoyx_version = 0.1.3 (the gate)
        bytes[69] = 0;
        bytes[70] = 1;
        bytes[71] = 3;
        // detect_axis_raw[0] = 0x1234, detect_axis_raw[7] = -1
        bytes[72] = 0x34;
        bytes[73] = 0x12;
        bytes[72 + 14] = 0xff;
        bytes[72 + 15] = 0xff;

        let p = ParamsReport::decode(&bytes).unwrap();
        let d = p
            .detect_axis_raw
            .expect("a 0.1.3 report should carry detect_axis_raw");
        assert_eq!(d[0], 0x1234);
        assert_eq!(d[7], -1);
        // 88-byte round trip is byte-identical.
        assert_eq!(p.encode(), bytes.to_vec());
    }

    #[test]
    fn detect_axis_raw_ignored_when_firmware_too_old() {
        // 88 bytes physically present, but firmware 0.1.2 < 0.1.3: the trailing
        // 16 bytes are uninitialised on the wire and must not be interpreted.
        let mut bytes = [0u8; PARAMS_REPORT_SIZE];
        bytes[69] = 0;
        bytes[70] = 1;
        bytes[71] = 2;
        bytes[72] = 0xAA; // would-be detect_axis_raw garbage
        let p = ParamsReport::decode(&bytes).unwrap();
        assert_eq!(p.detect_axis_raw, None);
        // Encodes back to the legacy length, dropping the ignored tail.
        assert_eq!(p.encode().len(), PARAMS_REPORT_LEGACY_SIZE);
    }

    #[test]
    fn buffer_shorter_than_legacy_errors() {
        assert!(matches!(
            ParamsReport::decode(&[0u8; 10]),
            Err(DecodeError::BufferTooShort {
                needed: PARAMS_REPORT_LEGACY_SIZE,
                got: 10,
            })
        ));
    }
}
