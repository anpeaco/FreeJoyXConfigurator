# Advanced Axes — port plan

Bringing the Qt configurator's full per-axis config surface to the Rust app.
Reference screenshot: Qt's axes-config row with collapsible "Show extended
settings" panel exposing I2C / Function / Button-action / Step / Prescaler /
Offset / "Buttons from axes" + calibration range visualization.

Locked decisions:
- **Extended panel toggle is per-row** — each axis card carries its own
  expand/collapse state (matches Qt). State stored on `app::State`.
- **Layout = "Append, 2-col tidy"** — extended panel docks below the
  current body in two clean columns of name+cell pairs, with the three
  Button N rows underneath. Card grows taller when expanded; width
  unchanged across collapsed vs expanded siblings. See §2 for the
  reference mockup.
- **Chevron sits at the bottom-right of the body** (above where the
  extended panel docks). Reads as "toggle what's underneath me."
- **Plan only** at this point. No code lands until a phase is picked.

---

## 1. Audit — what's already wired

All `axis_config_t` bytes round-trip (the codec test bundle proves this), so
no wire-format changes are needed. The gaps are accessor + domain + UI.

### Fully wired today (skip)

| Surface | Wire | Domain | Slint | Callback |
|---|---|---|---|---|
| Output checkbox | `flags1` bit 0 | `out_enabled()` | `out-enabled` | ✓ |
| Inverted checkbox | `flags1` bit 1 | `inverted()` | `inverted` | ✓ |
| Centered checkbox | `flags1` bit 2 | `is_centered()` | `is-centered` | ✓ |
| Out bar + raw value | live param | — | `live-out` | ✓ |
| Raw bar + raw value | live param | — | `live-raw` | ✓ |
| Calibrate / Reset / Set-center buttons | — | `AxisCalibration` SM | `calibrating` etc. | ✓ |
| Minimum / Center / Maximum spinboxes | `calib_min/center/max` i16 | direct fields | `calib-min/center/max` | ✓ |
| Filter (dropdown) | `flags1` bits 5..7 | `AxisFilter` enum | `filter-label/index` | ✓ |
| Deadband spinbox | `flags3` bits 0..6 | `deadband_size()` | `deadband-size` | ✓ |
| Dynamic deadband | `flags3` bit 7 | `is_dynamic_deadband()` | `is-dynamic-deadband` | ✓ |
| Resolution spinbox | `flags2` bits 0..3 | `resolution()` | `resolution` | ✓ |
| Channel spinbox | `flags2` bits 4..7 | `channel()` | `channel` | ✓ |
| Axis source + detect + back-to-pin | `source_main` i8 | `AxisSource` | `source-handle` | ✓ |

### Missing

| # | Surface | Wire layout (axis_config_t) | Status |
|---|---|---|---|
| M1 | Function dropdown | `flags1` bits 3..4 (2-bit). Enum: 0=NO_FUNCTION, 1=PLUS, 2=MINUS, 3=EQUAL | No accessor, no domain enum, no Slint |
| M2 | Function-axis dropdown | `source_secondary` (3-bit, 0..7 = axis index) | No accessor, no Slint |
| M3 | Offset ° spinbox | `offset_angle` (5-bit). Display value = raw × 15° | No accessor, no Slint |
| M4 | Step div spinbox | `divider` u8 | No accessor, no Slint |
| M5 | Prescaler % spinbox | `prescaler` u8 | No accessor, no Slint |
| M6 | I2C address dropdown | `i2c_address` u8. Values: 0x36=AS5600, 0x48..0x4B=ADS1115_00..11 | No accessor, no enum, no Slint |
| M7 | Button 1 slot + action | `button1` i8 + `button1_type` 3-bit | No accessor, no enum, no Slint |
| M8 | Button 2 slot + action | `button2` i8 + `button2_type` 2-bit (narrower!) | No accessor, no enum, no Slint |
| M9 | Button 3 slot + action | `button3` i8 + `button3_type` 3-bit | No accessor, no enum, no Slint |
| M10 | Buttons-from-axes count | `axes_to_buttons[i].buttons_cnt` u8. Per-axis structure (13 threshold points + count) | Codec decodes; no accessor, no Slint |
| M11 | Calibration range bar | Visual only — uses existing `calib_min/center/max` | No widget |
| M12 | Show-extended-settings chevron + collapsible panel | UI shell only | Not in Slint |

### Wire-format quick reference (`axis_config_t`, vendored line 50-82)

```
i16 calib_min, calib_center, calib_max
flags1 (u8): out_en:1 | inverted:1 | is_centered:1 | function:2 | filter:3
i8  curve_shape[11]
flags2 (u8): resolution:4 | channel:4
flags3 (u8): deadband_size:7 | is_dynamic_deadband:1
i8  source_main
flags4 (u8): source_secondary:3 | offset_angle:5
i8  button1, button2, button3
u8  divider
u8  i2c_address
flags5 (u8): button1_type:3 | button2_type:2 | button3_type:3
u8  prescaler
u8  reserved[1]
```

### Button-action enum (vendored line 40-48)

```c
AXIS_BUTTON_FUNC_EN       = 0   // Function enable
AXIS_BUTTON_PRESCALER_EN  = 1   // Prescaler enable
AXIS_BUTTON_CENTER        = 2
AXIS_BUTTON_RESET         = 3
AXIS_BUTTON_DOWN          = 4
AXIS_BUTTON_UP            = 5
```

`button2_type` is **2-bit**, so it only encodes values 0..3 (FUNC_EN /
PRESCALER_EN / CENTER / RESET — no DOWN / UP). Reflect this in the picker
by filtering the dropdown entries for slot 2.

### Per-axis `axis_to_buttons_t` (vendored line 401-407, axis_to_buttons array at config offset +1084)

```
u8 points[13]      // 13 evenly-spaced threshold breakpoints (axis range slices)
u8 buttons_cnt     // 0..13 — how many of the 13 segments fire a button
```

The Qt "Buttons from axes" spinbox displays `buttons_cnt`. The point array
holds the button-slot IDs (one per segment). v0.1 can ship with the count
spinbox alone; a per-segment editor is a stretch goal.

---

## 2. Layout

Locked layout — "Append, 2-col tidy". Each card has three logical zones:

```
┌──────────────────────────────────────────────────────────────────┐
│ Header (always visible)                                          │
│   X    [Source ▾] [←][⊕]            [✓ Output] [✓ Inverted]      │
├──────────────────────────────────────────────────────────────────┤
│ Body (visible when has-source)                                   │
│   Out  ▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓ 0       [Calibrate]                │
│   Raw  ▓▓▓▓▓▓▓▓▓▓▓░░░░░░░░░░░ 0       [reset] [center]           │
│   ─●──────────────●──────────●─    ← calibration range bar       │
│   min [___]   ctr [___]   max [___]            [✓ Centered]      │
│   filter [▾]  dead [__]  [✓ Dyn]  res [▾]  ch [▾]           [⌃]  │
├──────────────────────────────────────────────────────────────────┤
│ Extended (visible when expanded — chevron flips to ⌄)            │
│   Function    [None       ▾]     Function axis [X ▾]             │
│   I2C address [ADS1115_00 ▾]     Offset °      [0]↕              │
│   Step div    [50]↕              Prescaler %   [100]↕            │
│                                                                  │
│   Buttons from axes [0]↕                                         │
│   Button 1   [0]↕   [Down  ▾]                                    │
│   Button 2   [0]↕   [Reset ▾]   (Down/Up unavailable on slot 2)  │
│   Button 3   [0]↕   [Up    ▾]                                    │
└──────────────────────────────────────────────────────────────────┘
```

Card-height computation (`compute_axes_viewport_height` in `app.rs`) gets
a third tier:

```
h = 48                                      // header only (no source)
  | 172 (current body)                      // has-source, collapsed
  | 172 + EXTENDED_PANEL_H (~152)           // has-source, expanded
```

`EXTENDED_PANEL_H` ≈ 152 px = padding-top 8 + 3 rows × 28 + spacing 6
+ row gap 12 + 3 button rows × 28 + padding-bottom 8. Tighten in Phase 1
once the panel is laid out.

**Chevron**: lives at the bottom-right of the body row that holds the
filter / dead / Dyn / res / ch cells. Glyph `chevron-down` when
collapsed, `chevron-up` when expanded (both already in `Icons`).
Re-use `IconButton` with the `pressed` toggle.

---

## 3. Phasing

Each phase is independently shippable: wire → domain → row model → Slint
→ Rust callback → cargo test green.

### Phase 1 — Extended-panel shell + Function group (smallest)

Goal: validate the chevron + collapsible mechanic by shipping it with
the 5 simplest extended fields. No new picker-style UI primitives.

Touches:
- `crates/freejoyx-core/src/wire/config.rs::AxisConfig`
  - Add accessors: `function()`, `source_secondary()`, `offset_angle()` (raw, 0..31), `divider()`, `prescaler()`, plus setters.
  - Setters for the bitfields must preserve the other half of each byte (`set_function` writes only bits 3..4 of flags1, `set_offset_angle` writes only bits 3..7 of flags4).
  - Unit tests: bitfield mask isolation (same pattern as `axis_config_setters_isolate_their_bits`).
- `crates/freejoyx-core/src/domain/axes.rs`
  - `enum AxisFunction { None, Plus, Minus, Equal }` with `from_u8` / `to_u8` / `label`.
  - Inline helper for offset °: `raw_to_degrees(raw: u8) -> u16 = raw * 15`. Inverse for the setter input.
- `crates/freejoyx-ui/ui/types.slint::AxisRow`
  - New fields: `extended-expanded: bool`, `function-label`, `function-index`, `function-axis-index`, `function-axis-label`, `offset-deg`, `step-div`, `prescaler`.
- `crates/freejoyx-ui/src/app.rs::State`
  - New field `axes_extended_expanded: [bool; MAX_AXIS_NUM]`, default false.
  - Bump `compute_axes_viewport_height` to add the third tier.
- `crates/freejoyx-ui/src/app.rs::build_axis_row` (or wherever the row gets built — confirm in slice 6 glue)
  - Populate the new fields.
- `crates/freejoyx-ui/ui/rows/axis_row_view.slint`
  - Bottom-of-body chevron (`PillButton` or compact `IconButton` with `chevron-down`/`chevron-up`).
  - New `if root.entry.extended-expanded: VerticalLayout { ... }` block under the body.
  - 6 cells inside (3 + 3 layout): Function dropdown, Function-axis dropdown, Offset° NumberCell, Step div NumberCell, Prescaler% NumberCell. (Resolution / Channel stay where they are; moving them is Phase 5 polish.)
- `crates/freejoyx-ui/ui/tabs/axes_tab.slint`
  - Forward a `extended-toggled(int)` callback per row.
- `crates/freejoyx-ui/src/app.rs::wire_axis_callbacks`
  - `on_axis_extended_toggled`: flip the per-axis bool, rebuild the row, recompute viewport height.
  - Two new dropdown kinds: `DropdownKind::AxisFunction`, `DropdownKind::AxisFunctionRef`.
  - Three new number callbacks: `axis-offset-edited`, `axis-step-div-edited`, `axis-prescaler-edited`. Each goes through a `mutate_axis(slot, |a| { a.set_X(v) })` style helper.

Estimated effort: ~6 wire accessors + 6 setters with masks, 1 domain enum, 5 UI cells, 5 callbacks, 1 viewport recalc. Largest risk: getting the bitfield masks right (covered by tests).

### Phase 2 — Buttons-from-axes + axis-action buttons

Goal: the big advanced-config slab — three Button N rows in the extended
panel + the "Buttons from axes" count in the body.

Touches:
- Wire accessors for `button1`, `button2`, `button3`, `button1_type` (3-bit), `button2_type` (2-bit!), `button3_type` (3-bit). Setters for each, preserving flags5 neighbors.
- Wire accessor + setter for `axes_to_buttons[i].buttons_cnt` (already decoded as a field, just needs the getter/setter on the parent `DeviceConfig`).
- Domain enum `enum AxisButtonAction { FuncEn, PrescalerEn, Center, Reset, Down, Up }` with `from_u8` / `to_u8` / `label`.
  - Add a `valid_for_slot2()` helper or expose the variants list with a filter — `button2_type` is 2-bit and cannot store Down/Up.
- `AxisRow` gains: `buttons_from_axes`, three pairs (`btn1-slot`, `btn1-action-label`, `btn1-action-index`) etc.
- New row block in `axis_row_view.slint`: three lines, each `NumberCell` + `DropdownCell`.
- One new dropdown kind: `DropdownKind::AxisButtonAction(slot_in_row: u8)` — needs to gate Down/Up out when `slot_in_row == 1`.
  - Alternatively: two kinds, `AxisButtonAction3Bit` and `AxisButtonAction2Bit`, dispatch table chooses based on which dropdown opened.
- Five new callbacks: `axis-btn-slot-edited(axis, n, v)`, `axis-btn-action-edited` (via dropdown), `axis-buttons-from-axes-edited(axis, v)`.

Header strip gets a small `NumberCell` for `buttons_from_axes`, sized to ~50 px.

### Phase 3 — I2C address picker

Goal: the smallest standalone phase. Single dropdown, conditional on
`AxisSource::I2C`.

Touches:
- Wire accessor + setter for `i2c_address` (u8, already a plain field — just add the getter/setter).
- Domain enum `enum I2cAddress { As5600 = 0x36, Ads1115_00 = 0x48, Ads1115_01 = 0x49, Ads1115_10 = 0x4A, Ads1115_11 = 0x4B }`. Verify the encoding by reading Qt's `Converter::EnumToIndex` table for this field.
- `AxisRow` gains `i2c-addr-label`, `i2c-addr-index`, `i2c-enabled` (= `source == I2C`).
- Dropdown kind `DropdownKind::AxisI2cAddress`.
- One callback path (already covered by existing dropdown-picked dispatch).
- `axis_row_view.slint`: dropdown sits in the extended panel; greyed when `i2c-enabled == false`.

### Phase 4 — Calibration-range visualization

Goal: visual polish. No new wire fields.

Touches:
- New Slint cell `ui/cells/calib_range_bar.slint` — a Rectangle the width
  of the Out bar, with three thumbs:
  - leftmost: position of `(calib_min + 32768) / 65536 * width`
  - centre: position of `(calib_center + 32768) / 65536 * width` (only drawn when `is_centered`)
  - rightmost: same math for `calib_max`
  - red overlay between min and max to mirror Qt's shading
- Place between the Raw bar and the min/ctr/max number row.
- Live updates: rebuild on calibration edits (already trigger row rebuild). No live-tick subscription needed.

No accessors required.

### Phase 5 — Filter slider polish (optional, deferrable)

Convert the filter dropdown to a continuous slider. Visual only — value
domain stays the 8-level enum. Likely not worth the work unless other
slider use cases appear; cosmetic difference vs Qt.

---

## 4. Open questions before any implementation starts

1. **Button-action enum membership.** Confirm against Qt source which 6
   values actually appear in the dropdowns and in what order (the C enum
   declares all 6, but the Qt picker may filter further or relabel).
   Touch `FreeJoyXConfiguratorQt/src/widgets/axes/axisconfigwidget.cpp`
   before locking the domain enum.

2. **I2C address enum scope.** Confirm the AS5600 / ADS1115_xx mapping by
   reading Qt's converter table for `i2c_address`. There may be more
   chips than the screenshot reveals.

3. **`buttons_from_axes` semantics.** Confirm whether editing this count
   in Qt also rewrites the `points[13]` array (e.g., re-spaces the
   thresholds evenly). If so, our setter must do the same to stay
   compatible. Easiest check: capture two `config.bin`s from Qt at
   buttons_cnt=0 and buttons_cnt=8, diff the per-axis `points` bytes.

4. **Card-height recompute.** `compute_axes_viewport_height` currently
   sums header/body. Extended panel adds a third tier. Confirm the
   exact pixel budget after laying out the extended block in Slint —
   plan placeholder is 132 px, will tighten in Phase 1.

5. ~~Chevron placement.~~ Locked: bottom-right of body. See §2.

---

## 5. Files this plan will touch (cumulative across phases)

```
crates/freejoyx-core/src/wire/config.rs           // accessors + setters + tests
crates/freejoyx-core/src/domain/axes.rs           // AxisFunction, AxisButtonAction, I2cAddress
crates/freejoyx-ui/ui/types.slint                 // AxisRow extensions
crates/freejoyx-ui/ui/rows/axis_row_view.slint    // extended panel + chevron
crates/freejoyx-ui/ui/tabs/axes_tab.slint         // forward callbacks
crates/freejoyx-ui/ui/cells/calib_range_bar.slint // Phase 4 new cell
crates/freejoyx-ui/src/app.rs                     // State.axes_extended_expanded,
                                                   // wire_axis_callbacks additions,
                                                   // DropdownKind additions,
                                                   // build_axis_row population,
                                                   // compute_axes_viewport_height
```

No `vendored/`, no fixture, no `FIRMWARE_VERSION` bump — everything in
scope round-trips today.
