//! Per-tab modules.
//!
//! Each sub-module owns the view-model builders, the Slint refresh
//! glue, and the dropdown-pick arms for one tab in the configurator.
//! Cross-tab state (the current `DeviceConfig`, the worker handle)
//! lives on [`crate::app::State`]; tab modules borrow it on each call
//! rather than owning a slice of it as a struct field — keeps every
//! callback's borrow story explicit.
//!
//! The split reduces `app.rs` from a god-file with 280-line dropdown
//! match statements to a top-level coordinator that wires tabs into
//! the Slint property surface and dispatches events by tab. See
//! `ARCHITECTURE_BACKLOG.md` #1 for the full rationale.

pub mod advanced;
pub mod buttons;
pub mod encoders;
pub mod pins;
