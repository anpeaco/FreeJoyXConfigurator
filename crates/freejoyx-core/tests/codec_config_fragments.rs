//! Integration tests for `dev_config_t` fragment reassembly.
//!
//! The full `dev_config_t` codec lands in a follow-up commit; this
//! test fixes the foundation: the captured wire-stream fragments
//! (`config.fragments.bin`) reassemble byte-for-byte to the assembled
//! struct dump (`config.bin`). That proves:
//!
//! 1. `reassemble_fragments` handles the 26-fragment config path
//!    correctly.
//! 2. The Qt-side patch's fragment dumps and assembled-struct dump
//!    agree, i.e. our fixtures are self-consistent.
//!
//! If this test fails on a fresh capture, either (a) the patch
//! is no longer in sync with the Qt app's reassembly logic
//! (regenerate `config.fragments.bin` after re-applying the patch)
//! or (b) the fragment reassembler has a bug — investigate before
//! trusting fixture-driven codec tests.

use std::path::{Path, PathBuf};

use freejoyx_core::wire::{fragment_count, reassemble_fragments, REPORT_ID_CONFIG_IN};

const DEV_CONFIG_SIZE: usize = 1580;

fn fixtures_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest
        .parent()
        .and_then(Path::parent)
        .expect("workspace layout: crates/freejoyx-core has two parents up to repo root");
    workspace_root.join("fixtures")
}

fn load(set: &str, name: &str) -> Vec<u8> {
    let path = fixtures_root().join(set).join(name);
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "could not read fixture {} ({}). See fixtures/REGEN.md.",
            path.display(),
            e
        )
    })
}

#[test]
fn fragment_count_matches_constants() {
    // Sanity: the config wire format expects exactly 26 fragments.
    // If common_defines.h ever changes FREEJOY_DEV_CONFIG_SIZE in a
    // way that affects this number, the header-sync workflow flags
    // it; this test independently asserts the math.
    assert_eq!(fragment_count(DEV_CONFIG_SIZE), 26);
}

#[test]
fn config_fragments_have_expected_size() {
    // Each capture should contain exactly 26 fragments × 64 bytes.
    for set in ["minimal", "wide_coverage"] {
        let frames = load(set, "config.fragments.bin");
        assert_eq!(
            frames.len(),
            26 * 64,
            "{set}/config.fragments.bin length {} != 26 * 64",
            frames.len()
        );
    }
}

#[test]
fn config_fragments_reassemble_to_config_bin() {
    // The load-bearing claim: bytes the firmware put on the wire
    // (config.fragments.bin) reassemble to the same 1580 bytes the
    // Qt app reinterpreted as dev_config_t (config.bin).
    for set in ["minimal", "wide_coverage"] {
        let frames = load(set, "config.fragments.bin");
        let assembled_bin = load(set, "config.bin");
        assert_eq!(
            assembled_bin.len(),
            DEV_CONFIG_SIZE,
            "{set}/config.bin size {} != {DEV_CONFIG_SIZE}",
            assembled_bin.len()
        );

        // Config fragments use first_index=1 (request-response).
        let reports = reassemble_fragments(&frames, REPORT_ID_CONFIG_IN, DEV_CONFIG_SIZE, 1);
        assert_eq!(
            reports.len(),
            1,
            "{set}: expected exactly 1 reassembled config, got {}",
            reports.len()
        );

        assert_eq!(
            reports[0].len(),
            DEV_CONFIG_SIZE,
            "{set}: reassembled length {} != {DEV_CONFIG_SIZE}",
            reports[0].len()
        );

        if reports[0] != assembled_bin {
            // Find first divergence to point at the bug.
            for (i, (a, b)) in reports[0].iter().zip(assembled_bin.iter()).enumerate() {
                if a != b {
                    panic!(
                        "{set}: reassembled fragments diverge from config.bin at byte {i}: \
                         reassembled=0x{a:02x}, config.bin=0x{b:02x}"
                    );
                }
            }
            unreachable!("vectors differ in length but element-wise comparison found no diff");
        }
    }
}

#[test]
fn config_reassembly_starts_with_firmware_version_0x0010() {
    // Smoke check on the assembled bytes: the first two bytes of
    // dev_config_t are firmware_version (u16 LE), and every captured
    // device runs 0x0010 per Port.md §9.
    for set in ["minimal", "wide_coverage"] {
        let frames = load(set, "config.fragments.bin");
        let reports = reassemble_fragments(&frames, REPORT_ID_CONFIG_IN, DEV_CONFIG_SIZE, 1);
        assert_eq!(reports.len(), 1);
        let fv = u16::from_le_bytes([reports[0][0], reports[0][1]]);
        assert_eq!(
            fv, 0x0010,
            "{set}: reassembled config firmware_version 0x{fv:04x} != 0x0010"
        );
    }
}
