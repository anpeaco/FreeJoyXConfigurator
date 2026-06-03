//! HID fragment reassembly.
//!
//! FreeJoyX splits large reports across 64-byte HID frames:
//!
//! ```text
//! frame[0]    = report id    (REPORT_ID_PARAM, REPORT_ID_CONFIG_IN, ...)
//! frame[1]    = fragment idx (0, 1, ...)
//! frame[2..]  = 62 bytes of payload
//! ```
//!
//! `params_report_t` (72 bytes legacy, 88 from firmware v0.1.3) is split
//! across **two** frames:
//! - frame 0 carries payload bytes 0..62
//! - frame 1 carries the remaining bytes (62..72 legacy, or 62..88 with
//!   `detect_axis_raw`); the rest of the frame is unused / zero
//!
//! `dev_config_t` (1580 bytes) is split across **26** frames:
//! - frames 0..25 carry 62 bytes each = 1612 bytes total
//! - bytes 1580..1612 of the last frame are unused
//!
//! Firmware reference: `FreeJoyX/application/Src/usb_app.c` (params)
//! and the config read/write paths in the same file.

use super::error::DecodeError;

/// Size of a single HID frame on the wire.
pub const FRAME_SIZE: usize = 64;
/// Bytes of payload carried by each frame after the 2-byte header.
pub const FRAGMENT_PAYLOAD: usize = FRAME_SIZE - 2;

/// REPORT_ID_PARAM value from `common_defines.h`.
pub const REPORT_ID_PARAM: u8 = 2;
/// REPORT_ID_CONFIG_IN value from `common_defines.h`.
pub const REPORT_ID_CONFIG_IN: u8 = 3;

/// One on-the-wire HID frame, already validated for report id and
/// fragment index range.
#[derive(Debug, Clone, Copy)]
pub struct Frame<'a> {
    pub report_id: u8,
    pub fragment_index: u8,
    pub payload: &'a [u8; FRAGMENT_PAYLOAD],
}

impl<'a> Frame<'a> {
    /// Parse one 64-byte HID frame, validating the report id matches
    /// `expected_report_id` and that fragment_index <= max_fragment.
    pub fn parse(
        bytes: &'a [u8; FRAME_SIZE],
        expected_report_id: u8,
        max_fragment: u8,
    ) -> Result<Self, DecodeError> {
        if bytes[0] != expected_report_id {
            return Err(DecodeError::UnexpectedReportId {
                expected: expected_report_id,
                got: bytes[0],
            });
        }
        if bytes[1] > max_fragment {
            return Err(DecodeError::UnexpectedFragmentIndex {
                got: bytes[1],
                max: max_fragment,
            });
        }
        // SAFETY: bytes has length FRAME_SIZE = 64, so &bytes[2..64] is
        // exactly FRAGMENT_PAYLOAD bytes. The `as` cast turns it into
        // a fixed-size array reference.
        let payload: &[u8; FRAGMENT_PAYLOAD] = bytes[2..]
            .try_into()
            .expect("frame is FRAME_SIZE; slice [2..] is FRAGMENT_PAYLOAD");
        Ok(Self {
            report_id: bytes[0],
            fragment_index: bytes[1],
            payload,
        })
    }
}

/// Number of fragments needed to carry a payload of `total_size` bytes.
#[must_use]
pub const fn fragment_count(total_size: usize) -> usize {
    total_size.div_ceil(FRAGMENT_PAYLOAD)
}

/// Walk an on-the-wire stream of 64-byte HID frames, assembling
/// consecutive in-order fragments into logical reports of `total_size`
/// bytes.
///
/// Fragment-index convention depends on the report type:
/// - **Params** (push from firmware, fragmented in `usb_app.c`): indices
///   `0..N` where N = `fragment_count(total_size) - 1`.
/// - **Config-in** (request-response): the configurator requests
///   fragment 1, 2, ..., N; the device echoes the requested index in
///   `buffer[1]`. So the wire indices are `1..=N`.
///
/// `first_index` selects the convention (0 for params, 1 for config).
/// The assembler accepts indices `first_index .. first_index +
/// fragment_count`.
///
/// Returns a `Vec` of assembled `total_size`-byte logical reports.
/// A frame whose index doesn't match the expected next index resets
/// the assembly state.
///
/// Panics on impossible inputs (stream length not a multiple of
/// FRAME_SIZE; total_size == 0; first_index + fragment_count > u8::MAX).
#[must_use]
pub fn reassemble_fragments(
    stream: &[u8],
    expected_report_id: u8,
    total_size: usize,
    first_index: u8,
) -> Vec<Vec<u8>> {
    assert!(
        stream.len() % FRAME_SIZE == 0,
        "stream length {} is not a multiple of FRAME_SIZE",
        stream.len()
    );
    assert!(total_size > 0, "total_size must be positive");
    let n_fragments = fragment_count(total_size);
    let last_index_usize = first_index as usize + n_fragments - 1;
    assert!(
        last_index_usize <= u8::MAX as usize,
        "first_index + n_fragments - 1 ({last_index_usize}) exceeds u8::MAX",
    );
    let last_index: u8 = last_index_usize as u8;

    let mut out = Vec::new();
    let mut buf: Vec<u8> = Vec::with_capacity(total_size);
    // The index we expect the next valid fragment to carry. None means
    // "no assembly in progress; expecting fragment `first_index`."
    let mut expected_next: Option<u8> = None;

    for chunk in stream.chunks_exact(FRAME_SIZE) {
        let frame_bytes: &[u8; FRAME_SIZE] = chunk
            .try_into()
            .expect("chunks_exact gives exactly FRAME_SIZE slices");
        let Ok(frame) = Frame::parse(frame_bytes, expected_report_id, last_index) else {
            buf.clear();
            expected_next = None;
            continue;
        };

        let idx = frame.fragment_index;

        if idx < first_index {
            // Below the valid range — ignore.
            buf.clear();
            expected_next = None;
            continue;
        }

        if idx == first_index {
            // Always reset on the first fragment — handles restarts cleanly.
            buf.clear();
            buf.extend_from_slice(frame.payload);
            if n_fragments == 1 {
                buf.truncate(total_size);
                out.push(std::mem::take(&mut buf));
                expected_next = None;
            } else {
                expected_next = Some(first_index + 1);
            }
        } else if Some(idx) == expected_next {
            // Continuing an in-progress assembly. Last fragment may
            // carry < FRAGMENT_PAYLOAD bytes of meaningful payload;
            // truncate at total_size.
            let so_far = buf.len();
            let remaining = total_size - so_far;
            let take = remaining.min(FRAGMENT_PAYLOAD);
            buf.extend_from_slice(&frame.payload[..take]);
            if idx == last_index {
                debug_assert_eq!(buf.len(), total_size);
                out.push(std::mem::take(&mut buf));
                expected_next = None;
            } else {
                expected_next = Some(idx + 1);
            }
        } else {
            // Out-of-order / duplicate — drop assembly state.
            buf.clear();
            expected_next = None;
        }
    }

    out
}

/// Convenience for the params path (`first_index = 0`).
#[must_use]
pub fn reassemble_two_fragment(
    stream: &[u8],
    expected_report_id: u8,
    total_size: usize,
) -> Vec<Vec<u8>> {
    assert!(
        total_size <= 2 * FRAGMENT_PAYLOAD,
        "reassemble_two_fragment: total_size {total_size} > 2*FRAGMENT_PAYLOAD; \
         use reassemble_fragments for larger payloads"
    );
    reassemble_fragments(stream, expected_report_id, total_size, 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_valid_frame() {
        let mut bytes = [0u8; FRAME_SIZE];
        bytes[0] = REPORT_ID_PARAM;
        bytes[1] = 0;
        bytes[2] = 0xaa;
        bytes[63] = 0xff;
        let f = Frame::parse(&bytes, REPORT_ID_PARAM, 1).unwrap();
        assert_eq!(f.report_id, REPORT_ID_PARAM);
        assert_eq!(f.fragment_index, 0);
        assert_eq!(f.payload[0], 0xaa);
        assert_eq!(f.payload[FRAGMENT_PAYLOAD - 1], 0xff);
    }

    #[test]
    fn rejects_wrong_report_id() {
        let mut bytes = [0u8; FRAME_SIZE];
        bytes[0] = REPORT_ID_CONFIG_IN;
        bytes[1] = 0;
        let err = Frame::parse(&bytes, REPORT_ID_PARAM, 1).unwrap_err();
        assert!(matches!(
            err,
            DecodeError::UnexpectedReportId {
                expected: REPORT_ID_PARAM,
                got: REPORT_ID_CONFIG_IN,
            }
        ));
    }

    #[test]
    fn rejects_out_of_range_fragment() {
        let mut bytes = [0u8; FRAME_SIZE];
        bytes[0] = REPORT_ID_PARAM;
        bytes[1] = 2;
        let err = Frame::parse(&bytes, REPORT_ID_PARAM, 1).unwrap_err();
        assert!(matches!(
            err,
            DecodeError::UnexpectedFragmentIndex { got: 2, max: 1 }
        ));
    }

    #[test]
    fn reassembles_two_fragments() {
        let mut stream = vec![0u8; 2 * FRAME_SIZE];
        // First frame: report id, fragment 0, payload = 0xa* pattern
        stream[0] = REPORT_ID_PARAM;
        stream[1] = 0;
        for (i, b) in stream[2..FRAME_SIZE].iter_mut().enumerate() {
            *b = 0xa0 + (i % 16) as u8;
        }
        // Second frame: report id, fragment 1, payload = 0xb* pattern
        stream[FRAME_SIZE] = REPORT_ID_PARAM;
        stream[FRAME_SIZE + 1] = 1;
        for (i, b) in stream[FRAME_SIZE + 2..2 * FRAME_SIZE]
            .iter_mut()
            .enumerate()
        {
            *b = 0xb0 + (i % 16) as u8;
        }

        let reports = reassemble_two_fragment(&stream, REPORT_ID_PARAM, 72);
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].len(), 72);
        assert_eq!(reports[0][0], 0xa0); // first byte of fragment 0 payload
        assert_eq!(reports[0][61], 0xa0 + (61 % 16) as u8);
        // bytes 62..72 come from fragment 1 payload bytes 0..10
        assert_eq!(reports[0][62], 0xb0);
        assert_eq!(reports[0][71], 0xb0 + 9);
    }

    #[test]
    fn orphan_fragment_1_dropped() {
        let mut stream = vec![0u8; FRAME_SIZE];
        stream[0] = REPORT_ID_PARAM;
        stream[1] = 1;
        let reports = reassemble_two_fragment(&stream, REPORT_ID_PARAM, 72);
        assert!(reports.is_empty());
    }

    #[test]
    fn double_fragment_0_restarts() {
        // fragment 0, fragment 0 (restarted), fragment 1: produces 1 report
        let mut stream = vec![0u8; 3 * FRAME_SIZE];
        for i in 0..3 {
            stream[i * FRAME_SIZE] = REPORT_ID_PARAM;
        }
        stream[1] = 0;
        stream[FRAME_SIZE + 1] = 0;
        stream[2 * FRAME_SIZE + 1] = 1;
        // Mark second fragment 0's payload distinguishably
        stream[FRAME_SIZE + 2] = 0xee;
        let reports = reassemble_two_fragment(&stream, REPORT_ID_PARAM, 72);
        assert_eq!(reports.len(), 1);
        // The second fragment 0 is the one that paired with fragment 1.
        assert_eq!(reports[0][0], 0xee);
    }
}
