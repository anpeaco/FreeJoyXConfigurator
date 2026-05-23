# UX backlog

Captured 2026-05-18 from a hands-on session with a freshly-flashed
BlackPill plugged in. Ordered roughly cheapest → costliest so easy wins
build momentum.

---

## 1. De-emphasize system pins on Pins tab  ←  *landed 2026-05-18*

**Files.** `crates/freejoyx-ui/ui/rows/pin_row_strip.slint`,
`crates/freejoyx-ui/ui/types.slint`.

**Problem.** Fixed-role rows (`GND`, `3V3`, `5V`, `RST`, `VBUS`,
`BOOT0`, `USBD-`, `USBD+`) sit in the same column as configurable GPIO
rows and visually compete with them. Users can't tell at a glance that
GND can never carry a function.

**Solution.** Mute system rows: lower font weight on the silk label,
reduce opacity (or shift to a secondary palette colour) for the
role-label cell, drop the dropdown chrome entirely — render the role
text where the picker would be. The configurable-GPIO rows keep full
contrast.

Smallest contained win; touches Slint only.

---

## 2. Uppercase logic-op names in pickers  ←  *landed 2026-05-18*

**Files.** `crates/freejoyx-core/src/domain/logic.rs` (`LogicOp::label`
or equivalent), the LOGIC operator dropdown in the buttons tab.

**Problem.** Logic operators render as `And`, `Or`, `Not` —
inconsistent with how the user *thinks* about boolean algebra (`AND`,
`OR`, `NOT`). Uppercasing reinforces that these are operators, not
prose.

**Solution.** Change `LogicOp::label` (or the Slint-side display
binding) to uppercase strings. Verify the picker + selected-value
cell + any truth-table labels all flow through the same source.

---

## 3. Live "fires when" tooltip on Logic rows  ←  *landed 2026-05-18*

**Files.** `crates/freejoyx-core/src/domain/logic.rs`,
`crates/freejoyx-ui/src/tabs/buttons.rs`, the logic row UI in
`buttons_tab.slint`.

**Problem.** Once a LOGIC slot has an op + Source A + Source B, the
configurator gives no feedback about what it actually does. Users
must hold the truth table in their head: "NOR means it fires when
neither source is pressed."

**Solution.** A read-only summary line under each LOGIC row that
describes the condition in plain English: `"Fires when neither
PA0 nor PA1 is pressed."` Pure function from `(op, src_a, src_b,
&Board)` → `String` in domain; UI binds the result via the row's
view-model.

Pairs with #2 — same area of code; ship together.

---

## 4. Regroup button-type categories  ←  *landed 2026-05-18*

**Files.** `crates/freejoyx-core/src/domain/buttons.rs`
(`ButtonTypeCategory` definition + `all()` iter), the per-category
dropdown construction in `tabs/buttons.rs`.

**Problem.** The current Basic group is too narrow. TOGGLE and LOGIC
sit in their own groups but are conceptually as everyday as NORMAL —
they're the bread-and-butter button types for cockpit switches.

**Solution.** Move TOGGLE and LOGIC into `ButtonTypeCategory::Basic`
alongside NORMAL / LONG_PRESS / DOUBLE_TAP. RADIO / SEQUENTIAL / POV /
ENCODER stay in their dedicated groups.

Watch for: per-physical coexistence rule
(`{NORMAL, LONG_PRESS, DOUBLE_TAP}`-only — F103_GESTURE_PLAN.md) is
*not* a basic-vs-not split; it's an explicit allowlist. Regrouping
the picker doesn't change the validator.

---

## 5. Tab-strip "configured" indicator dots  ←  *landed 2026-05-18*

**Files.** Per-tab modules
(`tabs/{pins,buttons,encoders,advanced}.rs`), tab strip in
`app.slint`.

**Problem.** Eight tabs across the top, only one foreground at a time;
nothing tells the user that the Encoders tab has stuff configured.
Users discover content by clicking through.

**Solution.** Each tab module gains a
`fn has_content(&DeviceConfig) -> bool` predicate. The tab strip
binds a per-tab `has-content: bool` property; the tab button overlays
a 6 px dot or thickens the underline when true. Updates on
`ConfigReceived` and after every dropdown-pick (via the existing
mark-dirty path).

Independent of every other item.

---

## 6. Buttons filter bar: alignment + auto-detect  ←  *landed 2026-05-18*

**Files.** `crates/freejoyx-ui/ui/tabs/buttons_tab.slint` (filter
bar), `tabs/buttons.rs` (filter state), `app.rs` State + the
`ButtonCapture` mode for the "press to filter" mechanism.

**Problem.**
- Filter bar controls don't sit on a consistent baseline — visual
  jitter at the top of the buttons tab.
- The "filter by physical button #X" requires the user to remember
  which physical index they care about and type it in. Selecting it
  by *pressing the button itself* is the natural gesture.

**Solution.**
- Pure layout fix on the filter bar: align controls on a single
  baseline; standardize cell widths.
- Add a "filter on press" mode toggle. When armed, the next physical
  button press writes its index into the physical-filter cell and
  optionally disarms the mode. Reuses the existing
  `ButtonCapture::on_params_tick` edge detection; new mode type or a
  flag on `ButtonCapture` (whichever the in-progress Buttons-tab
  refactor wants).

Pairs naturally with the still-pending Buttons-tab extract from
`ARCHITECTURE_BACKLOG.md` #1 — once `ButtonsTab` owns its filter
state, this is a 2-field-change.

---

## 7. Combo-box popover clipping at window edges  ←  *landed 2026-05-18*

**Files.** `crates/freejoyx-ui/ui/cells/*.slint` (whichever element
hosts the dropdown overlay), `app.slint` (window root +
overlay layer).

**Problem.** Dropdowns triggered near the bottom of the window project
down past the window edge and get cut off. Same for cells near the
left or right edges.

**Solution.** Position the overlay anchored to the cell but flipped
to the opposite side when the natural side would clip the window.
For Slint, this is usually a `PopupWindow` with `close-on-click` and
explicit `x`/`y` properties driven by an "is there room below?"
calculation. May require restructuring how dropdowns are mounted — if
they're currently in-flow children, they need to move to a top-level
overlay layer.

Investigation step first; budget unknown until I dig into the popup
mounting.

---

## 8. Top bar redesign + device summary  ←  *landed 2026-05-18*

**Files.** `crates/freejoyx-ui/ui/app.slint` (toolbar block),
toolbar-related glue in `app.rs`.

**Problem.** Toolbar controls jump around as connection / read / write
state changes. The device summary line packs too much in a single
flat string (`FreeJoyX — name (VID 0xXXXX / PID 0xXXXX, if N, serial S)`)
and buries the board type. New users struggle to find Read / Write
quickly.

**Solution.** Design-first item — there's a layout question (grouped
sections vs. ribbon vs. status-bar split) and a content question
(what's on the chip, what's in a hover, what's in the About dialog).
Worth a grilling session before implementation.

Largest item by far. Hold until the cheaper wins land; then plan.

---

## Status legend

- *next up* — currently being worked on.
- *landed YYYY-MM-DD* — shipped; left in the doc as a record.
- *deferred* — known but pushed beyond the current session.
