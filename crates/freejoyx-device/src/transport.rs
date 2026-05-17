//! HID transport — enumerate, open, read params.
//!
//! Slice 2 scope: synchronous, single-device, foreground reads. The
//! `Device::read_params_blocking` API blocks the caller until one logical
//! `ParamsReport` is reassembled from two HID frames (or an error
//! occurs). Slice 4 will wrap this in a worker thread + `mpsc` channels
//! per Port.md §3 "Threading model".
//!
//! ## Device discovery
//!
//! Mirrors the Qt configurator's filter in
//! `FreeJoyXConfiguratorQt/src/hiddevice.cpp:108-160`:
//!
//! - Manufacturer string matches `"FreeJoyX"` (fork) or `"FreeJoy"`
//!   (upstream — kept for forward-compat enumeration; the firmware-version
//!   check downstream rejects upstream-only firmware per Port.md §1.2).
//! - Interface number is `1` (F103 custom HID, the configurator
//!   protocol), `0` (F411 single-interface), or `-1` (Windows raw HID
//!   path for F411 since it isn't a composite device).
//! - Dedup: when both interface 0 and interface 1 entries appear for the
//!   same serial number (F103 layout), drop the interface 0 entry. F411
//!   has only an interface-0 (or -1) entry, which survives.

use std::time::{Duration, Instant};

use freejoyx_core::wire::{
    fragment_count, reassemble_fragments, DeviceConfig, ParamsReport, DEV_CONFIG_SIZE,
    FRAGMENT_PAYLOAD, FRAME_SIZE, PARAMS_REPORT_SIZE, REPORT_ID_CONFIG_IN, REPORT_ID_PARAM,
};
use hidapi::{HidApi, HidDevice};

use crate::error::TransportError;

/// `REPORT_ID_CONFIG_OUT` from `vendored/common_defines.h`.
const REPORT_ID_CONFIG_OUT: u8 = 4;

/// Worst-case time the firmware should take to respond to either a
/// config-in fragment request or a config-out ACK. Matches the Qt
/// configurator's 5-second deadline (`hiddevice.cpp:494`, `:641`).
const CONFIG_EXCHANGE_TIMEOUT: Duration = Duration::from_secs(5);

/// Per-fragment read timeout during a config exchange. Matches Qt's
/// 100 ms slice (`hiddevice.cpp:498`, `:645`). Short enough to stay
/// responsive; long enough that one slow USB frame doesn't trip the
/// resend-on-silence path.
const CONFIG_READ_SLICE: Duration = Duration::from_millis(100);

const MANUFACTURER_STRINGS: &[&str] = &["FreeJoyX", "FreeJoy"];

/// A discovered HID device that matches the `FreeJoyX`
/// manufacturer/interface filter. Holds enough identifying information
/// to display in a picker and to open the device by path.
#[derive(Debug, Clone)]
pub struct DeviceCandidate {
    pub path: String,
    pub manufacturer_string: String,
    pub product_string: String,
    pub serial_number: Option<String>,
    pub vendor_id: u16,
    pub product_id: u16,
    /// HID interface number. `-1` on Windows for non-composite devices.
    pub interface_number: i32,
}

impl DeviceCandidate {
    /// One-line display for CLI listings.
    #[must_use]
    pub fn display_summary(&self) -> String {
        format!(
            "{} — {} (VID 0x{:04x} / PID 0x{:04x}, if {}, serial {})",
            self.manufacturer_string,
            self.product_string,
            self.vendor_id,
            self.product_id,
            self.interface_number,
            self.serial_number.as_deref().unwrap_or("?"),
        )
    }
}

/// Enumerate `FreeJoyX`-style HID devices currently attached.
///
/// Returns the deduplicated candidate list per the Qt configurator's
/// dedup rule (interface 1 wins over interface 0 when both share a
/// serial).
///
/// # Errors
///
/// Returns [`TransportError::HidInit`] if the underlying `hidapi`
/// context cannot be created (typically a missing system permission
/// on Linux or a backend init failure).
pub fn enumerate() -> Result<Vec<DeviceCandidate>, TransportError> {
    let api = HidApi::new().map_err(TransportError::HidInit)?;
    Ok(enumerate_with_api(&api))
}

fn enumerate_with_api(api: &HidApi) -> Vec<DeviceCandidate> {
    let mut raw: Vec<DeviceCandidate> = Vec::new();
    for info in api.device_list() {
        let manufacturer = info.manufacturer_string().unwrap_or("");
        if !MANUFACTURER_STRINGS.contains(&manufacturer) {
            continue;
        }
        let iface = info.interface_number();
        if !matches!(iface, -1..=1) {
            continue;
        }
        raw.push(DeviceCandidate {
            path: info.path().to_string_lossy().into_owned(),
            manufacturer_string: manufacturer.to_string(),
            product_string: info.product_string().unwrap_or("").to_string(),
            serial_number: info.serial_number().map(str::to_string),
            vendor_id: info.vendor_id(),
            product_id: info.product_id(),
            interface_number: iface,
        });
    }

    // Dedup: drop interface-0 entries whose serial also has an interface-1
    // entry. Matches `hiddevice.cpp:134-160`.
    let serials_with_if1: std::collections::HashSet<String> = raw
        .iter()
        .filter(|c| c.interface_number == 1)
        .filter_map(|c| c.serial_number.clone())
        .collect();

    raw.retain(|c| {
        if c.interface_number != 0 {
            return true;
        }
        match &c.serial_number {
            Some(s) => !serials_with_if1.contains(s),
            None => true,
        }
    });

    raw
}

/// An open HID handle to a FreeJoyX-style device, with a stash for
/// straggler fragments seen while assembling the next logical params
/// report.
pub struct Device {
    handle: HidDevice,
    pending: Vec<u8>,
}

impl Device {
    /// Open the device at `path` (typically obtained from a
    /// [`DeviceCandidate`]).
    ///
    /// # Errors
    ///
    /// Returns [`TransportError::HidInit`] if the `hidapi` context
    /// fails to initialise, or [`TransportError::Open`] if the path is
    /// unreachable (device unplugged, missing permission, NUL in the
    /// path string).
    pub fn open(path: &str) -> Result<Self, TransportError> {
        let api = HidApi::new().map_err(TransportError::HidInit)?;
        Self::open_with_api(&api, path)
    }

    fn open_with_api(api: &HidApi, path: &str) -> Result<Self, TransportError> {
        let cstr = std::ffi::CString::new(path).map_err(|_| TransportError::Open {
            path: path.to_string(),
            source: hidapi::HidError::HidApiError {
                message: "path contains interior NUL".into(),
            },
        })?;
        let handle = api
            .open_path(&cstr)
            .map_err(|source| TransportError::Open {
                path: path.to_string(),
                source,
            })?;
        Ok(Self {
            handle,
            pending: Vec::with_capacity(FRAME_SIZE * 2),
        })
    }

    /// Kick the firmware to start (or keep) pushing params reports.
    ///
    /// Mirrors `FreeJoyXConfiguratorQt/src/hiddevice.cpp:345` (open
    /// kickoff) and `hiddevice.cpp:361` (5-second refresh). The
    /// firmware treats a write of `[REPORT_ID_PARAM]` on the
    /// configurator's HID OUT endpoint as a subscription request and
    /// starts pushing params reports on the IN endpoint at the
    /// configured `exchange_period_ms`. Without this, the read loop
    /// observes nothing.
    ///
    /// # Errors
    ///
    /// [`TransportError::Read`] (re-used for write failures since
    /// `hidapi` surfaces them with the same `HidError` type) when the
    /// handle is closed or the OS rejects the write.
    pub fn request_params(&self) -> Result<(), TransportError> {
        let buf = [REPORT_ID_PARAM];
        self.handle
            .write(&buf)
            .map(|_| ())
            .map_err(TransportError::Read)
    }

    /// Read the full 1580-byte `dev_config_t` from the device, fragment
    /// by fragment, and decode it into a [`DeviceConfig`].
    ///
    /// Mirrors the Qt configurator's read loop in
    /// `FreeJoyXConfiguratorQt/src/hiddevice.cpp:467-591`:
    ///
    /// - Write `[REPORT_ID_CONFIG_IN, idx]` (2 bytes) to request
    ///   fragment `idx` (starting at 1).
    /// - Read frames; on a matching `[CONFIG_IN, idx]` frame, copy 62
    ///   bytes of payload (or `last_cfg_size` on the last fragment),
    ///   increment `idx`, request the next.
    /// - Stop after `idx > fragment_count(DEV_CONFIG_SIZE)` (26 today).
    ///
    /// # Errors
    ///
    /// - [`TransportError::Read`] — `hidapi` write or read failure.
    /// - [`TransportError::Timeout`] — the device did not deliver every
    ///   fragment within [`CONFIG_EXCHANGE_TIMEOUT`].
    /// - [`TransportError::Decode`] — assembled bytes failed
    ///   `DeviceConfig::decode` (should not happen against a healthy
    ///   firmware of a supported version).
    ///
    /// # Panics
    ///
    /// Panics only if `fragment_count(DEV_CONFIG_SIZE) > u8::MAX` (the
    /// wire protocol uses a u8 fragment index). Given `DEV_CONFIG_SIZE`
    /// is fixed at 1580 and the fragment size at 62 the count is 26;
    /// this is a compile-time invariant, not a runtime risk.
    pub fn read_config(&self) -> Result<Box<DeviceConfig>, TransportError> {
        let cfg_count = fragment_count(DEV_CONFIG_SIZE);
        let last_cfg_size = DEV_CONFIG_SIZE - (cfg_count - 1) * FRAGMENT_PAYLOAD;
        let cfg_count_u8 = u8::try_from(cfg_count).expect("fragment count fits in u8");
        let mut assembled = vec![0u8; DEV_CONFIG_SIZE];
        let mut next_idx: u8 = 1;

        // First request.
        self.write_config_in_request(next_idx)?;

        let deadline = Instant::now() + CONFIG_EXCHANGE_TIMEOUT;
        let mut frame = [0u8; FRAME_SIZE];
        loop {
            if Instant::now() >= deadline {
                return Err(TransportError::Timeout {
                    ms: CONFIG_EXCHANGE_TIMEOUT
                        .as_millis()
                        .try_into()
                        .unwrap_or(i32::MAX),
                });
            }
            let n = self
                .handle
                .read_timeout(&mut frame, slice_ms(deadline)?)
                .map_err(TransportError::Read)?;
            if n == 0 {
                continue;
            }
            if n != FRAME_SIZE {
                return Err(TransportError::ShortRead {
                    got: n,
                    expected: FRAME_SIZE,
                });
            }
            if frame[0] != REPORT_ID_CONFIG_IN || frame[1] != next_idx {
                // Stray frame (params interleave, retransmit lag) —
                // drop and keep waiting.
                continue;
            }
            let take = if next_idx == cfg_count_u8 {
                last_cfg_size
            } else {
                FRAGMENT_PAYLOAD
            };
            let offset = (next_idx as usize - 1) * FRAGMENT_PAYLOAD;
            assembled[offset..offset + take].copy_from_slice(&frame[2..2 + take]);

            if next_idx == cfg_count_u8 {
                break;
            }
            next_idx += 1;
            self.write_config_in_request(next_idx)?;
        }

        let array: [u8; DEV_CONFIG_SIZE] = assembled
            .try_into()
            .expect("assembled length is DEV_CONFIG_SIZE by construction");
        Ok(Box::new(DeviceConfig::decode(&array)?))
    }

    fn write_config_in_request(&self, idx: u8) -> Result<(), TransportError> {
        let buf = [REPORT_ID_CONFIG_IN, idx];
        self.handle
            .write(&buf)
            .map(|_| ())
            .map_err(TransportError::Read)
    }

    /// Write a [`DeviceConfig`] back to the device. Mirrors the Qt
    /// configurator's write loop in `hiddevice.cpp:594-690`:
    ///
    /// - Write `[REPORT_ID_CONFIG_OUT, 0]` (64 bytes, fragment 0 acts
    ///   as the "start" request).
    /// - Wait for the device to echo back `[CONFIG_OUT, idx+1]` as the
    ///   ACK for the previous fragment.
    /// - Send the next fragment with payload bytes from `cfg.encode()`.
    /// - Stop after ACK index reaches `fragment_count(DEV_CONFIG_SIZE)`
    ///   (26 today).
    ///
    /// # Errors
    ///
    /// Same shape as [`Device::read_config`].
    ///
    /// # Panics
    ///
    /// Same as [`Device::read_config`] — the fragment count is a fixed
    /// 26 today and `u8::try_from` cannot fail for that value.
    pub fn write_config(&self, cfg: &DeviceConfig) -> Result<(), TransportError> {
        let bytes = cfg.encode();
        let cfg_count = fragment_count(DEV_CONFIG_SIZE);
        let last_cfg_size = DEV_CONFIG_SIZE - (cfg_count - 1) * FRAGMENT_PAYLOAD;
        let cfg_count_u8 = u8::try_from(cfg_count).expect("fragment count fits in u8");

        // Initial start frame: report id + index 0 + zero payload.
        let mut out = [0u8; FRAME_SIZE];
        out[0] = REPORT_ID_CONFIG_OUT;
        out[1] = 0;
        self.handle.write(&out).map_err(TransportError::Read)?;

        let deadline = Instant::now() + CONFIG_EXCHANGE_TIMEOUT;
        let mut last_sent: u8 = 0;
        let mut frame = [0u8; FRAME_SIZE];

        loop {
            if Instant::now() >= deadline {
                return Err(TransportError::Timeout {
                    ms: CONFIG_EXCHANGE_TIMEOUT
                        .as_millis()
                        .try_into()
                        .unwrap_or(i32::MAX),
                });
            }
            let n = self
                .handle
                .read_timeout(&mut frame, slice_ms(deadline)?)
                .map_err(TransportError::Read)?;
            if n == 0 {
                continue;
            }
            if n != FRAME_SIZE {
                return Err(TransportError::ShortRead {
                    got: n,
                    expected: FRAME_SIZE,
                });
            }
            if frame[0] != REPORT_ID_CONFIG_OUT || frame[1] != last_sent + 1 {
                continue;
            }
            let next_idx = last_sent + 1;
            let take = if next_idx == cfg_count_u8 {
                last_cfg_size
            } else {
                FRAGMENT_PAYLOAD
            };
            let offset = (next_idx as usize - 1) * FRAGMENT_PAYLOAD;

            let mut out = [0u8; FRAME_SIZE];
            out[0] = REPORT_ID_CONFIG_OUT;
            out[1] = next_idx;
            out[2..2 + take].copy_from_slice(&bytes[offset..offset + take]);
            self.handle.write(&out).map_err(TransportError::Read)?;
            last_sent = next_idx;

            if next_idx == cfg_count_u8 {
                return Ok(());
            }
        }
    }

    /// Open the first device returned by [`enumerate`]. Convenience for
    /// the Slice 2 CLI; later slices will let the user pick from a list.
    ///
    /// # Errors
    ///
    /// Same as [`Device::open`] plus [`TransportError::NoDevice`] when
    /// the enumeration returns no matching candidates.
    pub fn open_first() -> Result<Self, TransportError> {
        let api = HidApi::new().map_err(TransportError::HidInit)?;
        let first = enumerate_with_api(&api)
            .into_iter()
            .next()
            .ok_or(TransportError::NoDevice)?;
        Self::open_with_api(&api, &first.path)
    }

    /// Read the next [`ParamsReport`] (blocks up to `timeout`). Frames
    /// for other report IDs are silently discarded — they belong to
    /// other host listeners (joy report, etc).
    ///
    /// The configurator only consumes params here; the firmware pushes
    /// one fragmented report roughly every `exchange_period_ms` (1 ms
    /// default on F103), so `timeout = 1s` is generous in normal
    /// operation and points at a real problem if it fires.
    ///
    /// # Errors
    ///
    /// - [`TransportError::Timeout`] — no full params report assembled
    ///   within `timeout`.
    /// - [`TransportError::Read`] — `hidapi` read failure (typically
    ///   device disconnected).
    /// - [`TransportError::ShortRead`] — the HID stack returned fewer
    ///   bytes than a full frame; almost certainly a driver bug or
    ///   wrong-class device.
    /// - [`TransportError::Decode`] — assembled bytes failed
    ///   `ParamsReport::decode`; should not happen against a healthy
    ///   firmware.
    ///
    /// # Panics
    ///
    /// Panics only if the internal reassembler returns a payload of
    /// the wrong length — that would be a bug in
    /// [`freejoyx_core::wire::reassemble_fragments`], not user input.
    pub fn read_params_blocking(
        &mut self,
        timeout: Duration,
    ) -> Result<ParamsReport, TransportError> {
        let deadline = std::time::Instant::now() + timeout;
        let mut frame = [0u8; FRAME_SIZE];

        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                return Err(TransportError::Timeout {
                    ms: timeout.as_millis().try_into().unwrap_or(i32::MAX),
                });
            }

            // hidapi takes a millisecond timeout; clamp to i32::MAX.
            let ms = i32::try_from(remaining.as_millis()).unwrap_or(i32::MAX);
            let n = self
                .handle
                .read_timeout(&mut frame, ms)
                .map_err(TransportError::Read)?;

            if n == 0 {
                continue;
            }
            if n != FRAME_SIZE {
                return Err(TransportError::ShortRead {
                    got: n,
                    expected: FRAME_SIZE,
                });
            }
            if frame[0] != REPORT_ID_PARAM {
                continue;
            }

            // Reset state when fragment 0 arrives; append otherwise.
            if frame[1] == 0 {
                self.pending.clear();
            }
            self.pending.extend_from_slice(&frame);

            let need = fragment_count(PARAMS_REPORT_SIZE);
            if self.pending.len() >= need * FRAME_SIZE {
                let reports = reassemble_fragments(
                    &self.pending[..need * FRAME_SIZE],
                    REPORT_ID_PARAM,
                    PARAMS_REPORT_SIZE,
                    0,
                );
                self.pending.clear();
                if let Some(bytes) = reports.into_iter().next() {
                    let array: [u8; PARAMS_REPORT_SIZE] = bytes
                        .try_into()
                        .expect("reassemble_fragments guarantees PARAMS_REPORT_SIZE length");
                    return Ok(ParamsReport::decode(&array)?);
                }
                // Fragments arrived but didn't assemble cleanly (e.g.
                // out-of-order). Loop and wait for the next valid pair.
            }
        }
    }
}

/// Compute the next `hidapi` read timeout slice as an `i32` millisecond
/// count, clamped to [`CONFIG_READ_SLICE`] and bounded by the
/// `deadline`. Returns [`TransportError::Timeout`] if the deadline has
/// already passed.
fn slice_ms(deadline: Instant) -> Result<i32, TransportError> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err(TransportError::Timeout {
            ms: CONFIG_EXCHANGE_TIMEOUT
                .as_millis()
                .try_into()
                .unwrap_or(i32::MAX),
        });
    }
    let slice = remaining.min(CONFIG_READ_SLICE);
    Ok(i32::try_from(slice.as_millis()).unwrap_or(i32::MAX))
}
