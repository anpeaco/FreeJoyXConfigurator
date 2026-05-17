//! Transport-layer errors.
//!
//! Wraps the small set of `hidapi` and codec failure modes that surface
//! to callers. Codec errors keep their full `DecodeError` payload so the
//! caller can distinguish a short read from a malformed packet.

use thiserror::Error;

use freejoyx_core::wire::DecodeError;

#[derive(Debug, Error)]
pub enum TransportError {
    #[error("hidapi init failed: {0}")]
    HidInit(hidapi::HidError),

    #[error("hidapi enumeration failed: {0}")]
    Enumerate(hidapi::HidError),

    #[error("no FreeJoyX device found (matched 0 candidates by manufacturer string)")]
    NoDevice,

    #[error("could not open device at {path}: {source}")]
    Open {
        path: String,
        source: hidapi::HidError,
    },

    #[error("hid read failed: {0}")]
    Read(hidapi::HidError),

    #[error("hid read returned {got} bytes; expected {expected}")]
    ShortRead { got: usize, expected: usize },

    #[error("decode failed: {0}")]
    Decode(#[from] DecodeError),

    #[error("read timed out after {ms} ms")]
    Timeout { ms: i32 },
}
