//! Configurator-side interactive-mode state machines.
//!
//! Each sub-module is one "the user is currently holding this UI mode
//! open" state machine. They have a uniform shape:
//!
//! - A struct owning the arm bit + per-slot disarm-tick counters.
//! - `arm(...)` / `disarm()` / `clear()` methods.
//! - `on_params_tick(&ParamsReport, &mut DeviceConfig, …) -> Outcome`
//!   that the UI calls once per device tick.
//! - An `Outcome` struct describing what the UI should refresh.
//!
//! Modes never touch Slint types — they take wire + domain types and
//! return plain Rust values, so each one is unit-testable against a
//! synthesised `ParamsReport` stream without a UI runtime.

pub mod axis_calibration;
pub mod axis_detect;
pub mod button_capture;

pub use axis_calibration::{AxisCalibration, AxisCalibrationOutcome};
pub use axis_detect::{AxisDetect, AxisDetectOutcome, AXIS_DETECT_THRESHOLD, AXIS_DETECT_TIMEOUT};
pub use button_capture::{ButtonCapture, ButtonCaptureOutcome, CaptureTarget};
