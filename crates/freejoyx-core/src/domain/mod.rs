//! Domain types + validators.
//!
//! The wire codec in [`super::wire`] produces idiomatic Rust types
//! (Path B per Port.md §3); this module supplies the supporting enums
//! and the validators that surface configuration mistakes back to the
//! UI.
//!
//! Sub-modules:
//!
//! - [`pins`] — `PinFunction` enum (the `pin_t` values from
//!   `vendored/common_types.h`), `Board` enum with per-board pin name
//!   tables, and `validate_pins` for the Pins tab.
//! - [`logic`] — `LogicOp` enum, the `BUTTON_TYPE_LOGIC` constant, and
//!   `validate_logic_buttons` (port of
//!   `ButtonLogical::isLogicConfigComplete`).
//! - [`axes`] — `AxisFilter` enum mirroring the 3-bit filter field.
//! - [`buttons`] — `ButtonType` enum, Button bitfield setters, and the
//!   per-physical coexistence rule from F103_GESTURE_PLAN.md.
//! - [`encoders`] — `EncoderMode` enum (1x/2x/4x) mirroring `encoder_t`.
//! - [`shift_registers`] — `ShiftRegType` enum mirroring
//!   `shift_reg_config_type_t` (HC165 / CD4021 × pull-down / pull-up).

pub mod axes;
pub mod buttons;
pub mod encoders;
pub mod logic;
pub mod modes;
pub mod pins;
pub mod shift_registers;
pub mod validation;

pub use axes::{
    analog_pin_slots, completed_fast_encoder_slots, AxisButtonAction, AxisFilter, AxisFunction,
    AxisSource, I2cAddress, AXIS_SOURCE_ENCODER, AXIS_SOURCE_I2C, AXIS_SOURCE_NONE,
};
pub use buttons::{physical_assignment_blocked, ButtonType, ButtonTypeCategory, CoexistenceCheck};
pub use encoders::{pair_soft_encoders, EncoderMode, SoftEncoderPair};
pub use logic::{validate_logic_buttons, LogicError, LogicOp, BUTTON_TYPE_LOGIC};
pub use modes::{
    AxisCalibration, AxisCalibrationOutcome, AxisDetect, AxisDetectOutcome, ButtonCapture,
    ButtonCaptureOutcome, CaptureTarget, AXIS_DETECT_THRESHOLD, AXIS_DETECT_TIMEOUT,
};
pub use pins::{
    validate_pins, Board, BoardSlot, PinConflict, PinConflictKind, PinFunction, PinFunctionFamily,
    BOARD_LAYOUT_LEN,
};
pub use shift_registers::ShiftRegType;
pub use validation::{validate_for_write, ConfigError};
