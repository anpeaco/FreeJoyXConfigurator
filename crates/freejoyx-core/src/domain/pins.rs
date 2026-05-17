//! Pin function enum + per-board pin name tables + conflict validator.
//!
//! ## Mapping between `pins[30]` and the physical chip pin
//!
//! The wire format stores pin functions as an array of 30 `int8_t`
//! values (`pin_t`). The array index 0..29 corresponds to a fixed
//! physical-pin slot per `FreeJoyXConfiguratorQt/src/global.h::enum
//! Pin` (PA0..PA10, PA15, PB0, PB1, PB3..PB15, PC13..PC15 — 12 PA + 15
//! PB + 3 PC = 30 slots).
//!
//! On the F411 BlackPill, slot 22 (the internal "B11") is silkscreened
//! "B2" because PB11 isn't bonded on the UFQFPN48 package; the
//! [`Board::pin_name`] lookup folds that override in.
//!
//! ## Pin function values
//!
//! [`PinFunction`] mirrors the `pin_t` enum in
//! `vendored/common_types.h`. Numeric values are stable on the wire
//! and must round-trip exactly via `as i8` / `from_i8`.
//!
//! ## Validation
//!
//! [`validate_pins`] flags "more than one pin claims this role" for
//! singleton functions (SPI master signals, I2C, UART, TLE5011 clock
//! generator). Board-specific timer-conflict rules (e.g. PA8 PWM vs
//! PA10 RGB on F103) are out of scope here; they belong in
//! Slice 6+ when the relevant tabs land. The Slice 5 done-when only
//! calls for "clashes lit up inline", which the singleton check
//! covers.

use std::fmt;

/// `pin_t` from `vendored/common_types.h`. Numeric values must match
/// the C enum exactly — round-trips via [`PinFunction::from_i8`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i8)]
pub enum PinFunction {
    NotUsed = 0,
    ButtonGnd = 1,
    ButtonVcc = 2,
    ButtonRow = 3,
    ButtonColumn = 4,
    AxisAnalog = 5,
    FastEncoder = 6,
    SpiSck = 7,
    SpiMosi = 8,
    SpiMiso = 9,
    Tle5011Gen = 10,
    Tle5011Cs = 11,
    Tle5012Cs = 12,
    Mcp3201Cs = 13,
    Mcp3202Cs = 14,
    Mcp3204Cs = 15,
    Mcp3208Cs = 16,
    Mlx90393Cs = 17,
    As5048aCs = 18,
    ShiftRegLatch = 19,
    ShiftRegData = 20,
    LedPwm = 21,
    LedSingle = 22,
    LedRow = 23,
    LedColumn = 24,
    I2cScl = 25,
    I2cSda = 26,
    Mlx90363Cs = 27,
    ShiftRegClk = 28,
    LedRgbWs2812b = 29,
    LedRgbPl9823 = 30,
    UartTx = 31,
}

impl PinFunction {
    /// Decode a stored `i8` slot value. Unknown values return `None` so
    /// the caller can decide whether to flag or quietly map to
    /// `NotUsed`.
    #[must_use]
    pub fn from_i8(v: i8) -> Option<Self> {
        Some(match v {
            0 => Self::NotUsed,
            1 => Self::ButtonGnd,
            2 => Self::ButtonVcc,
            3 => Self::ButtonRow,
            4 => Self::ButtonColumn,
            5 => Self::AxisAnalog,
            6 => Self::FastEncoder,
            7 => Self::SpiSck,
            8 => Self::SpiMosi,
            9 => Self::SpiMiso,
            10 => Self::Tle5011Gen,
            11 => Self::Tle5011Cs,
            12 => Self::Tle5012Cs,
            13 => Self::Mcp3201Cs,
            14 => Self::Mcp3202Cs,
            15 => Self::Mcp3204Cs,
            16 => Self::Mcp3208Cs,
            17 => Self::Mlx90393Cs,
            18 => Self::As5048aCs,
            19 => Self::ShiftRegLatch,
            20 => Self::ShiftRegData,
            21 => Self::LedPwm,
            22 => Self::LedSingle,
            23 => Self::LedRow,
            24 => Self::LedColumn,
            25 => Self::I2cScl,
            26 => Self::I2cSda,
            27 => Self::Mlx90363Cs,
            28 => Self::ShiftRegClk,
            29 => Self::LedRgbWs2812b,
            30 => Self::LedRgbPl9823,
            31 => Self::UartTx,
            _ => return None,
        })
    }

    /// Encode back to wire format.
    #[must_use]
    pub fn to_i8(self) -> i8 {
        self as i8
    }

    /// Short label for the function dropdown (matches the Qt UI's
    /// pin-type picker rows).
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::NotUsed => "Not used",
            Self::ButtonGnd => "Button GND",
            Self::ButtonVcc => "Button VCC",
            Self::ButtonRow => "Button row",
            Self::ButtonColumn => "Button column",
            Self::AxisAnalog => "Axis (analog)",
            Self::FastEncoder => "Fast encoder",
            Self::SpiSck => "SPI SCK",
            Self::SpiMosi => "SPI MOSI",
            Self::SpiMiso => "SPI MISO",
            Self::Tle5011Gen => "TLE5011 GEN",
            Self::Tle5011Cs => "TLE5011 CS",
            Self::Tle5012Cs => "TLE5012 CS",
            Self::Mcp3201Cs => "MCP3201 CS",
            Self::Mcp3202Cs => "MCP3202 CS",
            Self::Mcp3204Cs => "MCP3204 CS",
            Self::Mcp3208Cs => "MCP3208 CS",
            Self::Mlx90393Cs => "MLX90393 CS",
            Self::As5048aCs => "AS5048A CS",
            Self::ShiftRegLatch => "Shift reg latch",
            Self::ShiftRegData => "Shift reg data",
            Self::LedPwm => "LED PWM",
            Self::LedSingle => "LED single",
            Self::LedRow => "LED row",
            Self::LedColumn => "LED column",
            Self::I2cScl => "I2C SCL",
            Self::I2cSda => "I2C SDA",
            Self::Mlx90363Cs => "MLX90363 CS",
            Self::ShiftRegClk => "Shift reg clock",
            Self::LedRgbWs2812b => "RGB LED (WS2812B)",
            Self::LedRgbPl9823 => "RGB LED (PL9823)",
            Self::UartTx => "UART TX",
        }
    }

    /// Iterate every function (for combobox population).
    pub fn all() -> impl Iterator<Item = Self> {
        const ALL: [PinFunction; 32] = [
            PinFunction::NotUsed,
            PinFunction::ButtonGnd,
            PinFunction::ButtonVcc,
            PinFunction::ButtonRow,
            PinFunction::ButtonColumn,
            PinFunction::AxisAnalog,
            PinFunction::FastEncoder,
            PinFunction::SpiSck,
            PinFunction::SpiMosi,
            PinFunction::SpiMiso,
            PinFunction::Tle5011Gen,
            PinFunction::Tle5011Cs,
            PinFunction::Tle5012Cs,
            PinFunction::Mcp3201Cs,
            PinFunction::Mcp3202Cs,
            PinFunction::Mcp3204Cs,
            PinFunction::Mcp3208Cs,
            PinFunction::Mlx90393Cs,
            PinFunction::As5048aCs,
            PinFunction::ShiftRegLatch,
            PinFunction::ShiftRegData,
            PinFunction::LedPwm,
            PinFunction::LedSingle,
            PinFunction::LedRow,
            PinFunction::LedColumn,
            PinFunction::I2cScl,
            PinFunction::I2cSda,
            PinFunction::Mlx90363Cs,
            PinFunction::ShiftRegClk,
            PinFunction::LedRgbWs2812b,
            PinFunction::LedRgbPl9823,
            PinFunction::UartTx,
        ];
        ALL.into_iter()
    }

    /// Visual family the function belongs to. The UI uses this to pick
    /// a Lucide icon next to the function label so an at-a-glance scan
    /// of the pin list shows the functional zoning. Stable integers
    /// across the wire so the Slint side can drive an `@image-url`
    /// chain off them.
    #[must_use]
    pub fn family(self) -> PinFunctionFamily {
        use PinFunctionFamily::{Axis, Bus, Button, Encoder, Led, NotUsed, RgbLed, Sensor, ShiftReg};
        match self {
            Self::NotUsed => NotUsed,
            Self::ButtonGnd
            | Self::ButtonVcc
            | Self::ButtonRow
            | Self::ButtonColumn => Button,
            Self::AxisAnalog => Axis,
            Self::FastEncoder => Encoder,
            Self::SpiSck
            | Self::SpiMosi
            | Self::SpiMiso
            | Self::I2cScl
            | Self::I2cSda
            | Self::UartTx => Bus,
            Self::Tle5011Gen
            | Self::Tle5011Cs
            | Self::Tle5012Cs
            | Self::Mcp3201Cs
            | Self::Mcp3202Cs
            | Self::Mcp3204Cs
            | Self::Mcp3208Cs
            | Self::Mlx90393Cs
            | Self::As5048aCs
            | Self::Mlx90363Cs => Sensor,
            Self::ShiftRegLatch | Self::ShiftRegData | Self::ShiftRegClk => ShiftReg,
            Self::LedPwm | Self::LedSingle | Self::LedRow | Self::LedColumn => Led,
            Self::LedRgbWs2812b | Self::LedRgbPl9823 => RgbLed,
        }
    }

    /// `true` if the firmware allows only one pin in the array to
    /// carry this role. Validation flags any pin row that picks a
    /// singleton already taken by another row.
    #[must_use]
    pub fn is_singleton(self) -> bool {
        matches!(
            self,
            Self::SpiSck
                | Self::SpiMosi
                | Self::SpiMiso
                | Self::I2cScl
                | Self::I2cSda
                | Self::UartTx
                | Self::Tle5011Gen
        )
    }
}

impl fmt::Display for PinFunction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Visual grouping the UI uses to pick a per-function icon. Integer
/// values are stable so Slint's `@image-url` chain can switch off the
/// wire byte directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum PinFunctionFamily {
    NotUsed = 0,
    Button = 1,
    Axis = 2,
    Encoder = 3,
    /// SPI / I2C / UART signal pins.
    Bus = 4,
    /// External-sensor CS / GEN pins (TLE, MLX, MCP, AS5048).
    Sensor = 5,
    ShiftReg = 6,
    /// Single / PWM / row / column LED pins.
    Led = 7,
    /// Addressable RGB LED chain pins (WS2812B / PL9823).
    RgbLed = 8,
}

impl PinFunctionFamily {
    #[must_use]
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    /// Short label suitable for a compact pin chip on the board view
    /// (max ~5 chars). `NotUsed` returns an empty string so the chip
    /// shows only the silkscreen label.
    #[must_use]
    pub fn short_label(self) -> &'static str {
        match self {
            Self::NotUsed => "",
            Self::Button => "Btn",
            Self::Axis => "Axis",
            Self::Encoder => "Enc",
            Self::Bus => "Bus",
            Self::Sensor => "Snr",
            Self::ShiftReg => "SReg",
            Self::Led => "LED",
            Self::RgbLed => "RGB",
        }
    }
}

/// Boards the configurator knows pin layouts for. Other `board_id`
/// values fall back to [`Board::Bluepill`]'s naming scheme since the
/// internal pin identifiers are anchored on the F103 BluePill.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Board {
    Bluepill,
    Blackpill,
}

impl Board {
    /// Resolve a board id from `params_report_t.board_id`. Per
    /// `common_defines.h`:
    /// - `1 = BOARD_ID_F103_BLUEPILL`
    /// - `2 = BOARD_ID_F411_BLACKPILL`
    /// - anything else: `0 = unset / legacy`, treat as BluePill (the
    ///   historical default before the self-tag landed).
    #[must_use]
    pub fn from_id(id: u8) -> Self {
        match id {
            2 => Self::Blackpill,
            _ => Self::Bluepill,
        }
    }

    /// Total number of pin slots — always [`USED_PINS_NUM`]. Both
    /// boards have the same 30-slot wire format; only labels differ.
    #[must_use]
    pub fn pin_count(self) -> usize {
        USED_PINS_NUM
    }

    /// Display name for the pin at `slot` (0..30). Out-of-range
    /// returns `"?"`.
    #[must_use]
    pub fn pin_name(self, slot: usize) -> &'static str {
        if slot >= USED_PINS_NUM {
            return "?";
        }
        let base = BLUEPILL_PIN_NAMES[slot];
        if matches!(self, Self::Blackpill) {
            // F411 BlackPill V3.x silk: PB11 slot is labelled B2 (PB2
            // bonded instead). Matches `pinboardnames.h:30`.
            if base == "PB11" {
                return "PB2";
            }
        }
        base
    }
}

/// Number of pin slots in `dev_config_t.pins[]`. Mirrors
/// `USED_PINS_NUM` in `vendored/common_defines.h`.
pub const USED_PINS_NUM: usize = 30;

/// Internal pin names per slot, anchored on the F103 BluePill silkscreen
/// (see `FreeJoyXConfiguratorQt/src/global.h::enum Pin`). Other boards
/// fold per-pin renames over this via [`Board::pin_name`].
pub const BLUEPILL_PIN_NAMES: [&str; USED_PINS_NUM] = [
    "PA0", "PA1", "PA2", "PA3", "PA4", "PA5", "PA6", "PA7", "PA8", "PA9", "PA10", "PA15", //
    "PB0", "PB1", "PB3", "PB4", "PB5", "PB6", "PB7", "PB8", "PB9", "PB10", "PB11", "PB12", "PB13",
    "PB14", "PB15", //
    "PC13", "PC14", "PC15",
];

/// Why a pin slot is in conflict with the rest of the configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PinConflictKind {
    /// A singleton role (SPI, I2C, UART, TLE5011_GEN) is assigned to
    /// more than one pin. Carries the offending function so the UI can
    /// say "two pins picked SPI SCK".
    DuplicateSingleton(PinFunction),
    /// A stored wire value didn't match any known [`PinFunction`].
    UnknownFunction(i8),
}

impl PinConflictKind {
    /// Short label suitable for an inline UI badge.
    #[must_use]
    pub fn short_label(&self) -> String {
        match self {
            Self::DuplicateSingleton(f) => format!("duplicate {f}"),
            Self::UnknownFunction(v) => format!("unknown function 0x{:02x}", *v as u8),
        }
    }
}

/// One pin slot's complaint after running [`validate_pins`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinConflict {
    pub slot: usize,
    pub kind: PinConflictKind,
}

/// Walk the 30 pin slots and flag the rows that conflict.
///
/// Slice 5 scope: duplicate-singleton + unknown-function only.
/// Board-specific timer / silicon-conflict rules (PA8 PWM blocks PA10
/// RGB, PB6/7 FAST_ENCODER vs PB6 TLE5011_GEN on F103) belong in the
/// later slices that surface those toggles in the UI.
#[must_use]
pub fn validate_pins(pins: &[i8; USED_PINS_NUM]) -> Vec<PinConflict> {
    let mut out = Vec::new();

    // First pass: decode each slot, flag unknowns.
    let mut decoded: [Option<PinFunction>; USED_PINS_NUM] = [None; USED_PINS_NUM];
    for (i, &raw) in pins.iter().enumerate() {
        match PinFunction::from_i8(raw) {
            Some(f) => decoded[i] = Some(f),
            None => out.push(PinConflict {
                slot: i,
                kind: PinConflictKind::UnknownFunction(raw),
            }),
        }
    }

    // Second pass: for every singleton function, find each slot that
    // claims it; if there's more than one claimant, every claimant is
    // in conflict.
    for func in PinFunction::all().filter(|f| f.is_singleton()) {
        let slots: Vec<usize> = decoded
            .iter()
            .enumerate()
            .filter_map(|(i, opt)| if *opt == Some(func) { Some(i) } else { None })
            .collect();
        if slots.len() > 1 {
            for s in slots {
                out.push(PinConflict {
                    slot: s,
                    kind: PinConflictKind::DuplicateSingleton(func),
                });
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_every_function() {
        for f in PinFunction::all() {
            assert_eq!(PinFunction::from_i8(f.to_i8()), Some(f));
        }
    }

    #[test]
    fn from_i8_rejects_unknown() {
        assert_eq!(PinFunction::from_i8(99), None);
        assert_eq!(PinFunction::from_i8(-1), None);
    }

    #[test]
    fn no_conflict_on_zero_pins() {
        let pins = [0i8; USED_PINS_NUM];
        assert!(validate_pins(&pins).is_empty());
    }

    #[test]
    fn flags_two_spi_sck_pins() {
        let mut pins = [0i8; USED_PINS_NUM];
        pins[5] = PinFunction::SpiSck.to_i8();
        pins[12] = PinFunction::SpiSck.to_i8();
        let conflicts = validate_pins(&pins);
        assert_eq!(conflicts.len(), 2);
        assert!(conflicts.iter().any(|c| c.slot == 5));
        assert!(conflicts.iter().any(|c| c.slot == 12));
        assert!(matches!(
            conflicts[0].kind,
            PinConflictKind::DuplicateSingleton(PinFunction::SpiSck)
        ));
    }

    #[test]
    fn allows_many_button_gnd_pins() {
        // BUTTON_GND is not a singleton — a row of pins all GND is
        // exactly how a typical matrix wires up.
        let mut pins = [0i8; USED_PINS_NUM];
        for slot in &mut pins[..5] {
            *slot = PinFunction::ButtonGnd.to_i8();
        }
        assert!(validate_pins(&pins).is_empty());
    }

    #[test]
    fn flags_unknown_function_value() {
        let mut pins = [0i8; USED_PINS_NUM];
        pins[3] = 99;
        let conflicts = validate_pins(&pins);
        assert_eq!(conflicts.len(), 1);
        assert_eq!(conflicts[0].slot, 3);
        assert!(matches!(
            conflicts[0].kind,
            PinConflictKind::UnknownFunction(99)
        ));
    }

    #[test]
    fn bluepill_pin_names_have_expected_layout() {
        assert_eq!(Board::Bluepill.pin_name(0), "PA0");
        assert_eq!(Board::Bluepill.pin_name(11), "PA15");
        assert_eq!(Board::Bluepill.pin_name(12), "PB0");
        assert_eq!(Board::Bluepill.pin_name(13), "PB1");
        assert_eq!(Board::Bluepill.pin_name(14), "PB3"); // PB2 skipped on BluePill
        assert_eq!(Board::Bluepill.pin_name(22), "PB11");
        assert_eq!(Board::Bluepill.pin_name(27), "PC13");
        assert_eq!(Board::Bluepill.pin_name(29), "PC15");
    }

    #[test]
    fn every_function_has_a_family() {
        // Exhaustive — `family()` matches on all 32 variants, so a new
        // function added without a family arm fails to compile.
        for f in PinFunction::all() {
            let _fam = f.family();
        }
    }

    #[test]
    fn family_groups_expected_functions() {
        use PinFunctionFamily::*;
        assert_eq!(PinFunction::NotUsed.family(), NotUsed);
        assert_eq!(PinFunction::ButtonGnd.family(), Button);
        assert_eq!(PinFunction::ButtonColumn.family(), Button);
        assert_eq!(PinFunction::AxisAnalog.family(), Axis);
        assert_eq!(PinFunction::FastEncoder.family(), Encoder);
        assert_eq!(PinFunction::SpiSck.family(), Bus);
        assert_eq!(PinFunction::I2cScl.family(), Bus);
        assert_eq!(PinFunction::UartTx.family(), Bus);
        assert_eq!(PinFunction::Tle5011Gen.family(), Sensor);
        assert_eq!(PinFunction::Mlx90393Cs.family(), Sensor);
        assert_eq!(PinFunction::ShiftRegClk.family(), ShiftReg);
        assert_eq!(PinFunction::LedSingle.family(), Led);
        assert_eq!(PinFunction::LedPwm.family(), Led);
        assert_eq!(PinFunction::LedRgbWs2812b.family(), RgbLed);
        assert_eq!(PinFunction::LedRgbPl9823.family(), RgbLed);
    }

    #[test]
    fn blackpill_renames_pb11_to_pb2() {
        assert_eq!(Board::Blackpill.pin_name(22), "PB2");
        // Other slots stay anchored on the BluePill naming.
        assert_eq!(Board::Blackpill.pin_name(0), "PA0");
        assert_eq!(Board::Blackpill.pin_name(14), "PB3");
    }
}
