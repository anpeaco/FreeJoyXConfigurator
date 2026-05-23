# Ported from

This repo is a Rust + Slint reimplementation of the Qt 5 configurator
at [`anpeaco/FreeJoyXConfiguratorQt`](https://github.com/anpeaco/FreeJoyXConfiguratorQt).

## Reference checkout pins

When this repo was bootstrapped (Slice 0), the upstream sibling repos
were at:

| Repo | Commit | Date | Note |
|---|---|---|---|
| `anpeaco/FreeJoyX` (firmware) | `81f773c` | 2026-05-16 | Source of `vendored/common_*.h` |
| `anpeaco/FreeJoyXConfiguratorQt` | `06aad1b` | 2026-05-16 | The oracle the Rust codec is verified against |
| DCSBoards (Slint style reference, per `Style.MD`) | `cf65876` | per `Style.MD` | Visual reference only |

When the firmware repo's `common_*.h` change, the
`.github/workflows/header-sync.yml` workflow fails CI; refresh the
`vendored/` copies and update the codec to match.

When the Qt app changes in ways that affect the oracle (e.g. a bug fix
to how it interprets a field), update the fixture `expected.ron`
files; the bytes stay the same, only the interpretation changes.

## What was preserved

- Wire format: byte-identical (`FIRMWARE_VERSION = 0x0020`).
- USB HID protocol: same report IDs, same fragment structure
  (26 × 62-byte payload).
- Default config: matches the Qt app's `InitConfig()` output.

## What was changed

- Toolchain: Qt 5 / C++17 → Rust stable + Slint 1.13.
- UI aesthetic: Qt widget tree → DCSBoards-derived dark cockpit
  palette (see `Style.MD`).
- Config-on-disk format: INI / QSettings → RON.
- Legacy migration: not supported in Rust — the app refuses unknown
  firmware-version mask groups and points users at the Qt app.
- See Port.md §7 for the full list of dropped surfaces.
