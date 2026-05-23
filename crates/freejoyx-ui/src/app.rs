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
use std::collections::BTreeSet;
use std::rc::Rc;
use std::time::Duration;

use freejoyx_core::domain::{
    analog_pin_slots, completed_fast_encoder_slots, physical_assignment_blocked,
    validate_for_write, AxisCalibration, AxisDetect, AxisFilter, AxisSource, Board, ButtonCapture,
    ButtonType, ButtonTypeCategory, CoexistenceCheck, ConfigError, EncoderMode, PinFunction,
    PinFunctionFamily, ShiftRegType,
};
use freejoyx_core::persist::{load_from_file, save_to_file};
use freejoyx_core::wire::{
    is_supported_firmware_version, DeviceConfig, ParamsReport, BUTTON_BITMAP_BYTES, MAX_AXIS_NUM,
    MAX_BUTTONS_NUM, MAX_ENCODERS_NUM, MAX_FAST_ENCODER_NUM, MAX_SHIFT_REG_NUM,
    SUPPORTED_FIRMWARE_VERSION, USED_PINS_NUM,
};
use freejoyx_device::{spawn_for_serial, Command, DeviceCandidate, DeviceEvent, DeviceHandle};
use slint::{ComponentHandle, Global, Model, ModelRc, SharedString, VecModel};

use crate::settings;
use crate::tabs::advanced as advanced_glue;
use crate::tabs::buttons as buttons_glue;
use crate::tabs::encoders as encoders_glue;
use crate::tabs::pins as pins_glue;
use crate::tabs::pins::{axis_back_to_pin, pin_jump_target, shift_reg_back_to_pin};
use crate::{
    AppWindow, AxisRow, ButtonRow, CategoryChip, DeviceOption, DropdownEntry, EncoderRow,
    FastEncoderRow, LogEntry, Palette, PinRow, ShiftRegRow, ShiftSlot, TimerField,
};

/// Per-row pixel heights for the Axes tab's Flickable viewport. Must
/// stay in sync with the heights set on `AxisRowView` in `app.slint`:
/// no-source rows shrink to the header-only height, configured rows
/// expand to the full body.
const AXIS_ROW_COLLAPSED_PX: f32 = 48.0;
/// Phase 4 added the CalibRangeBar (10 px + 6 px gap) between the
/// bars-cluster row and the min/ctr/max number row; bump up by 16 px so
/// the existing rows don't get squeezed. Keep in sync with the
/// `has-source ? ... : 48px` height expression in `axis_row_view.slint`.
const AXIS_ROW_EXPANDED_PX: f32 = 188.0;
/// Extended-settings panel: two columns (Function group / Function-axis
/// + Prescaler + Offset) on top, a 4-px spacer, then three Button-N
/// rows. Each row is ~28 px with 6 px spacing; bump this if more rows
/// land on the panel. Keep in lockstep with the `if entry.extended-expanded:`
/// height arithmetic in `axis_row_view.slint`.
const AXIS_ROW_EXTENDED_EXTRA_PX: f32 = 224.0;
/// Inter-row spacing inside the AxesTab Flickable's `VerticalLayout`.
const AXIS_ROW_SPACING_PX: f32 = 4.0;

/// Which inline dropdown picker is currently open (issue #15). Values
/// must match the `kind: int` constants the Slint `DropdownCell`
/// passes through `dropdown-requested`. `slot` is the row index for
/// row-pickers (Pin / Button / Axis / Encoder / Shift register) or
/// `-1` for non-row pickers (the Buttons-tab filter category cell).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i32)]
pub(crate) enum DropdownKind {
    PinFunction = 0,
    ButtonType = 1,
    ButtonShift = 2,
    ButtonOp = 3,
    /// Per-button delay timer (`flags_c` bits 0..=2). Previously
    /// surfaced only on LOGIC rows as "debounce" — now shown on every
    /// row, since `delay_timer` is meaningful for most button types.
    ButtonDelay = 4,
    ButtonsFilterCategory = 5,
    AxisFilter = 6,
    AxisResolution = 7,
    AxisChannel = 8,
    EncoderSoftMode = 9,
    EncoderFastMode = 10,
    ShiftRegType = 11,
    /// Per-button press timer (`flags_c` bits 3..=5).
    ButtonPress = 12,
    /// Axis main-source picker (None / Encoder N / analog pins).
    /// Mirrors the Qt configurator's `comboBox_AxisSource1`.
    AxisSource = 13,
    /// Debug-tab min-level picker (Trace / Debug / Info / Warn / Error).
    DebugVerbosity = 14,
    /// Axis function operator (None / Plus / Minus / Equal).
    AxisFunction = 15,
    /// Axis "function axis" — picks another axis (0..=7) the
    /// function operator reads from. Maps to `source_secondary`.
    AxisFunctionRef = 16,
    /// Axis-button action pickers (Phase 2). Three distinct kinds
    /// because slot 2's action enum is narrower (no Down/Up) — the
    /// entries list is built differently. Slot is the axis index.
    AxisButtonAction1 = 17,
    AxisButtonAction2 = 18,
    AxisButtonAction3 = 19,
    /// Phase 3: I2C device address. Wire byte is the raw 8-bit address,
    /// not an index into the entries list.
    AxisI2cAddress = 20,
}

impl DropdownKind {
    fn from_i32(v: i32) -> Option<Self> {
        Some(match v {
            0 => Self::PinFunction,
            1 => Self::ButtonType,
            2 => Self::ButtonShift,
            3 => Self::ButtonOp,
            4 => Self::ButtonDelay,
            5 => Self::ButtonsFilterCategory,
            6 => Self::AxisFilter,
            7 => Self::AxisResolution,
            8 => Self::AxisChannel,
            9 => Self::EncoderSoftMode,
            10 => Self::EncoderFastMode,
            11 => Self::ShiftRegType,
            12 => Self::ButtonPress,
            13 => Self::AxisSource,
            14 => Self::DebugVerbosity,
            15 => Self::AxisFunction,
            16 => Self::AxisFunctionRef,
            17 => Self::AxisButtonAction1,
            18 => Self::AxisButtonAction2,
            19 => Self::AxisButtonAction3,
            20 => Self::AxisI2cAddress,
            _ => return None,
        })
    }
}

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
    /// True once we've flagged the currently-connected device as
    /// running an unsupported firmware version. Cleared on disconnect.
    /// Prevents the toast from re-asserting itself on every
    /// `ParamsTick` after the user dismisses it.
    unsupported_fw_flagged: bool,
    /// Buttons-tab filter state (issue #5). Drives which rows make it
    /// into the visible button model. Persisted across config reloads
    /// but cleared on disconnect.
    pub(crate) btn_hide_unused: bool,
    pub(crate) btn_filter_physical: Option<i8>,
    /// "Press to filter" mode. While true, the next physical button
    /// 0→1 transition observed in `ParamsTick` writes its slot index
    /// into [`Self::btn_filter_physical`] and clears this flag. Lets
    /// users narrow the visible-slots list by physically pressing the
    /// switch they care about, instead of remembering its index.
    pub(crate) btn_filter_arm: bool,
    /// `None` = show all categories. `Some(cat_index)` filters to
    /// `ButtonTypeCategory::all().nth(cat_index)`.
    pub(crate) btn_filter_category: Option<usize>,
    /// Wire-slot indices the user explicitly clicked "+ Add" on. The
    /// row is force-shown in the visible model even if its
    /// `physical_num` is still unassigned.
    pub(crate) btn_force_shown: BTreeSet<usize>,
    /// The (kind, slot) pair of the currently-open dropdown, captured
    /// on `dropdown-requested` so `dropdown-picked` knows where to
    /// dispatch the value. `None` whenever the overlay is closed.
    pub(crate) current_dropdown: Option<(DropdownKind, i32)>,
    /// Button-capture state machine. Owns the currently-armed cell
    /// (Physical or SrcB) plus the per-slot disarm-tick pair that
    /// drives the Slint `NumberCell.disarm-tick` properties. See
    /// `freejoyx-core::domain::modes::button_capture` for the rules.
    pub(crate) button_capture: ButtonCapture,
    /// Previous tick's `phy_button_data` bitmap. Used both by
    /// `ButtonCapture::on_params_tick` for press-edge detection and
    /// by the structured-event logger; kept on `State` rather than
    /// inside the Mode so the logger can read it without grabbing a
    /// borrow on the Mode itself.
    pub(crate) last_phy_button_data: [u8; BUTTON_BITMAP_BYTES],
    /// Axis-calibration state machine. Owns the currently-armed axis;
    /// widens calib_min / calib_max on every tick while armed.
    pub(crate) axis_calibrate: AxisCalibration,
    /// Axis auto-detect state machine. Owns the armed slot, the raw-
    /// value baseline captured at arm time, and the per-slot disarm
    /// ticks for the Detect IconButton.
    pub(crate) axis_detect: AxisDetect,
    /// Owns the single-shot timer that clears `flash-*-slot` after a
    /// pin-jump click. Dropping a `slint::Timer` cancels it, so we
    /// stash it here to keep it alive until it fires (or until the
    /// next click replaces it).
    pub(crate) pin_jump_flash_timer: Option<slint::Timer>,
    /// Per-shift-register "buttons per chip" UI factoring. The wire
    /// format only stores `button_cnt`; this array is the user's
    /// last-typed chip-size for each row, used to derive `num_chips`
    /// on display. Defaults to 8 (HC165/CD4021 chip size).
    pub(crate) shift_reg_chip_size: [u8; freejoyx_core::wire::MAX_SHIFT_REG_NUM],
    /// Per-axis "Show extended settings" expand/collapse state. UI
    /// only — not persisted. Drives the extra row in the Axes tab
    /// card and the third tier of `compute_axes_viewport_height`.
    pub(crate) axes_extended_expanded: [bool; MAX_AXIS_NUM],
    /// Debug-tab event ring + live filter. The pump-events timer pulls
    /// fresh entries from the buffer into the Slint log model; the
    /// filter is mutated by the verbosity / category controls on the
    /// Debug tab.
    pub(crate) log_buffer: crate::debug_log::LogBuffer,
    pub(crate) debug_filter: crate::debug_log::DebugFilterHandle,
    /// Last buffer sequence number copied into the UI model. The
    /// pump uses `LogBuffer::drain_new(this)` so each tick only
    /// re-marshals events that landed since the previous tick.
    pub(crate) log_cursor: u64,
}

/// Build a [`buttons_glue::ButtonFilter`] borrowing from the held
/// state. Used by every model-refresh call site that touches the
/// buttons model.
/// Push the toolbar identity-card fields from the latest
/// [`DeviceCandidate`] + [`DeviceConfig`]. Either side may be `None`:
///
/// - `candidate = None` (e.g. on `Disconnected`) zeroes the card.
/// - `cfg = None` (between `Connected` and the first `ConfigReceived`)
///   keeps the device-name + VID/PID/serial visible but blanks the
///   board / firmware / pins counters because they live on the config.
fn push_device_identity(
    window: &AppWindow,
    candidate: Option<&freejoyx_device::DeviceCandidate>,
    cfg: Option<&DeviceConfig>,
) {
    let Some(c) = candidate else {
        window.set_device_name(SharedString::from("—"));
        window.set_device_board(SharedString::from("—"));
        window.set_device_fw(SharedString::from(""));
        window.set_device_vid(SharedString::from("—"));
        window.set_device_pid(SharedString::from("—"));
        window.set_device_serial(SharedString::from("—"));
        window.set_pins_assigned(0);
        return;
    };
    window.set_device_name(SharedString::from(c.product_string.clone()));
    window.set_device_vid(SharedString::from(format!("0x{:04X}", c.vendor_id)));
    window.set_device_pid(SharedString::from(format!("0x{:04X}", c.product_id)));
    window.set_device_serial(SharedString::from(
        c.serial_number.clone().unwrap_or_else(|| "—".to_string()),
    ));
    if let Some(cfg) = cfg {
        let board = Board::from_id(cfg.board_id);
        window.set_device_board(SharedString::from(format!("{board:?}")));
        window.set_device_fw(SharedString::from(format!(
            "fw 0x{:04X}",
            cfg.firmware_version
        )));
        let assigned = cfg.pins.iter().filter(|&&p| p != 0).count();
        window.set_pins_assigned(i32::try_from(assigned).unwrap_or(0));
    } else {
        window.set_device_board(SharedString::from("—"));
        window.set_device_fw(SharedString::from(""));
        window.set_pins_assigned(0);
    }
}

/// Recompute the six per-tab "configured" indicators and push them to
/// the AppWindow. Called from every cfg-mutation entry point so the
/// dots on the tab strip stay live with the held config.
///
/// Tabs without a meaningful "has content" question (Advanced — always
/// has a device-name/VID/PID; Debug — no semantic) don't get a dot.
pub(crate) fn refresh_tab_indicators(window: &AppWindow, cfg: &DeviceConfig) {
    window.set_pins_has_content(pins_glue::has_content(cfg));
    window.set_axes_has_content(axes_has_content(cfg));
    window.set_buttons_has_content(buttons_glue::has_content(cfg));
    window.set_shifts_has_content(buttons_glue::shifts_has_content(cfg));
    window.set_encoders_has_content(encoders_glue::encoders_has_content(cfg));
    window.set_shift_regs_has_content(encoders_glue::shift_regs_has_content(cfg));
}

/// Does any axis carry a configured source? Inline here because there's
/// no dedicated `tabs/axes.rs` yet (deferred per
/// `ARCHITECTURE_BACKLOG.md` #1). Moves into that module when the Axes
/// tab gets extracted.
fn axes_has_content(cfg: &DeviceConfig) -> bool {
    use freejoyx_core::domain::AXIS_SOURCE_NONE;
    cfg.axis_config
        .iter()
        .any(|a| a.source_main != AXIS_SOURCE_NONE)
}

pub(crate) fn build_button_filter(state: &State) -> buttons_glue::ButtonFilter<'_> {
    buttons_glue::ButtonFilter {
        hide_unused: state.btn_hide_unused,
        filter_physical: state.btn_filter_physical,
        filter_category: state
            .btn_filter_category
            .and_then(|i| ButtonTypeCategory::all().nth(i)),
        force_shown: &state.btn_force_shown,
    }
}

impl State {
    fn new(
        handle: DeviceHandle,
        log_buffer: crate::debug_log::LogBuffer,
        debug_filter: crate::debug_log::DebugFilterHandle,
    ) -> Self {
        Self {
            handle,
            connected_device: None,
            last_config: None,
            board: Board::Bluepill,
            status: "waiting for device…".to_string(),
            last_params: None,
            unsupported_fw_flagged: false,
            btn_hide_unused: false,
            btn_filter_physical: None,
            btn_filter_arm: false,
            btn_filter_category: None,
            btn_force_shown: BTreeSet::new(),
            current_dropdown: None,
            button_capture: ButtonCapture::new(),
            last_phy_button_data: [0; BUTTON_BITMAP_BYTES],
            axis_calibrate: AxisCalibration::new(),
            axis_detect: AxisDetect::new(),
            pin_jump_flash_timer: None,
            shift_reg_chip_size: [8; freejoyx_core::wire::MAX_SHIFT_REG_NUM],
            axes_extended_expanded: [false; MAX_AXIS_NUM],
            log_buffer,
            debug_filter,
            log_cursor: 0,
        }
    }

    /// Forward a command to the device worker. Wrapper around
    /// `DeviceHandle::send` so callbacks outside this module don't
    /// need a direct reference to the worker handle.
    ///
    /// # Errors
    ///
    /// Propagates [`std::sync::mpsc::SendError`] from the worker
    /// channel when the worker thread has joined or panicked.
    pub(crate) fn handle_send(
        &self,
        cmd: Command,
    ) -> Result<(), std::sync::mpsc::SendError<Command>> {
        self.handle.send(cmd)
    }

    pub(crate) fn current_serial(&self) -> Option<&str> {
        self.connected_device
            .as_ref()
            .and_then(|c| c.serial_number.as_deref())
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
#[allow(clippy::too_many_lines)]
pub fn run(
    serial_filter: Option<String>,
    log_buffer: crate::debug_log::LogBuffer,
    debug_filter: crate::debug_log::DebugFilterHandle,
) -> Result<(), slint::PlatformError> {
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

    let candidates_model: Rc<VecModel<DeviceOption>> = Rc::new(VecModel::default());
    window.set_device_candidates(ModelRc::from(candidates_model.clone()));
    window.set_advanced(advanced_glue::empty_advanced_model());

    // Debug-tab models. `log_model` holds the rolling display copy
    // (re-marshalled from State.log_buffer each pump tick).
    // `category_model` holds the per-category toggle chips and is
    // rebuilt when the user clicks one. (Initial population happens
    // below, once `state` exists.)
    let log_model: Rc<VecModel<LogEntry>> = Rc::new(VecModel::default());
    window.set_log_entries(ModelRc::from(log_model.clone()));
    let category_model: Rc<VecModel<CategoryChip>> = Rc::new(VecModel::default());
    window.set_log_categories(ModelRc::from(category_model.clone()));

    window.set_app_version(SharedString::from(env!("CARGO_PKG_VERSION")));
    window.set_build_rev(SharedString::from(env!("FREEJOYX_BUILD_REV")));
    window.set_supported_fw(SharedString::from(format!(
        "{SUPPORTED_FIRMWARE_VERSION:04X}"
    )));

    // Reusable model the global dropdown overlay reads from. Rust
    // mutates this on every `dropdown-requested` to load the right
    // entries for the active picker (issue #15).
    let dropdown_entries: Rc<VecModel<DropdownEntry>> = Rc::new(VecModel::default());
    window.set_dropdown_entries(ModelRc::from(dropdown_entries.clone()));

    let state = Rc::new(RefCell::new(State::new(handle, log_buffer, debug_filter)));

    refresh_category_model(&category_model, &state.borrow().debug_filter);
    refresh_verbosity_labels(&window, &state.borrow().debug_filter);

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
    buttons_glue::wire_callbacks(&window, &state, &button_model, &shift_model, &timer_model);
    encoders_glue::wire_callbacks(
        &window,
        &state,
        &soft_encoder_model,
        &fast_encoder_model,
        &button_model,
        &shift_reg_model,
    );
    wire_dropdown_callbacks(
        &window,
        &state,
        &dropdown_entries,
        &DropdownSinks {
            pin_model: &pin_model,
            axis_model: &axis_model,
            button_model: &button_model,
            soft_encoder_model: &soft_encoder_model,
            fast_encoder_model: &fast_encoder_model,
            shift_reg_model: &shift_reg_model,
        },
    );
    advanced_glue::wire_callbacks(&window, &state, &candidates_model);
    wire_toast_callback(&window);
    wire_log_folder_callback(&window);
    wire_theme_callback(&window);
    wire_pin_jump_callback(&window, &state);
    wire_debug_callbacks(&window, &state, &log_model, &category_model);

    // Ask the worker for its candidate list up-front so the picker
    // dropdown has something to show on first open without the user
    // having to hit "refresh".
    let _ = state
        .borrow()
        .handle_send(freejoyx_device::Command::Enumerate);

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
        let candidates_model_for_timer = candidates_model.clone();
        let log_model_for_timer = log_model.clone();
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
                        candidates_model: &candidates_model_for_timer,
                    },
                    &rx,
                );
                pump_log_model(&state, &log_model_for_timer, &window);
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
    candidates_model: &'a Rc<VecModel<DeviceOption>>,
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

#[allow(clippy::too_many_lines)]
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
    let candidates_model = sinks.candidates_model;
    while let Ok(evt) = rx.try_recv() {
        match evt {
            DeviceEvent::Connected(c) => {
                let mut s = state.borrow_mut();
                s.connected_device = Some(c.clone());
                s.status = "connected — click Read Config to load".to_string();
                window.set_connected(true);
                window.set_device_summary(SharedString::from(c.display_summary()));
                window.set_status_text(SharedString::from(s.status.clone()));
                push_device_identity(window, Some(&c), None);
                window.set_can_read(true);
                window.set_can_write(false);
                // Ask the worker for an up-to-date candidate list so
                // the picker's accent dot moves to the newly-current
                // device.
                let _ = s.handle_send(Command::Enumerate);
            }
            DeviceEvent::Disconnected => {
                let mut s = state.borrow_mut();
                s.connected_device = None;
                s.status = "disconnected — waiting for device".to_string();
                s.unsupported_fw_flagged = false;
                // `clear()` rather than `disarm()` — the UI cells are
                // about to vanish anyway, so don't bump disarm-ticks
                // into a dead handle.
                s.button_capture.clear();
                s.axis_calibrate.clear();
                s.axis_detect.clear();
                s.last_phy_button_data = [0; BUTTON_BITMAP_BYTES];
                window.set_connected(false);
                window.set_device_summary(SharedString::from("no device"));
                window.set_status_text(SharedString::from(s.status.clone()));
                push_device_identity(window, None, None);
                window.set_can_read(false);
                window.set_can_write(false);
                clear_toast(window);
            }
            DeviceEvent::ParamsTick(p) => {
                let (cfg_opt, version_triple, captured, source_changed, arm_capture) = {
                    let mut s = state.borrow_mut();
                    let triple = (
                        p.freejoyx_version_major,
                        p.freejoyx_version_minor,
                        p.freejoyx_version_patch,
                    );
                    let fw = p.firmware_version;
                    if !is_supported_firmware_version(fw) && !s.unsupported_fw_flagged {
                        s.unsupported_fw_flagged = true;
                        set_toast(
                            window,
                            ToastSeverity::Error,
                            &format!(
                                "Unsupported firmware 0x{fw:04X}. \
                                FreeJoyXConfigurator v0.1 only supports mask group \
                                0x{:04X}. Use the Qt configurator (FreeJoyConfiguratorQt) \
                                for legacy boards on the 0x17XX line.",
                                SUPPORTED_FIRMWARE_VERSION & 0xFFF0
                            ),
                        );
                        window.set_can_read(false);
                        window.set_can_write(false);
                    }
                    // Drive the three interactive-mode state machines.
                    // Each is a no-op unless its UI mode is armed.
                    // Field-pattern destructuring is required so the
                    // borrow checker accepts simultaneous mutable
                    // borrows of `last_config` and the three Mode
                    // fields — disjoint fields, but NLL can't see that
                    // through chained `.as_mut()` calls without help.
                    let (captured, axis_changed, source_changed) = {
                        let State {
                            last_config,
                            button_capture,
                            last_phy_button_data,
                            axis_calibrate,
                            axis_detect,
                            ..
                        } = &mut *s;
                        if let Some(cfg) = last_config.as_mut() {
                            let cap = button_capture.on_params_tick(&p, last_phy_button_data, cfg);
                            let cal = axis_calibrate.on_params_tick(&p, cfg);
                            let det =
                                axis_detect.on_params_tick(&p, cfg, std::time::Instant::now());
                            (
                                cap.config_changed,
                                cal.config_changed || det.config_changed,
                                det.bound_to_slot.is_some(),
                            )
                        } else {
                            (false, false, false)
                        }
                    };
                    log_button_bitmap_edges(&s.last_phy_button_data, &p);
                    // "Press to filter" mode: every observed rising
                    // edge re-narrows `btn_filter_physical` to the
                    // pressed input. Stays armed across presses (the
                    // arm is only cleared by an explicit toggle, a tab
                    // switch, or a per-row Physical cell arming).
                    // Runs before `last_phy_button_data` is rolled
                    // forward so the edge detection uses the prior
                    // tick's snapshot.
                    let arm_capture = if s.btn_filter_arm {
                        first_rising_edge(&s.last_phy_button_data, &p.phy_button_data)
                    } else {
                        None
                    };
                    if let Some(idx) = arm_capture {
                        s.btn_filter_physical = i8::try_from(idx).ok();
                        tracing::info!(
                            target: "freejoyx::button",
                            slot = idx as u64,
                            "press-to-filter captured physical",
                        );
                    }
                    s.last_phy_button_data = p.phy_button_data;
                    // `params_report_t.board_id` is the ground truth for
                    // which board is on the wire. Update before
                    // ConfigReceived (which carries the same id) so the
                    // Pins-tab schematic flips to the right layout on
                    // first tick, not only after Read Device.
                    let reported_board = Board::from_id(p.board_id);
                    let board_changed = s.board != reported_board;
                    if board_changed {
                        tracing::info!(
                            target: "freejoyx::config",
                            board_id = p.board_id,
                            from = ?s.board,
                            to = ?reported_board,
                            "board detected from params tick",
                        );
                        s.board = reported_board;
                    }
                    s.last_params = Some(p);
                    (
                        s.last_config.clone(),
                        triple,
                        captured || axis_changed || source_changed,
                        source_changed || board_changed,
                        arm_capture,
                    )
                };
                if let Some(idx) = arm_capture {
                    // Mirror the Rust-side capture into Slint props
                    // and rebuild the visible-rows model so the user
                    // sees the filter narrow on the next frame. The
                    // arm stays on; we just bump the jump-tick so the
                    // captured row scrolls to the top of the list
                    // (filter narrows visible rows to slots mapped to
                    // this physical, so resetting viewport-y = 0
                    // lands the row in view).
                    window.set_buttons_filter_physical(i32::try_from(idx).unwrap_or(-1));
                    buttons_glue::rebuild_filtered(state, button_model, &window.as_weak());
                    window.set_buttons_jump_y(0.0);
                    window.set_buttons_jump_tick(window.get_buttons_jump_tick().wrapping_add(1));
                }
                if captured {
                    window.set_can_write(window.get_connected());
                    window.set_can_save(true);
                    if let Some(cfg) = state.borrow().last_config.as_deref() {
                        refresh_tab_indicators(window, cfg);
                    }
                }
                if let Some(cfg) = cfg_opt {
                    let params_snapshot = state.borrow().last_params.clone();
                    let live = params_snapshot
                        .as_ref()
                        .map(|p| (p.raw_axis_data, p.axis_data));
                    let inputs = AxisRenderInputs::from_state(&state.borrow());
                    refresh_axis_model(axis_model, &cfg, &inputs, live);
                    window.set_axes_viewport_height(compute_axes_viewport_height(
                        &cfg,
                        &inputs.expanded,
                    ));
                    // Auto-detect can flip an axis's source; mirror the
                    // change into the Pins-tab model so the jump
                    // button's enabled state stays in sync.
                    if source_changed {
                        let board = state.borrow().board;
                        pins_glue::refresh_pin_model(pin_model, &cfg, board);
                    }
                    {
                        let s = state.borrow();
                        let filter = build_button_filter(&s);
                        buttons_glue::refresh_button_model(
                            button_model,
                            &cfg,
                            params_snapshot.as_ref(),
                            &filter,
                            &s.button_capture,
                        );
                    }
                    encoders_glue::refresh_soft_encoder_model(
                        soft_encoder_model,
                        &cfg,
                        params_snapshot.as_ref(),
                    );
                    let merged = advanced_glue::merge_params_into_advanced(
                        &window.get_advanced(),
                        version_triple.0,
                        version_triple.1,
                        version_triple.2,
                    );
                    window.set_advanced(merged);
                }
            }
            DeviceEvent::ConfigReceived(cfg) => {
                let mut s = state.borrow_mut();
                s.board = Board::from_id(cfg.board_id);
                s.last_config = Some(cfg.clone());
                let assigned = cfg.pins.iter().filter(|&&p| p != 0).count();
                tracing::info!(
                    target: "freejoyx::config",
                    fw = format!("{:#06x}", cfg.firmware_version),
                    board = ?s.board,
                    pins_assigned = assigned,
                    "ConfigReceived"
                );
                s.status = format!(
                    "config received — fw 0x{:04x}, board {:?}, {} pins assigned",
                    cfg.firmware_version, s.board, assigned,
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
                let candidate_snapshot = s.connected_device.clone();
                drop(s);
                push_device_identity(window, candidate_snapshot.as_ref(), Some(&cfg));
                pins_glue::refresh_pin_model(pin_model, &cfg, board);
                let axis_inputs = AxisRenderInputs::from_state(&state.borrow());
                refresh_axis_model(axis_model, &cfg, &axis_inputs, live);
                window.set_axes_viewport_height(compute_axes_viewport_height(
                    &cfg,
                    &axis_inputs.expanded,
                ));
                {
                    let s = state.borrow();
                    let filter = build_button_filter(&s);
                    buttons_glue::refresh_button_model(
                        button_model,
                        &cfg,
                        params_for_buttons.as_ref(),
                        &filter,
                        &s.button_capture,
                    );
                }
                let visible = i32::try_from(button_model.row_count()).unwrap_or(0);
                window.set_buttons_visible_count(visible);
                buttons_glue::refresh_shift_model(shift_model, &cfg);
                buttons_glue::refresh_timer_model(timer_model, &cfg);
                encoders_glue::refresh_soft_encoder_model(
                    soft_encoder_model,
                    &cfg,
                    params_for_buttons.as_ref(),
                );
                encoders_glue::refresh_fast_encoder_model(fast_encoder_model, &cfg);
                let chip_size = state.borrow().shift_reg_chip_size;
                encoders_glue::refresh_shift_reg_model(shift_reg_model, &cfg, chip_size);
                // Preserve the live freejoyx-version line that
                // ParamsTick has been filling in.
                let prior = window.get_advanced();
                let mut next = advanced_glue::build_advanced_model(&cfg);
                next.freejoyx_version = prior.freejoyx_version;
                window.set_advanced(next);
                refresh_tab_indicators(window, &cfg);
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
            DeviceEvent::Candidates(list) => {
                let current = state.borrow().current_serial().map(str::to_string);
                advanced_glue::refresh_candidates_model(
                    candidates_model,
                    &list,
                    current.as_deref(),
                );
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
            tracing::error!(target: "freejoyx::config", "ReadConfig dispatch failed (worker exited)");
        } else {
            s.status = "reading config…".to_string();
            tracing::info!(target: "freejoyx::config", "ReadConfig dispatched");
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

        // Pre-write gate — block invalid configs from reaching the
        // firmware. The validator aggregates every domain rule
        // (pin conflicts, incomplete LOGIC slots, cross-slot button
        // coexistence) so this is the only check the write path needs.
        let errors = validate_for_write(&cfg);
        if !errors.is_empty() {
            let summary = format_validation_errors(&errors);
            s.status = format!("write blocked: {} configuration problem(s)", errors.len());
            tracing::warn!(
                target: "freejoyx::config",
                problem_count = errors.len(),
                "write blocked by pre-write validator:\n{summary}",
            );
            if let Some(w) = weak.upgrade() {
                w.set_status_text(SharedString::from(s.status.clone()));
                set_toast(&w, ToastSeverity::Error, &summary);
            }
            return;
        }

        let fw = cfg.firmware_version;
        if s.handle.send(Command::WriteConfig(cfg)).is_err() {
            s.status = "worker exited; cannot write".to_string();
            tracing::error!(target: "freejoyx::config", fw = format!("{fw:#06x}"), "WriteConfig dispatch failed (worker exited)");
        } else {
            s.status = "writing config…".to_string();
            tracing::info!(target: "freejoyx::config", fw = format!("{fw:#06x}"), "WriteConfig dispatched");
        }
        if let Some(w) = weak.upgrade() {
            w.set_status_text(SharedString::from(s.status.clone()));
            clear_toast(&w);
        }
    });
}

/// Render a list of `ConfigError`s as a single toast string —
/// "Fix N problem(s) before writing:" header followed by one bullet per
/// error, each tagged with its tab hint so the user knows where to go.
/// Caps at 8 lines so the toast doesn't tower up the window; the full
/// list is always in the structured-event log.
fn format_validation_errors(errors: &[ConfigError]) -> String {
    use std::fmt::Write;
    const MAX_LINES: usize = 8;

    let mut out = format!(
        "Fix {} problem{} before writing:",
        errors.len(),
        if errors.len() == 1 { "" } else { "s" },
    );
    for err in errors.iter().take(MAX_LINES) {
        let _ = write!(&mut out, "\n• [{}] {}", err.tab_hint(), err.human_summary());
    }
    if errors.len() > MAX_LINES {
        let _ = write!(
            &mut out,
            "\n… and {} more (see Debug tab)",
            errors.len() - MAX_LINES
        );
    }
    out
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
            Ok(()) => {
                tracing::info!(target: "freejoyx::config", path = %path.display(), "config saved to RON");
                set_status(&weak, &state, &format!("saved {}", path.display()));
            }
            Err(e) => {
                tracing::error!(target: "freejoyx::config", path = %path.display(), err = %e, "save failed");
                set_status(&weak, &state, &format!("save failed: {e}"));
            }
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
                pins_glue::refresh_pin_model(&pin_model, &cfg_for_model, board);
                let axis_inputs = AxisRenderInputs::from_state(&state.borrow());
                refresh_axis_model(&axis_model, &cfg_for_model, &axis_inputs, live);
                refresh_axes_viewport_height(&weak, &cfg_for_model, &axis_inputs.expanded);
                {
                    let s = state.borrow();
                    let filter = build_button_filter(&s);
                    buttons_glue::refresh_button_model(
                        &button_model,
                        &cfg_for_model,
                        params_for_buttons.as_ref(),
                        &filter,
                        &s.button_capture,
                    );
                }
                buttons_glue::refresh_shift_model(&shift_model, &cfg_for_model);
                buttons_glue::refresh_timer_model(&timer_model, &cfg_for_model);
                encoders_glue::refresh_soft_encoder_model(
                    &soft_encoder_model,
                    &cfg_for_model,
                    params_for_buttons.as_ref(),
                );
                encoders_glue::refresh_fast_encoder_model(&fast_encoder_model, &cfg_for_model);
                let chip_size = state.borrow().shift_reg_chip_size;
                encoders_glue::refresh_shift_reg_model(&shift_reg_model, &cfg_for_model, chip_size);
                if let Some(w) = weak.upgrade() {
                    let prior = w.get_advanced();
                    let mut next = advanced_glue::build_advanced_model(&cfg_for_model);
                    next.freejoyx_version = prior.freejoyx_version;
                    w.set_advanced(next);
                    w.set_can_save(true);
                    let connected = w.get_connected();
                    w.set_can_write(connected);
                    refresh_tab_indicators(&w, &cfg_for_model);
                    let candidate_snapshot = state.borrow().connected_device.clone();
                    push_device_identity(&w, candidate_snapshot.as_ref(), Some(&cfg_for_model));
                }
                tracing::info!(target: "freejoyx::config", path = %path.display(), "config loaded from RON");
                set_status(&weak, &state, &format!("loaded {}", path.display()));
            }
            Err(e) => {
                tracing::error!(target: "freejoyx::config", path = %path.display(), err = %e, "load failed");
                set_status(&weak, &state, &format!("load failed: {e}"));
            }
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

    // Output toggle honors the no-source-no-output rule: trying to flip
    // it on while source is None is a no-op so the UI's checked state
    // matches the underlying config after the rebuild.
    window.on_axis_out_toggled({
        let s = state.clone();
        let m = axis_model.clone();
        let w = window.as_weak();
        move |slot: i32| {
            mutate_axis(&s, &m, &w, slot, |a| {
                if matches!(a.source(), AxisSource::None) {
                    a.set_out_enabled(false);
                } else {
                    a.set_out_enabled(!a.out_enabled());
                }
            });
        }
    });
    window.on_axis_inverted_toggled(mk_toggle(|a| a.set_inverted(!a.inverted())));
    window.on_axis_centered_toggled(mk_toggle(|a| a.set_is_centered(!a.is_centered())));
    window.on_axis_dyn_deadband_toggled(mk_toggle(|a| {
        a.set_is_dynamic_deadband(!a.is_dynamic_deadband());
    }));
    window.on_axis_calib_min_edited(mk_int(|a, v| a.calib_min = clamp_i16(v)));
    window.on_axis_calib_center_edited(mk_int(|a, v| a.calib_center = clamp_i16(v)));
    window.on_axis_calib_max_edited(mk_int(|a, v| a.calib_max = clamp_i16(v)));
    window.on_axis_deadband_edited(mk_int(|a, v| {
        a.set_deadband_size(u8::try_from(v.clamp(0, 127)).unwrap_or(0));
    }));

    // Calibrate toggle. Entering calibration seeds min/max with the
    // *inverted* extremes (min = AXIS_MAX, max = AXIS_MIN) so the
    // very first raw value pulls both inward — matches the Qt
    // configurator's `on_pushButton_StartCalib_clicked`.
    window.on_axis_calibrate_toggled({
        let s = state.clone();
        let m = axis_model.clone();
        let w = window.as_weak();
        move |slot: i32| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            if slot >= MAX_AXIS_NUM {
                return;
            }
            let started;
            {
                let mut st = s.borrow_mut();
                if st.last_config.is_none() {
                    return;
                }
                let was_armed = st.axis_calibrate.armed_slot() == Some(slot);
                st.axis_calibrate.toggle(slot);
                started = !was_armed;
                if started {
                    let cfg = st.last_config.as_mut().expect("checked");
                    cfg.axis_config[slot].calib_min = AXIS_MAX_VALUE;
                    cfg.axis_config[slot].calib_max = AXIS_MIN_VALUE;
                }
            }
            tracing::info!(
                target: "freejoyx::axis",
                slot = slot as u64,
                action = if started { "start" } else { "stop" },
                "calibration toggled"
            );
            refresh_axis_row(&s, &m, slot);
            buttons_glue::mark_dirty(&w);
        }
    });

    // Reset to factory calibration: full ±32767 range, no centering.
    window.on_axis_reset_clicked({
        let s = state.clone();
        let m = axis_model.clone();
        let w = window.as_weak();
        move |slot: i32| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            if slot >= MAX_AXIS_NUM {
                return;
            }
            {
                let mut st = s.borrow_mut();
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                let a = &mut cfg.axis_config[slot];
                a.calib_min = AXIS_MIN_VALUE;
                a.calib_max = AXIS_MAX_VALUE;
                a.calib_center = 0;
                a.set_is_centered(false);
            }
            tracing::info!(target: "freejoyx::axis", slot = slot as u64, "calibration reset");
            refresh_axis_row(&s, &m, slot);
            buttons_glue::mark_dirty(&w);
        }
    });

    // Set center: snapshot the current raw value into calib_center,
    // enable the centered flag. Clamps the snapshot to the calibrated
    // range so a stray sample outside [min, max] doesn't poison the
    // center value.
    window.on_axis_set_center_clicked({
        let s = state.clone();
        let m = axis_model.clone();
        let w = window.as_weak();
        move |slot: i32| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            if slot >= MAX_AXIS_NUM {
                return;
            }
            let captured_center;
            {
                let mut st = s.borrow_mut();
                let raw = st.last_params.as_ref().map_or(0, |p| p.raw_axis_data[slot]);
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                let a = &mut cfg.axis_config[slot];
                let lo = a.calib_min.min(a.calib_max);
                let hi = a.calib_min.max(a.calib_max);
                a.calib_center = raw.clamp(lo, hi);
                a.set_is_centered(true);
                captured_center = a.calib_center;
            }
            tracing::info!(
                target: "freejoyx::axis",
                slot = slot as u64,
                center = captured_center as i64,
                "set centre"
            );
            refresh_axis_row(&s, &m, slot);
            buttons_glue::mark_dirty(&w);
        }
    });

    // Auto-detect arm / disarm. Arming captures the current raw_axis
    // snapshot as a baseline; the params-tick handler watches for an
    // axis whose |delta| crosses the threshold and copies that axis's
    // `source_main` onto the armed slot. Clicking Detect on a second
    // axis disarms the first.
    // Phase 1 advanced-axes: numeric edits in the extended panel.
    // Each maps to a u8 wire field — `apply_axis_field` clamps and
    // rebuilds the row.
    window.on_axis_step_div_edited({
        let s = state.clone();
        let m = axis_model.clone();
        let w = window.as_weak();
        move |slot: i32, v: i32| {
            apply_axis_field(&s, &m, &w, slot, v, 0, 255, |a, x| a.divider = x);
        }
    });
    window.on_axis_prescaler_edited({
        let s = state.clone();
        let m = axis_model.clone();
        let w = window.as_weak();
        move |slot: i32, v: i32| {
            apply_axis_field(&s, &m, &w, slot, v, 0, 255, |a, x| a.prescaler = x);
        }
    });
    // Offset is stored as a 5-bit raw value where each unit = 15°.
    // The UI shows degrees; convert by integer-dividing here, then
    // clamp before writing back to the 0..31 raw range.
    window.on_axis_offset_edited({
        let s = state.clone();
        let m = axis_model.clone();
        let w = window.as_weak();
        move |slot: i32, deg: i32| {
            let raw = (deg.max(0) / 15).clamp(0, 31);
            apply_axis_field(&s, &m, &w, slot, raw, 0, 31, |a, x| a.set_offset_angle(x));
        }
    });

    // Phase 2 advanced-axes: axis-triggered button slots take i8 with
    // -1 as the "unused" sentinel. Clamp to [-1, 127] to match the
    // wire type without losing the sentinel.
    let mk_btn_slot = |field: fn(&mut freejoyx_core::wire::AxisConfig, i8)| {
        let s = state.clone();
        let m = axis_model.clone();
        let w = window.as_weak();
        move |slot: i32, v: i32| {
            let v = i8::try_from(v.clamp(-1, i32::from(i8::MAX))).unwrap_or(-1);
            mutate_axis(&s, &m, &w, slot, move |a| field(a, v));
        }
    };
    window.on_axis_btn1_slot_edited(mk_btn_slot(|a, v| a.button1 = v));
    window.on_axis_btn2_slot_edited(mk_btn_slot(|a, v| a.button2 = v));
    window.on_axis_btn3_slot_edited(mk_btn_slot(|a, v| a.button3 = v));

    // buttons_from_axes count lives on `axes_to_buttons[slot]`, not
    // `axis_config[slot]` — different array, so a separate path.
    // Max value 12 mirrors Qt's `kMaxA2bPoints - 1` clamp.
    window.on_axis_buttons_from_axes_edited({
        let s = state.clone();
        let m = axis_model.clone();
        let w = window.as_weak();
        move |slot: i32, v: i32| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            if slot >= MAX_AXIS_NUM {
                return;
            }
            let clamped = u8::try_from(v.clamp(0, 12)).unwrap_or(0);
            {
                let mut st = s.borrow_mut();
                let Some(cfg) = st.last_config.as_mut() else {
                    return;
                };
                cfg.axes_to_buttons[slot].buttons_cnt = clamped;
            }
            refresh_axis_row(&s, &m, slot);
            buttons_glue::mark_dirty(&w);
        }
    });

    // Per-row extended-panel toggle. Flips the per-axis bool in
    // `State::axes_extended_expanded`, rebuilds that row + recomputes
    // the Flickable's viewport height so collapsed siblings don't
    // leave a gap.
    window.on_axis_extended_toggled({
        let s = state.clone();
        let m = axis_model.clone();
        let w = window.as_weak();
        move |slot: i32| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            if slot >= MAX_AXIS_NUM {
                return;
            }
            {
                let mut st = s.borrow_mut();
                st.axes_extended_expanded[slot] = !st.axes_extended_expanded[slot];
            }
            refresh_axis_row(&s, &m, slot);
            let st = s.borrow();
            if let Some(cfg) = st.last_config.as_ref() {
                refresh_axes_viewport_height(&w, cfg, &st.axes_extended_expanded);
            }
        }
    });

    window.on_axis_detect_toggled({
        let s = state.clone();
        let m = axis_model.clone();
        move |slot: i32| {
            let Ok(slot) = usize::try_from(slot) else {
                return;
            };
            if slot >= MAX_AXIS_NUM {
                return;
            }
            let prior = {
                let mut st = s.borrow_mut();
                let already_armed = st.axis_detect.armed_slot();
                if already_armed == Some(slot) {
                    st.axis_detect.disarm();
                    tracing::info!(target: "freejoyx::axis", slot = slot as u64, "auto-detect cancelled");
                    None
                } else {
                    // Snapshot raw values from last_params; if none yet,
                    // an empty params report serves as a benign baseline.
                    let baseline_params = st.last_params.clone().unwrap_or_else(|| {
                        freejoyx_core::wire::ParamsReport::decode(
                            &[0u8; freejoyx_core::wire::PARAMS_REPORT_SIZE],
                        )
                        .expect("zero-init params report decodes")
                    });
                    st.axis_detect.arm(slot, &baseline_params, std::time::Instant::now());
                    tracing::info!(target: "freejoyx::axis", slot = slot as u64, "auto-detect armed");
                    already_armed
                }
            };
            if let Some(prev) = prior {
                refresh_axis_row(&s, &m, prev);
            }
            refresh_axis_row(&s, &m, slot);
        }
    });
}

const AXIS_MIN_VALUE: i16 = -32767;
const AXIS_MAX_VALUE: i16 = 32767;

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
        let board = s.board;
        let calibrating = s.axis_calibrate.armed_slot() == Some(slot);
        let armed = s.axis_detect.armed_slot() == Some(slot);
        let disarm_tick = s.axis_detect.disarm_tick(slot);
        let expanded = s.axes_extended_expanded[slot];
        let cfg = s.last_config.as_mut().expect("checked is_none above");
        mutator(&mut cfg.axis_config[slot]);
        build_axis_row(
            slot,
            cfg,
            board,
            live_raw,
            live_out,
            calibrating,
            armed,
            disarm_tick,
            expanded,
        )
    };
    axis_model.set_row_data(slot, row);
    if let Some(w) = window.upgrade() {
        w.set_can_write(w.get_connected());
        w.set_can_save(true);
        if let Some(cfg) = state.borrow().last_config.as_deref() {
            refresh_tab_indicators(&w, cfg);
        }
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

/// Snapshot of axis-row UI state Rust derives every time a row is
/// (re)built. Carries everything `build_axis_row` needs that isn't on
/// the row's `AxisConfig`.
pub(crate) struct AxisRenderInputs {
    pub board: Board,
    pub calibrating: Option<usize>,
    pub detect_armed: Option<usize>,
    pub disarm_ticks: [i32; MAX_AXIS_NUM],
    pub expanded: [bool; MAX_AXIS_NUM],
}

impl AxisRenderInputs {
    pub(crate) fn from_state(state: &State) -> Self {
        let mut disarm_ticks = [0i32; MAX_AXIS_NUM];
        for (slot, tick) in disarm_ticks.iter_mut().enumerate() {
            *tick = state.axis_detect.disarm_tick(slot);
        }
        Self {
            board: state.board,
            calibrating: state.axis_calibrate.armed_slot(),
            detect_armed: state.axis_detect.armed_slot(),
            disarm_ticks,
            expanded: state.axes_extended_expanded,
        }
    }
}

/// Compute the Axes-tab Flickable viewport height in logical pixels.
/// Sums per-row collapsed/expanded heights so collapsed rows don't
/// leave empty scroll space below the list.
fn compute_axes_viewport_height(cfg: &DeviceConfig, expanded: &[bool; MAX_AXIS_NUM]) -> f32 {
    let mut total: f32 = 0.0;
    for slot in 0..MAX_AXIS_NUM {
        let has_source = !matches!(cfg.axis_config[slot].source(), AxisSource::None);
        total += axis_row_height(has_source, expanded[slot]);
    }
    // (rows - 1) inter-row gaps.
    total += AXIS_ROW_SPACING_PX * (MAX_AXIS_NUM.saturating_sub(1) as f32);
    total
}

/// Per-row height in logical pixels. Mirrors the conditional `height`
/// expression on `AxisRowView` in `axis_row_view.slint`.
fn axis_row_height(has_source: bool, expanded: bool) -> f32 {
    if !has_source {
        AXIS_ROW_COLLAPSED_PX
    } else if expanded {
        AXIS_ROW_EXPANDED_PX + AXIS_ROW_EXTENDED_EXTRA_PX
    } else {
        AXIS_ROW_EXPANDED_PX
    }
}

/// Push the recomputed viewport height to the window. Cheap — just an
/// f32 -> property write — so we call it from every path that can
/// flip a row's collapsed/expanded state.
fn refresh_axes_viewport_height(
    window: &slint::Weak<AppWindow>,
    cfg: &DeviceConfig,
    expanded: &[bool; MAX_AXIS_NUM],
) {
    if let Some(w) = window.upgrade() {
        w.set_axes_viewport_height(compute_axes_viewport_height(cfg, expanded));
    }
}

fn refresh_axis_model(
    model: &Rc<VecModel<AxisRow>>,
    cfg: &DeviceConfig,
    inputs: &AxisRenderInputs,
    live: Option<([i16; MAX_AXIS_NUM], [i16; MAX_AXIS_NUM])>,
) {
    let params_ref = live;
    let row_for = |slot: usize| {
        let raw = params_ref.map_or(0, |(raw, _)| i32::from(raw[slot]));
        let out = params_ref.map_or(0, |(_, out)| i32::from(out[slot]));
        build_axis_row(
            slot,
            cfg,
            inputs.board,
            raw,
            out,
            inputs.calibrating == Some(slot),
            inputs.detect_armed == Some(slot),
            inputs.disarm_ticks[slot],
            inputs.expanded[slot],
        )
    };
    // Initial build (or wholesale repopulate after a load/read).
    if model.row_count() != MAX_AXIS_NUM {
        while model.row_count() > 0 {
            model.remove(0);
        }
        for slot in 0..MAX_AXIS_NUM {
            model.push(row_for(slot));
        }
        return;
    }
    // Per-row update (live tick path or per-edit refresh).
    for slot in 0..MAX_AXIS_NUM {
        model.set_row_data(slot, row_for(slot));
    }
}

/// Axis title labels matching the Qt configurator
/// (`axes.cpp::axesList()`) — X, Y, Z, Rx, Ry, Rz, Slider 1, Slider 2.
const AXIS_TITLES: [&str; MAX_AXIS_NUM] = ["X", "Y", "Z", "Rx", "Ry", "Rz", "Slider 1", "Slider 2"];

/// Human label for an `AxisSource`. Pin labels use the board's
/// silkscreen naming so `PB11` reads as `PB2` on a `BlackPill`.
fn axis_source_label(src: AxisSource, board: Board) -> String {
    match src {
        AxisSource::None => "None".to_string(),
        AxisSource::I2C => "I2C".to_string(),
        AxisSource::Encoder(slot) => format!("Encoder {}", slot + 1),
        AxisSource::Pin(idx) => board.pin_name(idx as usize).to_string(),
    }
}

#[allow(clippy::too_many_arguments)]
fn build_axis_row(
    slot: usize,
    cfg: &DeviceConfig,
    board: Board,
    live_raw: i32,
    live_out: i32,
    calibrating: bool,
    detect_armed: bool,
    detect_disarm_tick: i32,
    extended_expanded: bool,
) -> AxisRow {
    let a = &cfg.axis_config[slot];
    let filter = AxisFilter::from_u8(a.filter());
    let source = a.source();
    let has_source = !matches!(source, AxisSource::None);
    let back_to_pin_enabled = axis_back_to_pin(cfg, slot).is_some();
    let function = freejoyx_core::domain::AxisFunction::from_u8(a.function());
    let fn_axis = usize::from(a.source_secondary()).min(MAX_AXIS_NUM - 1);
    let act = |raw: u8| freejoyx_core::domain::AxisButtonAction::from_u8(raw);
    let btn1_action = act(a.button1_type());
    let btn2_action = act(a.button2_type());
    let btn3_action = act(a.button3_type());
    let i2c_addr = freejoyx_core::domain::I2cAddress::from_u8(a.i2c_address);
    let i2c_enabled = matches!(source, AxisSource::I2C);
    AxisRow {
        title: SharedString::from(AXIS_TITLES[slot]),
        has_source,
        source_label: SharedString::from(axis_source_label(source, board)),
        source_handle: source.to_handle(),
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
        calibrating,
        detect_armed,
        detect_disarm_tick,
        back_to_pin_enabled,
        extended_expanded,
        function_label: SharedString::from(function.label()),
        function_index: i32::from(function.to_u8()),
        function_axis_index: i32::try_from(fn_axis).unwrap_or(0),
        function_axis_label: SharedString::from(AXIS_TITLES[fn_axis]),
        offset_deg: i32::from(a.offset_angle()) * 15,
        step_div: i32::from(a.divider),
        prescaler: i32::from(a.prescaler),
        buttons_from_axes: i32::from(cfg.axes_to_buttons[slot].buttons_cnt),
        btn1_slot: i32::from(a.button1),
        btn1_action_label: SharedString::from(btn1_action.label()),
        btn1_action_index: i32::from(btn1_action.to_u8()),
        btn2_slot: i32::from(a.button2),
        btn2_action_label: SharedString::from(btn2_action.label()),
        btn2_action_index: i32::from(btn2_action.to_u8()),
        btn3_slot: i32::from(a.button3),
        btn3_action_label: SharedString::from(btn3_action.label()),
        btn3_action_index: i32::from(btn3_action.to_u8()),
        i2c_addr_label: SharedString::from(i2c_addr.label()),
        i2c_addr_index: i32::from(i2c_addr.to_u8()),
        i2c_enabled,
    }
}

/// Toast severity codes mirrored from the Slint side (0=hidden,
/// 1=info, 2=warn, 3=error).
#[derive(Copy, Clone)]
enum ToastSeverity {
    #[allow(dead_code)]
    Info = 1,
    #[allow(dead_code)]
    Warn = 2,
    Error = 3,
}

fn set_toast(window: &AppWindow, severity: ToastSeverity, message: &str) {
    window.set_toast_severity(severity as i32);
    window.set_toast_message(SharedString::from(message));
}

fn clear_toast(window: &AppWindow) {
    window.set_toast_severity(0);
    window.set_toast_message(SharedString::from(""));
}

/// Flash hold time before the highlight clears. Long enough that a
/// post-scroll user can pick out the row, short enough that the pulse
/// reads as transient rather than a sticky selection.
const PIN_JUMP_FLASH_HOLD: Duration = Duration::from_millis(1500);

/// Compute the Y-offset of the axis row at `slot` inside the AxesTab
/// Flickable's viewport. Walks the upstream rows summing the same
/// per-row collapsed/expanded heights as `compute_axes_viewport_height`,
/// so the scroll target lines up with how rows actually render.
fn axes_row_y_offset(cfg: &DeviceConfig, expanded: &[bool; MAX_AXIS_NUM], slot: usize) -> f32 {
    let mut y: f32 = 0.0;
    for i in 0..slot.min(MAX_AXIS_NUM) {
        let has_source = !matches!(cfg.axis_config[i].source(), AxisSource::None);
        y += axis_row_height(has_source, expanded[i]);
        y += AXIS_ROW_SPACING_PX;
    }
    y
}

/// Schedule the flash highlight to clear after `PIN_JUMP_FLASH_HOLD`.
/// Each call replaces any pending timer — so re-jumps reset the
/// window rather than fighting an in-flight clear.
// --- Debug tab helpers --------------------------------------------------

/// Re-marshal the category-chip strip from the current filter state.
fn refresh_category_model(
    model: &Rc<VecModel<CategoryChip>>,
    filter: &crate::debug_log::DebugFilterHandle,
) {
    use crate::debug_log::EventCategory;
    let snap = filter.snapshot();
    while model.row_count() > 0 {
        model.remove(0);
    }
    for cat in EventCategory::ALL {
        model.push(CategoryChip {
            label: SharedString::from(cat.short_label()),
            on: snap.has_category(cat),
            category_index: cat.ui_index(),
        });
    }
}

/// Push the verbosity label / index from the filter to the window.
fn refresh_verbosity_labels(window: &AppWindow, filter: &crate::debug_log::DebugFilterHandle) {
    let level = filter.snapshot().min_level;
    window.set_log_verbosity_index(level.ui_index());
    window.set_log_verbosity_label(SharedString::from(level.short_label().trim()));
}

/// Drain new buffer entries into the Slint model. Bounded by the
/// buffer cap, so even after a long idle the worst case is N
/// allocations where N == BUFFER_CAPACITY.
fn pump_log_model(
    state: &Rc<RefCell<State>>,
    log_model: &Rc<VecModel<LogEntry>>,
    window: &AppWindow,
) {
    use crate::debug_log::{format_timestamp, BUFFER_CAPACITY};
    let (events, new_cursor) = {
        let s = state.borrow();
        s.log_buffer.drain_new(s.log_cursor)
    };
    if events.is_empty() {
        return;
    }
    {
        let mut s = state.borrow_mut();
        s.log_cursor = new_cursor;
    }
    for ev in events {
        let fields_summary = ev.fields_summary();
        log_model.push(LogEntry {
            timestamp: SharedString::from(format_timestamp(ev.timestamp)),
            level: ev.level.ui_index(),
            level_label: SharedString::from(ev.level.short_label()),
            category: ev.category.ui_index(),
            category_label: SharedString::from(ev.category.short_label()),
            message: SharedString::from(ev.message),
            fields_summary: SharedString::from(fields_summary),
        });
    }
    // Cap the displayed model. Buffer-cap matches; the display can
    // grow past it briefly before we trim because drain_new returns
    // events that might already have rolled off the buffer.
    while log_model.row_count() > BUFFER_CAPACITY {
        log_model.remove(0);
    }
    let total = i32::try_from(log_model.row_count()).unwrap_or(i32::MAX);
    window.set_log_total_count(total);
}

fn wire_debug_callbacks(
    window: &AppWindow,
    state: &Rc<RefCell<State>>,
    log_model: &Rc<VecModel<LogEntry>>,
    category_model: &Rc<VecModel<CategoryChip>>,
) {
    // Category toggle.
    {
        let state = state.clone();
        let category_model = category_model.clone();
        let weak = window.as_weak();
        window.on_log_category_toggled(move |idx| {
            use crate::debug_log::EventCategory;
            let Some(cat) = EventCategory::from_ui_index(idx) else {
                return;
            };
            {
                let s = state.borrow();
                s.debug_filter.update(|f| {
                    let on = !f.has_category(cat);
                    f.set_category(cat, on);
                });
            }
            refresh_category_model(&category_model, &state.borrow().debug_filter);
            if let Some(w) = weak.upgrade() {
                refresh_verbosity_labels(&w, &state.borrow().debug_filter);
            }
        });
    }

    // Clear log buffer + visible model.
    {
        let state = state.clone();
        let log_model = log_model.clone();
        let weak = window.as_weak();
        window.on_log_clear_clicked(move || {
            {
                let s = state.borrow();
                s.log_buffer.clear();
            }
            while log_model.row_count() > 0 {
                log_model.remove(0);
            }
            {
                // Reset cursor so the next pump doesn't replay
                // already-cleared events (they wouldn't be there
                // anyway, but the cursor would silently lap them).
                let mut s = state.borrow_mut();
                s.log_cursor = 0;
            }
            if let Some(w) = weak.upgrade() {
                w.set_log_total_count(0);
            }
        });
    }

    // Auto-scroll toggle.
    {
        let weak = window.as_weak();
        window.on_log_auto_scroll_toggled(move || {
            if let Some(w) = weak.upgrade() {
                w.set_log_auto_scroll(!w.get_log_auto_scroll());
            }
        });
    }

    // Verbose config-dump toggle. Flips the transport-side atomic; the
    // next Read/Write Config emits raw bytes + parsed contents at INFO
    // under `freejoyx::config`.
    {
        let weak = window.as_weak();
        window.on_log_verbose_config_toggled(move || {
            if let Some(w) = weak.upgrade() {
                let next = !w.get_log_verbose_config();
                w.set_log_verbose_config(next);
                freejoyx_device::set_config_dump_enabled(next);
                tracing::info!(
                    target: "freejoyx::config",
                    enabled = next,
                    "verbose config dumps {}",
                    if next { "ENABLED" } else { "disabled" }
                );
            }
        });
    }

    // Export to file. Writes the snapshot as plain-text lines to
    // `<log_dir>/freejoyx-debug-<timestamp>.log`. Drops a tracing
    // event reporting the path so the export action shows up in the
    // very buffer it just snapshotted (next tick).
    {
        let state = state.clone();
        window.on_log_export_clicked(move || {
            use crate::debug_log::format_timestamp;
            use std::fs::OpenOptions;
            use std::io::Write;
            let snapshot = state.borrow().log_buffer.snapshot();
            let dir = crate::log_dir::resolve();
            if std::fs::create_dir_all(&dir).is_err() {
                tracing::error!(target: "freejoyx::ui", "could not create log dir for export");
                return;
            }
            // ISO-ish file timestamp so the file sort matches recency.
            let now = std::time::SystemTime::now();
            let stamp = format_timestamp(now).replace(':', "-");
            let path = dir.join(format!("freejoyx-debug-{stamp}.log"));
            let Ok(mut f) = OpenOptions::new().create(true).write(true).truncate(true).open(&path)
            else {
                tracing::error!(target: "freejoyx::ui", path = %path.display(), "could not open export file");
                return;
            };
            for ev in snapshot {
                let line = format!(
                    "{} {} {} {} {}\n",
                    format_timestamp(ev.timestamp),
                    ev.level.short_label(),
                    ev.category.short_label(),
                    ev.message,
                    ev.fields_summary(),
                );
                if f.write_all(line.as_bytes()).is_err() {
                    break;
                }
            }
            tracing::info!(target: "freejoyx::ui", path = %path.display(), "log buffer exported");
        });
    }
}

pub(crate) fn schedule_flash_clear(state: &Rc<RefCell<State>>, weak: &slint::Weak<AppWindow>) {
    let flash_weak = weak.clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::SingleShot,
        PIN_JUMP_FLASH_HOLD,
        move || {
            if let Some(w) = flash_weak.upgrade() {
                w.set_flash_axis_slot(-1);
                w.set_flash_shift_reg_slot(-1);
                w.set_flash_pin_slot(-1);
                w.set_flash_button_slot_a(-1);
                w.set_flash_button_slot_b(-1);
            }
        },
    );
    state.borrow_mut().pin_jump_flash_timer = Some(timer);
}

fn wire_pin_jump_callback(window: &AppWindow, state: &Rc<RefCell<State>>) {
    // Pins → (Axes | Shift Registers).
    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_pin_jump_clicked(move |wire_slot| {
            let Ok(slot_usz) = usize::try_from(wire_slot) else {
                return;
            };
            let Some(target) = state
                .borrow()
                .last_config
                .as_ref()
                .and_then(|cfg| pin_jump_target(cfg, slot_usz))
            else {
                return;
            };
            let Some(w) = weak.upgrade() else { return };
            w.set_active_tab(target.tab);
            w.set_flash_pin_slot(-1);
            match target.tab {
                1 => {
                    w.set_flash_axis_slot(target.slot);
                    w.set_flash_shift_reg_slot(-1);
                    if target.slot >= 0 {
                        let st = state.borrow();
                        if let Some(cfg) = st.last_config.as_ref() {
                            let y = axes_row_y_offset(
                                cfg,
                                &st.axes_extended_expanded,
                                target.slot as usize,
                            );
                            w.set_axes_jump_y(y);
                            w.set_axes_jump_tick(w.get_axes_jump_tick().wrapping_add(1));
                        }
                    }
                }
                5 => {
                    w.set_flash_shift_reg_slot(target.slot);
                    w.set_flash_axis_slot(-1);
                }
                _ => {}
            }
            schedule_flash_clear(&state, &weak);
        });
    }

    // Axes → Pins.
    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_axis_back_to_pin_clicked(move |axis_slot| {
            let Ok(slot_usz) = usize::try_from(axis_slot) else {
                return;
            };
            let Some(pin_slot) = state
                .borrow()
                .last_config
                .as_ref()
                .and_then(|cfg| axis_back_to_pin(cfg, slot_usz))
            else {
                return;
            };
            let Some(w) = weak.upgrade() else { return };
            w.set_active_tab(0);
            w.set_flash_axis_slot(-1);
            w.set_flash_shift_reg_slot(-1);
            w.set_flash_pin_slot(i32::try_from(pin_slot).unwrap_or(-1));
            schedule_flash_clear(&state, &weak);
        });
    }

    // Shift Registers → Pins.
    {
        let state = state.clone();
        let weak = window.as_weak();
        window.on_shift_reg_back_to_pin_clicked(move |sr_slot| {
            let Ok(slot_usz) = usize::try_from(sr_slot) else {
                return;
            };
            let Some(pin_slot) = state
                .borrow()
                .last_config
                .as_ref()
                .and_then(|cfg| shift_reg_back_to_pin(cfg, slot_usz))
            else {
                return;
            };
            let Some(w) = weak.upgrade() else { return };
            w.set_active_tab(0);
            w.set_flash_axis_slot(-1);
            w.set_flash_shift_reg_slot(-1);
            w.set_flash_pin_slot(i32::try_from(pin_slot).unwrap_or(-1));
            schedule_flash_clear(&state, &weak);
        });
    }
}

fn wire_toast_callback(window: &AppWindow) {
    let weak = window.as_weak();
    window.on_toast_dismissed(move || {
        if let Some(w) = weak.upgrade() {
            clear_toast(&w);
        }
    });
}

fn wire_log_folder_callback(window: &AppWindow) {
    window.on_open_log_folder(move || {
        if let Err(e) = crate::log_dir::open_in_file_manager() {
            tracing::warn!("could not open log folder: {e}");
        }
    });
}

/// Apply the persisted theme preference and wire the Help-menu toggle.
/// The Palette global drives every colour in `app.slint`, so flipping
/// `Palette::dark` is enough to repaint the whole window.
fn wire_theme_callback(window: &AppWindow) {
    Palette::get(window).set_dark(settings::load_dark());
    let weak = window.as_weak();
    window.on_theme_toggled(move || {
        let Some(w) = weak.upgrade() else { return };
        let palette = Palette::get(&w);
        let next = !palette.get_dark();
        palette.set_dark(next);
        settings::save_dark(next);
    });
}

// --- Dropdown entries (issue #15) ---------------------------------------
//
// `flat_dropdown` and `header_dropdown` are constructors used by the
// kind→entries dispatch in `build_dropdown_for`.

fn flat_dropdown(label: &str, value: i32) -> DropdownEntry {
    DropdownEntry {
        is_header: false,
        label: SharedString::from(label),
        value,
        blocked: false,
        blocked_reason: SharedString::default(),
    }
}

fn header_dropdown(label: &str) -> DropdownEntry {
    DropdownEntry {
        is_header: true,
        label: SharedString::from(label),
        value: -1,
        blocked: false,
        blocked_reason: SharedString::default(),
    }
}

// Pin model + pin-jump resolvers moved to `tabs/pins.rs`. See
// `pins_glue::{refresh_pin_model, pin_jump_target, axis_back_to_pin,
// shift_reg_back_to_pin, PinJumpTarget}`.

// --- Dropdown dispatch (issue #15) --------------------------------------
//
// Single entry point for every inline dropdown across the configurator.
// Cells emit `dropdown-requested(kind, slot, abs-x, abs-y, width)` when
// clicked; the handler below loads the entries model for `kind`, reads
// the current value for `(kind, slot)`, and opens the AppWindow-level
// overlay. When the user picks, `on_dropdown_picked(value)` dispatches
// back to the right setter based on the (kind, slot) we captured on open.

#[allow(clippy::struct_field_names)]
struct DropdownSinks<'a> {
    pin_model: &'a Rc<VecModel<PinRow>>,
    axis_model: &'a Rc<VecModel<AxisRow>>,
    button_model: &'a Rc<VecModel<ButtonRow>>,
    soft_encoder_model: &'a Rc<VecModel<EncoderRow>>,
    fast_encoder_model: &'a Rc<VecModel<FastEncoderRow>>,
    shift_reg_model: &'a Rc<VecModel<ShiftRegRow>>,
}

#[allow(clippy::too_many_lines)]
fn wire_dropdown_callbacks(
    window: &AppWindow,
    state: &Rc<RefCell<State>>,
    entries: &Rc<VecModel<DropdownEntry>>,
    sinks: &DropdownSinks<'_>,
) {
    let pin_model = sinks.pin_model.clone();
    let axis_model = sinks.axis_model.clone();
    let button_model = sinks.button_model.clone();
    let soft_encoder_model = sinks.soft_encoder_model.clone();
    let fast_encoder_model = sinks.fast_encoder_model.clone();
    let shift_reg_model = sinks.shift_reg_model.clone();

    // --- request open ---------------------------------------------------
    {
        let state = state.clone();
        let entries = entries.clone();
        let w = window.as_weak();
        window.on_dropdown_requested(move |kind_raw, slot, abs_x, abs_y, width| {
            let Some(kind) = DropdownKind::from_i32(kind_raw) else {
                return;
            };
            let (fresh_entries, current_value) = {
                let st = state.borrow();
                let Some(cfg) = st.last_config.as_ref() else {
                    // No config loaded — the row models would be empty
                    // anyway; bail rather than show a stale dropdown.
                    return;
                };
                build_dropdown_for(kind, slot, cfg, &st)
            };
            // Refill the shared entries model.
            while entries.row_count() > 0 {
                entries.remove(0);
            }
            for e in fresh_entries {
                entries.push(e);
            }
            // Remember which picker opened so `picked` knows where to
            // dispatch.
            state.borrow_mut().current_dropdown = Some((kind, slot));
            if let Some(w) = w.upgrade() {
                w.set_dropdown_anchor_x(abs_x);
                w.set_dropdown_anchor_y(abs_y);
                w.set_dropdown_anchor_w(width);
                w.set_dropdown_current_value(current_value);
                w.set_dropdown_open(true);
            }
        });
    }

    // --- picked ---------------------------------------------------------
    {
        let state = state.clone();
        let w = window.as_weak();
        window.on_dropdown_picked(move |value| {
            let context = state.borrow().current_dropdown;
            let Some((kind, slot)) = context else {
                return;
            };
            apply_dropdown_pick(
                kind,
                slot,
                value,
                &state,
                &pin_model,
                &axis_model,
                &button_model,
                &soft_encoder_model,
                &fast_encoder_model,
                &shift_reg_model,
                &w,
            );
            state.borrow_mut().current_dropdown = None;
            if let Some(w) = w.upgrade() {
                w.set_dropdown_open(false);
            }
        });
    }

    // --- dismissed ------------------------------------------------------
    {
        let state = state.clone();
        let w = window.as_weak();
        window.on_dropdown_dismissed(move || {
            state.borrow_mut().current_dropdown = None;
            if let Some(w) = w.upgrade() {
                w.set_dropdown_open(false);
            }
        });
    }
}

/// Resolve `(entries, current_value)` for the requested dropdown kind +
/// row. `state` is borrowed read-only — the caller commits the
/// open-state changes after this returns.
#[allow(clippy::too_many_lines)]
fn build_dropdown_for(
    kind: DropdownKind,
    slot: i32,
    cfg: &DeviceConfig,
    state: &State,
) -> (Vec<DropdownEntry>, i32) {
    use buttons_glue::{
        build_button_op_entries, build_button_shift_entries, build_button_timer_entries,
        build_button_type_entries, build_filter_category_entries,
    };
    match kind {
        DropdownKind::PinFunction => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(USED_PINS_NUM - 1);
            let func_list: Vec<PinFunction> = PinFunction::all().collect();
            let cur = PinFunction::from_i8(cfg.pins[slot_usz]).unwrap_or(PinFunction::NotUsed);
            let cur_index = func_list
                .iter()
                .position(|f| *f == cur)
                .and_then(|i| i32::try_from(i).ok())
                .unwrap_or(0);
            (build_pin_function_entries(&func_list), cur_index)
        }
        DropdownKind::ButtonType => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_BUTTONS_NUM - 1);
            let phy = cfg.buttons[slot_usz].physical_num;
            let entries = build_button_type_entries(Some((&cfg.buttons, slot_usz, phy)));
            (entries, i32::from(cfg.buttons[slot_usz].button_type))
        }
        DropdownKind::ButtonShift => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_BUTTONS_NUM - 1);
            (
                build_button_shift_entries(),
                i32::from(cfg.buttons[slot_usz].shift_modificator()),
            )
        }
        DropdownKind::ButtonOp => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_BUTTONS_NUM - 1);
            (
                build_button_op_entries(),
                i32::from(cfg.buttons[slot_usz].op()),
            )
        }
        DropdownKind::ButtonDelay => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_BUTTONS_NUM - 1);
            (
                build_button_timer_entries(),
                i32::from(cfg.buttons[slot_usz].delay_timer()),
            )
        }
        DropdownKind::ButtonPress => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_BUTTONS_NUM - 1);
            (
                build_button_timer_entries(),
                i32::from(cfg.buttons[slot_usz].press_timer()),
            )
        }
        DropdownKind::ButtonsFilterCategory => {
            let cur = state
                .btn_filter_category
                .and_then(|i| i32::try_from(i).ok())
                .unwrap_or(-1);
            (build_filter_category_entries(), cur)
        }
        DropdownKind::AxisFilter => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_AXIS_NUM - 1);
            let entries: Vec<DropdownEntry> = AxisFilter::all()
                .map(|f| flat_dropdown(f.label(), i32::from(f.to_u8())))
                .collect();
            (entries, i32::from(cfg.axis_config[slot_usz].filter()))
        }
        DropdownKind::AxisResolution => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_AXIS_NUM - 1);
            let entries: Vec<DropdownEntry> = (0u8..16)
                .map(|raw| flat_dropdown(&format!("{}", raw + 1), i32::from(raw)))
                .collect();
            (entries, i32::from(cfg.axis_config[slot_usz].resolution()))
        }
        DropdownKind::AxisChannel => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_AXIS_NUM - 1);
            let entries: Vec<DropdownEntry> = (0u8..16)
                .map(|raw| flat_dropdown(&format!("{raw}"), i32::from(raw)))
                .collect();
            (entries, i32::from(cfg.axis_config[slot_usz].channel()))
        }
        DropdownKind::EncoderSoftMode => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_ENCODERS_NUM - 1);
            let entries: Vec<DropdownEntry> = EncoderMode::all()
                .map(|m| flat_dropdown(m.label(), i32::from(m.to_u8())))
                .collect();
            (entries, i32::from(cfg.encoders[slot_usz]))
        }
        DropdownKind::EncoderFastMode => {
            let slot_usz = usize::try_from(slot)
                .unwrap_or(0)
                .min(MAX_FAST_ENCODER_NUM - 1);
            let entries: Vec<DropdownEntry> = EncoderMode::all()
                .map(|m| flat_dropdown(m.label(), i32::from(m.to_u8())))
                .collect();
            (entries, i32::from(cfg.fast_encoders[slot_usz].mode))
        }
        DropdownKind::ShiftRegType => {
            let slot_usz = usize::try_from(slot)
                .unwrap_or(0)
                .min(MAX_SHIFT_REG_NUM - 1);
            let entries: Vec<DropdownEntry> = ShiftRegType::all()
                .map(|t| flat_dropdown(t.label(), i32::from(t.to_u8())))
                .collect();
            (entries, i32::from(cfg.shift_registers[slot_usz].reg_type))
        }
        DropdownKind::AxisSource => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_AXIS_NUM - 1);
            let entries = build_axis_source_entries(cfg, state.board);
            let cur = cfg.axis_config[slot_usz].source().to_handle();
            (entries, cur)
        }
        DropdownKind::DebugVerbosity => {
            use crate::debug_log::LogLevel;
            let entries: Vec<DropdownEntry> = [
                LogLevel::Trace,
                LogLevel::Debug,
                LogLevel::Info,
                LogLevel::Warn,
                LogLevel::Error,
            ]
            .into_iter()
            .map(|l| flat_dropdown(l.short_label().trim(), l.ui_index()))
            .collect();
            let cur = state.debug_filter.snapshot().min_level.ui_index();
            (entries, cur)
        }
        DropdownKind::AxisFunction => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_AXIS_NUM - 1);
            let entries: Vec<DropdownEntry> = freejoyx_core::domain::AxisFunction::all()
                .map(|f| flat_dropdown(f.label(), i32::from(f.to_u8())))
                .collect();
            (entries, i32::from(cfg.axis_config[slot_usz].function()))
        }
        DropdownKind::AxisFunctionRef => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_AXIS_NUM - 1);
            let entries: Vec<DropdownEntry> = (0..MAX_AXIS_NUM)
                .map(|i| flat_dropdown(AXIS_TITLES[i], i32::try_from(i).unwrap_or(0)))
                .collect();
            (
                entries,
                i32::from(cfg.axis_config[slot_usz].source_secondary()),
            )
        }
        DropdownKind::AxisButtonAction1 => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_AXIS_NUM - 1);
            let entries = build_axis_button_action_entries(false);
            (entries, i32::from(cfg.axis_config[slot_usz].button1_type()))
        }
        DropdownKind::AxisButtonAction2 => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_AXIS_NUM - 1);
            // Slot 2's action is 2-bit on the wire; filter Down / Up out.
            let entries = build_axis_button_action_entries(true);
            (entries, i32::from(cfg.axis_config[slot_usz].button2_type()))
        }
        DropdownKind::AxisButtonAction3 => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_AXIS_NUM - 1);
            let entries = build_axis_button_action_entries(false);
            (entries, i32::from(cfg.axis_config[slot_usz].button3_type()))
        }
        DropdownKind::AxisI2cAddress => {
            let slot_usz = usize::try_from(slot).unwrap_or(0).min(MAX_AXIS_NUM - 1);
            let entries: Vec<DropdownEntry> = freejoyx_core::domain::I2cAddress::all()
                .map(|a| flat_dropdown(a.label(), i32::from(a.to_u8())))
                .collect();
            (entries, i32::from(cfg.axis_config[slot_usz].i2c_address))
        }
    }
}

/// Build the action-picker entries. `slot2_only` keeps just the four
/// actions that fit the 2-bit `button2_type` field (Function enable /
/// Prescale enable / Center / Reset).
fn build_axis_button_action_entries(slot2_only: bool) -> Vec<DropdownEntry> {
    freejoyx_core::domain::AxisButtonAction::all()
        .filter(|a| !slot2_only || a.valid_for_slot2())
        .map(|a| flat_dropdown(a.label(), i32::from(a.to_u8())))
        .collect()
}

/// Build the entries list for the per-axis Source dropdown. Mirrors
/// the Qt configurator's `comboBox_AxisSource1`: None first, then
/// "Encoder N" rows for each completed fast-encoder pair, then one row
/// per pin currently assigned to `PinFunction::AxisAnalog`. Pins not
/// assigned to `AxisAnalog` don't appear — picking them would just
/// hand the firmware a non-analog pin.
fn build_axis_source_entries(cfg: &DeviceConfig, board: Board) -> Vec<DropdownEntry> {
    let mut entries: Vec<DropdownEntry> = Vec::new();
    entries.push(flat_dropdown("None", AxisSource::None.to_handle()));
    let encoder_slots = completed_fast_encoder_slots(&cfg.pins);
    if !encoder_slots.is_empty() {
        entries.push(header_dropdown("Encoder"));
        for slot in encoder_slots {
            entries.push(flat_dropdown(
                &format!("Encoder {}", slot + 1),
                AxisSource::Encoder(slot).to_handle(),
            ));
        }
    }
    let analog = analog_pin_slots(&cfg.pins);
    if !analog.is_empty() {
        entries.push(header_dropdown("Analog pins"));
        for pin_idx in analog {
            entries.push(flat_dropdown(
                board.pin_name(pin_idx as usize),
                AxisSource::Pin(pin_idx).to_handle(),
            ));
        }
    }
    entries
}

fn build_pin_function_entries(all: &[PinFunction]) -> Vec<DropdownEntry> {
    // Skipped: PinFunctionFamily::NotUsed — surfaced as a plain "—"
    // entry at the top of the list rather than its own category.
    let families = [
        (PinFunctionFamily::Button, "Buttons"),
        (PinFunctionFamily::Axis, "Axis"),
        (PinFunctionFamily::Encoder, "Encoder"),
        (PinFunctionFamily::Bus, "Bus (SPI / I2C / UART)"),
        (PinFunctionFamily::Sensor, "Sensor"),
        (PinFunctionFamily::ShiftReg, "Shift register"),
        (PinFunctionFamily::Led, "LED"),
        (PinFunctionFamily::RgbLed, "RGB LED"),
    ];
    let mut entries: Vec<DropdownEntry> = Vec::with_capacity(all.len() + families.len());
    // "Not used" lives at index 0 of `PinFunction::all()`. Surface it
    // as a bare dash with no category header.
    if let Some(unused_idx) = all
        .iter()
        .position(|f| f.family() == PinFunctionFamily::NotUsed)
    {
        entries.push(flat_dropdown("—", i32::try_from(unused_idx).unwrap_or(0)));
    }
    for (family, label) in families {
        let group: Vec<(usize, &PinFunction)> = all
            .iter()
            .enumerate()
            .filter(|(_, f)| f.family() == family)
            .collect();
        if group.is_empty() {
            continue;
        }
        entries.push(header_dropdown(label));
        for (i, f) in group {
            entries.push(flat_dropdown(f.label(), i32::try_from(i).unwrap_or(0)));
        }
    }
    entries
}

#[allow(clippy::too_many_arguments, clippy::too_many_lines)]
fn apply_dropdown_pick(
    kind: DropdownKind,
    slot: i32,
    value: i32,
    state: &Rc<RefCell<State>>,
    pin_model: &Rc<VecModel<PinRow>>,
    axis_model: &Rc<VecModel<AxisRow>>,
    button_model: &Rc<VecModel<ButtonRow>>,
    soft_encoder_model: &Rc<VecModel<EncoderRow>>,
    fast_encoder_model: &Rc<VecModel<FastEncoderRow>>,
    shift_reg_model: &Rc<VecModel<ShiftRegRow>>,
    window: &slint::Weak<AppWindow>,
) {
    match kind {
        DropdownKind::PinFunction => {
            let Ok(slot_usz) = usize::try_from(slot) else {
                return;
            };
            let Ok(fn_index) = usize::try_from(value) else {
                return;
            };
            if slot_usz >= USED_PINS_NUM {
                return;
            }
            let Some(new_fn) = PinFunction::all().nth(fn_index) else {
                return;
            };
            let (board, cfg_clone, prev_fn) = {
                let mut s = state.borrow_mut();
                let board = s.board;
                let Some(cfg) = s.last_config.as_mut() else {
                    return;
                };
                let prev_fn = PinFunction::from_i8(cfg.pins[slot_usz]);
                cfg.pins[slot_usz] = new_fn.to_i8();
                (board, cfg.clone(), prev_fn)
            };
            tracing::info!(
                target: "freejoyx::pin",
                slot = slot_usz as u64,
                pin = board.pin_name(slot_usz),
                prev = ?prev_fn,
                new = ?new_fn,
                "function changed"
            );
            pins_glue::refresh_pin_model(pin_model, &cfg_clone, board);
            // Pin function changes flip jump-eligibility on the
            // affected pin AND back-to-pin-eligibility on the rows
            // that referenced it. Rebuilding the dependent models
            // keeps the new "←" button state in sync with reality —
            // a freshly-cleared AxisAnalog pin greys out its axis's
            // back-button, dropping a ShiftRegData pin greys out the
            // back-button on the corresponding SR row.
            let axis_inputs = AxisRenderInputs::from_state(&state.borrow());
            let live = state
                .borrow()
                .last_params
                .as_ref()
                .map(|p| (p.raw_axis_data, p.axis_data));
            refresh_axis_model(axis_model, &cfg_clone, &axis_inputs, live);
            let chip_size = state.borrow().shift_reg_chip_size;
            encoders_glue::refresh_shift_reg_model(shift_reg_model, &cfg_clone, chip_size);
            if let Some(w) = window.upgrade() {
                w.set_can_write(true);
                w.set_can_save(true);
            }
        }
        DropdownKind::ButtonType => {
            let Ok(slot_usz) = usize::try_from(slot) else {
                return;
            };
            let Ok(value_u8) = u8::try_from(value) else {
                return;
            };
            if ButtonType::from_u8(value_u8).is_none() {
                return;
            }
            let _ = buttons_glue::with_button_slot(state, slot_usz, |b, all_buttons| {
                let candidate = ButtonType::from_u8(value_u8).unwrap_or(ButtonType::Normal);
                if let CoexistenceCheck::Ok =
                    physical_assignment_blocked(all_buttons, slot_usz, b.physical_num, candidate)
                {
                    b.button_type = value_u8;
                }
            });
            refresh_button_after_edit(state, button_model, slot_usz);
            buttons_glue::mark_dirty(window);
        }
        DropdownKind::ButtonShift => apply_button_int(
            state,
            button_model,
            window,
            slot,
            value,
            0,
            8,
            freejoyx_core::wire::Button::set_shift_modificator,
        ),
        DropdownKind::ButtonOp => apply_button_int(
            state,
            button_model,
            window,
            slot,
            value,
            0,
            6,
            freejoyx_core::wire::Button::set_op,
        ),
        DropdownKind::ButtonDelay => apply_button_int(
            state,
            button_model,
            window,
            slot,
            value,
            0,
            3,
            freejoyx_core::wire::Button::set_delay_timer,
        ),
        DropdownKind::ButtonPress => apply_button_int(
            state,
            button_model,
            window,
            slot,
            value,
            0,
            3,
            freejoyx_core::wire::Button::set_press_timer,
        ),
        DropdownKind::ButtonsFilterCategory => {
            {
                let mut st = state.borrow_mut();
                st.btn_filter_category = if value < 0 {
                    None
                } else {
                    usize::try_from(value).ok()
                };
            }
            buttons_glue::rebuild_filtered(state, button_model, window);
        }
        DropdownKind::AxisFilter => apply_axis_field(
            state,
            axis_model,
            window,
            slot,
            value,
            0,
            7,
            freejoyx_core::wire::AxisConfig::set_filter,
        ),
        DropdownKind::AxisResolution => apply_axis_field(
            state,
            axis_model,
            window,
            slot,
            value,
            0,
            15,
            freejoyx_core::wire::AxisConfig::set_resolution,
        ),
        DropdownKind::AxisChannel => apply_axis_field(
            state,
            axis_model,
            window,
            slot,
            value,
            0,
            15,
            freejoyx_core::wire::AxisConfig::set_channel,
        ),
        DropdownKind::EncoderSoftMode => {
            // The dropdown emits the *wire* slot (encoders[] index 2..15).
            // The display row index is wire_slot - MAX_FAST_ENCODER_NUM.
            let Ok(wire_slot) = usize::try_from(slot) else {
                return;
            };
            if wire_slot < MAX_FAST_ENCODER_NUM || wire_slot >= MAX_ENCODERS_NUM {
                return;
            }
            let row = wire_slot - MAX_FAST_ENCODER_NUM;
            let Ok(mode) = u8::try_from(value.clamp(0, 2)) else {
                return;
            };
            {
                let mut s = state.borrow_mut();
                let Some(cfg) = s.last_config.as_mut() else {
                    return;
                };
                cfg.encoders[wire_slot] = mode;
            }
            {
                let s = state.borrow();
                if let Some(cfg) = s.last_config.as_ref() {
                    let pairs = freejoyx_core::domain::pair_soft_encoders(&cfg.buttons);
                    soft_encoder_model.set_row_data(
                        row,
                        encoders_glue::build_soft_encoder_row(
                            row,
                            wire_slot,
                            cfg.encoders[wire_slot],
                            pairs[wire_slot],
                            s.last_params.as_ref(),
                        ),
                    );
                }
            }
            buttons_glue::mark_dirty(window);
        }
        DropdownKind::EncoderFastMode => {
            let Ok(slot_usz) = usize::try_from(slot) else {
                return;
            };
            if slot_usz >= MAX_FAST_ENCODER_NUM {
                return;
            }
            let Ok(mode) = u8::try_from(value.clamp(0, 2)) else {
                return;
            };
            {
                let mut s = state.borrow_mut();
                let Some(cfg) = s.last_config.as_mut() else {
                    return;
                };
                cfg.fast_encoders[slot_usz].mode = mode;
            }
            if let Some(cfg) = state.borrow().last_config.as_ref() {
                fast_encoder_model.set_row_data(
                    slot_usz,
                    encoders_glue::build_fast_encoder_row(slot_usz, &cfg.fast_encoders[slot_usz]),
                );
            }
            buttons_glue::mark_dirty(window);
        }
        DropdownKind::ShiftRegType => {
            let Ok(slot_usz) = usize::try_from(slot) else {
                return;
            };
            if slot_usz >= MAX_SHIFT_REG_NUM {
                return;
            }
            let Ok(reg_type) = u8::try_from(value.clamp(0, 3)) else {
                return;
            };
            {
                let mut s = state.borrow_mut();
                let Some(cfg) = s.last_config.as_mut() else {
                    return;
                };
                cfg.shift_registers[slot_usz].reg_type = reg_type;
            }
            {
                let st = state.borrow();
                if let Some(cfg) = st.last_config.as_ref() {
                    shift_reg_model.set_row_data(
                        slot_usz,
                        encoders_glue::build_shift_reg_row(
                            slot_usz,
                            cfg,
                            st.shift_reg_chip_size[slot_usz],
                        ),
                    );
                }
            }
            buttons_glue::mark_dirty(window);
        }
        DropdownKind::AxisSource => {
            let Ok(slot_usz) = usize::try_from(slot) else {
                return;
            };
            if slot_usz >= MAX_AXIS_NUM {
                return;
            }
            let new_source = AxisSource::from_handle(value);
            apply_axis_source(state, axis_model, pin_model, window, slot_usz, new_source);
        }
        DropdownKind::DebugVerbosity => {
            use crate::debug_log::LogLevel;
            let new_level = match value {
                0 => LogLevel::Trace,
                1 => LogLevel::Debug,
                2 => LogLevel::Info,
                3 => LogLevel::Warn,
                4 => LogLevel::Error,
                _ => return,
            };
            {
                let s = state.borrow();
                s.debug_filter.update(|f| f.min_level = new_level);
            }
            if let Some(w) = window.upgrade() {
                refresh_verbosity_labels(&w, &state.borrow().debug_filter);
            }
        }
        DropdownKind::AxisFunction => apply_axis_field(
            state,
            axis_model,
            window,
            slot,
            value,
            0,
            3,
            freejoyx_core::wire::AxisConfig::set_function,
        ),
        DropdownKind::AxisFunctionRef => apply_axis_field(
            state,
            axis_model,
            window,
            slot,
            value,
            0,
            i32::try_from(MAX_AXIS_NUM - 1).unwrap_or(7),
            freejoyx_core::wire::AxisConfig::set_source_secondary,
        ),
        DropdownKind::AxisButtonAction1 => apply_axis_field(
            state,
            axis_model,
            window,
            slot,
            value,
            0,
            7,
            freejoyx_core::wire::AxisConfig::set_button1_type,
        ),
        DropdownKind::AxisButtonAction2 => apply_axis_field(
            state,
            axis_model,
            window,
            slot,
            value,
            0,
            3,
            freejoyx_core::wire::AxisConfig::set_button2_type,
        ),
        DropdownKind::AxisButtonAction3 => apply_axis_field(
            state,
            axis_model,
            window,
            slot,
            value,
            0,
            7,
            freejoyx_core::wire::AxisConfig::set_button3_type,
        ),
        DropdownKind::AxisI2cAddress => apply_axis_field(
            state,
            axis_model,
            window,
            slot,
            value,
            0,
            i32::from(u8::MAX),
            |a, v| a.i2c_address = v,
        ),
    }
    // Every arm above either mutates `cfg` or no-ops; rather than
    // sprinkling indicator refreshes inside each arm, recompute once
    // at the bottom. The cost is six bool comparisons + six setter
    // calls on the held window — cheap enough to do on every pick.
    if let Some(w) = window.upgrade() {
        if let Some(cfg) = state.borrow().last_config.as_deref() {
            refresh_tab_indicators(&w, cfg);
        }
    }
}

/// Write `new_source` into `axis_config[slot]` and apply the
/// "no source -> no output" guard. Force-clears `out_enabled` when the
/// source flips to None so a freshly-loaded config with `source=None,
/// out=true` doesn't keep the firmware emitting axis data from an
/// invalid source. Also rebuilds the Pins-tab model so the per-pin
/// jump-button `enabled` flag flips when an axis newly points at (or
/// releases) a pin.
fn apply_axis_source(
    state: &Rc<RefCell<State>>,
    axis_model: &Rc<VecModel<AxisRow>>,
    pin_model: &Rc<VecModel<PinRow>>,
    window: &slint::Weak<AppWindow>,
    slot: usize,
    new_source: AxisSource,
) {
    let prev_source = {
        let mut s = state.borrow_mut();
        let Some(cfg) = s.last_config.as_mut() else {
            return;
        };
        let prev = cfg.axis_config[slot].source();
        cfg.axis_config[slot].set_source(new_source);
        if matches!(new_source, AxisSource::None) {
            cfg.axis_config[slot].set_out_enabled(false);
        }
        prev
    };
    tracing::info!(
        target: "freejoyx::axis",
        slot = slot as u64,
        prev = ?prev_source,
        new = ?new_source,
        "source changed"
    );
    refresh_axis_row(state, axis_model, slot);
    let st = state.borrow();
    let board = st.board;
    if let Some(cfg) = st.last_config.as_ref() {
        refresh_axes_viewport_height(window, cfg, &st.axes_extended_expanded);
        pins_glue::refresh_pin_model(pin_model, cfg, board);
    }
    drop(st);
    buttons_glue::mark_dirty(window);
}

/// Rebuild a single axis row using the current `State` + config.
/// Convenience over inlining the boilerplate at every callsite.
fn refresh_axis_row(state: &Rc<RefCell<State>>, axis_model: &Rc<VecModel<AxisRow>>, slot: usize) {
    let s = state.borrow();
    let Some(cfg) = s.last_config.as_ref() else {
        return;
    };
    let live_raw = s
        .last_params
        .as_ref()
        .map_or(0, |p| i32::from(p.raw_axis_data[slot]));
    let live_out = s
        .last_params
        .as_ref()
        .map_or(0, |p| i32::from(p.axis_data[slot]));
    let row = build_axis_row(
        slot,
        cfg,
        s.board,
        live_raw,
        live_out,
        s.axis_calibrate.armed_slot() == Some(slot),
        s.axis_detect.armed_slot() == Some(slot),
        s.axis_detect.disarm_tick(slot),
        s.axes_extended_expanded[slot],
    );
    axis_model.set_row_data(slot, row);
}

#[allow(clippy::too_many_arguments)]
fn apply_button_int(
    state: &Rc<RefCell<State>>,
    button_model: &Rc<VecModel<ButtonRow>>,
    window: &slint::Weak<AppWindow>,
    slot: i32,
    value: i32,
    lo: i32,
    hi: i32,
    cb: impl Fn(&mut freejoyx_core::wire::Button, u8),
) {
    let Ok(slot_usz) = usize::try_from(slot) else {
        return;
    };
    let Ok(clamped) = u8::try_from(value.clamp(lo, hi)) else {
        return;
    };
    let _ = buttons_glue::with_button_slot(state, slot_usz, |b, _| cb(b, clamped));
    refresh_button_after_edit(state, button_model, slot_usz);
    buttons_glue::mark_dirty(window);
}

#[allow(clippy::too_many_arguments)]
fn apply_axis_field(
    state: &Rc<RefCell<State>>,
    axis_model: &Rc<VecModel<AxisRow>>,
    window: &slint::Weak<AppWindow>,
    slot: i32,
    value: i32,
    lo: i32,
    hi: i32,
    cb: impl Fn(&mut freejoyx_core::wire::AxisConfig, u8),
) {
    let Ok(slot_usz) = usize::try_from(slot) else {
        return;
    };
    if slot_usz >= MAX_AXIS_NUM {
        return;
    }
    let Ok(clamped) = u8::try_from(value.clamp(lo, hi)) else {
        return;
    };
    let row = {
        let mut s = state.borrow_mut();
        if s.last_config.is_none() {
            return;
        }
        let live_raw = s
            .last_params
            .as_ref()
            .map_or(0, |p| i32::from(p.raw_axis_data[slot_usz]));
        let live_out = s
            .last_params
            .as_ref()
            .map_or(0, |p| i32::from(p.axis_data[slot_usz]));
        let board = s.board;
        let calibrating = s.axis_calibrate.armed_slot() == Some(slot_usz);
        let armed = s.axis_detect.armed_slot() == Some(slot_usz);
        let disarm_tick = s.axis_detect.disarm_tick(slot_usz);
        let expanded = s.axes_extended_expanded[slot_usz];
        let cfg = s.last_config.as_mut().expect("checked is_none above");
        cb(&mut cfg.axis_config[slot_usz], clamped);
        build_axis_row(
            slot_usz,
            cfg,
            board,
            live_raw,
            live_out,
            calibrating,
            armed,
            disarm_tick,
            expanded,
        )
    };
    axis_model.set_row_data(slot_usz, row);
    if let Some(w) = window.upgrade() {
        w.set_can_write(w.get_connected());
        w.set_can_save(true);
    }
}

fn refresh_button_after_edit(
    state: &Rc<RefCell<State>>,
    button_model: &Rc<VecModel<ButtonRow>>,
    slot: usize,
) {
    let s = state.borrow();
    if let Some(cfg) = s.last_config.as_ref() {
        buttons_glue::refresh_button_row(
            button_model,
            slot,
            cfg,
            s.last_params.as_ref(),
            s.button_capture.disarm_ticks(slot),
        );
    }
}

thread_local! {
    /// Previous tick's logical-button bitmap, kept here so the
    /// edge-detection in [`log_button_bitmap_edges`] doesn't need a
    /// matching field on `State`. Initialised to all-zero; we don't
    /// need to reset on disconnect because edges only fire when bits
    /// actually transition between consecutive ticks.
    static LAST_LOGICAL_BITMAP: std::cell::RefCell<[u8; BUTTON_BITMAP_BYTES]> =
        std::cell::RefCell::new([0; BUTTON_BITMAP_BYTES]);
}

/// Emit one `tracing::info!` per button bitmap edge (physical +
/// logical) between the previous tick and this one. Targets
/// `freejoyx::button` so the Debug tab's Button category catches it.
/// At 30 Hz with quiet inputs this fires zero events; pressing a
/// button fires one (press) + one (release).
/// First physical-button slot that transitioned 0→1 between
/// `prev` and `curr`. Returns the slot index (0..`MAX_BUTTONS_NUM`) or
/// `None` if no rising edges. Used by the buttons-tab "press to filter"
/// arm — the next press captures into `btn_filter_physical`.
fn first_rising_edge(
    prev: &[u8; BUTTON_BITMAP_BYTES],
    curr: &[u8; BUTTON_BITMAP_BYTES],
) -> Option<usize> {
    for byte_idx in 0..BUTTON_BITMAP_BYTES {
        let rising = curr[byte_idx] & !prev[byte_idx];
        if rising == 0 {
            continue;
        }
        // `trailing_zeros` gives the lowest-numbered set bit, which is
        // the slot the user actually pressed first within this byte.
        let bit = rising.trailing_zeros() as usize;
        let slot = byte_idx * 8 + bit;
        if slot < MAX_BUTTONS_NUM {
            return Some(slot);
        }
    }
    None
}

fn log_button_bitmap_edges(prev_phy: &[u8; BUTTON_BITMAP_BYTES], params: &ParamsReport) {
    let prev_log = LAST_LOGICAL_BITMAP.with(|b| *b.borrow());
    for byte_idx in 0..BUTTON_BITMAP_BYTES {
        let phy_now = params.phy_button_data[byte_idx];
        let phy_rising = phy_now & !prev_phy[byte_idx];
        let phy_falling = prev_phy[byte_idx] & !phy_now;
        let log_now = params.log_button_data[byte_idx];
        let log_rising = log_now & !prev_log[byte_idx];
        let log_falling = prev_log[byte_idx] & !log_now;
        if phy_rising == 0 && phy_falling == 0 && log_rising == 0 && log_falling == 0 {
            continue;
        }
        for bit in 0..8u32 {
            let mask = 1u8 << bit;
            let slot = byte_idx * 8 + bit as usize;
            if slot >= MAX_BUTTONS_NUM {
                break;
            }
            if phy_rising & mask != 0 {
                tracing::info!(target: "freejoyx::button", slot = slot as u64, "physical pressed");
            }
            if phy_falling & mask != 0 {
                tracing::info!(target: "freejoyx::button", slot = slot as u64, "physical released");
            }
            if log_rising & mask != 0 {
                tracing::info!(target: "freejoyx::button", slot = slot as u64, "logical pressed");
            }
            if log_falling & mask != 0 {
                tracing::info!(target: "freejoyx::button", slot = slot as u64, "logical released");
            }
        }
    }
    LAST_LOGICAL_BITMAP.with(|b| {
        *b.borrow_mut() = params.log_button_data;
    });
}

// `apply_button_capture`, `apply_axis_calibration`, `apply_axis_detect`
// were folded into the `domain::modes::*` state machines. See
// `freejoyx-core::domain::modes` — each Mode owns its arm/disarm state
// and has a single `on_params_tick` method the `DeviceEvent::ParamsTick`
// handler above calls.
