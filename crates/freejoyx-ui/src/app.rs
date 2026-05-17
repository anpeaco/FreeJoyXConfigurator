//! App glue: spawn worker, build window, route events.
//!
//! Thread model: the Slint event loop runs on the main thread; the
//! `freejoyx_device` worker runs on its own thread and exchanges
//! [`Command`]s and [`DeviceEvent`]s via `mpsc` channels. We poll the
//! event receiver from a Slint timer (100 ms cadence) so model
//! updates stay on the UI thread without needing `invoke_from_event_loop`
//! ceremony around every push.
//!
//! Live state owned here:
//!
//! - `connected_device` — the most recent `Connected` payload, or
//!   `None` while discovering.
//! - `last_config` — the most recent `dev_config_t` (whatever
//!   `ReadConfig` returned + the user's pin edits since). `WriteConfig`
//!   sends a clone of this.
//! - `pin_model` — Slint `VecModel<PinRow>` mirroring the 30-slot pin
//!   array; rebuilt every time `last_config.pins` or the conflict set
//!   changes.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use freejoyx_core::domain::{validate_pins, Board, PinConflict, PinFunction};
use freejoyx_core::persist::{load_from_file, save_to_file};
use freejoyx_core::wire::{DeviceConfig, USED_PINS_NUM};
use freejoyx_device::{spawn_for_serial, Command, DeviceCandidate, DeviceEvent, DeviceHandle};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::{AppWindow, PinRow};

/// State the UI mutates outside of Slint's reactivity. Held inside a
/// `RefCell` so callbacks (UI thread) and the event-poll tick (also UI
/// thread) can both borrow mutably without `Mutex` overhead.
struct State {
    handle: DeviceHandle,
    connected_device: Option<DeviceCandidate>,
    last_config: Option<Box<DeviceConfig>>,
    board: Board,
    status: String,
}

impl State {
    fn new(handle: DeviceHandle) -> Self {
        Self {
            handle,
            connected_device: None,
            last_config: None,
            board: Board::Bluepill,
            status: "waiting for device…".to_string(),
        }
    }
}

/// Launch the UI. Blocks the calling thread until the window closes.
///
/// `serial_filter` (if set) restricts the worker to a specific board
/// by HID serial number — same semantics as the CLI's `--serial` flag.
///
/// # Errors
///
/// Propagates any failure from `slint::run_event_loop`. The worker
/// thread cannot fail to spawn (the panic on OS refusal is fatal at
/// app startup either way).
pub fn run(serial_filter: Option<String>) -> Result<(), slint::PlatformError> {
    let (handle, rx) = spawn_for_serial(serial_filter);

    let window = AppWindow::new()?;
    let pin_model: Rc<VecModel<PinRow>> = Rc::new(VecModel::default());
    window.set_pins(ModelRc::from(pin_model.clone()));

    // Populate function-label list once — it's static.
    let labels: Vec<SharedString> = PinFunction::all()
        .map(|f| SharedString::from(f.label()))
        .collect();
    window.set_function_labels(ModelRc::from(Rc::new(VecModel::from(labels))));

    let state = Rc::new(RefCell::new(State::new(handle)));

    // Wire UI callbacks.
    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_read_clicked(move || {
            let mut s = state.borrow_mut();
            if s.handle.send(Command::ReadConfig).is_err() {
                s.status = "worker exited; cannot read".to_string();
            } else {
                s.status = "reading config…".to_string();
            }
            if let Some(w) = weak.upgrade() {
                w.set_status_text(SharedString::from(s.status.clone()));
            }
        });
    }
    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_write_clicked(move || {
            let mut s = state.borrow_mut();
            let Some(cfg) = s.last_config.clone() else {
                s.status = "no config loaded yet — read first".to_string();
                if let Some(w) = weak.upgrade() {
                    w.set_status_text(SharedString::from(s.status.clone()));
                }
                return;
            };
            if s.handle.send(Command::WriteConfig(cfg)).is_err() {
                s.status = "worker exited; cannot write".to_string();
            } else {
                s.status = "writing config…".to_string();
            }
            if let Some(w) = weak.upgrade() {
                w.set_status_text(SharedString::from(s.status.clone()));
            }
        });
    }
    wire_save_callback(&window, &state);
    wire_load_callback(&window, &state, &pin_model);
    {
        let state = state.clone();
        let weak = window.as_weak();
        let pin_model = pin_model.clone();
        window.on_pin_changed(move |slot, function_index| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            let Ok(function_index) = usize::try_from(function_index) else {
                return;
            };
            if slot >= USED_PINS_NUM {
                return;
            }
            let Some(new_fn) = PinFunction::all().nth(function_index) else {
                return;
            };
            let (board, cfg_clone) = {
                let mut s = state.borrow_mut();
                let board = s.board;
                let Some(cfg) = s.last_config.as_mut() else {
                    return;
                };
                cfg.pins[slot] = new_fn.to_i8();
                (board, cfg.clone())
            };
            refresh_pin_model(&pin_model, &cfg_clone, board);
            if let Some(w) = weak.upgrade() {
                w.set_can_write(true);
            }
        });
    }

    // Poll the worker's event channel from a Slint timer. 100 ms is
    // brisk enough for connect/disconnect responsiveness without
    // burning CPU on noop ticks.
    let timer = slint::Timer::default();
    {
        let state = state.clone();
        let weak = window.as_weak();
        let pin_model_for_timer = pin_model.clone();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                let Some(window) = weak.upgrade() else { return };
                pump_events(&state, &window, &pin_model_for_timer, &rx);
            },
        );
    }

    window.run()
}

fn pump_events(
    state: &Rc<RefCell<State>>,
    window: &AppWindow,
    pin_model: &Rc<VecModel<PinRow>>,
    rx: &std::sync::mpsc::Receiver<DeviceEvent>,
) {
    while let Ok(evt) = rx.try_recv() {
        match evt {
            DeviceEvent::Connected(c) => {
                let mut s = state.borrow_mut();
                s.connected_device = Some(c.clone());
                s.status = "connected — click Read Config to load".to_string();
                window.set_connected(true);
                window.set_device_summary(SharedString::from(c.display_summary()));
                window.set_status_text(SharedString::from(s.status.clone()));
                window.set_can_read(true);
                window.set_can_write(false);
            }
            DeviceEvent::Disconnected => {
                let mut s = state.borrow_mut();
                s.connected_device = None;
                s.status = "disconnected — waiting for device".to_string();
                window.set_connected(false);
                window.set_device_summary(SharedString::from("no device"));
                window.set_status_text(SharedString::from(s.status.clone()));
                window.set_can_read(false);
                window.set_can_write(false);
            }
            DeviceEvent::ParamsTick(_) => {
                // Slice 5 doesn't surface live params. Slice 6's Axes
                // tab will bind to this.
            }
            DeviceEvent::ConfigReceived(cfg) => {
                let mut s = state.borrow_mut();
                s.board = Board::from_id(cfg.board_id);
                s.last_config = Some(cfg.clone());
                s.status = format!(
                    "config received — fw 0x{:04x}, board {:?}, {} pins assigned",
                    cfg.firmware_version,
                    s.board,
                    cfg.pins.iter().filter(|&&p| p != 0).count()
                );
                let board = s.board;
                window.set_status_text(SharedString::from(s.status.clone()));
                window.set_can_write(true);
                window.set_can_save(true);
                drop(s);
                refresh_pin_model(pin_model, &cfg, board);
            }
            DeviceEvent::ConfigSent => {
                let mut s = state.borrow_mut();
                s.status = "config written successfully".to_string();
                window.set_status_text(SharedString::from(s.status.clone()));
            }
            DeviceEvent::ConfigError(msg) => {
                let mut s = state.borrow_mut();
                s.status = format!("config error: {msg}");
                window.set_status_text(SharedString::from(s.status.clone()));
            }
            DeviceEvent::Error(msg) => {
                let mut s = state.borrow_mut();
                s.status = format!("transport: {msg}");
                window.set_status_text(SharedString::from(s.status.clone()));
            }
        }
    }
}

fn wire_save_callback(window: &AppWindow, state: &Rc<RefCell<State>>) {
    let state = state.clone();
    let weak = window.as_weak();
    window.on_save_clicked(move || {
        let cfg = state.borrow().last_config.clone();
        let Some(cfg) = cfg else {
            set_status(&weak, &state, "nothing to save — read or load first");
            return;
        };
        let Some(path) = rfd::FileDialog::new()
            .add_filter("FreeJoyX RON", &["ron"])
            .set_file_name("freejoyx-config.ron")
            .save_file()
        else {
            return;
        };
        match save_to_file(&cfg, &path) {
            Ok(()) => set_status(&weak, &state, &format!("saved {}", path.display())),
            Err(e) => set_status(&weak, &state, &format!("save failed: {e}")),
        }
    });
}

fn wire_load_callback(
    window: &AppWindow,
    state: &Rc<RefCell<State>>,
    pin_model: &Rc<VecModel<PinRow>>,
) {
    let state = state.clone();
    let weak = window.as_weak();
    let pin_model = pin_model.clone();
    window.on_load_clicked(move || {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("FreeJoyX RON", &["ron"])
            .pick_file()
        else {
            return;
        };
        match load_from_file(&path) {
            Ok(cfg) => {
                let (board, cfg_for_model) = {
                    let mut s = state.borrow_mut();
                    let board = Board::from_id(cfg.board_id);
                    s.board = board;
                    let cfg_box = Box::new(cfg);
                    s.last_config = Some(cfg_box.clone());
                    (board, cfg_box)
                };
                refresh_pin_model(&pin_model, &cfg_for_model, board);
                if let Some(w) = weak.upgrade() {
                    w.set_can_save(true);
                    let connected = w.get_connected();
                    w.set_can_write(connected);
                }
                set_status(&weak, &state, &format!("loaded {}", path.display()));
            }
            Err(e) => set_status(&weak, &state, &format!("load failed: {e}")),
        }
    });
}

fn set_status(weak: &slint::Weak<AppWindow>, state: &Rc<RefCell<State>>, msg: &str) {
    {
        let mut s = state.borrow_mut();
        s.status = msg.to_string();
    }
    if let Some(w) = weak.upgrade() {
        w.set_status_text(SharedString::from(msg));
    }
}

fn refresh_pin_model(model: &Rc<VecModel<PinRow>>, cfg: &DeviceConfig, board: Board) {
    let conflicts: Vec<PinConflict> = validate_pins(&cfg.pins);
    // Drop everything; rebuild. 30 rows are cheap.
    while model.row_count() > 0 {
        model.remove(0);
    }
    let func_list: Vec<PinFunction> = PinFunction::all().collect();
    for (slot, &raw) in cfg.pins.iter().enumerate() {
        let function = PinFunction::from_i8(raw).unwrap_or(PinFunction::NotUsed);
        let function_index = func_list.iter().position(|f| *f == function).unwrap_or(0);
        let conflict_msg = conflicts
            .iter()
            .find(|c| c.slot == slot)
            .map(|c| c.kind.short_label())
            .unwrap_or_default();
        model.push(PinRow {
            pin_name: SharedString::from(board.pin_name(slot)),
            function_label: SharedString::from(function.label()),
            function_index: i32::try_from(function_index).unwrap_or(0),
            conflict_msg: SharedString::from(conflict_msg),
        });
    }
}
