//! Integration tests for `wire::params` against captured device
//! fixtures.
//!
//! Fixtures live in `<repo>/fixtures/<set>/params.bin` — append-only
//! streams of 64-byte HID frames as captured by the throwaway patch
//! to `FreeJoyXConfiguratorQt/src/hiddevice.cpp`. See
//! `<repo>/fixtures/REGEN.md` for the regeneration recipe.
//!
//! NOTE: all current fixtures were captured from firmware v0.1
//! (`firmware_version=0x0010`, pre-0.1.3), so the logical report is the
//! 72-byte legacy form — hence `PARAMS_REPORT_LEGACY_SIZE` throughout. The
//! 88-byte `detect_axis_raw` form (firmware >= 0.1.3) is covered by the
//! synthetic unit tests in `wire::params`; add an 88-byte fixture here once a
//! >= 0.1.3 device can be captured (see `fixtures/REGEN.md`).

use std::path::{Path, PathBuf};

use freejoyx_core::wire::{
    reassemble_two_fragment, ParamsReport, FRAME_SIZE, PARAMS_REPORT_LEGACY_SIZE, REPORT_ID_PARAM,
};

/// Walk up from the current test binary's manifest directory to find
/// the repo root by looking for `fixtures/`.
fn fixtures_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    // CARGO_MANIFEST_DIR for this crate is .../crates/freejoyx-core; the
    // workspace root is two levels up.
    let workspace_root = manifest
        .parent()
        .and_then(Path::parent)
        .expect("workspace layout: crates/freejoyx-core has two parents up to repo root");
    workspace_root.join("fixtures")
}

fn load_stream(set: &str) -> Vec<u8> {
    let path = fixtures_root().join(set).join("params.bin");
    std::fs::read(&path).unwrap_or_else(|e| {
        panic!(
            "could not read fixture {} ({}). See fixtures/REGEN.md to regenerate.",
            path.display(),
            e
        )
    })
}

#[test]
fn fixture_streams_are_frame_aligned() {
    for set in ["minimal", "wide_coverage", "params_stream"] {
        let stream = load_stream(set);
        assert_eq!(
            stream.len() % FRAME_SIZE,
            0,
            "{set}/params.bin length {} is not a multiple of FRAME_SIZE={FRAME_SIZE}",
            stream.len()
        );
        assert!(
            !stream.is_empty(),
            "{set}/params.bin is empty — regenerate per fixtures/REGEN.md"
        );
    }
}

#[test]
fn fixture_streams_have_expected_report_id() {
    for set in ["minimal", "wide_coverage", "params_stream"] {
        let stream = load_stream(set);
        let n_frames = stream.len() / FRAME_SIZE;
        let mut mismatched = 0;
        for i in 0..n_frames {
            if stream[i * FRAME_SIZE] != REPORT_ID_PARAM {
                mismatched += 1;
            }
        }
        assert_eq!(
            mismatched, 0,
            "{set}/params.bin: {mismatched}/{n_frames} frames have wrong report id"
        );
    }
}

#[test]
fn reassembly_produces_72_byte_reports() {
    for set in ["minimal", "wide_coverage", "params_stream"] {
        let stream = load_stream(set);
        let reports = reassemble_two_fragment(&stream, REPORT_ID_PARAM, PARAMS_REPORT_LEGACY_SIZE);
        assert!(
            !reports.is_empty(),
            "{set}: reassembly produced no logical reports from {} bytes",
            stream.len()
        );
        for (i, r) in reports.iter().enumerate() {
            assert_eq!(
                r.len(),
                PARAMS_REPORT_LEGACY_SIZE,
                "{set} report {i}: reassembled length {} != {PARAMS_REPORT_LEGACY_SIZE}",
                r.len()
            );
        }
        // Each pair of frames produces one logical report.
        let expected_reports = (stream.len() / FRAME_SIZE) / 2;
        // Allow ±1 in case the stream starts or ends mid-pair.
        let diff = (reports.len() as i64 - expected_reports as i64).abs();
        assert!(
            diff <= 1,
            "{set}: got {} reports, expected ~{expected_reports}",
            reports.len()
        );
    }
}

#[test]
fn every_report_decodes_without_error() {
    for set in ["minimal", "wide_coverage", "params_stream"] {
        let stream = load_stream(set);
        let reports = reassemble_two_fragment(&stream, REPORT_ID_PARAM, PARAMS_REPORT_LEGACY_SIZE);
        for (i, raw) in reports.iter().enumerate() {
            ParamsReport::decode(raw)
                .unwrap_or_else(|e| panic!("{set} report {i}: decode error {e}"));
        }
    }
}

#[test]
fn firmware_version_matches_target() {
    // Every captured packet was produced by a board running
    // FIRMWARE_VERSION=0x0010 (the only version v0.1 supports).
    const EXPECTED: u16 = 0x0010;
    for set in ["minimal", "wide_coverage", "params_stream"] {
        let stream = load_stream(set);
        let reports = reassemble_two_fragment(&stream, REPORT_ID_PARAM, PARAMS_REPORT_LEGACY_SIZE);
        for (i, raw) in reports.iter().enumerate() {
            let p = ParamsReport::decode(raw).unwrap();
            assert_eq!(
                p.firmware_version, EXPECTED,
                "{set} report {i}: firmware_version 0x{:04x} != expected 0x{EXPECTED:04x}",
                p.firmware_version
            );
        }
    }
}

#[test]
fn board_id_is_consistent_within_a_capture() {
    // Within one capture, board_id should be constant — it identifies
    // which physical board is plugged in. (Across captures it may
    // differ, since fixtures may come from BluePill vs BlackPill.)
    for set in ["minimal", "wide_coverage", "params_stream"] {
        let stream = load_stream(set);
        let reports = reassemble_two_fragment(&stream, REPORT_ID_PARAM, PARAMS_REPORT_LEGACY_SIZE);
        let first = ParamsReport::decode(&reports[0]).unwrap().board_id;
        for (i, raw) in reports.iter().enumerate() {
            let p = ParamsReport::decode(raw).unwrap();
            assert_eq!(
                p.board_id, first,
                "{set} report {i}: board_id {} drifted from {first}",
                p.board_id
            );
        }
    }
}

#[test]
fn fixture_round_trip_is_byte_identical() {
    // The core correctness claim of Path B: decode then encode is the
    // identity. If this fails on a real fixture, the codec is wrong
    // somewhere (likely an offset / sign / endianness slip).
    for set in ["minimal", "wide_coverage", "params_stream"] {
        let stream = load_stream(set);
        let reports = reassemble_two_fragment(&stream, REPORT_ID_PARAM, PARAMS_REPORT_LEGACY_SIZE);
        for (i, raw) in reports.iter().enumerate() {
            let p = ParamsReport::decode(raw).unwrap();
            let re = p.encode();
            assert_eq!(
                re.as_slice(),
                raw.as_slice(),
                "{set} report {i}: round-trip diverged at first differing byte"
            );
        }
    }
}

#[test]
fn params_stream_has_axis_movement() {
    // The params_stream fixture was captured while inputs were
    // exercised. At least one axis should have non-zero values
    // somewhere in the stream — proves the codec sees varying
    // multi-byte field values, not just zeros.
    let stream = load_stream("params_stream");
    let reports = reassemble_two_fragment(&stream, REPORT_ID_PARAM, PARAMS_REPORT_LEGACY_SIZE);
    let saw_nonzero_axis = reports.iter().any(|raw| {
        let p = ParamsReport::decode(raw).unwrap();
        p.axis_data.iter().any(|&v| v != 0)
    });
    assert!(
        saw_nonzero_axis,
        "params_stream: no report had non-zero axis_data — regenerate with stick movement"
    );
}

#[test]
fn params_stream_has_button_or_shift_activity() {
    // Soft check on capture quality: at some point during the
    // capture, a button (physical or logical) or shift should have
    // been active. Useful for validating button-bitmap decode.
    //
    // If this fails, the codec is still correct (proven by the
    // round-trip test) — just regenerate params_stream while
    // pressing buttons to harden the field coverage.
    let stream = load_stream("params_stream");
    let reports = reassemble_two_fragment(&stream, REPORT_ID_PARAM, PARAMS_REPORT_LEGACY_SIZE);
    let saw_activity = reports.iter().any(|raw| {
        let p = ParamsReport::decode(raw).unwrap();
        p.phy_button_data.iter().any(|&b| b != 0)
            || p.log_button_data.iter().any(|&b| b != 0)
            || p.shift_button_data != 0
    });
    if !saw_activity {
        // Not a hard failure — note it loudly so future captures
        // can fix it without re-running the whole codec suite.
        eprintln!(
            "WARNING: params_stream has no button/shift activity. \
             Codec passes, but the fixture would benefit from a recapture \
             that includes button presses (see fixtures/REGEN.md step 3.3)."
        );
    }
}
