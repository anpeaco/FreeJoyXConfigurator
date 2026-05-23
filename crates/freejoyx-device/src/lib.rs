//! freejoyx-device
//!
//! HID transport + device worker thread.
//!
//! - [`transport`] — synchronous `Device` over `hidapi`. Mockable
//!   surface for tests; real I/O on the worker thread.
//! - [`worker`] — background thread + `mpsc` channels. The UI (or CLI)
//!   calls [`spawn`] and consumes [`DeviceEvent`]s from the returned
//!   receiver; commands flow the other way via [`DeviceHandle::send`].
//!
//! Per Port.md §3 "Threading model": one thread owns the `hidapi`
//! handle, exchanges messages with the UI via `std::sync::mpsc`
//! channels, and Slint's `invoke_from_event_loop` plays the role of
//! Qt's `Qt::QueuedConnection` on the consumer side.

#![forbid(unsafe_code)]
#![warn(clippy::pedantic)]

pub mod error;
pub(crate) mod subscription;
pub mod transport;
pub mod worker;

pub use error::TransportError;
pub use transport::{
    config_dump_enabled, enumerate, set_config_dump_enabled, Device, DeviceCandidate, Transport,
};
pub use worker::{spawn, spawn_for_serial, Command, DeviceEvent, DeviceHandle};
