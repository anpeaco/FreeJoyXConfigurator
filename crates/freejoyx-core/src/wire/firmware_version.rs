//! FIRMWARE_VERSION compatibility constants and gate.
//!
//! Port.md §9 locks the v0.1 wire-format target at `0x0010`, mask group
//! `& 0xFFF0`. Firmware versions outside that mask group (the historical
//! upstream `0x17XX` lineage, future post-bump groups `0x0020+`) are
//! out of scope for this codec — the configurator refuses them with a
//! toast pointing at the Qt app rather than risk a misread.

/// FreeJoyX wire-format generation this codec targets.
///
/// Matches `vendored/common_defines.h::FIRMWARE_VERSION`.
pub const SUPPORTED_FIRMWARE_VERSION: u16 = 0x0010;

/// Mask used to group compatible firmware revisions. The low nibble is
/// the firmware build number; the high three nibbles identify the wire
/// format. Two firmware versions with the same masked value are codec-
/// compatible.
pub const FIRMWARE_VERSION_MASK: u16 = 0xFFF0;

/// Return the wire-format mask group of `v` — the high three nibbles.
#[must_use]
pub const fn mask_group(v: u16) -> u16 {
    v & FIRMWARE_VERSION_MASK
}

/// True iff `v` is in the same mask group as
/// [`SUPPORTED_FIRMWARE_VERSION`].
#[must_use]
pub const fn is_supported_firmware_version(v: u16) -> bool {
    mask_group(v) == mask_group(SUPPORTED_FIRMWARE_VERSION)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn supported_target_is_supported() {
        assert!(is_supported_firmware_version(SUPPORTED_FIRMWARE_VERSION));
    }

    #[test]
    fn build_nibble_drift_is_supported() {
        assert!(is_supported_firmware_version(0x001F));
        assert!(is_supported_firmware_version(0x0011));
    }

    #[test]
    fn legacy_upstream_versions_are_not_supported() {
        assert!(!is_supported_firmware_version(0x1700));
        assert!(!is_supported_firmware_version(0x1770));
        assert!(!is_supported_firmware_version(0x1790));
    }

    #[test]
    fn future_mask_group_is_not_supported() {
        assert!(!is_supported_firmware_version(0x0020));
        assert!(!is_supported_firmware_version(0x0030));
    }

    #[test]
    fn mask_group_strips_low_nibble() {
        assert_eq!(mask_group(0x001F), 0x0010);
        assert_eq!(mask_group(0x1770), 0x1770);
    }
}
