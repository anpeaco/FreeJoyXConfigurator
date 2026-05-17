# Session log — through 2026-05-17

Continues the overnight session that started 2026-05-16. The previous
entries (bootstrap + Slice 0 + Slice 0.5(a)) are below this header.

## Update 2026-05-17 — Slice 7 (Buttons + Logic + Shifts & Timers) DONE

Largest slice in the plan (3.5-day estimate). Picks up after Slice 6.
Landed in three sub-steps (7a–7c) with verification at 7d.

### 7a — domain::buttons (ButtonType + coexistence + setters)

New module `freejoyx_core::domain::buttons` lands three pieces:

- **`ButtonType` enum** — 36 variants mirroring `button_type_t` from
  `vendored/common_types.h` byte-for-byte (Normal..DoubleTap). Includes
  the gestures appended in v1.7.x at slots 34/35 (TAP renamed from
  LONG_PRESS, value preserved). `from_u8` returns `None` for unknown
  bytes so the wire codec's raw-byte storage continues to round-trip
  garbage faithfully. `label()` matches the Qt configurator dropdown
  entries. `all()` enumerates in wire order — used by the cycle
  picker.
- **`physical_assignment_blocked`** + `CoexistenceCheck` — the
  F103_GESTURE_PLAN.md rule, expressed against `&[Button; 128]`:
  a physical input may host slots only from
  `{NORMAL, TAP, DOUBLE_TAP}`. Returns the offending sister slot so
  the UI can point at where the conflict came from.
- **Button bitfield setters** — paired with the getters from Slice 3:
  `set_shift_modificator` (4 bits), `set_is_inverted`, `set_is_disabled`,
  `set_op` (3 bits), `set_delay_timer` (3 bits), `set_press_timer`
  (3 bits). Truncate-on-oversize so a runaway UI never spills into a
  neighbouring field.

`BUTTON_TYPE_LOGIC = 33` re-exported from this module too so the
buttons-tab glue doesn't need to know `domain::logic` exists.

Eleven unit tests anchor this module: enum round-trip, unknown-value
handling, gesture-compatible spec, four coexistence scenarios, two
setter scenarios (round-trip + truncate-oversize).

### 7b — Slint UI: ButtonsTab + ShiftsTimersTab

`ui/app.slint` grows three new structs (`ButtonRow`, `ShiftSlot`,
`TimerField`) and two new tab components:

- **`ButtonsTab`** — scrollable list of 128 rows. Each row's top half:
  slot number, physical-input `NumberCell`, type `CycleCell` (140 px
  wide for the long Toggle Switch Off label), shift `CycleCell` (0-8),
  Inverted / Disabled `CheckCell`s, and two `LiveDot`s (amber =
  physical pressed, green = logical state). When `is-logic` is true a
  second row appears with op cycle, Source B `NumberCell`, debounce
  timer cycle, and a validator badge driven by
  `validate_logic_buttons`.
- **`ShiftsTimersTab`** — 8 shift-slot rows (each a button-index
  `NumberCell` with -1 = unused) followed by 6 timer rows: Button
  Timer 1/2/3, Button Debounce, Tap cutoff, Double-tap window. Each
  timer row carries a `hint` cell with the user-friendly explanation
  (the Qt configurator has these as tooltips; here they sit inline
  to save a hover round-trip).

Both new tabs use the existing `CheckCell` / `NumberCell` /
`CycleCell` primitives from Slice 6. The Buttons tab `Flickable`
sizes its viewport for the worst case (every row LOGIC = 72 px) so
the scrollbar math is stable as rows flip between 38 and 72 px.

Twelve new callbacks bubble up through `AppWindow`:
button-physical-edited / type-cycled / shift-cycled / inverted-toggled
/ disabled-toggled / src-b-edited / op-cycled / debounce-cycled +
shift-edited + timer-edited.

The Buttons + Logic and Shifts & Timers tab buttons are now enabled.

### 7c — app glue: separate `crate::buttons` module

The bulk of Slice 7's Rust glue lands in a new `crates/freejoyx-ui/
src/buttons.rs` rather than `app.rs`, so `app::run` stayed under the
clippy `too_many_lines` cap. The new module owns:

- `refresh_button_model` / `refresh_button_row` — wholesale-or-per-row
  rebuild from `DeviceConfig.buttons`, with the LOGIC validator
  pre-computed once per refresh.
- `refresh_shift_model` / `refresh_timer_model` — same pattern for
  the Shifts & Timers tab. `TIMER_FIELDS` is a const table of
  `(label, hint)` pairs that drives both the model and
  `set_timer_value` index-→-field dispatch.
- `next_compatible_type` — `(physical_num, current_type)` → next
  ButtonType that passes `physical_assignment_blocked`. Wraps after
  36 steps so cycle-on-blocked rows still rotate.
- `wire_callbacks` — wires all 10 button + shift + timer Slint
  callbacks. The type-cycle one consults the coexistence rule; every
  other callback funnels through `with_button_slot` /
  `mark_dirty`.

`app.rs`:
- `State` made `pub(crate)` so the new module can take `&mut last_config`
  via `RefCell::borrow_mut`.
- `pump_events` now refreshes button + shift + timer models on
  `ConfigReceived` and the button model on `ParamsTick` (live press
  state).
- Load callback refreshes all five models (pin / axis / button /
  shift / timer) — file-loaded configs surface live across every
  tab without a Read Device round-trip.
- New `EventSinks<'a>` bundle passed to `pump_events` so the timer
  closure doesn't accumulate a parameter per slice.

### 7d — verification

Five-command discipline holds:

- `cargo fmt --all --check` ✓
- `cargo clippy --workspace --all-targets -- -D warnings` ✓
- `cargo test --workspace` ✓ — **73 tests pass** (was 62 at end of
  Slice 6; +11 from `domain::buttons`)
- `cargo build --workspace --release` ✓
- `cargo run -p freejoyx-app -- list` + `ui` smoke ✓ — window came
  up, worker connected.

### Notes for downstream slices

- **128-row Flickable viewport math is conservative.** Pre-computed
  as `len × 72 px` (worst case, every row LOGIC). When most rows
  aren't LOGIC there's empty scroll space at the bottom. A future
  polish item is to recompute viewport-height as a running sum.
- **Live-state refresh on `ParamsTick` rebuilds every button row**,
  not just rows whose bit changed. 128 set_row_data calls × 20 Hz =
  2560/s, fine on the dev machine but the next slice that adds
  per-slot wiring (encoder live-state in Slice 8?) should profile
  before piling on.
- **The type-cycle picker still skips blocked variants**, which can
  feel surprising when the user expects a particular advancement
  order. A real popup combobox with greyed-out blocked entries is
  cleaner UX — same trade-off flagged in Slice 6's Notes. Slice 10
  polish is the right time.
- **Coexistence rule only enforces the gesture set**, not other
  cross-slot rules from `F103_LOGIC_PLAN.md` (e.g. a LOGIC slot
  referencing a disabled physical slot). LOGIC's `validate_logic_buttons`
  catches the "Source unset" cases; expressing the firmware's
  actual evaluation semantics is intentionally out of scope per
  Port.md §9 (firmware evaluates, configurator only checks
  completeness).
- **A2B debounce (`a2b_debounce_ms`) is unedited**. The wire codec
  round-trips it; the Shifts & Timers tab v0.1 surfaces six fields,
  not seven, to avoid clutter for a field most users won't touch.
  Trivial to add when someone asks.
- **`encoder_press_time_ms` / `exchange_period_ms`** also unedited
  for the same reason. Encoder timing belongs on the Encoders tab
  (Slice 8); exchange period is firmware-internal pacing.
- **Bench verification of LOGIC + gesture buttons still pending.**
  Read → make a LOGIC slot → Write → read-back-byte-identical is the
  trip that proves the new setters wire through. Same for setting up
  TAP + DOUBLE_TAP on a shared physical and confirming the
  coexistence rule is mirrored on the firmware side.

### What's next

Per Port.md §5: **Slice 8 — Encoders + Shift Registers.** 16 soft
encoders + 2 fast encoders (`MAX_FAST_ENCODER_NUM = 2` per Port.md
§9). Pin pickers per encoder + the 1x/2x/4x speed picker. 4 shift
registers (HC165 / CD4021) with chain length + channel mapping.

Then Slice 9 (Advanced Settings — name / VID / PID), then Slice 10
(polish + v0.1 release).

---

## Update 2026-05-17 — Slice 6 (Axes tab, linear) DONE

Picks up after Slice 3. Per Port.md §5 Slice 6's surface area is
"Calibration, filter, deadband, resolution, channel. No curve editor.
Live axis value overlay from ParamsTick." Landed in three sub-steps
(6a–6c). Also committed the backlog of prior-slice work that had been
sitting in the working tree (8 commits, see history) before starting.

### 6a — axis flag setters + AxisFilter enum

`freejoyx_core::wire::config::AxisConfig` gets setters paired with
every v0.1-surface getter: `set_out_enabled`, `set_inverted`,
`set_is_centered`, `set_filter`, `set_resolution`, `set_channel`,
`set_deadband_size`, `set_is_dynamic_deadband`. Each goes through
two new private helpers: `set_bit(byte, mask, v)` for single-bit
toggles, and `set_bits(byte, shift, mask, v)` for multi-bit fields
(which also truncate oversize values to the mask width — so the UI
never spills into adjacent bitfields even if a caller passes garbage).

Two new unit tests anchor the behaviour:

| Test | What it proves |
|---|---|
| `axis_config_setters_isolate_their_bits` | Each setter touches only its own bit positions; the surrounding bits in the same byte don't shift |
| `axis_config_setters_truncate_oversize_values` | Calling `set_filter(0xff)` writes `0x07`, not `0xff` — same for resolution / channel / deadband_size |

`freejoyx_core::domain::axes` (new module): `AxisFilter` enum mirroring
the 3-bit `filter_t` from `vendored/common_defines.h`. Labels match
`axesextended.h::m_filterList` so the Slint dropdown reads identically
to the Qt slider tooltip. `from_u8` masks the input to 3 bits before
matching so every `u8` round-trips deterministically through the eight
defined variants. Re-exported from `domain::`.

Three unit tests cover the round-trip, the high-bit masking edge
case, and the `all()` iterator ordering.

### 6b — Slint AxesTab + AxisRow struct

`ui/app.slint` gains:

- `AxisRow` struct with all v0.1 fields (booleans, calib min/center/max,
  filter index + label, deadband size + dynamic flag, resolution,
  channel) plus `live-raw` / `live-out` for the params overlay.
- `CheckCell`, `NumberCell` (`TextInput` with the dark-cockpit
  styling and number input-type), and `CycleCell` (click-to-cycle
  pattern, same shape as the Pins tab function picker) — three
  reusable controls that compose the per-axis card.
- `AxisRowView` — one card per axis. Two rows: header (title +
  three checkboxes + live values), then form (3× calib + filter +
  deadband + dyn + resolution + channel).
- `AxesTab` — scrollable list of 8 cards. Surfaces 11 callbacks
  upward (one per editable control).

Eleven new callbacks bubble up through `AppWindow`:
`axis-out-toggled`, `axis-inverted-toggled`, `axis-centered-toggled`,
`axis-calib-{min,center,max}-edited`, `axis-filter-cycled`,
`axis-deadband-edited`, `axis-dyn-deadband-toggled`,
`axis-resolution-cycled`, `axis-channel-cycled`. The Axes tab button
in the strip is now enabled.

### 6c — app.rs glue + live params overlay

`State` gains `last_params: Option<ParamsReport>` so the most recent
tick is available when rebuilding axis rows (live raw/out columns).

`pump_events`:
- `ParamsTick(p)` now updates `state.last_params` and, if there's a
  config loaded, refreshes the axis model per-row (`set_row_data`,
  not wholesale rebuild — TextInput focus would lose its place
  otherwise).
- `ConfigReceived` and Load both push the params snapshot into
  `refresh_axis_model` so the first render has live values when
  the device is already streaming.

`wire_axis_callbacks` (new): factors the 11 callbacks through two
helpers — `mk_toggle` for the boolean/cycle callbacks (`fn(&mut
AxisConfig)`), `mk_int` for the int-edited ones. Each callback funnels
through `mutate_axis`, which clones the relevant params slice out of
state *before* taking the `&mut DeviceConfig`, so the borrow checker
stays happy. Side effects of every mutation: row refresh in the
model, `can_write = connected`, `can_save = true`.

`refresh_axis_model` (new) and `build_axis_row` (new) own the
config-→-AxisRow mapping. The first does a wholesale rebuild if the
model size is wrong (load / first-read), otherwise per-row update
(live tick).

`run()` shrank back under the clippy `too_many_lines` cap (was 105
lines after adding the axis-model wiring) by factoring the read /
write / pin-changed callback wirings into `wire_read_callback`,
`wire_write_callback`, `wire_pin_callback` helpers — same pattern
the save/load helpers already used.

### 6d — verification

Five-command discipline holds:

- `cargo fmt --all --check` ✓
- `cargo clippy --workspace --all-targets -- -D warnings` ✓
- `cargo test --workspace` ✓ — **62 tests pass** (was 57 at end of
  Slice 3; +3 AxisFilter unit + 2 AxisConfig setter unit)
- `cargo build --workspace --release` ✓
- `cargo run -p freejoyx-app -- list` ✓ — all 6 boards still visible
- `timeout 4 cargo run -p freejoyx-app -- ui` ✓ — window came up,
  worker connected to the first board, exited via SIGTERM at the
  4-second mark.

### Notes for downstream slices

- **Live overlay throttling deferred.** Port.md §6's risk register
  flags `ParamsTick` at full cadence (1 kHz on the F103 firmware,
  measured ~20 Hz over USB on the bench) as a potential UI burden.
  Per-row `set_row_data` is cheap enough that 8 rows × 20 Hz is
  invisible on the dev machine; if a future slice surfaces 128
  button overlays at the same cadence, batch on the worker side
  to ~30 Hz then.
- **Cycle pattern, not popup combobox, for filter / resolution /
  channel.** Same trade-off as the Pin tab function picker (Slice
  5 notes). For 8 / 16 / 16 entries it's tolerable; if the Buttons
  tab in Slice 7 needs a longer dropdown (button type has ~30
  entries) it'll force the swap to `std-widgets::ComboBox`. Doing
  it then also covers all three existing cycle cells.
- **Source-main / source-secondary, prescaler, divider, function,
  per-axis button hooks, axes-to-buttons, sensor I2C** all round-
  trip but stay unedited. Port.md §1.1 explicitly defers the sensor
  surface to v0.1.1+; the source-main picker is the most likely
  next-pull from the deferred set if/when an actual user complains.
- **No format coercion on i16 inputs.** `clamp_i16` saturates;
  pasted `999999` into calib-min lands as `32767` silently. A toast
  on saturation would be a polite polish item for Slice 10.
- **Bench verification of write-back still pending.** Read → edit
  axis → Write → read-back-byte-identical is the trip that proves
  the setters wire through the codec correctly under real I/O.
  Trivial smoke for the next time the maintainer is at the bench.

### What's next

Per Port.md §5: **Slice 7 — Buttons + Logic + Shifts & Timers.** The
largest slice in the plan (3.5 days estimate). 128-button grid, type
picker including NORMAL / TOGGLE / LOGIC / LONG_PRESS / DOUBLE_TAP /
POVs / encoders-as-buttons / radios / sequentials. LOGIC operator
picker + Source B + shift modifier. Per-physical coexistence filter
(Step 4 firmware rule: `{NORMAL, LONG_PRESS, DOUBLE_TAP}` only on a
given physical input). Global timers (Timer1/2/3 + Debounce +
LongPress + DoubleTap windows) on the Shifts & Timers tab. Live state
overlay from `ParamsTick.phy_button_data` / `log_button_data` /
`shift_button_data`.

This is also where Slice 3's LOGIC validator gets its first UI
consumer — `validate_logic_buttons` already returns the
`SourceAUnset` / `SourceBUnsetForBinaryOp` / `OpOutOfRange` set;
Slice 7 needs to surface it on the per-button row the same way the
Pins tab surfaces `PinConflict`.

Slice 7 is also the most likely place the cycle-cell-vs-combobox
trade-off forces the swap; budget for a `std-widgets::ComboBox`
detour early in the slice if the button-type dropdown ends up at
~30 entries.

---

## Update 2026-05-17 — Slice 3 (on-disk RON + validators) DONE

Picks up directly after Slice 5. Per Port.md §5 ordering, Slice 3 comes
after Slice 5 (UI shell + Pins tab); the deliberate inversion lets the
codec be exercised end-to-end against real hardware before the RON
shape ossifies. Slice 3 lands six sub-steps (3a–3f).

### 3a — serde derives on wire types

`Serialize` + `Deserialize` derived on `DeviceConfig` and every
sub-struct (`AxisConfig`, `Button`, `AxisToButtons`, `ShiftRegConfig`,
`FastEncoder`, `PhysBreakdown`). Path B's single-layer rule
(Port.md §3) holds: the wire types serialize directly as the on-disk
shape — no mirror, no `From` impls.

Modern serde (1.0.228) supports arbitrary-length array derive *for
generic T* but Deserialize still caps `[T; N]` at 32 elements unless
helped along. Three fields needed the `serde-big-array` crate
(`buttons: [Button; 128]`, `leds_raw: [u8; 48]`, `rgb_leds_raw: [u8;
250]`); the rest of the arrays fit under 32. One new workspace
dependency: `serde-big-array = "0.5"` — same reason as `rfd`, the
shorter path to correctness wins.

### 3b — `freejoyx_core::persist` module

`save_to_file` / `load_from_file` over `DeviceConfig`. Pretty-printed
RON via `ron::ser::PrettyConfig::new().struct_names(true).compact_arrays(true)`.
`PersistError` wraps `io::Error`, `ron::SpannedError`, `ron::Error`
via `thiserror::From` chains. Module-level docs anchor the
load-bearing property: cross-trip is byte-identical, the persist
layer adds no drift on top of the wire codec.

One unit test (`empty_config_round_trips_through_string`) anchors the
zero-config baseline at the module boundary.

### 3c — cross-trip integration test

`crates/freejoyx-core/tests/persist_roundtrip.rs` — four tests:

| Test | What it proves |
|---|---|
| `cross_trip_is_byte_identical` | **THE load-bearing claim:** wire bytes → DeviceConfig → RON → DeviceConfig → wire bytes is byte-identical for `minimal` and `wide_coverage` fixtures |
| `value_round_trip_preserves_struct` | DeviceConfig → RON → DeviceConfig is `PartialEq`-equal (catches any field that ser/de drops or rewrites) |
| `pretty_output_is_recognizable` | RON output contains `DeviceConfig`, `firmware_version`, `buttons`, `pins`, `saved_breakdown` so a human can find fields |
| `save_then_load_file_round_trips` | Full file API exercise against `std::env::temp_dir` |

All four pass on the first run — no surprises in serde/RON's handling
of nested structs, fixed-size arrays, or the big-array fields.

### 3d — LOGIC validator (`domain::logic`)

Port of `ButtonLogical::isLogicConfigComplete()` from the Qt
configurator. Surfaces two ways a LOGIC slot is incomplete:

- `SourceAUnset` — `physical_num == -1` (no Source A picked)
- `SourceBUnsetForBinaryOp` — binary op picked (AND/OR/NAND/NOR/XOR/
  A_AND_NOT_B) but `src_b == -1`
- `OpOutOfRange` — `op` field is 7 (`LOGIC_OP_COUNT` sentinel, reserved)

Plus `BUTTON_TYPE_LOGIC = 33` constant + `LogicOp` enum with
`is_binary()` (everything but `Not`). Wire-layer validator only —
the "op not picked" UI sentinel handling lives in Slice 7 when the
Buttons tab lands; on the wire `op` is a 3-bit unsigned so always
picked (defaults to AND).

Eight unit tests cover: empty config passes, well-formed binary
passes, NOT with unset src_b passes, binary with unset src_b flags,
unset Source A flags, op-count sentinel flags, both A+B unset flag
both errors, non-LOGIC slots ignored.

`validate_logic_buttons` + `LogicError` + `LogicOp` +
`BUTTON_TYPE_LOGIC` re-exported from `domain::`.

### 3e — Save/Load buttons in Slint toolbar

`ui/app.slint` toolbar gained two PillButtons: "Load File…" (always
enabled) and "Save File…" (enabled iff `can-save`, set when a config
is loaded from device or disk). `Toolbar` and `AppWindow` got the new
`can-save` / `save-clicked` / `load-clicked` properties and
callbacks; the existing Read/Write Config buttons are renamed
"Read Device" / "Write Device" to make the device-vs-file
distinction obvious.

`app.rs`:

- New workspace dep: `rfd = "0.14"` for native file dialogs (defaults,
  no async runtime). Both Win/Mac native and Linux GTK3 backends.
- `wire_save_callback` extracted: borrows `state.last_config`, opens
  a native save dialog filtered to `*.ron`, calls
  `persist::save_to_file`, surfaces success/failure in the status
  text.
- `wire_load_callback` extracted: native open dialog, calls
  `persist::load_from_file`, replaces `last_config`, re-runs the pin
  conflict validator via `refresh_pin_model`. Sets `can_save` true
  and `can_write` to match the current `connected` state.
- `set_status` helper centralises "update both `State.status` and
  `AppWindow.status_text`" — removes 6 borrow-and-set pairs from the
  callback bodies.

The two new callback wirings live in their own helper functions so
`run()` stays under the clippy `too_many_lines` cap (was 151 lines —
extracted to ~80).

### 3f — verification

Five-command discipline holds:

- `cargo fmt --all --check` ✓
- `cargo clippy --workspace --all-targets -- -D warnings` ✓
- `cargo test --workspace` ✓ — **57 tests pass** (was 44 at Slice 5
  end; +1 persist unit + 4 persist integration + 8 logic unit)
- `cargo build --workspace --release` ✓
- `cargo run -p freejoyx-app -- list` ✓ — all 6 boards on the bench
  surface as before

### Notes for downstream slices

- **Save/Load smoke verification on a UX-tested device is pending**:
  the cross-trip tests prove byte parity from fixtures, but
  open-dialog → pick file → load → modify → save → reload on a
  real machine hasn't been exercised by hand. Trivial smoke for the
  next time the maintainer is at the bench.
- **The pretty-printed RON renders all 128 buttons / 50 RGB LEDs /
  etc.** even when most slots are zero. A `wide_coverage`
  fixture's RON is ~30 KB; `minimal` is ~25 KB. Default `serde` is
  field-complete; we could add `#[serde(skip_serializing_if = "is_default")]`
  later if the maintainer wants to eyeball-diff configs, but that
  would mean implementing `Default` + `is_default` on a dozen sub-types
  and the load-bearing tests already pass.
- **`compact_arrays(true)`** makes 30-element arrays render inline
  rather than one per line — keeps the buttons/leds section navigable.
- **No `Default` impl** on `DeviceConfig` or sub-structs. The
  "factory default" config lives on the device's ROM, not in the
  Rust code — the workflow is "Read Device" then edit, or "Load
  File" with a previously-saved file.
- **No `serde(deny_unknown_fields)`** so future wire-format
  additions can be loaded by older configurators (or vice versa).
  This is consistent with Port.md §1.2's "Rust app refuses unknown
  firmware-version mask groups" rule — version compat is the
  gatekeeper, not field-level deny.

### What's next

Per Port.md §5: **Slice 6 — Axes tab (linear)**. Calibration, filter,
deadband, resolution, channel; no curve editor (deferred to v0.1.1+).
Live axis-value overlay binds the `ParamsTick` event (already wired
through the device worker; just needs UI consumption).

Then Slice 7 (Buttons + Logic + Shifts & Timers) — the largest slice,
where the LOGIC validator from 3d gets its UI consumer and the per-
physical coexistence filter from F103_GESTURE_PLAN.md surfaces.

Then 8 (Encoders + Shift Registers), 9 (Advanced Settings), 10
(polish + v0.1 release).

---

## Update 2026-05-17 — Slice 5 (Slint shell + Pins tab) DONE

The 3-day slice per Port.md §5; the largest one in the plan. Landed in
four phases (5a/5b/5c/5d).

### 5a — config read/write protocol

- `Device::read_config()` / `write_config()` in `transport.rs` —
  fragment exchange per `hiddevice.cpp:467-690`. Read = request fragment
  N (2-byte `[CONFIG_IN, idx]` write), read until matching frame, advance
  idx through `fragment_count(DEV_CONFIG_SIZE)` = 26. Write = send
  start frame `[CONFIG_OUT, 0]` then push fragments 1..=26 in response
  to the device's ACK echo. Both wrapped in a 5-second deadline.
- `worker.rs` extended: `Command::{ReadConfig, WriteConfig(Box<DeviceConfig>)}`,
  `DeviceEvent::{ConfigReceived(Box<DeviceConfig>), ConfigSent,
  ConfigError(String)}`. Worker dispatches them between params reads,
  re-subscribes to params after the exchange.
- `spawn_for_serial(Option<String>)` lets a multi-board user pick a
  specific board by HID serial — needed because `open_first()` on a
  6-board bench is arbitrary, and on this dev machine it landed on an
  upstream FreeJoy (fw 0x1713) board that doesn't speak the FreeJoyX
  config protocol cleanly.
- New CLI subcommand: `freejoyx-app read-config [--serial <hex>]`.
  Spawns worker, waits for Connect, sends `ReadConfig`, prints a
  human-readable dump on `ConfigReceived`. Maintainer-asked feature
  for codec debugging; also serves as the load-bearing end-to-end
  test for the entire stack.
- `freejoyx_core::wire::format_config()` — grep-able multi-line
  rendering of `dev_config_t` that skips default / unused entries so
  the output stays readable on a real device with most slots empty.
  Sections: header / timers / pins / axes / buttons / encoders /
  fast_encoders / shift_registers / shift_config / axes_to_buttons /
  led-rgb summary / saved_breakdown.

### 5a end-to-end verification

`cargo run -p freejoyx-app -- read-config --serial 0067926A364B`
against the maintainer's factory-default `FreeJoyX 0.0.2` BluePill
produced a full 1580-byte dump in ~1s. Fields read back correctly:

- `firmware_version: 0x0010` ✓
- `board_id: 1 (F103 BluePill)` ✓
- `device_name: "FreeJoyX 0.0.2"` ✓ (factory default)
- Timers all at init_config defaults (50/50/200/300, tap 500, double-tap 300)
- All 16 soft encoders default to `2x`, fast encoders disabled
- Pins all unset (factory default), no buttons / shift regs / etc.

This is the first proof that decode works against a live device — the
fixture tests proved it on captured bytes, this proves it on bytes
coming straight off the wire.

### 5b — pin domain types + conflict validator

`freejoyx_core::domain::pins` (new module):

- `PinFunction` enum (32 variants) mirroring `pin_t` from
  `common_types.h`. `from_i8` / `to_i8` round-trip; `label()` for
  combobox display; `is_singleton()` flags SPI/I2C/UART/TLE5011_GEN
  as functions that can only appear on one pin.
- `Board::{Bluepill, Blackpill}` with `pin_name(slot)` lookup.
  BluePill silkscreen is the canonical naming; BlackPill renames
  slot 22 (`PB11` → `PB2`) per `pinboardnames.h`.
- `validate_pins(&[i8; 30]) -> Vec<PinConflict>` — flags duplicate
  singletons (e.g. two pins both picked `SPI_SCK`) and unknown
  function values. Board-specific timer-conflict rules (PA8 PWM
  blocks PA10 RGB, PB6/7 FAST_ENCODER vs TLE5011_GEN) are deferred to
  later slices when those toggles surface in the UI.
- 8 unit tests, all pass.

### 5c — Slint UI shell + Pins tab

`crates/freejoyx-ui/`:

- `ui/app.slint` — `AppWindow` with Style.MD-aligned dark cockpit
  palette (`#2a2a32` panel on `#000000` window, amber `#ffcc33`
  accent). Toolbar with status dot + device summary + Read/Write
  pill buttons. Tab strip showing all v0.1 tabs (Pins active, others
  greyed with placeholder bodies labelled "coming in Slice N").
  Pins tab body: scrollable list of 30 rows (pin name + function
  cell + conflict badge). The function picker advances on click
  (proper popup combobox deferred — Slint 1.13's std-widgets
  ComboBox can swap in later without changing the data model).
- `build.rs` invokes `slint_build::compile`.
- `src/app.rs` — UI glue:
  - Spawns the device worker via `spawn_for_serial`.
  - 100 ms Slint `Timer` drains the worker's event channel into
    `AppWindow` properties and rebuilds the `VecModel<PinRow>` on
    each `ConfigReceived`.
  - UI callbacks: `read_clicked` sends `Command::ReadConfig`;
    `write_clicked` clones the held config and sends
    `Command::WriteConfig`; `pin_changed(slot, fn)` mutates the
    held config, re-runs the validator, refreshes the model.

`freejoyx-app` extended: `ui` subcommand (and the no-arg default)
launches the Slint window; `--serial <hex>` forwards through to the
worker for board selection.

End-to-end smoke (5-second `timeout` run against the FreeJoyX 0.0.2
board) confirmed the worker spawned and connected; the window
appeared briefly. Full manual exercise (Read → edit pin → Write →
verify on device) is gated on the maintainer at the bench since the
window needs human interaction to drive.

### 5d — verification

All five commands clean:

- `cargo fmt --all --check` ✓
- `cargo clippy --workspace --all-targets -- -D warnings` ✓
  (pedantic on freejoyx-device + freejoyx-ui; required `# Errors` /
  `# Panics` doc sections + `try_from` for usize-to-i32 conversions)
- `cargo test --workspace` ✓ — **44 tests pass** (was 35 at end of
  Slice 4; +8 pin domain unit tests, +1 display test)
- `cargo build --workspace --release` ✓
- `cargo run -p freejoyx-app -- list` / `read-config` / `ui` ✓

### Notes for downstream slices

- **Function picker is click-to-cycle, not a popup combobox.** Fine
  for proving the data flow but ergonomically poor with 32 options.
  Slice 6+ should swap in `std-widgets::ComboBox`. The data model
  (`PinRow.function_index` + a `[string]` labels array) already
  supports that swap without churn.
- **Board-specific pin-conflict rules not implemented.** PA8 PWM /
  PA10 RGB and Encoder2 / TLE5011_GEN PB6/PB7 mutexes are coded into
  `pinconfig.cpp:408-510` (Qt). They belong wherever the relevant
  feature surfaces (encoders → Slice 8, LEDs → v0.1.1+); the
  singleton-only validator covers the common-case clashes for now.
- **Pins model rebuilds every edit.** Slint's `VecModel::set_row` is
  more granular but for 30 rows the full rebuild is invisible to a
  user. Optimise if profiling ever points there.
- **Hot-plug works during Slice 4 verification.** Slice 5 inherits
  that; the worker re-discovers on disconnect, the UI sees Connected
  again.
- **Upstream FreeJoy firmware rejection is still missing.** Per
  Port.md §1.2 the Rust app refuses unknown firmware-version mask
  groups with a toast pointing at the Qt app. The Slice 5 UI accepts
  anything the worker hands it; this is a polish item for Slice 10
  (or wherever we add a proper firmware-version gate).

### What's next

Per Port.md §5 ordering: **Slice 3 — on-disk RON + validators**, then
Slice 6 (Axes), 7 (Buttons + Logic + Shifts & Timers), 8 (Encoders +
Shift Registers), 9 (Advanced Settings), 10 (polish + v0.1 release).

Slice 3 turns the in-memory `DeviceConfig` into `.freejoyx-config.ron`
on disk: serde derives on the wire types, RON read/write in
`freejoyx_core::persist`, Save/Load buttons added to the toolbar. The
cross-trip property test (`wire bytes → DeviceConfig → RON →
DeviceConfig → wire bytes` is byte-identical) makes round-trip
parity the load-bearing claim again.

---

## Update 2026-05-17 — Slice 4 (device worker + channel API) DONE

Per Port.md §5 the ordering after Slice 2 is Slice 4 before Slice 5
(UI shell). Slice 4 wraps the synchronous transport from Slice 2 in a
worker thread and `mpsc` channels so the upcoming Slint shell can drive
reads without blocking the UI event loop.

### What landed

- `crates/freejoyx-device/src/worker.rs` (new):
  - `pub fn spawn() -> (DeviceHandle, mpsc::Receiver<DeviceEvent>)`
  - `Command::Shutdown` (the only command this slice; Slice 5+ adds
    `ReadConfig`, `WriteConfig`, `SetLeds`).
  - `DeviceEvent::{Connected(DeviceCandidate), Disconnected,
    ParamsTick(ParamsReport), Error(String)}`.
  - `DeviceHandle::shutdown()` joins cleanly; `Drop` also signals
    shutdown so leaks are impossible.
  - Discovery polls every 600 ms (matches `hiddevice.cpp:64`'s
    `hid_enumerate` cadence). On disconnect (read error / unplug) the
    worker emits `Disconnected` and resumes discovery — hot-plug works
    by construction.
  - Read loop uses a 200 ms `read_params_blocking` timeout so the
    worker can poll the command channel between reads. Treats a
    timeout as device-idle, not as a disconnect.
- `crates/freejoyx-device/src/transport.rs`:
  - New `Device::request_params()` writes a 1-byte `[REPORT_ID_PARAM]`
    HID report. The firmware treats this as a subscription request and
    starts pushing params reports; without it the read loop sits idle
    even though the device is open. Mirrors `hiddevice.cpp:345` (open
    kickoff) and `hiddevice.cpp:361` (5-second refresh).
- `crates/freejoyx-device/src/lib.rs` re-exports `spawn`, `Command`,
  `DeviceEvent`, `DeviceHandle` alongside the existing transport
  surface.
- `crates/freejoyx-app/src/main.rs` `watch` subcommand refactored: now
  spawns the worker and consumes events from the channel. `list` stays
  one-shot. Behaviour is unchanged from Slice 2 modulo the new
  Connected / Disconnected lines visible in the output (Slice 4
  done-when in Port.md: "behaviour unchanged; hot-plug works").

### End-to-end verification on this machine

`timeout 3 cargo run -p freejoyx-app -- watch` on the dev machine
(6 boards plugged in) produced — within ~1 s of startup — a steady
stream of decoded params reports:

```
watching device events ...
INFO ... device worker started
INFO ... connected: FreeJoy — MFDRight (... if 1, serial 3F3E4C383030)
connected: FreeJoy — MFDRight (... if 1, serial 3F3E4C383030)
axes:      0 ... shifts: 10  fw: 0x1713  board: 0
axes:      0 ... shifts: 10  fw: 0x1713  board: 0
... (~20 ticks in 3s)
```

That's the first end-to-end proof that a real device round-trips
through worker → channel → CLI consumer. The two surfaces this exercise
shook out:

1. **The params-subscribe write is mandatory.** Without it the first
   watch attempt sat silent for the full 6-second test window even
   though Connected fired immediately. Added `request_params()` and
   wired the worker to send it on Connect + every 5 s (Qt's cadence).
2. **`Device::open_first()` happens to be the upstream FreeJoy
   (fw 0x1713) MFDRight.** Per Port.md §1.2 the Rust app refuses
   non-FreeJoyX firmware-version groups — that gate belongs in Slice 5
   (Slint shell + toast UX). For now the watch CLI accepts whatever
   the first candidate hands it; the upstream params layout is
   essentially identical for the fields we display.

### Tests

35 tests pass (33 from prior slices + 2 new worker tests):

- `worker::tests::spawn_then_shutdown_joins` — spawn + immediate
  shutdown joins cleanly with no device present. Exercises the
  discovery loop's shutdown responsiveness.
- `worker::tests::drop_handle_terminates_worker` — dropping the handle
  without explicit shutdown still terminates the worker (via `Drop`).
  Would hang under cargo test if the worker leaked.

All five verification commands clean: `cargo fmt --all --check`,
`cargo clippy --workspace --all-targets -- -D warnings` (pedantic on
freejoyx-device; required `# Errors` / `# Panics` doc sections),
`cargo test --workspace`, `cargo build --workspace --release`,
`cargo run -p freejoyx-app -- list` (and `watch` smoke-verified
above).

### Notes for downstream slices

- **Multi-board picker still missing.** `open_first` is arbitrary —
  multi-board users want a `--serial <hex>` filter. Trivial to add
  but not on Port.md §5; let Slice 5's device dropdown drive that
  need.
- **No throttling on ParamsTick.** Port.md §6 risk register calls
  out "Batch `ParamsTick` updates at ~30 Hz max on the worker side;
  profile in Slice 5." Current implementation sends every tick; UI
  binding in Slice 5 will surface whether this is a problem and
  where to throttle.
- **Slice 5 commands.** When `ReadConfig` / `WriteConfig` /
  `SetLeds` land, `poll_shutdown` needs to grow into a proper
  dispatch helper. The forward-compat `Ok(_)` arms got removed to
  silence `clippy::unreachable_patterns`; the rustc warning will
  re-surface as soon as `Command` grows a second variant, which is
  the right reminder.

### What's next

Per Port.md §5 the next slice is **Slice 5 — Slint shell + Pins tab.**
First slice where the Rust UI exercises the `dev_config_t` codec
end-to-end against a real device. After that: Slice 3 (on-disk RON +
validators), then Slices 6-10 (Axes, Buttons/Logic/Timers, Encoders +
Shift Registers, Advanced Settings, polish).

---

## Update 2026-05-17 — Slice 2 (params parser + tiny CLI) DONE

First end-to-end proof that the Rust stack talks to real hardware.

### What landed

- `crates/freejoyx-device/src/error.rs` — `TransportError` (thiserror)
  with explicit variants for `HidInit`, `Enumerate`, `NoDevice`, `Open`,
  `Read`, `ShortRead`, `Decode`, `Timeout`.
- `crates/freejoyx-device/src/transport.rs` — synchronous transport:
  - `enumerate()` mirrors `hiddevice.cpp:108-160`'s discovery filter:
    manufacturer string ∈ {`FreeJoyX`, `FreeJoy`}, interface in
    `-1..=1`, dedup interface-0 entries whose serial also appears as
    interface 1 (F103 layout — joystick HID #0 + custom HID #1 → keep
    #1). F411's single-interface or `-1` entries survive untouched.
  - `Device::open(path)` / `Device::open_first()` — HID handle plus a
    pending-fragments buffer.
  - `Device::read_params_blocking(timeout)` — accumulates frames until
    `fragment_count(PARAMS_REPORT_SIZE) = 2` arrive, calls
    `freejoyx_core::wire::reassemble_fragments` (`first_index = 0` for
    the push path), then `ParamsReport::decode`. Drops non-param report
    IDs silently (other host listeners' joy frames etc).
- `crates/freejoyx-app/src/main.rs` — CLI with `list` and `watch`
  subcommands. No `clap` — `std::env::args` is enough for two verbs,
  and Port.md §3's "no new deps without reason" rule applies. `watch`
  prints one line per tick:
  `axes: ...8 i16...  phy: ...32-hex...  log: ...32-hex...  shifts: XX  fw: 0xNNNN  board: B`

### End-to-end verification on this machine

`cargo run -p freejoyx-app -- list` (run on the dev machine, which has
six boards plugged in) returned:

```
FreeJoy   — MFDRight    (VID 0x0483 / PID 0x5762, if 1)
FreeJoy   — MFDLeft     (VID 0x0483 / PID 0x5761, if 1)
FreeJoyX  — LftPnl      (VID 0x0483 / PID 0x5765, if 1)
FreeJoyX  — F15E UFC    (VID 0x0484 / PID 0x5757, if 1)
FreeJoyX  — FreeJoyX 0.0.2  (VID 0x0483 / PID 0x5760, if 1)
FreeJoyX  — FreeJoy v1.7.2 Set (VID 0x0483 / PID 0x5770, if 1)
```

The dedup worked (no interface-0 duplicates) and both manufacturer
strings (fork + upstream) surface as the Qt configurator would. `watch`
wasn't exercised in this session — maintainer can verify at the bench.

### Notes for downstream slices

- **Upstream FreeJoy firmware lands in the candidate list.** Per
  Port.md §1.2 the Rust app refuses unknown firmware-version mask groups
  ("use the Qt app" toast). That gate belongs in Slice 5 (the UI shell)
  — the Slice 2 `watch` CLI just decodes whatever bytes arrive and
  displays the raw `firmware_version` field, which is enough to tell
  upstream `0x17XX` from FreeJoyX `0x0010` at a glance.
- **`Device::open_first()` picks alphabetically arbitrary.** Multi-board
  users will want a `--device <serial>` selector before the CLI is
  load-bearing. Not blocking Slice 2.
- **Single-threaded by design** — Port.md §3 calls for a worker thread
  + `mpsc` channels but explicitly defers that to Slice 4. The CLI
  reads on the main thread; Ctrl-C terminates the loop.

### Test count + verification

**33 tests still pass** (no new tests — transport is in the
"smoke-tested, not TDD'd" bucket per Port.md §4). All five verification
commands clean: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings` (pedantic-level on `freejoyx-device`,
required `# Errors` / `# Panics` doc sections on each `pub fn`),
`cargo test --workspace`, `cargo build --workspace --release`, `cargo
run -p freejoyx-app` (prints help cleanly).

### What's next

Per Port.md §5 the ordering after Slice 2 is **Slice 4** (device
worker + channel API), then **Slice 5** (Slint shell + Pins tab).
Slice 4 wraps the current synchronous `Device` in a `spawn() ->
(DeviceHandle, mpsc::Receiver<DeviceEvent>)` so the UI can drive reads
from a background worker; the Slice 2 CLI gets refactored onto the new
API with no behaviour change. After Slice 5 comes Slice 3 (on-disk RON
+ validators).

---

## Update 2026-05-17 — Slice 1B (dev_config_t codec) DONE

Picked up from task #13 in the previous session log. Implemented the
full 1580-byte `dev_config_t` codec in
`crates/freejoyx-core/src/wire/config.rs`.

### What landed

- `DeviceConfig` top-level struct mirroring `dev_config_t` field order
  byte-for-byte (including the 1-byte alignment pad GCC inserts before
  `rgb_delay_ms`, kept literal in `rgb_pad`).
- Sub-structs as idiomatic Rust types where the v0.1 UI surfaces them:
  `AxisConfig`, `Button`, `AxisToButtons`, `ShiftRegConfig`,
  `FastEncoder`, `PhysBreakdown`.
- Deferred surfaces (per Port.md §1.1) stored as raw byte blocks so
  bytes round-trip without forcing a domain model: `led_pwm_config_raw`
  (8 B), `leds_raw` (48 B), `led_timer_ms_raw` (8 B), `rgb_leds_raw`
  (250 B). The v0.1.1+ UI work that adds those tabs will introduce the
  domain types; for now the bytes pass through untouched.
- Bitfield-packed bytes are stored raw with `pub` accessor methods that
  mask/shift the named fields (e.g. `AxisConfig::filter()`,
  `Button::shift_modificator()`). Raw storage means unused high bits in
  partially-populated bytes round-trip even if the device shipped
  non-zero garbage there. Mutators land when Slice 5+ wires up the UI.
- Integration test `crates/freejoyx-core/tests/codec_config.rs` proves
  the load-bearing claim: `fixture_round_trip_is_byte_identical` —
  every byte of `fixtures/{minimal,wide_coverage}/config.bin` decodes
  and re-encodes to itself. Plus three sanity tests
  (decode-succeeds, firmware-version-matches, wide-coverage-non-empty).

### Layout discoveries surfaced by the round-trip test

- `axis_config_t` is **30 bytes**, not 31 — confirmed against
  `../sizetest_full2.c` (`_Static_assert sizeof(axis_config_t) == 30`).
  Initial decoder added a speculative 1-byte trailing pad for u16
  alignment; round-trip test caught it (offset 312 vs expected 304
  after `axis_config[8]`). Removed the pad.
- GCC LSB-first bitfield packing on ARM Cortex-M matches MinGW's host
  packing on every fixture byte tested — no MSVC-style swap needed
  (was a tracked Port.md §6 risk).
- `button_t` is 6 bytes; the `shift_modificator :4` widening from
  `anpeaco/FreeJoyX#1` does spill `op` into a second storage byte
  exactly as the in-source comment claims.

### Test count

**33 tests pass** (16 freejoyx-core unit + 4 codec_config + 4
codec_config_fragments + 9 codec_params). All five verification
commands (`cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `cargo test --workspace`,
`cargo build --workspace --release`, `cargo run -p freejoyx-app`)
clean.

### Slice 1 is now complete

Both params and dev_config_t codecs are TDD'd against captured
fixtures, with byte-identical round-trip on every captured set. The
foundation Slice 5+ (UI tabs) will lean on is solid.

### What's next

Per Port.md §5 the next slice is **Slice 2** — params parser + tiny
CLI. The params codec already exists (Slice 1 part 1); Slice 2 wires
it through a `freejoyx-device` HID transport and a `cargo run -p
freejoyx-app -- watch` subcommand that prints live axis + button
state.

Slice 4 (device worker + channel API) comes after, then Slice 5 (Slint
shell + Pins tab) is the first slice that exercises the
`dev_config_t` codec end-to-end against a real device.

---

## Update 2026-05-17 — Slice 0.5(b) captured, Slice 1 (params) complete

Maintainer captured all three fixture sets between sessions. Verified
byte sanity:

- `fixtures/minimal/config.bin` = 1580 bytes, firmware_version=0x0010 ✓
- `fixtures/wide_coverage/config.bin` = 1580 bytes, firmware_version=0x0010 ✓
- `fixtures/{minimal,wide_coverage}/config.fragments.bin` = 1664 bytes
  each (26 × 64) ✓
- `fixtures/*/params.bin` are multiples of 64; every frame starts with
  REPORT_ID_PARAM=0x02; fragment indices alternate 0/1 as expected ✓

### "BUFFERSIZE < FREEJOY_PARAMS_REPORT_SIZE" finding resolved

The Qt-side flag from yesterday turned out **not** to be a bug.
Reading `FreeJoyX/application/Src/usb_app.c:381-413`, the firmware
fragments `params_report_t` (72 bytes) across **two** 64-byte HID
frames using the same `[report_id, fragment_index, 62-byte payload]`
scheme as config. So:

- One wire frame = 64 bytes, payload 62 bytes
- One logical params report = 72 bytes = 62 + 10
- `params.bin` fixtures are append-only wire streams; the codec
  reassembles fragment pairs

`fixtures/REGEN.md`'s "Known issues" §"Qt BUFFERSIZE=64..." section is
now superseded. Closed in this commit.

### Slice 1 (params codec) — DONE

Implemented in `crates/freejoyx-core/src/wire/`:

- `error.rs` — `DecodeError` + `EncodeError` (`thiserror`)
- `cursor.rs` — `Cursor` (LE reader) + `Writer` (LE writer) over byte
  slices, with unit tests
- `fragments.rs` — `Frame::parse` + `reassemble_two_fragment`; handles
  orphan fragment 1 (drop), restarted fragment 0 (keep the latest)
- `params.rs` — `ParamsReport` with `decode` / `encode` for all 11
  fields of `params_report_t`

Integration tests in `crates/freejoyx-core/tests/codec_params.rs`:

| Test | What it proves |
|---|---|
| `fixture_streams_are_frame_aligned` | All `params.bin` lengths are multiples of 64 |
| `fixture_streams_have_expected_report_id` | Every frame has report id 0x02 |
| `reassembly_produces_72_byte_reports` | Fragment pairing matches the firmware's split |
| `every_report_decodes_without_error` | The codec accepts every byte sequence the device produces |
| `firmware_version_matches_target` | Every captured packet shows 0x0010 |
| `board_id_is_consistent_within_a_capture` | board_id doesn't drift mid-capture |
| `fixture_round_trip_is_byte_identical` | **The load-bearing claim** — decode→encode is bit-exact for every fixture packet |
| `params_stream_has_axis_movement` | Capture-quality check; fixture has varying values |
| `params_stream_has_button_or_shift_activity` | Soft check (warning only); current fixture has no button presses |

**21/21 tests pass.** `cargo fmt`, `cargo clippy --all-targets -- -D warnings`,
`cargo test --workspace`, `cargo build --release` all clean.

### Open punch-list discovered during Slice 1

1. **`params_stream` fixture lacks button presses.** The test
   `params_stream_has_button_or_shift_activity` is currently a soft
   warning, not a hard failure. If/when the maintainer re-captures
   `params_stream/params.bin` while pressing buttons during the 3-5
   second window, that test will assert proper coverage. Not a
   blocker for Slice 1 — the codec is proven correct by the
   round-trip test, this is fixture quality.
2. **`reserved_layout` field is `0x0f` on this build.** That's the
   `FIRMWARE_BUILD_ID & 0xFF` wraparound counter from the armgcc
   Makefile. The configurator UI eventually shows this in the sidebar
   per `usb_app.c:391` comment; not urgent for Slice 1 but flag for
   UI layout in Slice 5+.

### Slice 1A — fragment reassembly generalized to N fragments

Bonus landed: the fragment reassembler now handles arbitrary
fragment counts, not just two. Required for config (26 fragments).
The generalization surfaced an important wire-format detail:

**Config fragments use a different index convention than params.**
Config is request-response (Qt requests fragment 1, 2, ..., 26; the
device echoes the requested index back in `buffer[1]`), so wire
indices are `1..=26`. Params is push (firmware just alternates 0/1).
The reassembler now takes a `first_index` parameter to distinguish.

New integration test `tests/codec_config_fragments.rs` proves:

- `fixtures/{minimal,wide_coverage}/config.fragments.bin` (the raw
  wire stream) reassembles **byte-for-byte** to
  `fixtures/{minimal,wide_coverage}/config.bin` (the post-assembly
  struct dump). This validates both (a) `reassemble_fragments` works
  correctly on the 26-fragment path, and (b) the capture patch's
  fragment-dump and struct-dump paths are self-consistent.
- The reassembled config's first two bytes parse as
  `firmware_version = 0x0010`.

That makes **25/25 codec tests passing** at session end.

### Slice 1B NOT yet done

Slice 1 in Port.md covers **both** params and the full `dev_config_t`.
What's still missing: the field-by-field codec for the 1580-byte
struct. Tracked as task #13: "Slice 1 (continued): dev_config_t codec".

The architectural pieces — cursor, fragment reassembly, error types,
test harness — all generalize. The config codec adds:

- A 26-fragment reassembler variant (or generalize the existing
  two-fragment one — the latter is cleaner; will likely refactor
  `reassemble_two_fragment` → `reassemble_n_fragment(stream, expected_id, total_size)`).
- Per-struct decode/encode for `axis_config_t`, `button_t`,
  `encoder_t`, `shift_reg_t`, the pin map, `sensor_t`, LED arrays,
  the timers block, etc.
- Bitfield accessor helpers (mask + shift) for the ~25 bitfield
  clusters. The vendored header analysis from §3 gives the exact
  bit-width for each.

### Next session entry point

Read this section first. Verification commands still work as listed
at the bottom. Start at task #13 (`Slice 1 (continued): dev_config_t codec`).

Strategy for that task:

1. Generalize `reassemble_two_fragment` → `reassemble_fragments` with
   total_size handling arbitrary fragment counts (the 26-fragment
   config path is just `total_size = 1580`, `fragment_count =
   ceil(1580 / 62) = 26`).
2. Author `wire/config.rs` with `DeviceConfig` as a top-level struct
   mirroring the field ordering of `dev_config_t` in
   `vendored/common_types.h`.
3. Implement each inner struct's `decode_<struct>` /
   `encode_<struct>` as paired functions in the same file. Start
   with the easy ones (timers, name, top-level fields), then axes
   (bitfields), then buttons (bitfields + the most fields), then
   encoders/shift-reg.
4. The integration test `tests/codec_config.rs` mirrors
   `codec_params.rs`: load `fixtures/{minimal,wide_coverage}/config.bin`,
   decode, encode, assert byte-identical round trip.
5. **The round-trip test is the safety net** — if it fails, the codec
   has a wrong offset / sign / endianness / bitfield-order
   somewhere; the failing byte's offset tells you which field's
   encode/decode pair to inspect.

Per Port.md §3: do NOT introduce a mirror struct. Walk the bytes
manually. Bitfield accessors are hand-written `(byte >> shift) & mask`.

---

# Session log — 2026-05-16 overnight (original)

Bootstrap session for the Rust + Slint port. Picks up where the grill-me
session in Port.md left off.

## What this session did

**Slice 0 (workspace + CI bootstrap) — COMPLETE locally.**
**Slice 0.5(a) (capture patch + REGEN.md) — COMPLETE, ready for maintainer to apply.**

Slice 0.5(b) (actually capturing the fixtures) is **blocked on hardware**
and is the next session's first task once the maintainer is at the bench.

Slice 1 (wire codec) is **blocked on Slice 0.5(b)**. Do not start Slice 1
without real fixtures — that's the "guess-and-pray" failure mode the plan
explicitly warns about.

## Status of the slice plan

| Slice | Status | Notes |
|---|---|---|
| 0 | ✅ Local pass | All 4 crates compile; `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, `cargo build --release` all clean on Windows. CI not yet validated on Linux/macOS — that runs first time on push. |
| 0.5(a) | ✅ Done | Capture patch + REGEN.md committed-but-uncommitted. See files list below. |
| 0.5(b) | ✅ Done (2026-05-17) | Maintainer captured all three fixture sets between sessions. |
| 1 (params) | ✅ Done (2026-05-17) | 21 tests pass against real fixtures. See top of this file. |
| 1A (fragments N) | ✅ Done (2026-05-17) | reassemble_fragments generalized; 26-fragment config reassembly verified byte-for-byte. |
| 1B (config codec) | ⏸ Next | Task #13. Field-by-field decode of 1580-byte dev_config_t. |
| ≥ 2 | ⏸ Sequenced | Per Port.md §5. |

## Uncommitted changes

**Nothing was committed.** Per the CLAUDE.md "only commit when explicitly
asked" rule, all changes are uncommitted in the working tree. When the
maintainer reviews and is satisfied, suggested commit grouping:

1. **Port.md + Style.MD** — grill-session output from earlier in this
   conversation.
2. **Workspace bootstrap** — `crates/`, `Cargo.toml`, `Cargo.lock`,
   `rust-toolchain.toml`, the new `README.md`, `LICENSE`, `.gitignore`.
3. **Vendored headers + drift workflow** — `vendored/`,
   `.github/workflows/header-sync.yml`, `docs/ported-from.md`.
4. **CI** — `.github/workflows/ci.yml`.
5. **Fixture-capture scaffolding** — `docs/qt-capture-patch.diff`,
   `fixtures/REGEN.md`, `fixtures/README.md`.
6. **Real fixture data** — `fixtures/{minimal,wide_coverage,params_stream}/*.bin`.
   Note: binary blobs; consider whether git LFS is wanted (likely not at
   this size — total ~700 KB).
7. **Slice 1 params codec** — `crates/freejoyx-core/src/wire/`,
   `crates/freejoyx-core/tests/codec_params.rs`. Update to
   `crates/freejoyx-core/src/lib.rs`.

Suggested messages:

```
docs: port plan and visual style guide from grill session
build: bootstrap Rust + Slint workspace (Slice 0)
ci(headers): vendor common_*.h from FreeJoyX with drift workflow
ci(rust): cargo fmt/clippy/test/build matrix on ubuntu/windows/macos
docs(fixtures): capture patch and regeneration guide (Slice 0.5a)
test(fixtures): captured params + config from BluePill (Slice 0.5b)
feat(core): params_report_t wire codec (Slice 1 part 1)
```

## Verification commands

```sh
cd C:/Users/anpea/OneDrive/Documents/DevProjects/Freejoy/FreeJoyXConfigurator
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
cargo run -p freejoyx-app   # prints one tracing::info line and exits
```

All five pass as of session end.

## Maintainer to-do (post-Slice 1 params)

1. **Review uncommitted changes.** Commit grouping suggestion above.
2. **Optional: re-capture `fixtures/params_stream/params.bin`** with
   button presses to harden field coverage. Not blocking. See REGEN.md
   step 3.3.
3. **Then unblock Slice 1 (config)** — the next session picks up task #13.

## File inventory (cumulative)

Bootstrap (2026-05-16):

```
Cargo.toml
Cargo.lock                                # auto-generated
LICENSE                                   # GPLv3
README.md                                 # replaces placeholder
rust-toolchain.toml
.gitignore

crates/freejoyx-core/Cargo.toml
crates/freejoyx-core/src/lib.rs
crates/freejoyx-device/Cargo.toml
crates/freejoyx-device/src/lib.rs
crates/freejoyx-ui/Cargo.toml
crates/freejoyx-ui/src/lib.rs
crates/freejoyx-app/Cargo.toml
crates/freejoyx-app/src/main.rs

vendored/common_defines.h
vendored/common_types.h
vendored/README.md

docs/ported-from.md
docs/qt-capture-patch.diff

fixtures/README.md
fixtures/REGEN.md

.github/workflows/ci.yml
.github/workflows/header-sync.yml
```

Slice 0.5(b) captures (2026-05-17):

```
fixtures/minimal/config.bin                # 1580 bytes
fixtures/minimal/config.fragments.bin      # 1664 bytes
fixtures/minimal/params.bin                # 113344 bytes
fixtures/wide_coverage/config.bin          # 1580 bytes
fixtures/wide_coverage/config.fragments.bin # 1664 bytes
fixtures/wide_coverage/params.bin          # 73024 bytes
fixtures/params_stream/params.bin          # 176128 bytes
```

Slice 1 params + 1A fragments (2026-05-17):

```
crates/freejoyx-core/src/wire/mod.rs
crates/freejoyx-core/src/wire/error.rs
crates/freejoyx-core/src/wire/cursor.rs
crates/freejoyx-core/src/wire/fragments.rs        # incl. N-fragment generalization
crates/freejoyx-core/src/wire/params.rs
crates/freejoyx-core/tests/codec_params.rs
crates/freejoyx-core/tests/codec_config_fragments.rs   # Slice 1A
# lib.rs updated (forbid unsafe + warn clippy::all only — pedantic was too noisy)
```

## Reminders

- **Read Port.md first.** Especially §5 (slice order) and §9 (locked
  decisions).
- **No new dependencies without reason.** The `workspace.dependencies`
  list in `Cargo.toml` is intentionally minimal. Slice 1 (config) does
  not need any new ones.
- **Don't commit `.claude/`** unless the maintainer says so.
- **The codec uses no `unsafe`.** `#![forbid(unsafe_code)]` is set on
  `freejoyx-core`. The Path B strategy means there's no `transmute`,
  no pointer casts, just `from_le_bytes` and explicit masking. If a
  later slice needs unsafe, change this attribute deliberately and
  justify in the commit.
