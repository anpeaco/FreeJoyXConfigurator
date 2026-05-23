# FreeJoyXConfigurator v0.1.0

Initial release of the native Rust + Slint configurator for FreeJoyX
firmware. Covers the surfaces the maintainer actually uses on their
build; legacy upstream FreeJoy firmware (0x17XX mask group) stays with
the Qt configurator.

## Highlights

- **Full 1580-byte `dev_config_t` round-trip.** Every byte the device
  hands the configurator comes back unchanged on write. Bytes for
  deferred surfaces (LED tabs, RGB, sensors, axis curves) are preserved
  faithfully even though they have no UI yet.
- **Seven tabs**: Pins, Axes (linear + deadband + filter), Buttons +
  Logic, Shifts & Timers, Encoders, Shift Registers, Advanced Settings.
- **Live state overlay** on the Axes and Buttons tabs — raw + processed
  axis values, per-button live dots.
- **On-disk RON save / load** with byte-identical
  `wire → domain → RON → domain → wire`.
- **Multi-board picker** in the toolbar (the worker enumerates every
  FreeJoy / FreeJoyX HID candidate).
- **Unsupported-firmware gate**: an explicit toast points users at the
  Qt configurator for legacy boards on the 0x17XX line. Read / Write
  are disabled while a known-incompatible board is connected.
- **Rolling log file** via `tracing-appender` in an OS-appropriate
  user-state directory; "Open Log Folder" in the toolbar's Help menu
  opens it in the system file manager.
- **About dialog** with version, build revision, supported firmware
  mask group.

## Supported wire format

- `FIRMWARE_VERSION` mask group **0x0020** (FreeJoyX gen 2 — LONG_PRESS
  → TAP rename; `dev_config_t` byte layout unchanged from gen 1).
- Versions in the same mask group (0x002F build-nibble drift) are
  accepted.
- Anything outside that group (legacy 0x17XX, previous gen-1 0x0010,
  future 0x0030+) is refused with a clear toast.

## Platforms

Built on the Slint femtovg renderer + winit backend. Verified to build
on Linux, macOS, and Windows via CI (`ubuntu-latest`,
`windows-latest`, `macos-latest`).

Distributables:
- Windows / macOS — `cargo bundle --release -p freejoyx-app` produces
  `.msi` / `.dmg`.
- Linux — `./packaging/build-appimage.sh` produces an x86_64
  AppImage. Requires `libudev` / `libusb-1.0` at build time and
  `appimagetool` on `$PATH`.

## What's not in v0.1

Deferred to v0.1.1+:

- Single/PWM LED tab (24 channels)
- RGB LED tab (50 LEDs, color wheel + effect editor)
- Sensor configuration (TLE5011, MLX90393, AS5048A, AS5600)
- 11-point axis curve editor

Permanently out of scope:

- Legacy upstream FreeJoy firmware support (use the Qt app)
- Localization (English-only)
- Firmware flashing

## Known gaps

- **No bench-verified write-back roundtrip across every editable
  field** yet. Codec is exercised end-to-end against real hardware on
  the Pins and Axes tabs; the rest of the surface relies on the
  fixture-driven codec tests + on-disk RON cross-trip until the
  bench loop closes.
- **Picker doesn't show a transitional state during Reopen.** When you
  pick a different device the toolbar briefly shows the prior device
  name until the worker emits `Disconnected` → `Connected` for the
  new one.

## License

GPLv3 (forced by upstream FreeJoy + FreeJoyConfiguratorQt, both
GPLv3; Slint's royalty-free GPLv3 tier is the only license-compatible
option). Source at
<https://github.com/anpeaco/FreeJoyXConfigurator>.
