//! Integration tests for the full `dev_config_t` codec.
//!
//! The load-bearing assertion is `fixture_round_trip_is_byte_identical`:
//! every captured `config.bin` decodes to a `DeviceConfig`, re-encodes
//! to bytes, and the bytes match the input exactly. This is what proves
//! the codec's field offsets, widths, signs, endianness, and bitfield
//! masks all match the firmware-side struct layout.
//!
//! Companion: `codec_config_fragments.rs` already proved
//! `config.fragments.bin` reassembles to `config.bin`. Together, the two
//! test files cover the full wire → struct → wire path.

use std::path::{Path, PathBuf};

use freejoyx_core::wire::{DeviceConfig, DEV_CONFIG_SIZE};

fn fixtures_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest
        .parent()
        .and_then(Path::parent)
        .expect("workspace layout: crates/freejoyx-core has two parents up to repo root");
    workspace_root.join("fixtures")
}

fn load_config(set: &str) -> [u8; DEV_CONFIG_SIZE] {
    let path = fixtures_root().join(set).join("config.bin");
    let bytes = std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "could not read fixture {} ({}). See fixtures/REGEN.md.",
            path.display(),
            e
        )
    });
    bytes.try_into().unwrap_or_else(|v: Vec<u8>| {
        panic!(
            "fixture {} length {} != expected {DEV_CONFIG_SIZE}",
            set,
            v.len()
        )
    })
}

#[test]
fn every_fixture_decodes_without_error() {
    for set in ["minimal", "wide_coverage"] {
        let bytes = load_config(set);
        DeviceConfig::decode(&bytes).unwrap_or_else(|e| {
            panic!("decode failed for fixture {set}: {e}");
        });
    }
}

#[test]
fn fixture_firmware_version_is_historical_0x0010() {
    // The bundled fixtures were captured on devices running
    // FIRMWARE_VERSION = 0x0010 (pre-TAP-rename). Since the 0x0010 →
    // 0x0020 bump was a SEMANTIC-only change (LONG_PRESS → TAP), the
    // captured bytes still exercise the current codec's shape — the
    // version field itself is the only divergence from a fresh 0x0020
    // capture. See fixtures/REGEN.md for the formal recapture path.
    for set in ["minimal", "wide_coverage"] {
        let bytes = load_config(set);
        let cfg = DeviceConfig::decode(&bytes).unwrap();
        assert_eq!(
            cfg.firmware_version, 0x0010,
            "{set}: firmware_version 0x{:04x} != 0x0010 (historical capture)",
            cfg.firmware_version
        );
    }
}

/// THE load-bearing test for the whole codec. If this passes, every
/// field's offset / width / sign / endianness / bitfield mask agrees
/// with what the firmware wrote to flash. If it fails, the panic message
/// points at the first divergent byte — that byte's offset tells you
/// which field's encode/decode pair to inspect.
#[test]
fn fixture_round_trip_is_byte_identical() {
    for set in ["minimal", "wide_coverage"] {
        let bytes = load_config(set);
        let cfg = DeviceConfig::decode(&bytes).unwrap();
        let re_encoded = cfg.encode();
        if re_encoded != bytes {
            for (i, (a, b)) in re_encoded.iter().zip(bytes.iter()).enumerate() {
                if a != b {
                    panic!(
                        "{set}: round-trip diverged at byte {i} (0x{i:04x}): \
                         encoded=0x{a:02x}, original=0x{b:02x}. \
                         Inspect the encode/decode pair for the field containing this offset \
                         (see vendored/common_types.h::dev_config_t)."
                    );
                }
            }
            unreachable!("arrays differ but element-wise comparison found no diff");
        }
    }
}

/// Sanity: the wide_coverage fixture should have *some* non-default
/// content somewhere (otherwise the round-trip test only proves zeros
/// round-trip, which roundtrip_zeros already covers).
#[test]
fn wide_coverage_has_nontrivial_content() {
    let bytes = load_config("wide_coverage");
    let cfg = DeviceConfig::decode(&bytes).unwrap();
    let any_pin = cfg.pins.iter().any(|&p| p != 0);
    let any_button = cfg.buttons.iter().any(|b| b.button_type != 0);
    let any_axis = cfg.axis_config.iter().any(|a| a.flags1 != 0);
    assert!(
        any_pin || any_button || any_axis,
        "wide_coverage fixture has zero pins, buttons, and axis flags — \
         re-capture per fixtures/REGEN.md or rename to 'minimal'"
    );
}
