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

use freejoyx_core::domain::{validate_pins, AxisFilter, Board, PinConflict, PinFunction};
use freejoyx_core::persist::{load_from_file, save_to_file};
use freejoyx_core::wire::{DeviceConfig, ParamsReport, MAX_AXIS_NUM, USED_PINS_NUM};
use freejoyx_device::{spawn_for_serial, Command, DeviceCandidate, DeviceEvent, DeviceHandle};
use slint::{ComponentHandle, Model, ModelRc, SharedString, VecModel};

use crate::buttons as buttons_glue;
use crate::encoders as encoders_glue;
use crate::{
    AppWindow, AxisRow, ButtonRow, EncoderRow, FastEncoderRow, PinRow, ShiftRegRow, ShiftSlot,
    TimerField,
};

/// State the UI mutates outside of Slint's reactivity. Held inside a
/// `RefCell` so callbacks (UI thread) and the event-poll tick (also UI
/// thread) can both borrow mutably without `Mutex` overhead.
pub(crate) struct State {
    handle: DeviceHandle,
    connected_device: Option<DeviceCandidate>,
    pub(crate) last_config: Option<Box<DeviceConfig>>,
    board: Board,
    status: String,
    /// Most recent params snapshot. Drives the live raw/out columns on
    /// the Axes tab and the per-button live dots; `None` until the
    /// worker has shipped at least one `ParamsTick`.
    pub(crate) last_params: Option<ParamsReport>,
}

impl State {
    fn new(handle: DeviceHandle) -> Self {
        Self {
            handle,
            connected_device: None,
            last_config: None,
            board: Board::Bluepill,
            status: "waiting for device…".to_string(),
            last_params: None,
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

    let axis_model: Rc<VecModel<AxisRow>> = Rc::new(VecModel::default());
    window.set_axes(ModelRc::from(axis_model.clone()));

    let button_model: Rc<VecModel<ButtonRow>> = Rc::new(VecModel::default());
    window.set_buttons(ModelRc::from(button_model.clone()));

    let shift_model: Rc<VecModel<ShiftSlot>> = Rc::new(VecModel::default());
    window.set_shifts(ModelRc::from(shift_model.clone()));

    let timer_model: Rc<VecModel<TimerField>> = Rc::new(VecModel::default());
    window.set_timers(ModelRc::from(timer_model.clone()));

    let soft_encoder_model: Rc<VecModel<EncoderRow>> = Rc::new(VecModel::default());
    window.set_soft_encoders(ModelRc::from(soft_encoder_model.clone()));

    let fast_encoder_model: Rc<VecModel<FastEncoderRow>> = Rc::new(VecModel::default());
    window.set_fast_encoders(ModelRc::from(fast_encoder_model.clone()));

    let shift_reg_model: Rc<VecModel<ShiftRegRow>> = Rc::new(VecModel::default());
    window.set_shift_registers(ModelRc::from(shift_reg_model.clone()));

    // Populate function-label list once — it's static.
    let labels: Vec<SharedString> = PinFunction::all()
        .map(|f| SharedString::from(f.label()))
        .collect();
    window.set_function_labels(ModelRc::from(Rc::new(VecModel::from(labels))));

    let state = Rc::new(RefCell::new(State::new(handle)));

    wire_read_callback(&window, &state);
    wire_write_callback(&window, &state);
    wire_save_callback(&window, &state);
    wire_load_callback(
        &window,
        &state,
        &LoadSinks {
            pin_model: &pin_model,
            axis_model: &axis_model,
            button_model: &button_model,
            shift_model: &shift_model,
            timer_model: &timer_model,
            soft_encoder_model: &soft_encoder_model,
            fast_encoder_model: &fast_encoder_model,
            shift_reg_model: &shift_reg_model,
        },
    );
    wire_axis_callbacks(&window, &state, &axis_model);
    wire_pin_callback(&window, &state, &pin_model);
    buttons_glue::wire_callbacks(&window, &state, &button_model, &shift_model, &timer_model);
    encoders_glue::wire_callbacks(
        &window,
        &state,
        &soft_encoder_model,
        &fast_encoder_model,
        &shift_reg_model,
    );

    // Poll the worker's event channel from a Slint timer. 100 ms is
    // brisk enough for connect/disconnect responsiveness without
    // burning CPU on noop ticks.
    let timer = slint::Timer::default();
    {
        let state = state.clone();
        let weak = window.as_weak();
        let pin_model_for_timer = pin_model.clone();
        let axis_model_for_timer = axis_model.clone();
        let button_model_for_timer = button_model.clone();
        let shift_model_for_timer = shift_model.clone();
        let timer_model_for_timer = timer_model.clone();
        let soft_encoder_model_for_timer = soft_encoder_model.clone();
        let fast_encoder_model_for_timer = fast_encoder_model.clone();
        let shift_reg_model_for_timer = shift_reg_model.clone();
        timer.start(
            slint::TimerMode::Repeated,
            Duration::from_millis(100),
            move || {
                let Some(window) = weak.upgrade() else { return };
                pump_events(
                    &state,
                    &window,
                    &EventSinks {
                        pin_model: &pin_model_for_timer,
                        axis_model: &axis_model_for_timer,
                        button_model: &button_model_for_timer,
                        shift_model: &shift_model_for_timer,
                        timer_model: &timer_model_for_timer,
                        soft_encoder_model: &soft_encoder_model_for_timer,
                        fast_encoder_model: &fast_encoder_model_for_timer,
                        shift_reg_model: &shift_reg_model_for_timer,
                    },
                    &rx,
                );
            },
        );
    }

    window.run()
}

/// Bundle of Slint models that `pump_events` may need to refresh in
/// response to a worker event. Grouped into a single struct to keep
/// the function signature from growing one parameter per slice.
#[allow(clippy::struct_field_names)]
struct EventSinks<'a> {
    pin_model: &'a Rc<VecModel<PinRow>>,
    axis_model: &'a Rc<VecModel<AxisRow>>,
    button_model: &'a Rc<VecModel<ButtonRow>>,
    shift_model: &'a Rc<VecModel<ShiftSlot>>,
    timer_model: &'a Rc<VecModel<TimerField>>,
    soft_encoder_model: &'a Rc<VecModel<EncoderRow>>,
    fast_encoder_model: &'a Rc<VecModel<FastEncoderRow>>,
    shift_reg_model: &'a Rc<VecModel<ShiftRegRow>>,
}

/// Same shape as [`EventSinks`] but for the load-file callback. Lets
/// the callback take all eight models without an 11-parameter
/// function signature.
#[allow(clippy::struct_field_names)]
struct LoadSinks<'a> {
    pin_model: &'a Rc<VecModel<PinRow>>,
    axis_model: &'a Rc<VecModel<AxisRow>>,
    button_model: &'a Rc<VecModel<ButtonRow>>,
    shift_model: &'a Rc<VecModel<ShiftSlot>>,
    timer_model: &'a Rc<VecModel<TimerField>>,
    soft_encoder_model: &'a Rc<VecModel<EncoderRow>>,
    fast_encoder_model: &'a Rc<VecModel<FastEncoderRow>>,
    shift_reg_model: &'a Rc<VecModel<ShiftRegRow>>,
}

fn pump_events(
    state: &Rc<RefCell<State>>,
    window: &AppWindow,
    sinks: &EventSinks<'_>,
    rx: &std::sync::mpsc::Receiver<DeviceEvent>,
) {
    let pin_model = sinks.pin_model;
    let axis_model = sinks.axis_model;
    let button_model = sinks.button_model;
    let shift_model = sinks.shift_model;
    let timer_model = sinks.timer_model;
    let soft_encoder_model = sinks.soft_encoder_model;
    let fast_encoder_model = sinks.fast_encoder_model;
    let shift_reg_model = sinks.shift_reg_model;
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
            DeviceEvent::ParamsTick(p) => {
                let cfg_opt = {
                    let mut s = state.borrow_mut();
                    s.last_params = Some(p);
                    s.last_config.clone()
                };
                if let Some(cfg) = cfg_opt {
                    let params_snapshot = state.borrow().last_params.clone();
                    let live = params_snapshot
                        .as_ref()
                        .map(|p| (p.raw_axis_data, p.axis_data));
                    refresh_axis_model(axis_model, &cfg, live);
                    buttons_glue::refresh_button_model(
                        button_model,
                        &cfg,
                        params_snapshot.as_ref(),
                    );
                }
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
                let live = s
                    .last_params
                    .as_ref()
                    .map(|p| (p.raw_axis_data, p.axis_data));
                let params_for_buttons = s.last_params.clone();
                window.set_status_text(SharedString::from(s.status.clone()));
                window.set_can_write(true);
                window.set_can_save(true);
                drop(s);
                refresh_pin_model(pin_model, &cfg, board);
                refresh_axis_model(axis_model, &cfg, live);
                buttons_glue::refresh_button_model(button_model, &cfg, params_for_buttons.as_ref());
                buttons_glue::refresh_shift_model(shift_model, &cfg);
                buttons_glue::refresh_timer_model(timer_model, &cfg);
                encoders_glue::refresh_soft_encoder_model(soft_encoder_model, &cfg);
                encoders_glue::refresh_fast_encoder_model(fast_encoder_model, &cfg);
                encoders_glue::refresh_shift_reg_model(shift_reg_model, &cfg);
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

fn wire_read_callback(window: &AppWindow, state: &Rc<RefCell<State>>) {
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

fn wire_write_callback(window: &AppWindow, state: &Rc<RefCell<State>>) {
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

fn wire_pin_callback(
    window: &AppWindow,
    state: &Rc<RefCell<State>>,
    pin_model: &Rc<VecModel<PinRow>>,
) {
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

fn wire_load_callback(window: &AppWindow, state: &Rc<RefCell<State>>, sinks: &LoadSinks<'_>) {
    let state = state.clone();
    let weak = window.as_weak();
    let pin_model = sinks.pin_model.clone();
    let axis_model = sinks.axis_model.clone();
    let button_model = sinks.button_model.clone();
    let shift_model = sinks.shift_model.clone();
    let timer_model = sinks.timer_model.clone();
    let soft_encoder_model = sinks.soft_encoder_model.clone();
    let fast_encoder_model = sinks.fast_encoder_model.clone();
    let shift_reg_model = sinks.shift_reg_model.clone();
    window.on_load_clicked(move || {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("FreeJoyX RON", &["ron"])
            .pick_file()
        else {
            return;
        };
        match load_from_file(&path) {
            Ok(cfg) => {
                let (board, cfg_for_model, live, params_for_buttons) = {
                    let mut s = state.borrow_mut();
                    let board = Board::from_id(cfg.board_id);
                    s.board = board;
                    let cfg_box = Box::new(cfg);
                    s.last_config = Some(cfg_box.clone());
                    let live = s
                        .last_params
                        .as_ref()
                        .map(|p| (p.raw_axis_data, p.axis_data));
                    let params = s.last_params.clone();
                    (board, cfg_box, live, params)
                };
                refresh_pin_model(&pin_model, &cfg_for_model, board);
                refresh_axis_model(&axis_model, &cfg_for_model, live);
                buttons_glue::refresh_button_model(
                    &button_model,
                    &cfg_for_model,
                    params_for_buttons.as_ref(),
                );
                buttons_glue::refresh_shift_model(&shift_model, &cfg_for_model);
                buttons_glue::refresh_timer_model(&timer_model, &cfg_for_model);
                encoders_glue::refresh_soft_encoder_model(&soft_encoder_model, &cfg_for_model);
                encoders_glue::refresh_fast_encoder_model(&fast_encoder_model, &cfg_for_model);
                encoders_glue::refresh_shift_reg_model(&shift_reg_model, &cfg_for_model);
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

/// Wire all axis-edit callbacks. Each mutates the held config and
/// refreshes only the touched row in the axis model so the rest of
/// the in-progress UI (`TextInput` focus, in particular) doesn't lose
/// its place to a wholesale model rebuild.
#[allow(clippy::too_many_lines)]
fn wire_axis_callbacks(
    window: &AppWindow,
    state: &Rc<RefCell<State>>,
    axis_model: &Rc<VecModel<AxisRow>>,
) {
    let mk_toggle = |cb: fn(&mut freejoyx_core::wire::AxisConfig)| {
        let s = state.clone();
        let m = axis_model.clone();
        let w = window.as_weak();
        move |slot: i32| {
            mutate_axis(&s, &m, &w, slot, cb);
        }
    };
    let mk_int = |cb: fn(&mut freejoyx_core::wire::AxisConfig, i32)| {
        let s = state.clone();
        let m = axis_model.clone();
        let w = window.as_weak();
        move |slot: i32, v: i32| {
            mutate_axis(&s, &m, &w, slot, move |a| cb(a, v));
        }
    };

    window.on_axis_out_toggled(mk_toggle(|a| a.set_out_enabled(!a.out_enabled())));
    window.on_axis_inverted_toggled(mk_toggle(|a| a.set_inverted(!a.inverted())));
    window.on_axis_centered_toggled(mk_toggle(|a| a.set_is_centered(!a.is_centered())));
    window.on_axis_dyn_deadband_toggled(mk_toggle(|a| {
        a.set_is_dynamic_deadband(!a.is_dynamic_deadband());
    }));
    window.on_axis_filter_cycled(mk_toggle(|a| {
        a.set_filter((a.filter() + 1) % 8);
    }));
    window.on_axis_resolution_cycled(mk_toggle(|a| {
        a.set_resolution((a.resolution() + 1) % 16);
    }));
    window.on_axis_channel_cycled(mk_toggle(|a| {
        a.set_channel((a.channel() + 1) % 16);
    }));

    window.on_axis_calib_min_edited(mk_int(|a, v| a.calib_min = clamp_i16(v)));
    window.on_axis_calib_center_edited(mk_int(|a, v| a.calib_center = clamp_i16(v)));
    window.on_axis_calib_max_edited(mk_int(|a, v| a.calib_max = clamp_i16(v)));
    window.on_axis_deadband_edited(mk_int(|a, v| {
        a.set_deadband_size(u8::try_from(v.clamp(0, 127)).unwrap_or(0));
    }));
}

fn mutate_axis(
    state: &Rc<RefCell<State>>,
    axis_model: &Rc<VecModel<AxisRow>>,
    window: &slint::Weak<AppWindow>,
    slot: i32,
    mutator: impl FnOnce(&mut freejoyx_core::wire::AxisConfig),
) {
    let Ok(slot) = usize::try_from(slot) else {
        return;
    };
    if slot >= MAX_AXIS_NUM {
        return;
    }
    let row = {
        let mut s = state.borrow_mut();
        if s.last_config.is_none() {
            return;
        }
        let live_raw = s
            .last_params
            .as_ref()
            .map_or(0, |p| i32::from(p.raw_axis_data[slot]));
        let live_out = s
            .last_params
            .as_ref()
            .map_or(0, |p| i32::from(p.axis_data[slot]));
        let cfg = s.last_config.as_mut().expect("checked is_none above");
        mutator(&mut cfg.axis_config[slot]);
        build_axis_row(slot, &cfg.axis_config[slot], live_raw, live_out)
    };
    axis_model.set_row_data(slot, row);
    if let Some(w) = window.upgrade() {
        w.set_can_write(w.get_connected());
        w.set_can_save(true);
    }
}

fn clamp_i16(v: i32) -> i16 {
    i16::try_from(v.clamp(i32::from(i16::MIN), i32::from(i16::MAX))).unwrap_or(0)
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

fn refresh_axis_model(
    model: &Rc<VecModel<AxisRow>>,
    cfg: &DeviceConfig,
    live: Option<([i16; MAX_AXIS_NUM], [i16; MAX_AXIS_NUM])>,
) {
    let params_ref = live;
    // Initial build (or wholesale repopulate after a load/read).
    if model.row_count() != MAX_AXIS_NUM {
        while model.row_count() > 0 {
            model.remove(0);
        }
        for slot in 0..MAX_AXIS_NUM {
            let raw = params_ref.map_or(0, |(raw, _)| i32::from(raw[slot]));
            let out = params_ref.map_or(0, |(_, out)| i32::from(out[slot]));
            model.push(build_axis_row(slot, &cfg.axis_config[slot], raw, out));
        }
        return;
    }
    // Per-row update (live tick path or per-edit refresh).
    for slot in 0..MAX_AXIS_NUM {
        let raw = params_ref.map_or(0, |(raw, _)| i32::from(raw[slot]));
        let out = params_ref.map_or(0, |(_, out)| i32::from(out[slot]));
        let row = build_axis_row(slot, &cfg.axis_config[slot], raw, out);
        model.set_row_data(slot, row);
    }
}

fn build_axis_row(
    slot: usize,
    a: &freejoyx_core::wire::AxisConfig,
    live_raw: i32,
    live_out: i32,
) -> AxisRow {
    let filter = AxisFilter::from_u8(a.filter());
    AxisRow {
        title: SharedString::from(format!("Axis {}", slot + 1)),
        out_enabled: a.out_enabled(),
        inverted: a.inverted(),
        is_centered: a.is_centered(),
        calib_min: i32::from(a.calib_min),
        calib_center: i32::from(a.calib_center),
        calib_max: i32::from(a.calib_max),
        filter_index: i32::from(filter.to_u8()),
        filter_label: SharedString::from(filter.label()),
        deadband_size: i32::from(a.deadband_size()),
        is_dynamic_deadband: a.is_dynamic_deadband(),
        resolution: i32::from(a.resolution()),
        channel: i32::from(a.channel()),
        live_raw,
        live_out,
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
