//! Device worker thread + channel API.
//!
//! Wraps the synchronous Slice 2 transport in the threading model
//! Port.md §3 calls for: a single thread owns the `hidapi` handle and
//! exchanges messages with the UI via `std::sync::mpsc` channels.
//! Slint's `invoke_from_event_loop` plays the role of Qt's
//! `Qt::QueuedConnection` on the consumer side.
//!
//! The worker has two responsibilities:
//!
//! 1. **Discovery loop.** When no device is open it polls
//!    [`enumerate`] every [`DISCOVERY_POLL`] and opens the first
//!    `FreeJoyX`-style candidate that appears. Mirrors the Qt
//!    configurator's `hid_enumerate` cadence (`hiddevice.cpp:64`).
//! 2. **Read loop.** While a device is open it calls
//!    [`Device::read_params_blocking`] with a short timeout so it can
//!    interleave [`Command`] processing between reads. Each successful
//!    decode produces a [`DeviceEvent::ParamsTick`]. Read failures fall
//!    back to discovery after emitting [`DeviceEvent::Disconnected`].
//!
//! Command surface in this slice is intentionally minimal —
//! [`Command::Shutdown`] is the only verb. Slice 5+ adds
//! `ReadConfig` / `WriteConfig` / `SetLeds` and the corresponding
//! `ConfigReceived` / `ConfigSent` events.

use std::sync::mpsc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use freejoyx_core::wire::{DeviceConfig, ParamsReport};
use tracing::{debug, info, warn};

use crate::transport::{enumerate, Device, DeviceCandidate};
use crate::TransportError;

/// How often the worker re-enumerates while no device is open. Matches
/// the Qt configurator's `hid_enumerate` cadence in `hiddevice.cpp:64`.
const DISCOVERY_POLL: Duration = Duration::from_millis(600);

/// Per-read timeout while a device is connected. Short so the worker
/// can poll its command channel between reads.
const READ_TIMEOUT: Duration = Duration::from_millis(200);

/// Cadence for refreshing the firmware's params subscription. The
/// firmware stops pushing params reports if it doesn't see a renewal
/// within ~5 seconds. Matches `hiddevice.cpp:360` (the 5000 ms timer).
const PARAMS_REQUEST_REFRESH: Duration = Duration::from_secs(5);

/// Commands the UI sends to the worker.
#[derive(Debug)]
pub enum Command {
    /// Stop the read/discovery loop and return from the worker thread.
    Shutdown,
    /// Request a full `dev_config_t` read from the connected device.
    /// The worker pauses params reading, runs the fragment exchange,
    /// and emits [`DeviceEvent::ConfigReceived`] on success or
    /// [`DeviceEvent::ConfigError`] on failure.
    ReadConfig,
    /// Write a `dev_config_t` to the connected device. The worker
    /// pauses params reading, runs the fragment exchange, and emits
    /// [`DeviceEvent::ConfigSent`] on success or
    /// [`DeviceEvent::ConfigError`] on failure.
    WriteConfig(Box<DeviceConfig>),
}

/// Events the worker pushes to the UI. The receiver side is consumed
/// either directly (CLI / tests) or fed into Slint via
/// `invoke_from_event_loop`.
#[derive(Debug)]
pub enum DeviceEvent {
    /// A `FreeJoyX`-style device opened successfully.
    Connected(DeviceCandidate),
    /// The currently-open device dropped (unplug, read error). The
    /// worker resumes discovery after emitting this.
    Disconnected,
    /// One params report decoded from the connected device.
    ParamsTick(ParamsReport),
    /// A full `dev_config_t` was read back in response to a
    /// [`Command::ReadConfig`].
    ConfigReceived(Box<DeviceConfig>),
    /// A `dev_config_t` was written successfully in response to a
    /// [`Command::WriteConfig`]. The device typically re-enumerates
    /// after this, so a `Disconnected` will follow shortly.
    ConfigSent,
    /// A config read or write failed. Carries a human-readable detail
    /// for surfacing in the UI.
    ConfigError(String),
    /// Recoverable transport failure surfaced for diagnostics. The
    /// worker keeps running after these.
    Error(String),
}

/// Handle returned by [`spawn`]. Owns the command sender and the
/// thread's join handle so the caller can request shutdown cleanly.
pub struct DeviceHandle {
    cmd_tx: mpsc::Sender<Command>,
    join: Option<JoinHandle<()>>,
}

impl DeviceHandle {
    /// Send a command to the worker. Returns `Err` only if the worker
    /// thread has already exited.
    ///
    /// # Errors
    ///
    /// [`mpsc::SendError`] propagated from the underlying channel when
    /// the worker has joined or panicked.
    pub fn send(&self, cmd: Command) -> Result<(), mpsc::SendError<Command>> {
        self.cmd_tx.send(cmd)
    }

    /// Ask the worker to exit and wait for it. Best-effort: if the
    /// worker already exited, `Shutdown` is dropped and `join` still
    /// runs to completion.
    ///
    /// # Errors
    ///
    /// Propagates a thread-panic payload from [`JoinHandle::join`]; the
    /// payload's concrete type is `Box<dyn Any + Send>` per std.
    pub fn shutdown(mut self) -> thread::Result<()> {
        // Best-effort send; the worker may have already returned.
        let _ = self.cmd_tx.send(Command::Shutdown);
        match self.join.take() {
            Some(h) => h.join(),
            None => Ok(()),
        }
    }
}

impl Drop for DeviceHandle {
    /// Drop semantics: signal shutdown and join. Mirrors the Qt
    /// destructor's behaviour where the worker thread is asked to stop
    /// before the owning object goes out of scope.
    fn drop(&mut self) {
        if let Some(h) = self.join.take() {
            let _ = self.cmd_tx.send(Command::Shutdown);
            let _ = h.join();
        }
    }
}

/// Spawn the device worker. Returns the handle and an event receiver.
///
/// The worker thread runs until either `Command::Shutdown` is received
/// or both the handle and the receiver are dropped (the latter only
/// affects sends; the worker still polls its command channel).
///
/// # Panics
///
/// Panics if the OS refuses to spawn the worker thread — practically
/// this only happens under fork bombs or hard rlimit caps. The
/// configurator can't make progress without the worker so a panic at
/// startup is preferable to silently dead state.
#[must_use]
pub fn spawn() -> (DeviceHandle, mpsc::Receiver<DeviceEvent>) {
    spawn_for_serial(None)
}

/// Like [`spawn`], but the worker only opens devices whose HID serial
/// number matches `serial` (case-insensitive). Useful on multi-board
/// benches where the first-enumerated device is arbitrary. `None`
/// falls back to first-enumerated.
///
/// # Panics
///
/// Same as [`spawn`].
#[must_use]
pub fn spawn_for_serial(serial: Option<String>) -> (DeviceHandle, mpsc::Receiver<DeviceEvent>) {
    let (cmd_tx, cmd_rx) = mpsc::channel::<Command>();
    let (evt_tx, evt_rx) = mpsc::channel::<DeviceEvent>();

    let join = thread::Builder::new()
        .name("freejoyx-device-worker".to_string())
        .spawn(move || run_worker(&cmd_rx, &evt_tx, serial.as_deref()))
        .expect("OS refused to spawn worker thread");

    (
        DeviceHandle {
            cmd_tx,
            join: Some(join),
        },
        evt_rx,
    )
}

/// Worker thread main loop. Returns when [`Command::Shutdown`] is
/// observed or the event receiver is dropped. `serial_filter` (if set)
/// restricts the worker to devices with that HID serial number.
fn run_worker(
    cmd_rx: &mpsc::Receiver<Command>,
    evt_tx: &mpsc::Sender<DeviceEvent>,
    serial_filter: Option<&str>,
) {
    info!("device worker started (serial filter: {:?})", serial_filter);
    loop {
        if poll_shutdown(cmd_rx, evt_tx) {
            info!("device worker shutting down");
            return;
        }

        let Some(candidate) = find_first_candidate(serial_filter) else {
            // No device — wait and re-poll. Sleep in small slices so
            // shutdown stays responsive.
            if sleep_with_shutdown(cmd_rx, evt_tx, DISCOVERY_POLL) {
                return;
            }
            continue;
        };

        let mut device = match Device::open(&candidate.path) {
            Ok(d) => d,
            Err(e) => {
                emit_error(evt_tx, format!("open {} failed: {e}", candidate.path));
                if sleep_with_shutdown(cmd_rx, evt_tx, DISCOVERY_POLL) {
                    return;
                }
                continue;
            }
        };

        if evt_tx
            .send(DeviceEvent::Connected(candidate.clone()))
            .is_err()
        {
            debug!("event receiver gone; worker exiting");
            return;
        }
        info!("connected: {}", candidate.display_summary());

        // Kick the firmware to start pushing params. Without this the
        // read loop sits idle even though the device is open.
        if let Err(e) = device.request_params() {
            emit_error(evt_tx, format!("params subscribe failed: {e}"));
            let _ = evt_tx.send(DeviceEvent::Disconnected);
            continue;
        }

        match pump_until_disconnect(&mut device, cmd_rx, evt_tx) {
            PumpOutcome::Shutdown => return,
            PumpOutcome::Disconnected => {
                let _ = evt_tx.send(DeviceEvent::Disconnected);
                info!("disconnected; resuming discovery");
            }
        }
    }
}

/// Outcome of the pumping loop.
enum PumpOutcome {
    /// Shutdown was requested; the worker should exit entirely.
    Shutdown,
    /// The device went away (read error / unplug); resume discovery.
    Disconnected,
}

/// Read loop while a device is open. Interleaves params reads with
/// command dispatch so [`Command::ReadConfig`] / [`Command::WriteConfig`]
/// can be serviced without dropping the params subscription.
fn pump_until_disconnect(
    device: &mut Device,
    cmd_rx: &mpsc::Receiver<Command>,
    evt_tx: &mpsc::Sender<DeviceEvent>,
) -> PumpOutcome {
    let mut next_refresh = std::time::Instant::now() + PARAMS_REQUEST_REFRESH;
    loop {
        match dispatch_pending_commands(device, cmd_rx, evt_tx) {
            CommandLoopResult::Continue => {}
            CommandLoopResult::Shutdown => return PumpOutcome::Shutdown,
            CommandLoopResult::Disconnect => return PumpOutcome::Disconnected,
        }
        if std::time::Instant::now() >= next_refresh {
            if let Err(e) = device.request_params() {
                warn!("params refresh failed, treating as disconnect: {e}");
                return PumpOutcome::Disconnected;
            }
            next_refresh = std::time::Instant::now() + PARAMS_REQUEST_REFRESH;
        }
        match device.read_params_blocking(READ_TIMEOUT) {
            Ok(report) => {
                if evt_tx.send(DeviceEvent::ParamsTick(report)).is_err() {
                    debug!("event receiver gone mid-pump; exiting");
                    return PumpOutcome::Shutdown;
                }
            }
            Err(TransportError::Timeout { .. }) => {
                // No report this slice — fall through to re-poll
                // commands. Treated as device-idle, not disconnect.
            }
            Err(e) => {
                warn!("read failure, treating as disconnect: {e}");
                return PumpOutcome::Disconnected;
            }
        }
    }
}

enum CommandLoopResult {
    /// No more pending commands; resume the read loop.
    Continue,
    /// `Shutdown` arrived — exit the worker entirely.
    Shutdown,
    /// A config command failed because the device dropped — fall back
    /// to discovery.
    Disconnect,
}

/// Drain all pending commands and run them inline. Config exchanges
/// pause params reading for their duration; on success the worker
/// re-subscribes so params resume cleanly.
fn dispatch_pending_commands(
    device: &mut Device,
    cmd_rx: &mpsc::Receiver<Command>,
    evt_tx: &mpsc::Sender<DeviceEvent>,
) -> CommandLoopResult {
    loop {
        match cmd_rx.try_recv() {
            Ok(Command::Shutdown) => return CommandLoopResult::Shutdown,
            Ok(Command::ReadConfig) => match device.read_config() {
                Ok(cfg) => {
                    let _ = evt_tx.send(DeviceEvent::ConfigReceived(cfg));
                    if let Err(e) = device.request_params() {
                        warn!("re-subscribe after read_config failed: {e}");
                        return CommandLoopResult::Disconnect;
                    }
                }
                Err(e) => {
                    let _ = evt_tx.send(DeviceEvent::ConfigError(format!("read: {e}")));
                    if matches!(e, TransportError::Read(_)) {
                        return CommandLoopResult::Disconnect;
                    }
                }
            },
            Ok(Command::WriteConfig(cfg)) => match device.write_config(&cfg) {
                Ok(()) => {
                    let _ = evt_tx.send(DeviceEvent::ConfigSent);
                    // Device often re-enumerates after a write; the
                    // next read or refresh will catch it. Re-subscribe
                    // optimistically.
                    if let Err(e) = device.request_params() {
                        debug!("re-subscribe after write_config failed (expected on re-enum): {e}");
                        return CommandLoopResult::Disconnect;
                    }
                }
                Err(e) => {
                    let _ = evt_tx.send(DeviceEvent::ConfigError(format!("write: {e}")));
                    if matches!(e, TransportError::Read(_)) {
                        return CommandLoopResult::Disconnect;
                    }
                }
            },
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => {
                return CommandLoopResult::Continue;
            }
        }
    }
}

/// Non-blocking check for [`Command::Shutdown`] on the command channel.
/// Used in the discovery loop where there's no device to dispatch
/// config commands against; non-shutdown commands are dropped with a
/// [`DeviceEvent::ConfigError`] so the UI sees its request didn't
/// land instead of silently waiting forever. Returns `true` if
/// shutdown was observed.
fn poll_shutdown(cmd_rx: &mpsc::Receiver<Command>, evt_tx: &mpsc::Sender<DeviceEvent>) -> bool {
    loop {
        match cmd_rx.try_recv() {
            Ok(Command::Shutdown) => return true,
            Ok(Command::ReadConfig | Command::WriteConfig(_)) => {
                let _ = evt_tx.send(DeviceEvent::ConfigError(
                    "no device connected — command dropped".to_string(),
                ));
            }
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => return false,
        }
    }
}

/// Sleep for at most `total`, but wake immediately if a
/// [`Command::Shutdown`] arrives. Non-shutdown commands that arrive
/// while no device is open are dropped silently — the discovery phase
/// has no way to service them.
fn sleep_with_shutdown(
    cmd_rx: &mpsc::Receiver<Command>,
    evt_tx: &mpsc::Sender<DeviceEvent>,
    total: Duration,
) -> bool {
    let deadline = std::time::Instant::now() + total;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return false;
        }
        let slice = remaining.min(Duration::from_millis(50));
        match cmd_rx.recv_timeout(slice) {
            Ok(Command::Shutdown) => return true,
            Ok(Command::ReadConfig | Command::WriteConfig(_)) => {
                let _ = evt_tx.send(DeviceEvent::ConfigError(
                    "no device connected — command dropped".to_string(),
                ));
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return false,
        }
    }
}

/// First candidate from [`enumerate`] matching the optional serial
/// filter, or `None` if no match. Errors are logged at debug level
/// and treated the same as "no device".
fn find_first_candidate(serial_filter: Option<&str>) -> Option<DeviceCandidate> {
    let mut list = match enumerate() {
        Ok(v) => v,
        Err(e) => {
            debug!("enumerate failed: {e}");
            return None;
        }
    };
    if let Some(filter) = serial_filter {
        list.retain(|c| {
            c.serial_number
                .as_deref()
                .is_some_and(|s| s.eq_ignore_ascii_case(filter))
        });
    }
    if list.is_empty() {
        None
    } else {
        Some(list.remove(0))
    }
}

fn emit_error(evt_tx: &mpsc::Sender<DeviceEvent>, msg: String) {
    warn!("worker error: {msg}");
    let _ = evt_tx.send(DeviceEvent::Error(msg));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spawning + immediate shutdown joins cleanly even with no device
    /// present. Exercises the discovery loop's shutdown responsiveness
    /// without needing real hardware.
    #[test]
    fn spawn_then_shutdown_joins() {
        let (handle, _evt) = spawn();
        // Give the worker a moment to enter its first discovery sleep
        // so shutdown_with_sleep actually races a `recv_timeout`.
        thread::sleep(Duration::from_millis(50));
        handle.shutdown().expect("worker panicked");
    }

    /// Dropping the handle without explicit shutdown still terminates
    /// the worker (via the `Drop` impl).
    #[test]
    fn drop_handle_terminates_worker() {
        let (handle, _evt) = spawn();
        thread::sleep(Duration::from_millis(50));
        drop(handle);
        // If the worker leaked, this test would hang under cargo test's
        // default timeout. No explicit assertion — clean exit is the
        // success criterion.
    }
}
