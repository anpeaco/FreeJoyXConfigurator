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

pub mod logic;
pub mod pins;

pub use logic::{validate_logic_buttons, LogicError, LogicOp, BUTTON_TYPE_LOGIC};
pub use pins::{validate_pins, Board, PinConflict, PinConflictKind, PinFunction};
