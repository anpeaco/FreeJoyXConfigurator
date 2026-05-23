# Fixture regeneration guide

These fixtures are the **oracle** for the Rust codec. Round-trip tests
alone don't catch field-width drift or bitfield-order bugs — only a
real device producing real bytes does. When the wire format bumps,
**regenerate every fixture** before touching the codec.

## When to regenerate

- `FIRMWARE_VERSION` mask group changes (e.g. 0x0010 → 0x0020).
- Any field is added, removed, or resized in `common_types.h`.
- `params_report_t` shape changes (then `params.bin` is the canonical
  capture of the new shape).
- A field's bit-order interpretation changes (rare but possible).

If only legacy migration code changes — i.e. the *current* wire format
is unchanged — fixtures stay valid; do not regenerate.

## Prerequisites

- A healthy FreeJoyX board (BluePill or BlackPill) running the target
  `FIRMWARE_VERSION` (currently `0x0020`).
- A working build of `FreeJoyXConfiguratorQt` on a throwaway branch.
  The branch has `docs/qt-capture-patch.diff` applied (see that file
  for the patch and notes).
- The patched Qt build can launch with `FREEJOYX_DUMP_FIXTURE=<path>`
  set in the environment.

## What we capture

Three fixture sets, each in its own subdirectory under `fixtures/`:

| Set | Purpose | Required device state |
|---|---|---|
| `wide_coverage/` | Exercises every type and field — the codec's main test surface | Hand-tuned config (see §"Wide-coverage config", below) |
| `minimal/` | Factory-default sanity baseline | Factory-reset device |
| `params_stream/` | 2-3 seconds of params packets covering all input modalities | Wide-coverage device + maintainer exercising inputs |

Each subdirectory contains:

- `config.bin` — the assembled 1580-byte `dev_config_t` written once
  after "All config received".
- `config.fragments.bin` — append-only stream of 26 × 64-byte HID
  frames as received on the wire (each frame: 1-byte report-id +
  1-byte fragment-index + 62 bytes payload).
- `params.bin` — append-only stream of 64-byte params packets (see
  the "Known Qt truncation" note in §"Known issues").
- `expected.ron` — **the oracle**. Hand-authored RON file describing
  what the Qt app shows for `config.bin`. The Rust codec's first
  fixture test is: decode `config.bin`, assert the result equals
  `expected.ron` (under the Rust domain types).

## Step-by-step

### 0. One-time setup

```powershell
# Clone or update the Qt repo
cd C:\path\to\FreeJoyXConfiguratorQt
git checkout -b throwaway-fixture-capture origin/main
# Apply the patch
git apply C:\path\to\FreeJoyXConfigurator\docs\qt-capture-patch.diff
# Build the Qt app (your usual qmake/make/build dance)
```

The branch is throwaway — do not commit changes to FreeJoyXConfiguratorQt.

### 1. Capture `minimal/`

1. Factory-reset a board:
   - Plug it in, open the Qt app.
   - Go to **Advanced** tab → **Reset all settings to defaults**.
   - Click **Write to device**. Wait for re-enumeration.
2. Close the Qt app.
3. From PowerShell:
   ```powershell
   $env:FREEJOYX_DUMP_FIXTURE = "C:\path\to\FreeJoyXConfigurator\fixtures\minimal"
   # Make sure the directory does not already contain stale captures.
   Remove-Item -Recurse -Force C:\path\to\FreeJoyXConfigurator\fixtures\minimal -ErrorAction SilentlyContinue
   .\FreeJoyQt.exe
   ```
4. In the Qt app: click **Read from device**. Wait until the title bar
   or log shows the config arrived ("All config received").
5. Close the Qt app. Confirm three files exist in `fixtures/minimal/`:
   - `config.bin` (size = 1580 bytes for `FIRMWARE_VERSION = 0x0020`;
     unchanged from the 0x0010 generation since the bump was semantic only)
   - `config.fragments.bin` (size = 26 × 64 = 1664 bytes)
   - `params.bin` (some multiple of 64; depends on capture duration)
6. Hand-author `fixtures/minimal/expected.ron` from the factory-default
   config description in `FreeJoyXConfiguratorQt/src/deviceconfig.cpp`
   (initial-state values). Keep it short — every field has a known
   default value; just record them as RON.

### 2. Capture `wide_coverage/`

The wide-coverage config exercises every type and bitfield. **Authoring
this config is itself the most error-prone step** — get the device state
right *before* capturing, because a missed field is a hole in coverage
the codec tests can't see.

#### Wide-coverage config

Set up via the Qt app and **Write to device**. Confirm the read-back
matches before capturing.

- **Device name:** `FJX-WIDE-FIXTURE` (proves device-name bytes
  round-trip; pick exactly this string so `expected.ron` is reproducible).
- **VID / PID:** leave at defaults; bytes still get captured.
- **Pins (BluePill layout):** set at least one pin to each of these
  functions, distributed across the pin range:
  - `BUTTON_GND`, `BUTTON_VCC`, `BUTTON_ROW`, `BUTTON_COLUMN`
  - `SHIFT_REG_CS`, `SHIFT_REG_DATA`, `SHIFT_REG_CLK`
  - `ENCODER_A`, `ENCODER_B` (at the fast-encoder-capable pin pair —
    PA8/PA9 or PB6/PB7)
  - At least one axis pin (`AXIS_ANALOG`)
- **Buttons (rows 0-9):**
  - Row 0: NORMAL, physical 0
  - Row 1: TOGGLE, physical 1
  - Row 2: LOGIC, op = AND, source-A = 0, source-B = 1, no shift, not inverted
  - Row 3: LOGIC, op = NOT, source-A = 0 (single-source op)
  - Row 4: LOGIC, op = A_AND_NOT_B, source-A = 0, source-B = 1
  - Row 5: POV1_UP, physical 2
  - Row 6: TAP, physical 3  (renamed from LONG_PRESS in firmware 0x0020)
  - Row 7: DOUBLE_TAP, physical 4
  - Row 8: NORMAL, physical 5, **inverted = true**, shift modifier = Shift 1
  - Row 9: NORMAL, physical 6, **disabled = true**
- **Shifts & Timers tab:**
  - Timer 1 = 250 ms, Timer 2 = 500 ms, Timer 3 = 1000 ms, Debounce = 30 ms
  - Tap cutoff = 600 ms, Double-tap window = 350 ms
  - Shift 1 source = physical 7
- **Axes (axis 0 and axis 1):**
  - Axis 0: calib_min = 100, calib_center = 2048, calib_max = 4000,
    function = LINEAR, deadband = 5, filter = level 2, resolution = 12, channel = 0
  - Axis 1: inverted = true, deadband = 20 with `is_dynamic_deadband = true`,
    source_secondary = 3, offset_angle = 7
- **Shift register 0:**
  - Type = HC165, chain length = 2 (so we see > 1 channel filled)
- **Encoder 0 (fast):**
  - Type = fast, mapped to the encoder pin pair set above
- **Encoder 1 (soft):**
  - Type = soft, source-A pin index = some pin, source-B pin index =
    another pin
- Other tabs: leave defaults. The codec round-trips them via raw bytes
  even though the v0.1 UI doesn't touch them.

#### Capture

After writing the wide-coverage config and verifying read-back:

```powershell
$env:FREEJOYX_DUMP_FIXTURE = "C:\path\to\FreeJoyXConfigurator\fixtures\wide_coverage"
Remove-Item -Recurse -Force C:\path\to\FreeJoyXConfigurator\fixtures\wide_coverage -ErrorAction SilentlyContinue
.\FreeJoyQt.exe
```

In the app: **Read from device**, wait for completion, **close**.

Then author `fixtures/wide_coverage/expected.ron` to match what the
Qt app displays for each tab. This is tedious but it's the oracle —
take the time.

### 3. Capture `params_stream/`

Re-use the wide-coverage device.

```powershell
$env:FREEJOYX_DUMP_FIXTURE = "C:\path\to\FreeJoyXConfigurator\fixtures\params_stream"
Remove-Item -Recurse -Force C:\path\to\FreeJoyXConfigurator\fixtures\params_stream -ErrorAction SilentlyContinue
.\FreeJoyQt.exe
```

In the Qt app (do NOT read config — the patch dumps the config too if
you do, which would pollute this fixture):

1. Wait 1 second.
2. Move the analog stick fully across each axis at least once.
3. Press and release each of buttons 0-9 once.
4. Rotate the encoder one full turn in each direction.
5. Trigger a shift register state change (close one of the chained
   button inputs).
6. Tap a TAP-configured button (release within the cutoff window) to trigger it.
7. Double-tap one of the DOUBLE_TAP-configured buttons.

Close the Qt app. `fixtures/params_stream/params.bin` should be ~6000
bytes (≈100 packets × 64 bytes assuming ~30 Hz rate). The `config.bin`
file should NOT exist in this directory — if it does, you accidentally
read the config; redo this step.

No `expected.ron` for the params stream — instead the test asserts:
(a) every 64-byte slice decodes without error; (b) ranges of decoded
axis values overlap the full int16 range; (c) at least one packet has
each button bit set; (d) at least one shift bit toggles between
packets.

## Known issues / open questions

### Qt `BUFFERSIZE=64` vs `FREEJOY_PARAMS_REPORT_SIZE=72`

The Qt app reads params packets into a 64-byte buffer, but
`params_report_t` is 72 bytes per `common_defines.h`. Possible
explanations:

1. The Qt app truncates each packet at byte 64 and silently loses
   the last 8 bytes (which include `shift_button_data` and the
   `freejoyx_version_*` triplet).
2. The device only sends 64 bytes, and the `_Static_assert` is a
   theoretical claim that doesn't match wire reality.
3. The 8-byte tail is read into the adjacent `deviceBuffer` on stack
   (which IS 64 bytes immediately after `buffer`) and consumed
   harmlessly on the next iteration.

`params_stream/params.bin` will resolve this. If recorded packets
show all-zero bytes past offset 64, explanation (1) is correct and
the Rust port needs to capture the full 72 bytes (a fix worth making
on the Rust side, with care). If packets reach offset 71 with
non-zero data, explanation (3) is correct and our captures are
already lossy — bump the patch's read size to 80 and recapture.

**Track this as a Slice 2 (params parser) investigation item.** It's
flagged in `SESSION_LOG.md` for the next session.

### `expected.ron` schema not yet finalized

Until Slice 1 lands the Rust domain types, the RON schema for the
oracle is hand-rolled. Once the Rust types exist, regenerate the
oracle files by running the Rust codec on the captured `config.bin`
and pretty-printing the resulting `DeviceConfig` with RON — then
hand-verify the output against the Qt app's display before committing
it as the canonical oracle.

## Quick reference

| Fixture | Source device | Capture trigger | Files |
|---|---|---|---|
| `minimal/` | Factory-reset board | "Read from device" once | `config.bin`, `config.fragments.bin`, `params.bin`, `expected.ron` |
| `wide_coverage/` | Hand-tuned board | "Read from device" once | same as above |
| `params_stream/` | Wide-coverage board | Exercise inputs for 3-5 seconds | `params.bin` only |
