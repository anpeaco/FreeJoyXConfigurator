//! freejoyx-core
//!
//! Pure-logic crate: wire codec, domain types, validators, on-disk serde.
//! No I/O. All TDD targets live here.
//!
//! Module layout (per Port.md §3 "Domain layering inside freejoyx-core"):
//! - `wire`   — encode/decode bytes <-> domain types (Path B manual codec)
//! - `domain` — idiomatic Rust types + validators (pin conflicts, LOGIC completeness)
//! - `persist`— serde derives + RON read/write for `.freejoyx-config.ron`

#![forbid(unsafe_code)]
#![warn(clippy::all)]

pub mod domain;
pub mod persist;
pub mod wire;
