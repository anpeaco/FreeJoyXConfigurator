//! Wire-codec errors.
//!
//! `DecodeError` is the failure mode for `wire::*::decode` functions:
//! a byte stream did not satisfy the layout contract of the target
//! struct. Each variant names the field that failed and (where useful)
//! the byte offset, so a fixture test failure points straight at the
//! line of codec code that read it.

use thiserror::Error;

/// Reasons a byte stream could not be decoded into a wire struct.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum DecodeError {
    /// Input was shorter than the struct requires.
    #[error("buffer too short: needed {needed} bytes, got {got}")]
    BufferTooShort { needed: usize, got: usize },

    /// First byte of a HID frame was not the expected report ID.
    #[error("unexpected report id: expected 0x{expected:02x}, got 0x{got:02x}")]
    UnexpectedReportId { expected: u8, got: u8 },

    /// Fragment index byte was not in the valid range for this report.
    #[error("unexpected fragment index {got} (valid range: 0..={max})")]
    UnexpectedFragmentIndex { got: u8, max: u8 },

    /// Two fragments were assembled but their report-id or version
    /// fields disagree — almost certainly the second fragment is from
    /// a different logical report.
    #[error("fragment pair disagrees on header: {detail}")]
    FragmentMismatch { detail: String },

    /// A field's value was outside the range its type permits.
    #[error("invalid value for {field}: 0x{value:x}")]
    InvalidValue { field: &'static str, value: u64 },

    /// `FIRMWARE_VERSION` mask group is not one this codec supports.
    /// Carries the value so the caller can report it to the user.
    #[error("unsupported firmware version: 0x{0:04x}")]
    UnsupportedFirmwareVersion(u16),
}

/// Reasons a wire struct could not be encoded back to bytes.
///
/// Most encode paths are infallible (the domain types' invariants
/// already constrain values to fit the wire), so this enum currently
/// has no variants — added as needed.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum EncodeError {}
