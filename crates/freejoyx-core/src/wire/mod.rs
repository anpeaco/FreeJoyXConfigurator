//! Wire codec — encode/decode bytes ↔ domain types.
//!
//! Path B per Port.md §3 "Codec strategy": manual, single-layer,
//! `from_le_bytes` everywhere, explicit bitfield masking. No mirror
//! struct, no `zerocopy`/`bytemuck`.
//!
//! Module layout:
//! - [`error`]: typed errors for decode failures
//! - [`cursor`]: little-endian read/write helpers
//! - [`fragments`]: HID frame splitter / reassembler (2-byte header + 62 payload)
//! - [`params`]: `params_report_t` codec (72 bytes after reassembly)
//! - `config` (not yet implemented): `dev_config_t` codec (1580 bytes
//!   after reassembly across 26 fragments)
//!
//! Per Port.md, every field gets paired encode/decode tests so a typo
//! in one half is caught by the other.

pub mod config;
pub mod cursor;
pub mod display;
pub mod error;
pub mod firmware_version;
pub mod fragments;
pub mod params;

pub use display::format_config;
pub use firmware_version::{
    is_supported_firmware_version, mask_group, FIRMWARE_VERSION_MASK, SUPPORTED_FIRMWARE_VERSION,
};

pub use config::{
    AxisConfig, AxisToButtons, Button, DeviceConfig, FastEncoder, PhysBreakdown, ShiftRegConfig,
    DEV_CONFIG_SIZE, MAX_BUTTONS_NUM, MAX_ENCODERS_NUM, MAX_FAST_ENCODER_NUM, MAX_LEDS_NUM,
    MAX_SHIFTS_NUM, MAX_SHIFT_REG_NUM, NUM_RGB_LEDS, USED_PINS_NUM,
};
pub use error::{DecodeError, EncodeError};
pub use fragments::{
    fragment_count, reassemble_fragments, reassemble_two_fragment, Frame, FRAGMENT_PAYLOAD,
    FRAME_SIZE, REPORT_ID_CONFIG_IN, REPORT_ID_PARAM,
};
pub use params::{ParamsReport, BUTTON_BITMAP_BYTES, MAX_AXIS_NUM, PARAMS_REPORT_SIZE};
