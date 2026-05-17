//! On-disk RON serde for [`DeviceConfig`].
//!
//! Configs persist as `.freejoyx-config.ron` files. RON was chosen over JSON
//! because the wire format has many small integer arrays (`pins`,
//! `curve_shape`, raw LED blocks) that RON renders more compactly without
//! quoting every key.
//!
//! Round-trip parity is the load-bearing property: a config loaded from
//! disk must encode to bytes byte-identical to the bytes that originally
//! produced it. The integration test
//! `crates/freejoyx-core/tests/persist_roundtrip.rs` proves this against
//! the captured fixtures.

use std::path::Path;

use thiserror::Error;

use crate::wire::config::DeviceConfig;

/// Errors returned by [`save_to_file`] / [`load_from_file`].
#[derive(Debug, Error)]
pub enum PersistError {
    /// Underlying file I/O failed.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// RON failed to parse the file as a `DeviceConfig`.
    #[error("parse error: {0}")]
    Parse(#[from] ron::error::SpannedError),
    /// RON failed to serialize a `DeviceConfig` to text.
    #[error("encode error: {0}")]
    Encode(#[from] ron::Error),
}

/// Serialize `config` as pretty RON and write it to `path`.
///
/// # Errors
///
/// Returns [`PersistError::Encode`] if RON serialization fails (should not
/// occur for any well-formed `DeviceConfig`), or [`PersistError::Io`] if
/// the write fails.
pub fn save_to_file(config: &DeviceConfig, path: impl AsRef<Path>) -> Result<(), PersistError> {
    let text = to_string(config)?;
    std::fs::write(path, text)?;
    Ok(())
}

/// Read a `.freejoyx-config.ron` file and decode it.
///
/// # Errors
///
/// Returns [`PersistError::Io`] if the file can't be read, or
/// [`PersistError::Parse`] if the contents aren't a valid `DeviceConfig`.
pub fn load_from_file(path: impl AsRef<Path>) -> Result<DeviceConfig, PersistError> {
    let text = std::fs::read_to_string(path)?;
    from_str(&text)
}

/// Serialize a `DeviceConfig` to a pretty RON string.
///
/// Exposed for tests + future in-memory clipboard / network transports.
///
/// # Errors
///
/// Returns [`PersistError::Encode`] if RON serialization fails.
pub fn to_string(config: &DeviceConfig) -> Result<String, PersistError> {
    let pretty = ron::ser::PrettyConfig::new()
        .depth_limit(8)
        .struct_names(true)
        .compact_arrays(true);
    Ok(ron::ser::to_string_pretty(config, pretty)?)
}

/// Parse a RON string into a `DeviceConfig`.
///
/// # Errors
///
/// Returns [`PersistError::Parse`] if the RON is malformed or doesn't match
/// the `DeviceConfig` shape.
pub fn from_str(text: &str) -> Result<DeviceConfig, PersistError> {
    Ok(ron::de::from_str(text)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_round_trips_through_string() {
        let bytes = [0u8; crate::wire::config::DEV_CONFIG_SIZE];
        let cfg = DeviceConfig::decode(&bytes).unwrap();
        let text = to_string(&cfg).unwrap();
        let decoded = from_str(&text).unwrap();
        assert_eq!(cfg, decoded);
        assert_eq!(decoded.encode(), bytes);
    }
}
