# FreeJoyXConfigurator

A native Rust + Slint configurator for the
[FreeJoyX](https://github.com/anpeaco/FreeJoyX) DIY USB HID game
controller firmware.

**Status:** Slice 0 (workspace bootstrap). The codec, transport, and UI
all land slice-by-slice in subsequent commits. See `Port.md` for the
execution plan.

## Why a new repo

This is a from-scratch reimplementation of the existing Qt 5
configurator
([`FreeJoyXConfiguratorQt`](https://github.com/anpeaco/FreeJoyXConfiguratorQt))
in Rust + Slint. The Qt app stays the authoritative tool until this
one reaches v0.1 (target: ~4 weeks of focused work) — see Port.md §1
for the goal/non-goal scope. The Qt app continues to support legacy
firmware versions; the Rust app targets only the current FreeJoyX
wire-format generation (`FIRMWARE_VERSION = 0x0010`).

## Layout

```
crates/
├── freejoyx-core/    # wire codec, domain types, validators, on-disk serde
├── freejoyx-device/  # HID transport, worker thread, channel API
├── freejoyx-ui/      # Slint UI
└── freejoyx-app/     # bin crate; wires the above
fixtures/             # byte fixtures captured from real devices (the codec oracle)
vendored/             # read-only copies of FreeJoyX firmware headers
docs/                 # capture patch + port-from notes
Port.md               # the execution plan (read this before contributing)
Style.MD              # UI visual style guide
```

## Build

Requires Rust stable (1.85+). Standard cargo:

```sh
cargo build
cargo test
cargo fmt --check
cargo clippy -- -D warnings
```

The CI workflow (`.github/workflows/ci.yml`) runs the same on
Ubuntu / Windows / macOS on every push.

## Wire-format drift

This repo vendors `common_defines.h` + `common_types.h` from
`anpeaco/FreeJoyX` into `vendored/`. The
`.github/workflows/header-sync.yml` job clones the firmware repo on
every push and fails CI if the vendored copies drift. See Port.md
§10 for the cross-repo lockstep procedure.

## License

GPLv3 — inherited from the upstream FreeJoy projects.
