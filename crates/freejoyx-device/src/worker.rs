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
use std::time::{Duration, Instant};

use freejoyx_core::wire::{DeviceConfig, ParamsReport};
use tracing::{debug, info, warn};

use crate::subscription::{ParamsSubscription, RenewalOutcome, RenewalReason};
use crate::transport::{enumerate, Device, DeviceCandidate, Transport};
use crate::TransportError;

/// How often the worker re-enumerates while no device is open. Matches
/// the Qt configurator's `hid_enumerate` cadence in `hiddevice.cpp:64`.
const DISCOVERY_POLL: Duration = Duration::from_millis(600);

/// Per-read timeout while a device is connected. Short so the worker
/// can poll its command channel between reads.
const READ_TIMEOUT: Duration = Duration::from_millis(200);

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
    /// Re-enumerate HID devices and emit the candidate list via
    /// [`DeviceEvent::Candidates`]. Used by the toolbar device picker
    /// to refresh its dropdown.
    Enumerate,
    /// Switch the worker's serial filter. The currently-open device
    /// (if any) is dropped and discovery restarts; the first candidate
    /// matching `serial` is opened next. `None` falls back to first-
    /// enumerated.
    Reopen { serial: Option<String> },
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
    /// Latest HID candidate list, emitted in response to
    /// [`Command::Enumerate`]. Drives the toolbar device picker.
    Candidates(Vec<DeviceCandidate>),
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
/// restricts the worker to devices with that HID serial number. The
/// filter is mutable across the lifetime of the worker so
/// [`Command::Reopen`] can switch devices without a restart.
fn run_worker(
    cmd_rx: &mpsc::Receiver<Command>,
    evt_tx: &mpsc::Sender<DeviceEvent>,
    serial_filter: Option<&str>,
) {
    let mut serial_filter: Option<String> = serial_filter.map(str::to_owned);
    info!("device worker started (serial filter: {:?})", serial_filter);
    loop {
        match poll_idle_commands(cmd_rx, evt_tx, serial_filter.as_deref()) {
            IdleAction::Continue => {}
            IdleAction::Shutdown => {
                info!("device worker shutting down");
                return;
            }
            IdleAction::Reopen(new_filter) => {
                info!("reopen requested while idle; new filter: {new_filter:?}");
                serial_filter = new_filter;
                continue;
            }
        }

        let Some(candidate) = find_first_candidate(serial_filter.as_deref()) else {
            // No device — wait and re-poll. Sleep in small slices so
            // shutdown stays responsive.
            match sleep_with_shutdown(cmd_rx, evt_tx, DISCOVERY_POLL, serial_filter.as_deref()) {
                IdleAction::Continue => {}
                IdleAction::Shutdown => return,
                IdleAction::Reopen(new_filter) => {
                    info!("reopen during sleep; new filter: {new_filter:?}");
                    serial_filter = new_filter;
                }
            }
            continue;
        };

        let mut device = match Device::open(&candidate.path) {
            Ok(d) => d,
            Err(e) => {
                emit_error(evt_tx, format!("open {} failed: {e}", candidate.path));
                match sleep_with_shutdown(cmd_rx, evt_tx, DISCOVERY_POLL, serial_filter.as_deref())
                {
                    IdleAction::Continue => {}
                    IdleAction::Shutdown => return,
                    IdleAction::Reopen(new_filter) => serial_filter = new_filter,
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
        let mut subscription = match ParamsSubscription::subscribe(Instant::now(), &mut device) {
            Ok(s) => s,
            Err(e) => {
                emit_error(evt_tx, format!("params subscribe failed: {e}"));
                let _ = evt_tx.send(DeviceEvent::Disconnected);
                continue;
            }
        };

        match pump_until_disconnect(&mut device, &mut subscription, cmd_rx, evt_tx) {
            PumpOutcome::Shutdown => return,
            PumpOutcome::Disconnected => {
                let _ = evt_tx.send(DeviceEvent::Disconnected);
                info!("disconnected; resuming discovery");
            }
            PumpOutcome::ReopenRequested(new_filter) => {
                let _ = evt_tx.send(DeviceEvent::Disconnected);
                serial_filter = new_filter;
                info!("reopen requested; new filter: {serial_filter:?}");
            }
        }
    }
}

/// Outcome of the pumping loop.
#[derive(Debug)]
enum PumpOutcome {
    /// Shutdown was requested; the worker should exit entirely.
    Shutdown,
    /// The device went away (read error / unplug); resume discovery.
    Disconnected,
    /// The UI asked to switch devices. The current device is dropped,
    /// the worker swaps in the new serial filter, and discovery
    /// restarts.
    ReopenRequested(Option<String>),
}

/// What an idle-state command poll just observed.
enum IdleAction {
    /// Keep iterating the outer discovery loop.
    Continue,
    /// `Shutdown` arrived — exit the worker entirely.
    Shutdown,
    /// Caller should swap in this serial filter and restart discovery.
    Reopen(Option<String>),
}

/// Read loop while a device is open. Interleaves params reads with
/// command dispatch so [`Command::ReadConfig`] / [`Command::WriteConfig`]
/// can be serviced without dropping the params subscription.
///
/// Takes `device: &mut dyn Transport` (not `&mut Device`) so the pump's
/// behaviour can be tested against a scriptable fake — see the
/// `pump_tests` module at the bottom of this file.
fn pump_until_disconnect(
    device: &mut dyn Transport,
    subscription: &mut ParamsSubscription,
    cmd_rx: &mpsc::Receiver<Command>,
    evt_tx: &mpsc::Sender<DeviceEvent>,
) -> PumpOutcome {
    loop {
        match dispatch_pending_commands(device, subscription, cmd_rx, evt_tx) {
            CommandLoopResult::Continue => {}
            CommandLoopResult::Shutdown => return PumpOutcome::Shutdown,
            CommandLoopResult::Disconnect => return PumpOutcome::Disconnected,
            CommandLoopResult::Reopen(new_filter) => {
                return PumpOutcome::ReopenRequested(new_filter)
            }
        }
        if let RenewalOutcome::Lost(e) =
            subscription.renew(Instant::now(), RenewalReason::Periodic, device)
        {
            warn!("params refresh failed, treating as disconnect: {e}");
            return PumpOutcome::Disconnected;
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
    /// `Reopen` arrived; drop the current device and swap in this
    /// serial filter.
    Reopen(Option<String>),
}

/// Drain all pending commands and run them inline. Config exchanges
/// pause params reading for their duration; on success the worker
/// re-subscribes so params resume cleanly.
fn dispatch_pending_commands(
    device: &mut dyn Transport,
    subscription: &mut ParamsSubscription,
    cmd_rx: &mpsc::Receiver<Command>,
    evt_tx: &mpsc::Sender<DeviceEvent>,
) -> CommandLoopResult {
    loop {
        match cmd_rx.try_recv() {
            Ok(Command::Shutdown) => return CommandLoopResult::Shutdown,
            Ok(Command::ReadConfig) => match device.read_config() {
                Ok(cfg) => {
                    let _ = evt_tx.send(DeviceEvent::ConfigReceived(cfg));
                    if let RenewalOutcome::Lost(e) =
                        subscription.renew(Instant::now(), RenewalReason::AfterRead, device)
                    {
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
                    // subscription module swallows the expected
                    // renewal failure and the next periodic tick
                    // retries if the device is still present.
                    let _ =
                        subscription.renew(Instant::now(), RenewalReason::AfterWrite, device);
                }
                Err(e) => {
                    let _ = evt_tx.send(DeviceEvent::ConfigError(format!("write: {e}")));
                    if matches!(e, TransportError::Read(_)) {
                        return CommandLoopResult::Disconnect;
                    }
                }
            },
            Ok(Command::Enumerate) => {
                let _ = evt_tx.send(DeviceEvent::Candidates(enumerate_or_empty()));
            }
            Ok(Command::Reopen { serial }) => {
                return CommandLoopResult::Reopen(serial);
            }
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => {
                return CommandLoopResult::Continue;
            }
        }
    }
}

/// Convenience wrapper around [`enumerate`] that returns an empty
/// `Vec` on transport error rather than propagating. Used by
/// `Command::Enumerate` so the UI always gets a list back, even if
/// it's empty.
fn enumerate_or_empty() -> Vec<DeviceCandidate> {
    enumerate().unwrap_or_else(|e| {
        debug!("enumerate for picker failed: {e}");
        Vec::new()
    })
}

/// Non-blocking command drain for the idle (no-device) phase.
/// [`Command::Enumerate`] is serviced inline (always — it's the
/// picker's primary mechanism for refreshing its dropdown). Config
/// commands have nowhere to go and are dropped with a friendly
/// [`DeviceEvent::ConfigError`]. Returns the appropriate
/// [`IdleAction`] for the caller to act on.
fn poll_idle_commands(
    cmd_rx: &mpsc::Receiver<Command>,
    evt_tx: &mpsc::Sender<DeviceEvent>,
    current_filter: Option<&str>,
) -> IdleAction {
    loop {
        match cmd_rx.try_recv() {
            Ok(Command::Shutdown) => return IdleAction::Shutdown,
            Ok(Command::ReadConfig | Command::WriteConfig(_)) => {
                let _ = evt_tx.send(DeviceEvent::ConfigError(
                    "no device connected — command dropped".to_string(),
                ));
            }
            Ok(Command::Enumerate) => {
                let _ = evt_tx.send(DeviceEvent::Candidates(enumerate_or_empty()));
            }
            Ok(Command::Reopen { serial }) => {
                if serial.as_deref() == current_filter {
                    debug!("reopen to same filter; ignoring");
                } else {
                    return IdleAction::Reopen(serial);
                }
            }
            Err(mpsc::TryRecvError::Empty | mpsc::TryRecvError::Disconnected) => {
                return IdleAction::Continue;
            }
        }
    }
}

/// Sleep for at most `total`, but wake immediately on any actionable
/// command. Behaves the same way as [`poll_idle_commands`] for
/// non-timeout returns.
fn sleep_with_shutdown(
    cmd_rx: &mpsc::Receiver<Command>,
    evt_tx: &mpsc::Sender<DeviceEvent>,
    total: Duration,
    current_filter: Option<&str>,
) -> IdleAction {
    let deadline = std::time::Instant::now() + total;
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return IdleAction::Continue;
        }
        let slice = remaining.min(Duration::from_millis(50));
        match cmd_rx.recv_timeout(slice) {
            Ok(Command::Shutdown) => return IdleAction::Shutdown,
            Ok(Command::ReadConfig | Command::WriteConfig(_)) => {
                let _ = evt_tx.send(DeviceEvent::ConfigError(
                    "no device connected — command dropped".to_string(),
                ));
            }
            Ok(Command::Enumerate) => {
                let _ = evt_tx.send(DeviceEvent::Candidates(enumerate_or_empty()));
            }
            Ok(Command::Reopen { serial }) => {
                if serial.as_deref() == current_filter {
                    debug!("reopen to same filter; ignoring");
                } else {
                    return IdleAction::Reopen(serial);
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return IdleAction::Continue,
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

#[cfg(test)]
mod pump_tests {
    //! Tests for the pump loop in isolation from `hidapi`. Drives
    //! `pump_until_disconnect` against a scriptable
    //! [`super::Transport`] fake — verifies command interleaving,
    //! disconnect handling, and the ReadConfig / WriteConfig
    //! round-trip without a real device.
    //!
    //! What's *not* covered here: device enumeration + `Device::open`.
    //! Those still need a real `hidapi` context; if testing reconnect
    //! becomes important later we add a `trait DeviceFactory` as a
    //! second-layer seam. For now the open-device-and-pump half is the
    //! testable surface.
    use super::*;
    use crate::transport::Transport;
    use freejoyx_core::wire::config::DEV_CONFIG_SIZE;
    use freejoyx_core::wire::params::PARAMS_REPORT_SIZE;
    use freejoyx_core::wire::DeviceConfig;
    use std::sync::Mutex;

    /// One scripted reply the fake will deliver on the *next*
    /// `read_params_blocking` call. The fake walks this queue in order;
    /// once empty, calls block out to a `Timeout`.
    enum NextRead {
        /// Hand back this params report on the next read.
        Params(ParamsReport),
        /// Return `TransportError::Timeout` — caller should treat as
        /// idle, not disconnect.
        Timeout,
        /// Return a `TransportError::Read` — caller should treat as
        /// disconnect.
        Disconnect,
    }

    /// Scriptable [`Transport`] for pump tests. Records every method
    /// call in `events` so assertions can pin call ordering, and
    /// delivers `read_params_blocking` results from `pending_reads`
    /// in FIFO order.
    #[derive(Default)]
    struct FakeTransport {
        pending_reads: Mutex<std::collections::VecDeque<NextRead>>,
        events: Mutex<Vec<FakeCall>>,
        read_config_result: Mutex<Option<Result<Box<DeviceConfig>, TransportError>>>,
        write_config_result: Mutex<Option<Result<(), TransportError>>>,
    }

    #[derive(Debug, PartialEq, Eq)]
    enum FakeCall {
        RequestParams,
        ReadParams,
        ReadConfig,
        WriteConfig,
    }

    impl FakeTransport {
        fn push_read(&self, r: NextRead) {
            self.pending_reads.lock().unwrap().push_back(r);
        }
        fn set_read_config(&self, r: Result<Box<DeviceConfig>, TransportError>) {
            *self.read_config_result.lock().unwrap() = Some(r);
        }
        fn set_write_config(&self, r: Result<(), TransportError>) {
            *self.write_config_result.lock().unwrap() = Some(r);
        }
        fn events(&self) -> Vec<FakeCall> {
            std::mem::take(&mut *self.events.lock().unwrap())
        }
    }

    impl Transport for FakeTransport {
        fn request_params(&self) -> Result<(), TransportError> {
            self.events.lock().unwrap().push(FakeCall::RequestParams);
            Ok(())
        }
        fn read_params_blocking(
            &mut self,
            _timeout: Duration,
        ) -> Result<ParamsReport, TransportError> {
            self.events.lock().unwrap().push(FakeCall::ReadParams);
            match self.pending_reads.lock().unwrap().pop_front() {
                Some(NextRead::Params(p)) => Ok(p),
                Some(NextRead::Timeout) | None => Err(TransportError::Timeout { ms: 0 }),
                Some(NextRead::Disconnect) => Err(TransportError::Read(
                    hidapi::HidError::HidApiError {
                        message: "fake disconnect".into(),
                    },
                )),
            }
        }
        fn read_config(&self) -> Result<Box<DeviceConfig>, TransportError> {
            self.events.lock().unwrap().push(FakeCall::ReadConfig);
            self.read_config_result
                .lock()
                .unwrap()
                .take()
                .unwrap_or_else(|| Ok(Box::new(empty_config())))
        }
        fn write_config(&self, _cfg: &DeviceConfig) -> Result<(), TransportError> {
            self.events.lock().unwrap().push(FakeCall::WriteConfig);
            self.write_config_result.lock().unwrap().take().unwrap_or(Ok(()))
        }
    }

    fn empty_config() -> DeviceConfig {
        DeviceConfig::decode(&[0u8; DEV_CONFIG_SIZE]).unwrap()
    }

    fn empty_params() -> ParamsReport {
        ParamsReport::decode(&[0u8; PARAMS_REPORT_SIZE]).unwrap()
    }

    /// Drive `pump_until_disconnect` in a background thread so the
    /// test can feed it commands and reads. Returns the worker thread's
    /// JoinHandle (carrying the `PumpOutcome`) plus the cmd sender and
    /// evt receiver.
    fn spawn_pump(
        mut fake: FakeTransport,
    ) -> (
        std::thread::JoinHandle<PumpOutcome>,
        mpsc::Sender<Command>,
        mpsc::Receiver<DeviceEvent>,
    ) {
        let (cmd_tx, cmd_rx) = mpsc::channel();
        let (evt_tx, evt_rx) = mpsc::channel();
        let join = std::thread::spawn(move || {
            let mut sub =
                ParamsSubscription::subscribe(Instant::now(), &mut fake).expect("fake subscribe");
            pump_until_disconnect(&mut fake, &mut sub, &cmd_rx, &evt_tx)
        });
        (join, cmd_tx, evt_rx)
    }

    /// Drain events received within `dur`. Returns once the channel
    /// goes quiet or the deadline passes. Used to assert "we got these
    /// events" without timing-dependent sleeps.
    fn drain_events(rx: &mpsc::Receiver<DeviceEvent>, dur: Duration) -> Vec<DeviceEvent> {
        let mut out = Vec::new();
        let deadline = std::time::Instant::now() + dur;
        while let Ok(e) = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())) {
            out.push(e);
        }
        out
    }

    #[test]
    fn shutdown_command_exits_pump() {
        let fake = FakeTransport::default();
        let (join, cmd_tx, _evt) = spawn_pump(fake);
        cmd_tx.send(Command::Shutdown).unwrap();
        let outcome = join.join().expect("pump panicked");
        assert!(matches!(outcome, PumpOutcome::Shutdown));
    }

    #[test]
    fn params_read_emits_tick_event() {
        let fake = FakeTransport::default();
        fake.push_read(NextRead::Params(empty_params()));
        let (join, cmd_tx, evt_rx) = spawn_pump(fake);

        let events = drain_events(&evt_rx, Duration::from_millis(200));
        cmd_tx.send(Command::Shutdown).unwrap();
        join.join().unwrap();

        assert!(events.iter().any(|e| matches!(e, DeviceEvent::ParamsTick(_))));
    }

    #[test]
    fn read_failure_treated_as_disconnect() {
        let fake = FakeTransport::default();
        fake.push_read(NextRead::Disconnect);
        let (join, _cmd_tx, evt_rx) = spawn_pump(fake);
        let outcome = join.join().expect("pump panicked");
        assert!(matches!(outcome, PumpOutcome::Disconnected));
        // Drop the event sender side via the receiver going out of scope
        // is fine — we don't assert on emitted events here.
        let _ = evt_rx;
    }

    #[test]
    fn read_config_command_roundtrips() {
        let fake = FakeTransport::default();
        fake.set_read_config(Ok(Box::new(empty_config())));
        let (join, cmd_tx, evt_rx) = spawn_pump(fake);

        cmd_tx.send(Command::ReadConfig).unwrap();
        let events = drain_events(&evt_rx, Duration::from_millis(300));
        cmd_tx.send(Command::Shutdown).unwrap();
        join.join().unwrap();

        assert!(
            events
                .iter()
                .any(|e| matches!(e, DeviceEvent::ConfigReceived(_))),
            "expected ConfigReceived; got {events:?}",
        );
    }

    #[test]
    fn write_config_command_roundtrips() {
        let fake = FakeTransport::default();
        fake.set_write_config(Ok(()));
        let (join, cmd_tx, evt_rx) = spawn_pump(fake);

        cmd_tx
            .send(Command::WriteConfig(Box::new(empty_config())))
            .unwrap();
        let events = drain_events(&evt_rx, Duration::from_millis(300));
        cmd_tx.send(Command::Shutdown).unwrap();
        join.join().unwrap();

        assert!(events.iter().any(|e| matches!(e, DeviceEvent::ConfigSent)));
    }

    #[test]
    fn config_read_failure_surfaces_as_config_error() {
        let fake = FakeTransport::default();
        fake.set_read_config(Err(TransportError::Timeout { ms: 5000 }));
        let (join, cmd_tx, evt_rx) = spawn_pump(fake);

        cmd_tx.send(Command::ReadConfig).unwrap();
        let events = drain_events(&evt_rx, Duration::from_millis(300));
        cmd_tx.send(Command::Shutdown).unwrap();
        join.join().unwrap();

        assert!(
            events
                .iter()
                .any(|e| matches!(e, DeviceEvent::ConfigError(msg) if msg.contains("read:"))),
            "expected ConfigError(read: ...); got {events:?}",
        );
    }

    #[test]
    fn reopen_command_returns_reopen_outcome() {
        let fake = FakeTransport::default();
        let (join, cmd_tx, _evt) = spawn_pump(fake);
        cmd_tx
            .send(Command::Reopen {
                serial: Some("ABC123".into()),
            })
            .unwrap();
        let outcome = join.join().expect("pump panicked");
        match outcome {
            PumpOutcome::ReopenRequested(Some(s)) => assert_eq!(s, "ABC123"),
            other => panic!("expected ReopenRequested(Some); got {other:?}"),
        }
    }
}
