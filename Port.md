# FreeJoyXConfigurator — Rust + Slint Port Plan

Status: **planning artifact, locked-in decisions** — no Rust code yet. This
document is the execution spec for the new `FreeJoyXConfigurator` Rust
workspace and replaces the FreeJoyXConfiguratorQt Qt 5 app.

Reference / oracle repo:
[`anpeaco/FreeJoyXConfiguratorQt`](https://github.com/anpeaco/FreeJoyXConfiguratorQt)
(GPLv3). Clone as a sibling checkout before starting Slice 0 — the codec,
fixtures, and behaviour parity are all anchored against it.

Last revised by the planning grill: 2026-05-16.

---

## 1. Goals & non-goals

### Goals

- Native single-binary configurator written in **Rust + Slint** that talks
  to the existing FreeJoyX firmware over HID using the on-the-wire format
  from `common_defines.h` / `common_types.h`.
- **Round-trip parity** for the *full* 1580-byte `dev_config_t`: every
  byte the device hands us must come back unchanged on write. This is the
  non-negotiable correctness floor.
- **UI parity** for the surfaces the maintainer actually uses on their
  build: Pins, Axes (linear + deadband + filter), Encoders, Buttons +
  Logic + Shifts & Timers, Shift Registers. Advanced Settings
  (name/VID/PID). See §1.1 for the deferred surfaces.
- A well-tested **codec + domain core** (TDD'd against captured byte
  fixtures), so future firmware revs and config schema changes are safe.
- Build & run on Windows, Linux, macOS from one source tree.

### 1.1 Surfaces deferred to v0.1.1+

These surfaces **round-trip bytes faithfully** but have no UI in v0.1.
Configs loaded from the Qt app that contain them are preserved unchanged
on write.

- Single/PWM LEDs (24 channels)
- RGB LEDs (50, color wheel + effect editor — single largest custom-painted
  widget; deferred so v0.1 doesn't hinge on the Slint color-wheel port)
- Sensor configuration (TLE5011, MLX90393, AS5048A, AS5600, etc.)
- 11-point axis curve editor (linear + deadband + filter cover the
  maintainer's build; spline editor returns when there's a use for it)

These get UI tabs in a later release once the v0.1 foundation is stable.

### 1.2 Non-goals (v0.1 and forever, unless reopened)

- Legacy upstream FreeJoy firmware support (`0x1700`, `0x1770`, etc).
  Stays in the Qt app. The Rust app refuses unknown firmware-version
  mask groups with a clear message pointing at the Qt app.
- Pixel-perfect parity with the Qt UI. The Slint UI follows
  `Style.MD` (DCSBoards-style dark cockpit panel), not the Qt
  widget tree.
- Localization (de_DE, ru, zh_CN). English-only first; add `fluent-rs`
  later if demand exists.
- Firmware flashing. Out of scope for v0.1 entirely — defer indefinitely
  until someone actually needs it from the Rust app.

---

## 2. Source repo at a glance

Anchor file references (in `FreeJoyXConfiguratorQt`) the Rust port is
written against:

| Concern | Path |
|---|---|
| Wire-format header (sync'd w/ firmware) | `src/common_defines.h`, `src/common_types.h` |
| HID I/O + worker thread | `src/hiddevice.{h,cpp}` (~42 KB) |
| Device config model | `src/deviceconfig.{h,cpp}` |
| INI serializer | `src/configtofile.{h,cpp}` |
| Top-level UI controller | `src/mainwindow.{h,cpp,ui}` |
| Tab widgets | `src/widgets/{pins,buttons,shifts-timers,axes,axes-curves,encoders,shift-reg,adv-settings,...}/` |
| Vendored HIDAPI (per-OS) | `src/{windows,linux,mac}/hidapi.c` |
| CI | `.github/workflows/configurator.yml`, `.github/workflows/header-sync.yml` |

### Wire-format constants the Rust port locks to

- `FIRMWARE_VERSION = 0x0020` (FreeJoyX gen 2; mask group `& 0xFFF0`).
  Bumped from gen 1 (`0x0010`) for the LONG_PRESS → TAP semantic
  rename; `dev_config_t` byte layout is unchanged.
- `FREEJOY_DEV_CONFIG_SIZE = 1580` bytes — transmitted as 26 × 62-byte
  HID fragments via `REPORT_ID_CONFIG_OUT`
- `FREEJOY_PARAMS_REPORT_SIZE = 72` bytes — params/state report, ID = 2
- Report IDs: `JOY=1`, `PARAM=2`, `CONFIG_IN=3`, `CONFIG_OUT=4`, `FIRMWARE=5`,
  `LED=…` (see `common_defines.h`)
- The C side enforces sizes with `static_assert` at `common_types.h:643-651`.
  The Rust codec asserts the same via integration test that the vendored
  `_Static_assert` expressions still hold for the values we encode.

### Threading model in the Qt app (what we're replacing)

`HidDevice` runs on a `QThread`; internally it spawns a `std::thread` that
calls `hid_enumerate()` every ~600 ms. Signals
(`deviceConnected`, `paramsPacketReceived`, `configReceived`, etc.) cross
back to the UI thread via Qt's queued connections. `sendLedState` uses
`Qt::DirectConnection` because the worker has no event loop.

Rust equivalent: a single `std::thread` (or `tokio` task) owning the
`hidapi` handle, exchanging messages with the UI via `std::sync::mpsc`
channels. Slint's `invoke_from_event_loop` plays the role of
`Qt::QueuedConnection`.

### LOGIC button evaluation — NOT in the configurator

The Qt configurator (and therefore the Rust port) does **not** evaluate
LOGIC buttons. The firmware does. The configurator only:

- Writes LOGIC config fields (`op`, `src_a`, `src_b`, `shift_modificator`,
  `is_inverted`) into `dev_config_t`.
- Receives the resulting **already-evaluated** logical-button bitmap in
  the params packet and shows it live.
- Validates that LOGIC config is complete before write (`op` picked,
  `src_b` set if op is binary) — port of `ButtonLogical::isLogicConfigComplete()`.

No logic evaluator in the codec or domain layer. The original plan
listed one as the #1 test-coverage priority — that was wrong; it would
have duplicated firmware behaviour the configurator never needs.

---

## 3. Target architecture

### Crate layout (3 crates, not 4)

```
FreeJoyXConfigurator/
├── Cargo.toml          # workspace
├── crates/
│   ├── freejoyx-core/    # pure: codec, domain types, validators, on-disk serde. TDD here.
│   ├── freejoyx-device/  # impure: HID transport, device worker, channel API
│   ├── freejoyx-ui/      # Slint UI; depends on core + device
│   └── freejoyx-app/     # bin crate; wires UI + worker; main()
├── vendored/
│   ├── common_defines.h  # read-only mirror of FreeJoyX firmware's header
│   └── common_types.h    # ditto
└── docs/
    └── ported-from.md    # link back to FreeJoyXConfiguratorQt + commit SHA
```

Three crates, not four. The original plan split `freejoy-proto` (codec)
from `freejoy-config` (domain) — collapsed because:

- The codec strategy is Path B (manual encode/decode walking bytes
  field-by-field), which produces idiomatic Rust domain types directly.
  There is no separate "wire mirror struct" to translate from.
- One crate = one place where wire format and domain model live. The
  drift surface that bit the Qt app (per
  `memory/feedback_wire_format_archival.md`) doesn't get a chance to form.

If someone ever needs `freejoyx-core` as a standalone library, it can be
split later; doing it speculatively up front pays no dividend.

### Key dependencies

| Need | Crate |
|---|---|
| HID I/O | `hidapi` (Rust bindings) |
| Sync / channels | `std::sync::mpsc` (no `tokio` — the worker loop is one thread polling HID at ~600 ms) |
| On-disk config | `serde` + `ron` |
| UI | `slint` 1.13 (`default-features = false`, features per `Style.MD`) |
| Bundling | `cargo-bundle` (Win/Mac) + AppImage script (Linux) |
| Logging | `tracing` + `tracing-subscriber` |

No `zerocopy`, no `bytemuck`. The codec is hand-written.

### Codec strategy — Path B: manual, single-layer

The codec walks bytes explicitly: `u16::from_le_bytes(...)` for
multi-byte fields, manual mask + shift for bitfields. No
`#[repr(C, packed)]` mirror struct.

```rust
pub struct DeviceConfig {
    pub firmware_version: u16,
    pub axes: [AxisConfig; 8],
    pub buttons: [Button; 128],
    // ... etc
}

impl DeviceConfig {
    pub fn decode(bytes: &[u8; 1580]) -> Result<Self, DecodeError> { ... }
    pub fn encode(&self, out: &mut [u8; 1580]) { ... }
}
```

Why Path B over a `zerocopy` mirror struct:

1. **Bitfields force manual code anyway.** `common_types.h` has ~25
   bitfield declarations across ~10 inner structs (most cluster cleanly
   to 8 bits; three have explicit `:0` aligners). Neither `zerocopy` nor
   `bytemuck` handles C bitfields, so the hard fields are the same work
   in both paths. A mirror struct only saves typing for easy fields.
2. **No mirror = no `From`-impl drift.** Per the maintainer's
   `feedback_wire_format_archival.md` discipline, drift between layers
   has bitten before. Path B has one boundary (domain ↔ bytes); a mirror
   approach has two (mirror ↔ bytes ↔ domain).
3. **Better failure mode.** Fixture test fails → grep field name → find
   the exact encode/decode line. With a mirror, you trace through
   accessors and `From` impls first.

Safety rule against offset typos: **every field gets paired
`encode_<field>` / `decode_<field>` unit tests** that round-trip a known
value at a known offset. A typo in one half is caught by the test.
Endianness is explicit everywhere (`from_le_bytes` / `to_le_bytes`) so
the assumption is visible in source.

### Domain layering inside `freejoyx-core`

Even within one crate, three sub-modules keep concerns clean:

- `wire`: `encode` / `decode` for `DeviceConfig` and `ParamsReport`.
  Pure functions over `&[u8]`. No I/O, no validation beyond
  "byte-stream is well-formed."
- `domain`: the `DeviceConfig` type and its companions (`AxisConfig`,
  `Button`, etc), with idiomatic Rust enums (`ButtonType`, `LogicOp`,
  `PinFunction`, …). Validators (pin-conflict detection, LOGIC
  completeness) live here.
- `persist`: `serde` derives + RON read/write for `.freejoyx-config.ron`.

The HID transport (`freejoyx-device`) talks bytes to/from the device and
calls into `wire::decode` / `wire::encode`. It does not touch the
`domain` layer directly — that's the UI's job.

---

## 4. TDD strategy

### TDD-first modules

Write failing tests before implementation; tests assert against captured
byte fixtures (see §4.1).

1. **Wire codec** (`freejoyx-core::wire`)
   - 1580-byte config ↔ `DeviceConfig` round-trip (decode → encode is
     byte-identical to the input)
   - Per-field round-trip: every bitfield, every multi-byte int, every
     enum tag — paired `encode_<field>` / `decode_<field>` test
   - Fragmentation: 1580 bytes → 26 × 62-byte chunks, reassembly with
     out-of-order fragments and duplicates
   - Endianness asserted explicitly (a fixture byte at a known offset has
     a known field value)
2. **Params report parser** (`freejoyx-core::wire`)
   - 72-byte input → raw + processed axis values, button bitmaps, shift
     state, logical-button bitmap
   - Property test: random valid bytes never panic, only produce typed
     errors or valid values
3. **Domain validators** (`freejoyx-core::domain`)
   - Pin-conflict detection (two functions on the same pin → error
     enumerating both)
   - LOGIC config completeness — port of
     `ButtonLogical::isLogicConfigComplete()`. Operator must be picked
     (not the `-1` sentinel); if op is binary (`AND`, `OR`, `NAND`,
     `NOR`, `XOR`, `A_AND_NOT_B`), `src_b` must be set
   - Per-board pin layouts (BluePill / BlackPill) — assigning a
     non-existent pin → error
4. **On-disk RON serde** (`freejoyx-core::persist`)
   - Round-trip: `DeviceConfig → RON → DeviceConfig` is value-identical
   - Cross-trip: `wire bytes → DeviceConfig → RON → DeviceConfig → wire bytes`
     is byte-identical

### 4.1 Fixture capture — Slice 0.5

The codec's TDD only works if the byte fixtures are real. Round-trip
tests alone prove the codec is internally consistent; they do **not**
prove the Rust struct interpretation matches the firmware's C
interpretation. Two ways a codec can be silently wrong while passing
round-trip:

1. Field width or sign drift (a `uint16_t` read as `i16`) — round-trips
   itself but writes garbage to the device.
2. Bitfield order swap inside a packed byte — round-trips itself,
   misinterprets every value.

The fixtures **must** be captured from a real device + Qt app pairing,
because the Qt app is the working oracle for byte→meaning interpretation.

**Three fixture artifacts (committed to the repo):**

1. **`fixtures/config_wide_coverage.bin`** — raw 1580 bytes captured
   from a device whose `dev_config_t` was hand-tuned via the Qt app to
   exercise every type: non-zero pins across the range, several buttons
   of each type (NORMAL, TOGGLE, LOGIC w/ AND, LOGIC w/ NOT, POV1_UP,
   LONG_PRESS, DOUBLE_TAP), non-default axis calibration, at least one
   shift register, at least one fast encoder, at least one soft encoder.
2. **`fixtures/config_minimal.bin`** — raw 1580 bytes of factory-default
   `dev_config_t`. Sanity baseline.
3. **`fixtures/params_stream.bin`** — 2-3 seconds of params packets
   captured while inputs are exercised (stick movement, button presses,
   encoder rotation, shift register changes).

Each `.bin` is accompanied by a `.fragments.bin` (the 26 HID frames as
written on the wire) and a `.expected.ron` (the oracle — what the Qt
app interprets these bytes as).

**Capture mechanism:** a ~5-line patch to `hiddevice.cpp` that dumps
incoming fragments and the assembled config to disk when an env var
(e.g. `FREEJOYX_DUMP_FIXTURE=path`) is set. The patch lives in a
**throwaway branch** of `FreeJoyXConfiguratorQt`; we capture, commit
the bytes to the Rust repo, then discard the branch.

**`fixtures/REGEN.md`** — step-by-step instructions for regenerating
each fixture from scratch (which Qt fields to set to which values, in
which order, then the dump command). This is the load-bearing
document: without it, the fixtures become opaque magic numbers the
first time the wire format bumps. Trigger to regenerate: any
`FIRMWARE_VERSION` mask-group bump.

### Smoke-tested (not TDD'd)

- Slint views and bindings
- HID transport itself (mock the `Transport` trait for unit tests; real
  I/O verified manually with a device on the bench)
- Device discovery / hot-plug

---

## 5. Slice-by-slice roadmap

Each slice ends with something runnable + committable.

| # | Slice | Days | Done-when |
|---|---|---|---|
| 0 | Repo + CI bootstrap | 0.5 | Empty `cargo` workspace builds & tests green on `ubuntu-latest`, `windows-latest`, `macos-latest`. README points back to FreeJoyXConfiguratorQt + this plan. |
| 0.5 | **Fixture capture** | 0.5 | Three fixture artifacts (§4.1) checked in. `fixtures/REGEN.md` written. Qt-side capture patch noted in `docs/ported-from.md`. Healthy BluePill + BlackPill used as sources. |
| 1 | Wire codec (TDD) | 2 | `wire::decode(config_wide_coverage.bin)` round-trips byte-identical; every field tested individually; params parser parses every packet in `params_stream.bin` without error. |
| 2 | Params parser + tiny CLI | 1 | `cargo run -p freejoyx-app -- watch` opens the real board and prints live axis values + button bitmap to stdout. First end-to-end proof the Rust stack talks to the device. |
| 4 | Device worker + channel API | 1 | `freejoyx-device` exposes `spawn() -> (DeviceHandle, mpsc::Receiver<DeviceEvent>)`. CLI from Slice 2 refactored onto this API; behaviour unchanged; hot-plug works. |
| 5 | Slint shell + **Pins tab** | 3 | App window, tab strip skeleton (all v0.1 tabs visible, only Pins implemented). Read config → display pins → edit → write back to device → device acknowledges. Pin-conflict validator lights up clashes inline. |
| 3 | On-disk RON + validators | 1 | Save / Load buttons in toolbar produce/consume `.freejoyx-config.ron` files. `wire → domain → RON → domain → wire` is byte-identical (test added in Slice 1 extended). |
| 6 | Axes tab (linear) | 1.5 | Calibration, filter, deadband, resolution, channel. No curve editor (deferred). Live axis value overlay from `ParamsTick`. |
| 7 | Buttons + Logic + Shifts & Timers | 3.5 | 128-button grid, type picker (incl. NORMAL / TOGGLE / LOGIC / LONG_PRESS / DOUBLE_TAP / POVs / encoders-as-buttons / radios / sequentials). LOGIC operator picker + Source B + shift modifier. Per-physical coexistence filter (Step 4 firmware rule: `{NORMAL, LONG_PRESS, DOUBLE_TAP}` only). Global timers (Timer1/2/3 + Debounce + LongPress + DoubleTap windows) on the Shifts & Timers tab. Live state overlay. |
| 8 | Encoders + Shift Registers | 2 | 16 soft + 2 fast encoders; pin pickers, type. 4 shift registers (HC165/CD4021); chain length, channel mapping. Parity with corresponding Qt tabs. |
| 9 | Advanced Settings | 0.5 | Device name, VID/PID, firmware-version display. No flasher. |
| 10 | Polish + v0.1 release | 2 | App icon, About dialog, error toasts (incl. unknown-firmware-version → "use Qt app"), log file via `tracing`. `cargo-bundle` for `.msi` / `.dmg`; AppImage script. v0.1.0 tag + release notes. |

**Total: ~18 focused days ≈ 3-4 weeks.**

Ordering notes:

- Slice 0.5 is mandatory before Slice 1; the codec without fixtures is
  guess-and-pray.
- Slice 5 is deliberately placed **before** Slice 3 (on-disk RON). The
  first round-trip arc is `device → codec → UI → codec → device` — RON
  is third on a list of two. Doing UI first forces the codec to be
  exercised end-to-end against real hardware before the RON shape
  ossifies around any codec bug.
- Slice 7 is the largest slice. It's also the one with the highest
  feature density for the maintainer's actual use case (Step 2 + Step 4
  firmware features both surface here). Worth the time.
- Slice 9 (LEDs / RGB) from the original plan **does not exist** in
  v0.1. Bytes round-trip; UI is post-release.

---

## 6. Risk register

| Risk | Mitigation |
|---|---|
| Bitfield ordering differs between MSVC (Qt app) and GCC/Clang/MSVC (Rust + `from_le_bytes` masking) | Slice 0.5 fixtures + per-field tests catch this on day one of Slice 1. If discovered, the fix is a one-line bit-swap in the affected accessor, not a rewrite. |
| `hidapi-rs` enumeration semantics differ from vendored C HIDAPI on Windows (SetupAPI quirks) | Slice 4 validates discovery on Windows before any UI is built on top. Slice 2's CLI is the canary. |
| Fixture interpretation wrong (bytes captured correctly, but the `.expected.ron` oracle has a mistake) | Cross-check: a fixture's `.expected.ron` is hand-authored by reading the Qt app's display of the same `dev_config_t`. Any disagreement between Rust codec and Qt UI → re-check the oracle before blaming the codec. |
| Wire-format drift between Rust port and firmware repo while port is in flight | See §10. `header-sync.yml` in this repo clones `anpeaco/FreeJoyX` and normalized-diffs `vendored/common_*.h`. Mirror workflows already exist in both the firmware and Qt repos; the Rust repo joins the triangle. |
| LOGIC validator subtly wrong → user writes incomplete LOGIC config to device | Validator surface is tiny (`op` picked + `src_b` set if binary). Covered by a small exhaustive test matrix. Lower risk than the original plan's phantom evaluator. |
| Cross-thread Slint updates feel laggy vs Qt's queued signals | Batch `ParamsTick` updates at ~30 Hz max on the worker side; profile in Slice 5. |
| Slint's `Path` element insufficient for future curve editor (when re-introduced) | Not a v0.1 risk. Re-evaluate when the curve editor returns. |

---

## 7. What gets dropped (and why)

### Dropped for v0.1 (UI only — bytes still round-trip)

- **Single/PWM LED tab** — maintainer's build doesn't use it. Add when there's a use case.
- **RGB LED tab** — same. Removes the color-wheel-as-Slint-widget risk from v0.1.
- **Sensor config tab** — same.
- **11-point spline curve editor** — linear + deadband + filter covers the maintainer's build. Add when the spline becomes useful.

### Dropped permanently

- **Localization (`.ts/.qm` files).** English-only. Revisit with
  `fluent-rs` if users ask.
- **Legacy firmware migration (`legacy/`).** Only relevant for old
  upstream FreeJoy boards. The Rust app refuses unknown firmware
  versions and points users at the Qt app.
- **InnoSetup installer.** Replaced by `cargo-bundle` / AppImage.
- **Qt resource system, themes, `QSettings`.** Replaced by
  `include_bytes!`, Slint built-in styles, serde-on-disk.
- **Debug window (`debugwindow.{h,cpp}`).** Replaced by `tracing` logs
  to a file the user can find from Help → Open Log Folder.
- **Firmware flasher.** Out of scope; firmware updates via the existing
  DFU / Qt-app path.

---

## 8. How the next session picks this up

When the next Claude Code session opens this directory:

1. Clone `FreeJoyXConfiguratorQt` as a sibling at
   `../FreeJoyXConfiguratorQt-reference/` (or use the maintainer's
   existing checkout at `../FreeJoyXConfiguratorQt/`). The agent reads
   it for protocol and UI lookups during execution; does not write to
   it.
2. Open this plan as the working spec.
3. **Start with Slice 0** — don't skip CI on 3 OSes; every later slice
   leans on `cargo test` running everywhere.
4. **Slice 0.5 cannot be deferred.** The codec without fixtures is
   guess-and-pray; the fixtures need real hardware which the maintainer
   has on hand (BluePill + BlackPill, both healthy).

---

## 9. Decisions locked in (do not relitigate without explicit maintainer input)

| Topic | Decision |
|---|---|
| Workspace name | `FreeJoyXConfigurator` (this directory) |
| Repo home | `github.com/anpeaco/FreeJoyXConfigurator` (parallel to FreeJoyXConfiguratorQt) |
| License | GPLv3 (forced by upstream FreeJoy + FreeJoyXConfiguratorQt, both GPLv3) |
| Slint license tier | Royalty-free GPLv3 (only option compatible with above) |
| Crate prefix | `freejoyx-` (so `freejoyx-core`, `freejoyx-device`, `freejoyx-ui`, `freejoyx-app`) |
| Crate count | 3 + 1 bin (proto + config collapse into `freejoyx-core`) |
| Codec strategy | Path B: manual encode/decode, single-layer, no mirror struct |
| Codec deps | None (no `zerocopy`, no `bytemuck`) — explicit `from_le_bytes` + masks |
| Codec correctness rule | Paired `encode_<field>` / `decode_<field>` unit tests for every field |
| Codec scope | Full 1580-byte `dev_config_t` round-trip, including LED/sensor bytes the v0.1 UI doesn't surface |
| LOGIC evaluator | **Not implemented** in configurator — the firmware evaluates; configurator only validates config completeness |
| Async runtime | `std::sync::mpsc` + plain thread (no `tokio`) |
| Wire-format target | `FIRMWARE_VERSION = 0x0020`, mask group `& 0xFFF0` |
| Unknown firmware versions | Refuse with a clear toast pointing at the Qt app — no legacy migration in Rust |
| CI runners | Free GitHub Actions: `ubuntu-latest`, `windows-latest`, `macos-latest` |
| UI style | Per `Style.MD` (DCSBoards-derived dark cockpit palette, amber accent) |
| Fixtures | Three artifacts (wide / minimal / params-stream), each as `.bin` + `.fragments.bin` + `.expected.ron`, with `fixtures/REGEN.md` |
| Hardware available for capture | Healthy BluePill + BlackPill, both maintainer-owned |
| Curve editor | Deferred to v0.1.1+ |
| LED tabs (single + RGB) | Deferred to v0.1.1+ |
| Sensor tabs | Deferred to v0.1.1+ |
| Flasher | Permanently out of scope |
| Localization | Permanently out of scope |

---

## 10. Drift detection between Rust port and firmware repo

The wire format has three parties now: firmware (`FreeJoyX`), Qt
configurator (`FreeJoyXConfiguratorQt`), and Rust port
(`FreeJoyXConfigurator`). The firmware is the canonical source; the
two configurators each vendor a copy of `common_defines.h` +
`common_types.h` and must stay in sync.

The firmware and Qt repos already enforce drift via mirror
`header-sync.yml` workflows. The Rust repo joins the triangle:

- **`vendored/common_defines.h`** and **`vendored/common_types.h`** —
  read-only mirror of the firmware repo's headers. The Rust codec is
  written against these.
- **`.github/workflows/header-sync.yml`** — on every push, clones
  `anpeaco/FreeJoyX` as a sibling checkout, runs the same
  normalized-diff used by the firmware-side workflow (strips comments,
  collapses whitespace, removes `/* SYNC_SKIP_BEGIN ... SYNC_SKIP_END */`
  blocks). Fails CI on any wire-format drift.

The maintainer's existing "four items move in lockstep" rule from
`memory/feedback_wire_format_archival.md` becomes **five**, with this
repo as the fifth item:

1. `FIRMWARE_VERSION` (cross the `& 0xFFF0` boundary)
2. `FREEJOY_DEV_CONFIG_SIZE` / `FREEJOY_PARAMS_REPORT_SIZE` constants in
   *both* `common_defines.h` copies
3. Legacy archive entry in `legacy_types.h` + `legacy_migrator.cpp`
   (Qt repo)
4. Two header copies kept in manual sync (firmware ↔ Qt repo)
5. **Rust `vendored/common_*.h` + codec + fixtures** — when the format
   bumps, regenerate fixtures (per `fixtures/REGEN.md`) and update the
   codec to match the new layout. Bump rejected if either step is
   skipped.

What CI catches (and what it doesn't):

- ✅ Header drift between the three repos
- ✅ Codec build / test breakage on any of the three OSes
- ✅ `dev_config_t` size assertion failures (the codec's integration
  test asserts the vendored `_Static_assert` expressions still hold)
- ❌ Hardware/runtime behaviour — no real device on the runner
- ❌ Slint UI runtime behaviour — CI only verifies the UI builds

---

## 11. Visual style

See `Style.MD` (in this directory). The Slint UI follows DCSBoards'
Settings-panel aesthetic — dark cockpit panel, amber accent, custom
`DarkCheckBox` / `LucideIcon` / `IconCell` lifted near-verbatim.
`Style.MD` pins to DCSBoards commit `cf65876`; capture snippets locally
if the upstream repo isn't sibling-cloneable for the Slice 5 author.
