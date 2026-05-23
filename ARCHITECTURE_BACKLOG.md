# Architecture deepening backlog

Captured from an `/improve-codebase-architecture` pass on 2026-05-18. Four
candidates, ordered by impact. Vocabulary follows `~/.claude/skills/improve-codebase-architecture/LANGUAGE.md`
— *module*, *interface*, *implementation*, *seam*, *adapter*, *depth*,
*leverage*, *locality*, *deletion test*.

---

## 1. Per-tab modules  ←  *partially landed; see remaining scope below*

**Files.** `crates/freejoyx-ui/src/app.rs` (2923 lines), plus the partial
extracts already in `buttons.rs`, `encoders.rs`, `advanced.rs`. Touches
`wire/config.rs` and every `domain/*.rs`.

**Problem.** No Pins / Axes / Buttons / Encoders / ShiftRegs module on
the UI side. There's a 63-field `State` struct every callback mutates by
line-of-sight, an 8-arm `match` for `DeviceEvent::ParamsTick` that
re-renders every tab unconditionally, and a 280-line `apply_dropdown_pick`
(lines 2255–2535) routing 15 dropdown kinds with cascade refreshes.
Deletion test points at concentration: a Buttons-tab feature touches
`app.rs::run`, `app.rs::apply_dropdown_pick`, `app.rs::apply_button_capture`,
`buttons.rs::refresh_button_model`, plus a slice of `State`. If a
`pins_tab` / `axes_tab` / `buttons_tab` module each owned the dropdown
arms, refresh, and per-tab state, `run()` collapses to wiring.

**Solution.** Promote `buttons.rs` from "the buttons view-model helpers"
to "the Buttons tab module" — same shape for every tab. Each tab module
owns: its slice of `State` (button capture, axis calibration, etc.); the
view-model builders (`build_*_row`); the refresh decision (incl. the
layout-dedup check `buttons.rs` already has); and the dropdown-pick arms
for kinds that belong to that tab. `app.rs::run()` constructs them,
hands each a `DeviceEvent::ParamsTick`, and dispatches edits by
kind→tab.

**Benefits.**
- *Locality.* A Buttons-tab change lives in one file, not five. Cascade
  rules (pin change → axis row refresh) become explicit cross-module
  calls instead of side effects in a match arm.
- *Leverage.* `run()` shrinks from 200+ lines of wiring to a dispatch
  loop. Adding a new tab stops being a re-read of `app.rs`.
- *Tests.* Each tab module is testable end-to-end against a fixture
  `DeviceConfig` + synthetic `ParamsReport` — today `build_axis_row` is
  pure but untested because the surrounding glue isn't testable, so
  nobody set up the fixtures. The interface *is* the test surface.

The largest deepening. Also the largest diff.

**Landed on 2026-05-18:**
- `crates/freejoyx-ui/src/tabs/` directory created.
- Existing `advanced.rs`, `buttons.rs`, `encoders.rs` moved under `tabs/`.
- New `tabs/pins.rs` carved out of `app.rs` — owns `refresh_pin_model`,
  `pin_jump_target`, `axis_back_to_pin`, `shift_reg_back_to_pin`,
  `PinJumpTarget`, and the cross-tab encoder-pin helpers. `app.rs`
  shed ~250 lines.

**Still to do (deferred to a future session):**
- **Axes tab** — `build_axis_row`, `refresh_axis_*`, `mutate_axis`,
  `apply_axis_field`, `apply_axis_source`, viewport calculations,
  source-dropdown apply, and the calibrate / reset / set-centre /
  detect callbacks. ~600 lines still in `app.rs`. Will absorb the
  `AxisCalibration` + `AxisDetect` modes as struct fields when it
  lands.
- **Buttons tab** — already in `tabs/buttons.rs`; needs the filter
  state + `ButtonCapture` mode reframed as fields on a `ButtonsTab`
  struct, and the dropdown-pick arms moved out of `apply_dropdown_pick`.
- **Shifts & Timers**, **Shift Registers** — small surfaces still
  living inside `tabs/buttons.rs` and `tabs/encoders.rs` respectively.
  Spin out into their own files once the heavier tabs are done.
- **`apply_dropdown_pick`** — the 280-line match still in `app.rs`.
  Each tab module wants to own its kind→handler arms. Dispatcher
  collapses to `match kind_to_tab(kind) { ... }`.
- **`State` god-struct** — would shrink dramatically when each tab
  owns its own slice. Today it carries ~60 fields; ~40 would move.

---

## 2. Interactive-mode state machines  ←  *landed 2026-05-18*

**Files.** `app.rs:159–170` (`State.button_capture`, `axis_detect`,
`axis_detect_disarm_ticks`, `last_phy_button_data`), `:1081–1130`
(`mutate_axis` calibration glue), `:2758–2835` (`apply_button_capture`),
`:2836–2920` (`apply_axis_calibration`, `apply_axis_detect`).

**Problem.** Three independent state machines — **button-capture mode**
(click a physical field → next press writes the index), **axis
calibrate** (move axis through range → save), **axis detect** (move axis
→ bind source) — live as loose fields on `State` and update logic
scattered across the `ParamsTick` handler. State and transitions
physically far apart. Each one needs a `disarm-tick` u32 fed through
Slint to force a UI cell out of capture mode.

**Solution.** Each mode becomes a `Mode` module (`ButtonCapture`,
`AxisCalibration`, `AxisDetect`). The module owns its arm/disarm state,
the `disarm-tick` counter, and a single `on_params_tick(&ParamsReport,
&mut DeviceConfig) -> ModeOutcome` that the UI layer calls in one place.

**Benefits.**
- *Locality.* All transitions for "button capture" live next to all
  reads. A bug in edge-detection (`phy & !last_phy`) gets fixed in one
  file with one test.
- *Leverage.* The UI layer stops needing to know these are state
  machines — it sees `outcome.cell_should_disarm = true` and renders.
- *Tests.* Each Mode is a state machine over a stream of `ParamsReport`.
  Wire it to a `Vec<ParamsReport>` fixture and assert transitions — no
  Slint runtime, no real device.

Smaller than #1, stands on its own, slots into the tab modules naturally
if you do #1 later.

---

## 3. Pre-write configuration validator  ←  *next up*

**Files.** `crates/freejoyx-core/src/domain/{pins,buttons,logic}.rs` —
partial validators today (`validate_pins`, `validate_logic_buttons`,
`physical_assignment_blocked`). Caller side: `app.rs::write-clicked`
callback.

**Problem.** Validators are scattered and partial. `validate_pins`
checks singleton conflicts (SPI/I2C/UART) only. `validate_logic_buttons`
flags incomplete LOGIC slots inline in the row but doesn't block Write.
`physical_assignment_blocked` is checked at dropdown-pick time, not at
write time. There is no single "is this `dev_config_t` shippable?"
question the UI can ask. Result: the UI can ship a config with a
half-completed LOGIC button, and the inline error is the only signal.

**Solution.** A `validate_for_write(&DeviceConfig) -> Result<(),
Vec<ConfigError>>` module in `freejoyx-core::domain` that aggregates
every existing partial validator and returns a structured list of
problems. The UI's Write button calls it, surfaces errors as a toast or
inline panel, and only sends `Command::WriteConfig` when it returns Ok.

**Benefits.**
- *Locality.* Every "is this rule allowed?" question lives in `domain/`.
  The Buttons tab loses `physical_assignment_blocked` from its
  dropdown-arm checks (it calls the validator instead) — fewer places
  to forget.
- *Leverage.* The validator is the *only* thing the firmware-write path
  trusts. New rules drop in centrally.
- *Tests.* Pure function from `DeviceConfig` → `Result<_,
  Vec<ConfigError>>`. Fixtures + test pattern already exist in
  `domain/`.

Independent of #1 and #2. Small contained win.

---

## 4. Transport seam (trait Transport)  ←  *landed 2026-05-18*

**Files.** `crates/freejoyx-device/src/transport.rs` (`Device` wrapping
`HidDevice`), `worker.rs` (calls `Device::open` / `read_params_blocking`
/ `read_config` / `write_config` directly).

**Problem.** The worker thread's interesting logic — reconnect on
disconnect, the 250 ms resend-on-silence loop in config reads, the
params-subscription refresh — is welded to a real hidapi handle. Zero
test coverage of the worker today because mocking `hidapi::HidDevice`
isn't practical. One adapter today (real hidapi `Device`) =
*hypothetical* seam, not a real one.

**Solution.** Promote `Device`'s public methods to `trait Transport`.
Real adapter = what exists today. Test adapter = scriptable fake
returning canned frames / errors. Worker depends on `&mut dyn Transport`
(or generic `T: Transport`), not on `Device` directly.

**Benefits.**
- *Locality.* Reconnect, resend, and command dispatch get tests that
  pin behaviour — today verified only when you plug in a real board.
- *Leverage.* A future second backend (the bridged test harness Port.md
  mentions, or a CDC/serial fallback) drops in as another adapter. Two
  adapters = real seam.
- *Tests.* "Worker handles mid-read disconnect cleanly" becomes a unit
  test instead of a hardware-only check.

Smaller than #1, narrower than #3.

---

## 5. `ParamsSubscription` module  ←  *landed 2026-05-18*

**Files.** `crates/freejoyx-device/src/subscription.rs` (new),
`crates/freejoyx-device/src/{lib,worker}.rs`.

**Problem.** `device.request_params()` was called from five places in
the worker — initial subscribe at `worker.rs:263`, periodic 5 s tick at
`:329-335`, post-`ReadConfig` at `:382`, post-`WriteConfig` at `:400`,
and implicitly every reconnect. The renewal cadence + the per-site
failure policy were tangled inline in `pump_until_disconnect` and
`dispatch_pending_commands`. The post-`WriteConfig` site additionally
had a comment ("expected on re-enum") that contradicted the code
(returned `Disconnect` anyway). Wall-clock `Instant::now()` made the
5 s renewal untestable — `pump_tests` runs in milliseconds.

**Solution.** A `pub(crate) ParamsSubscription` struct in
`subscription.rs` owns the periodic deadline and concentrates every
renewal moment. Constructor `subscribe(now, transport)` doubles as the
initial subscribe — the type system prevents constructing a
subscription without sending the initial `request_params()`. The single
`renew(now, reason, transport) -> RenewalOutcome` method drives every
other call site; the `RenewalReason` enum (`Periodic | AfterRead |
AfterWrite`) carries the per-site failure policy into the module:
- `Periodic` — no-op before deadline; failure → `Lost`.
- `AfterRead` — always sends; failure → `Lost`.
- `AfterWrite` — always sends; failure swallowed with a
  `tracing::debug!` because the device is mid-re-enum.

Pump-side delta: ~30 lines removed from `worker.rs`; nine renewal call
sites collapsed to four `subscription.renew(...)` calls plus one
`ParamsSubscription::subscribe(...)`. Both functions gain a
`subscription: &mut ParamsSubscription` arg.

**Benefits.**
- *Locality.* The "what keeps the firmware streaming" rule lives in
  one file with the policy comments next to the deadline math, not
  scattered across two functions plus a top-level constant.
- *Leverage.* Five call sites → one constructor + one method. The
  "constructor doubles as the initial subscribe" shape makes
  forget-to-subscribe unrepresentable.
- *Tests.* Nine new unit tests in `subscription::tests` against a
  tiny `RecordingTransport` fake (separate from
  `worker::pump_tests::FakeTransport`). The 5 s deadline behaviour is
  now covered — periodic-before-deadline is a no-op,
  periodic-after-deadline sends and resets, periodic-failure leaves
  the deadline so the next tick retries, `AfterWrite` swallows.

**Behaviour change.** Write-time renewal failure no longer disconnects.
Previously `dispatch_pending_commands` returned `Disconnect` on
post-`WriteConfig` `request_params()` failure even though the device
was usually mid-re-enum; the next ~200 ms read timeout + periodic
5 s renewal now handle the real disconnect case more accurately.

Smaller than #1, narrower than #3, sibling of #4.
