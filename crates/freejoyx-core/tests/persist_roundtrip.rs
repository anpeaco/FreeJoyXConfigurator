//! Cross-trip integration test for the on-disk RON layer.
//!
//! The load-bearing assertion is `cross_trip_is_byte_identical`: starting
//! from real device bytes, decode → encode-to-RON → decode-from-RON →
//! encode-to-bytes must produce bytes byte-identical to the input. This
//! proves the RON layer adds no drift on top of the wire codec, which is
//! the only correctness property the persist module owes.

use std::path::{Path, PathBuf};

use freejoyx_core::persist::{from_str, to_string};
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
        panic!("fixture {set} length {} != {DEV_CONFIG_SIZE}", v.len())
    })
}

/// Cross-trip: wire bytes → DeviceConfig → RON → DeviceConfig → wire bytes
/// is byte-identical for every captured fixture.
#[test]
fn cross_trip_is_byte_identical() {
    for set in ["minimal", "wide_coverage"] {
        let bytes = load_config(set);
        let cfg = DeviceConfig::decode(&bytes).unwrap();

        let ron = to_string(&cfg).unwrap_or_else(|e| panic!("{set}: to_string failed: {e}"));
        let reloaded = from_str(&ron).unwrap_or_else(|e| panic!("{set}: from_str failed: {e}"));

        let re_encoded = reloaded.encode();
        if re_encoded != bytes {
            for (i, (a, b)) in re_encoded.iter().zip(bytes.iter()).enumerate() {
                if a != b {
                    panic!(
                        "{set}: cross-trip diverged at byte {i} (0x{i:04x}): \
                         encoded=0x{a:02x}, original=0x{b:02x}"
                    );
                }
            }
            unreachable!("arrays differ but element-wise comparison found no diff");
        }
    }
}

/// Value-identical: DeviceConfig → RON → DeviceConfig is `PartialEq` equal.
/// Catches any RON ser/de path that drops or rewrites a field without
/// touching its raw byte form.
#[test]
fn value_round_trip_preserves_struct() {
    for set in ["minimal", "wide_coverage"] {
        let bytes = load_config(set);
        let cfg = DeviceConfig::decode(&bytes).unwrap();
        let ron = to_string(&cfg).unwrap();
        let reloaded = from_str(&ron).unwrap();
        assert_eq!(cfg, reloaded, "{set}: struct comparison failed");
    }
}

/// Pretty-printed RON is reasonable on a real device — the maintainer
/// will eyeball these files. Sanity-check that it contains expected
/// struct names so a human can find the field they want to edit.
#[test]
fn pretty_output_is_recognizable() {
    let bytes = load_config("minimal");
    let cfg = DeviceConfig::decode(&bytes).unwrap();
    let ron = to_string(&cfg).unwrap();

    // Structural anchors — if RON omits these names, struct_names is broken.
    for needle in [
        "DeviceConfig",
        "firmware_version",
        "buttons",
        "pins",
        "saved_breakdown",
    ] {
        assert!(
            ron.contains(needle),
            "RON output missing {needle:?}; first 200 chars: {}",
            &ron[..ron.len().min(200)]
        );
    }
}

/// Tempfile-style save / load through the public file API. Smoke test that
/// the save/load path opens and closes correctly on the host OS.
#[test]
fn save_then_load_file_round_trips() {
    use freejoyx_core::persist::{load_from_file, save_to_file};

    let bytes = load_config("wide_coverage");
    let cfg = DeviceConfig::decode(&bytes).unwrap();

    let path = std::env::temp_dir().join(format!("freejoyx-persist-{}.ron", std::process::id()));
    save_to_file(&cfg, &path).expect("save");
    let reloaded = load_from_file(&path).expect("load");
    let _ = std::fs::remove_file(&path);

    assert_eq!(reloaded.encode(), bytes);
}
