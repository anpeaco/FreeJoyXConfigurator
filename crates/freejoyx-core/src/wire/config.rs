//! `dev_config_t` wire codec.
//!
//! Layout reference: `vendored/common_types.h` `dev_config_t` (1580 bytes).
//! Constants: `vendored/common_defines.h` (FREEJOY_DEV_CONFIG_SIZE,
//! MAX_AXIS_NUM = 8, MAX_BUTTONS_NUM = 128, USED_PINS_NUM = 30, etc.).
//!
//! Per Port.md §3 ("Codec strategy — Path B"), this is hand-written: every
//! multi-byte int is `from_le_bytes`/`to_le_bytes`, every bitfield is
//! explicit mask + shift. There is no mirror struct.
//!
//! Bitfield convention used throughout: GCC packs `uintN_t` bitfields
//! LSB-first within their storage unit (so the first-declared field
//! occupies the lowest bits). The firmware is built with arm-none-eabi-gcc
//! and runs on a little-endian Cortex-M, so this is the canonical layout.
//!
//! Round-trip parity is the load-bearing claim: `DeviceConfig::decode →
//! encode` is byte-identical for every fixture. The integration test
//! `tests/codec_config.rs` proves this against the captured device bytes.

use serde::{Deserialize, Serialize};
use serde_big_array::BigArray;

use super::cursor::{Cursor, Writer};
use super::error::DecodeError;

/// `FREEJOY_DEV_CONFIG_SIZE` from `vendored/common_defines.h`.
pub const DEV_CONFIG_SIZE: usize = 1580;

/// `MAX_AXIS_NUM` from `vendored/common_defines.h`.
pub const MAX_AXIS_NUM: usize = 8;
/// `MAX_BUTTONS_NUM` from `vendored/common_defines.h`.
pub const MAX_BUTTONS_NUM: usize = 128;
/// `USED_PINS_NUM` from `vendored/common_defines.h`.
pub const USED_PINS_NUM: usize = 30;
/// `MAX_ENCODERS_NUM` from `vendored/common_defines.h`.
pub const MAX_ENCODERS_NUM: usize = 16;
/// `MAX_FAST_ENCODER_NUM` from `vendored/common_defines.h`.
pub const MAX_FAST_ENCODER_NUM: usize = 2;
/// `MAX_SHIFT_REG_NUM` from `vendored/common_defines.h`.
pub const MAX_SHIFT_REG_NUM: usize = 4;
/// `MAX_SHIFTS_NUM` from `vendored/common_defines.h`.
pub const MAX_SHIFTS_NUM: usize = 8;
/// `MAX_LEDS_NUM` from `vendored/common_defines.h`.
pub const MAX_LEDS_NUM: usize = 24;
/// `NUM_RGB_LEDS` from `vendored/common_defines.h`.
pub const NUM_RGB_LEDS: usize = 50;

/// `led_pwm_config_t` size (the `:0` aligner forces 2-byte size).
const LED_PWM_CONFIG_SIZE: usize = 2;
/// `led_config_t` size (the `:0` aligner forces 2-byte size).
const LED_CONFIG_SIZE: usize = 2;
/// `argb_led_t` size: rgb_t (3) + input_num (1) + bitfield byte (1).
const ARGB_LED_SIZE: usize = 5;

// =============================================================================
// Sub-struct types (idiomatic where v0.1 UI surfaces them; raw bytes for
// deferred surfaces per Port.md §1.1).
// =============================================================================

/// One slot of `axis_config_t`.
///
/// Calibration, filter, deadband, and curve-shape are surfaced by the
/// v0.1 Axes tab. Sensor-related fields (`source_main`,
/// `source_secondary`, `offset_angle`, `i2c_address`, `divider`) and the
/// per-axis button hooks round-trip but are not edited by v0.1; they are
/// kept as fields so the configurator preserves whatever the device or
/// the Qt app set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxisConfig {
    pub calib_min: i16,
    pub calib_center: i16,
    pub calib_max: i16,
    /// Bitfield byte 1: bit 0 out_enabled, bit 1 inverted, bit 2 is_centered,
    /// bits 3..=4 function (2 bits), bits 5..=7 filter (3 bits).
    /// Stored raw so unused-bit drift round-trips even if the device
    /// shipped non-zero garbage there.
    pub flags1: u8,
    pub curve_shape: [i8; 11],
    /// Bitfield byte 2: bits 0..=3 resolution, bits 4..=7 channel.
    pub flags2: u8,
    /// Bitfield byte 3: bits 0..=6 deadband_size, bit 7 is_dynamic_deadband.
    pub flags3: u8,
    pub source_main: i8,
    /// Bitfield byte 4: bits 0..=2 source_secondary, bits 3..=7 offset_angle.
    pub flags4: u8,
    pub button1: i8,
    pub button2: i8,
    pub button3: i8,
    pub divider: u8,
    pub i2c_address: u8,
    /// Bitfield byte 5: bits 0..=2 button1_type, bits 3..=4 button2_type,
    /// bits 5..=7 button3_type.
    pub flags5: u8,
    pub prescaler: u8,
    pub reserved: u8,
}

impl AxisConfig {
    fn decode(cur: &mut Cursor) -> Result<Self, DecodeError> {
        let calib_min = cur.read_i16_le()?;
        let calib_center = cur.read_i16_le()?;
        let calib_max = cur.read_i16_le()?;
        let flags1 = cur.read_u8()?;
        let curve_shape = read_i8_array::<11>(cur)?;
        let flags2 = cur.read_u8()?;
        let flags3 = cur.read_u8()?;
        let source_main = cur.read_i8()?;
        let flags4 = cur.read_u8()?;
        let button1 = cur.read_i8()?;
        let button2 = cur.read_i8()?;
        let button3 = cur.read_i8()?;
        let divider = cur.read_u8()?;
        let i2c_address = cur.read_u8()?;
        let flags5 = cur.read_u8()?;
        let prescaler = cur.read_u8()?;
        let reserved = cur.read_u8()?;
        Ok(Self {
            calib_min,
            calib_center,
            calib_max,
            flags1,
            curve_shape,
            flags2,
            flags3,
            source_main,
            flags4,
            button1,
            button2,
            button3,
            divider,
            i2c_address,
            flags5,
            prescaler,
            reserved,
        })
    }

    fn encode(&self, w: &mut Writer) {
        w.write_i16_le(self.calib_min);
        w.write_i16_le(self.calib_center);
        w.write_i16_le(self.calib_max);
        w.write_u8(self.flags1);
        write_i8_array(w, &self.curve_shape);
        w.write_u8(self.flags2);
        w.write_u8(self.flags3);
        w.write_i8(self.source_main);
        w.write_u8(self.flags4);
        w.write_i8(self.button1);
        w.write_i8(self.button2);
        w.write_i8(self.button3);
        w.write_u8(self.divider);
        w.write_u8(self.i2c_address);
        w.write_u8(self.flags5);
        w.write_u8(self.prescaler);
        w.write_u8(self.reserved);
    }

    // -- accessors over flags1 --
    #[must_use]
    pub fn out_enabled(&self) -> bool {
        self.flags1 & 0x01 != 0
    }
    #[must_use]
    pub fn inverted(&self) -> bool {
        self.flags1 & 0x02 != 0
    }
    #[must_use]
    pub fn is_centered(&self) -> bool {
        self.flags1 & 0x04 != 0
    }
    /// `function` 2-bit field (NO_FUNCTION..FUNCTION_EQUAL).
    #[must_use]
    pub fn function(&self) -> u8 {
        (self.flags1 >> 3) & 0x03
    }
    /// `filter` 3-bit field (FILTER_NO..FILTER_LEVEL_7).
    #[must_use]
    pub fn filter(&self) -> u8 {
        (self.flags1 >> 5) & 0x07
    }

    // -- accessors over flags2 --
    #[must_use]
    pub fn resolution(&self) -> u8 {
        self.flags2 & 0x0f
    }
    #[must_use]
    pub fn channel(&self) -> u8 {
        (self.flags2 >> 4) & 0x0f
    }

    // -- accessors over flags3 --
    #[must_use]
    pub fn deadband_size(&self) -> u8 {
        self.flags3 & 0x7f
    }
    #[must_use]
    pub fn is_dynamic_deadband(&self) -> bool {
        self.flags3 & 0x80 != 0
    }
}

/// One slot of `button_t`.
///
/// Six wire bytes per button (the `shift_modificator :4` widening pushed
/// `op` into a new storage byte; see `vendored/common_types.h::button_t`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Button {
    /// When `button_type == LOGIC`, carries Source A.
    pub physical_num: i8,
    pub button_type: u8,
    /// Source B for LOGIC; `-1` / unused otherwise.
    pub src_b: i8,
    /// Bitfield byte A: bits 0..=3 shift_modificator (0=none, 1..8 = shift slot),
    /// bit 4 is_inverted, bit 5 is_disabled. Bits 6..=7 unused.
    pub flags_a: u8,
    /// Bitfield byte B: bits 0..=2 op (logic operator). Bits 3..=7 unused.
    pub flags_b: u8,
    /// Bitfield byte C: bits 0..=2 delay_timer, bits 3..=5 press_timer.
    /// Bits 6..=7 unused.
    pub flags_c: u8,
}

impl Button {
    fn decode(cur: &mut Cursor) -> Result<Self, DecodeError> {
        let physical_num = cur.read_i8()?;
        let button_type = cur.read_u8()?;
        let src_b = cur.read_i8()?;
        let flags_a = cur.read_u8()?;
        let flags_b = cur.read_u8()?;
        let flags_c = cur.read_u8()?;
        Ok(Self {
            physical_num,
            button_type,
            src_b,
            flags_a,
            flags_b,
            flags_c,
        })
    }

    fn encode(&self, w: &mut Writer) {
        w.write_i8(self.physical_num);
        w.write_u8(self.button_type);
        w.write_i8(self.src_b);
        w.write_u8(self.flags_a);
        w.write_u8(self.flags_b);
        w.write_u8(self.flags_c);
    }

    // -- accessors over flags_a --
    /// `shift_modificator` 4-bit field. 0 = none, 1..=8 = shift slot.
    #[must_use]
    pub fn shift_modificator(&self) -> u8 {
        self.flags_a & 0x0f
    }
    #[must_use]
    pub fn is_inverted(&self) -> bool {
        self.flags_a & 0x10 != 0
    }
    #[must_use]
    pub fn is_disabled(&self) -> bool {
        self.flags_a & 0x20 != 0
    }

    // -- accessor over flags_b --
    /// `op` 3-bit logic-operator field (only meaningful when type == LOGIC).
    #[must_use]
    pub fn op(&self) -> u8 {
        self.flags_b & 0x07
    }

    // -- accessors over flags_c --
    /// `delay_timer` 3-bit field. 0=OFF, 1..=3 = TIMER_1..TIMER_3.
    /// Also serves as the debounce-timer picker when type == LOGIC.
    #[must_use]
    pub fn delay_timer(&self) -> u8 {
        self.flags_c & 0x07
    }
    #[must_use]
    pub fn press_timer(&self) -> u8 {
        (self.flags_c >> 3) & 0x07
    }
}

/// One slot of `axis_to_buttons_t`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxisToButtons {
    pub points: [u8; 13],
    pub buttons_cnt: u8,
}

impl AxisToButtons {
    fn decode(cur: &mut Cursor) -> Result<Self, DecodeError> {
        let points = cur.read_array::<13>()?;
        let buttons_cnt = cur.read_u8()?;
        Ok(Self {
            points,
            buttons_cnt,
        })
    }

    fn encode(&self, w: &mut Writer) {
        w.write_array(&self.points);
        w.write_u8(self.buttons_cnt);
    }
}

/// One slot of `shift_reg_config_t`. Note this is the *config* struct in
/// `dev_config_t` (4 bytes), not the runtime `shift_reg_t` (which is 6
/// bytes and not part of the wire format).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShiftRegConfig {
    pub reg_type: u8,
    pub button_cnt: u8,
    pub reserved: [i8; 2],
}

impl ShiftRegConfig {
    fn decode(cur: &mut Cursor) -> Result<Self, DecodeError> {
        let reg_type = cur.read_u8()?;
        let button_cnt = cur.read_u8()?;
        let reserved = read_i8_array::<2>(cur)?;
        Ok(Self {
            reg_type,
            button_cnt,
            reserved,
        })
    }

    fn encode(&self, w: &mut Writer) {
        w.write_u8(self.reg_type);
        w.write_u8(self.button_cnt);
        write_i8_array(w, &self.reserved);
    }
}

/// One slot of `fast_encoder_t`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FastEncoder {
    pub enabled: u8,
    pub mode: u8,
}

impl FastEncoder {
    fn decode(cur: &mut Cursor) -> Result<Self, DecodeError> {
        let enabled = cur.read_u8()?;
        let mode = cur.read_u8()?;
        Ok(Self { enabled, mode })
    }

    fn encode(&self, w: &mut Writer) {
        w.write_u8(self.enabled);
        w.write_u8(self.mode);
    }
}

/// Snapshot of how button slots were divided across physical categories
/// at the last serialise. Configurator-only metadata per
/// `vendored/common_types.h::phys_breakdown_t`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysBreakdown {
    pub matrix: u8,
    pub per_sr: [u8; MAX_SHIFT_REG_NUM],
    pub per_a2b: [u8; MAX_AXIS_NUM],
    pub direct: u8,
}

impl PhysBreakdown {
    fn decode(cur: &mut Cursor) -> Result<Self, DecodeError> {
        let matrix = cur.read_u8()?;
        let per_sr = cur.read_array::<MAX_SHIFT_REG_NUM>()?;
        let per_a2b = cur.read_array::<MAX_AXIS_NUM>()?;
        let direct = cur.read_u8()?;
        Ok(Self {
            matrix,
            per_sr,
            per_a2b,
            direct,
        })
    }

    fn encode(&self, w: &mut Writer) {
        w.write_u8(self.matrix);
        w.write_array(&self.per_sr);
        w.write_array(&self.per_a2b);
        w.write_u8(self.direct);
    }
}

// =============================================================================
// Deferred surfaces — kept as raw byte blocks per Port.md §1.1 so v0.1
// round-trips faithfully without surfacing a UI for them.
// =============================================================================

/// `led_pwm_config[4]` — 4 × 2 bytes = 8 bytes raw.
const LED_PWM_CONFIG_TOTAL: usize = LED_PWM_CONFIG_SIZE * 4;
/// `leds[24]` — 24 × 2 bytes = 48 bytes raw.
const LEDS_TOTAL: usize = LED_CONFIG_SIZE * MAX_LEDS_NUM;
/// `led_timer_ms[4]` — 4 × u16 = 8 bytes raw.
const LED_TIMER_MS_TOTAL: usize = 2 * 4;
/// `rgb_leds[50]` — 50 × 5 bytes = 250 bytes raw.
const RGB_LEDS_TOTAL: usize = ARGB_LED_SIZE * NUM_RGB_LEDS;

// =============================================================================
// Top-level DeviceConfig
// =============================================================================

/// Decoded `dev_config_t` (1580 bytes on the wire).
///
/// Field ordering exactly mirrors `vendored/common_types.h::dev_config_t`,
/// including the 1-byte alignment pad GCC inserts before `rgb_delay_ms`
/// (kept literal in `rgb_pad`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub firmware_version: u16,
    pub board_id: u8,
    /// Repurposed as `FIRMWARE_BUILD_ID & 0xFF` in `params_report_t`;
    /// here it's the persisted board-layout variant slot.
    pub reserved_layout: u8,
    /// 26 bytes, NUL-terminated when populated. Stored raw so trailing
    /// garbage / non-NUL bytes round-trip exactly.
    pub device_name: [u8; 26],
    pub button_debounce_ms: u16,
    pub encoder_press_time_ms: u8,
    pub exchange_period_ms: u8,
    pub pins: [i8; USED_PINS_NUM],
    pub axis_config: [AxisConfig; MAX_AXIS_NUM],
    #[serde(with = "BigArray")]
    pub buttons: [Button; MAX_BUTTONS_NUM],
    pub button_timer1_ms: u16,
    pub button_timer2_ms: u16,
    pub button_timer3_ms: u16,
    pub a2b_debounce_ms: u16,
    pub tap_cutoff_ms: u16,
    pub double_tap_window_ms: u16,
    pub axes_to_buttons: [AxisToButtons; MAX_AXIS_NUM],
    pub shift_registers: [ShiftRegConfig; MAX_SHIFT_REG_NUM],
    /// `shift_config[8]` — `shift_modificator_t` is 1 byte (int8 button id).
    pub shift_config: [i8; MAX_SHIFTS_NUM],
    pub vid: u16,
    pub pid: u16,
    /// `led_pwm_config[4]` raw bytes (UI deferred to v0.1.1+).
    pub led_pwm_config_raw: [u8; LED_PWM_CONFIG_TOTAL],
    /// `leds[24]` raw bytes (UI deferred to v0.1.1+).
    #[serde(with = "BigArray")]
    pub leds_raw: [u8; LEDS_TOTAL],
    /// `led_timer_ms[4]` raw bytes (UI deferred to v0.1.1+).
    pub led_timer_ms_raw: [u8; LED_TIMER_MS_TOTAL],
    pub encoders: [u8; MAX_ENCODERS_NUM],
    pub fast_encoders: [FastEncoder; MAX_FAST_ENCODER_NUM],
    pub button_polling_interval_ticks: u8,
    pub encoder_polling_interval_ticks: u8,
    pub rgb_effect: u8,
    pub rgb_count: u8,
    pub rgb_brightness: u8,
    /// GCC inserts 1 byte of alignment padding before `rgb_delay_ms` (the
    /// preceding 5 × u8 leaves the offset at 1313, but u16 requires
    /// alignment 2). Stored literal for round-trip exactness.
    pub rgb_pad: u8,
    pub rgb_delay_ms: u16,
    /// `rgb_leds[50]` raw bytes (UI deferred to v0.1.1+).
    #[serde(with = "BigArray")]
    pub rgb_leds_raw: [u8; RGB_LEDS_TOTAL],
    pub saved_breakdown: PhysBreakdown,
}

impl DeviceConfig {
    /// Decode the assembled 1580-byte `dev_config_t` buffer.
    ///
    /// The caller is responsible for fragment reassembly (see
    /// `wire::fragments::reassemble_fragments` with `first_index = 1`
    /// for the config-in path).
    pub fn decode(bytes: &[u8; DEV_CONFIG_SIZE]) -> Result<Self, DecodeError> {
        let mut cur = Cursor::new(bytes);
        let firmware_version = cur.read_u16_le()?;
        let board_id = cur.read_u8()?;
        let reserved_layout = cur.read_u8()?;
        let device_name = cur.read_array::<26>()?;
        let button_debounce_ms = cur.read_u16_le()?;
        let encoder_press_time_ms = cur.read_u8()?;
        let exchange_period_ms = cur.read_u8()?;
        let pins = read_i8_array::<USED_PINS_NUM>(&mut cur)?;
        debug_assert_eq!(
            cur.position(),
            64,
            "axis_config[] should start at offset 64"
        );

        let axis_config = decode_array::<AxisConfig, MAX_AXIS_NUM>(&mut cur, AxisConfig::decode)?;
        debug_assert_eq!(cur.position(), 304, "buttons[] should start at offset 304");

        let buttons = decode_array::<Button, MAX_BUTTONS_NUM>(&mut cur, Button::decode)?;
        debug_assert_eq!(
            cur.position(),
            1072,
            "button_timer1_ms should start at offset 1072"
        );

        let button_timer1_ms = cur.read_u16_le()?;
        let button_timer2_ms = cur.read_u16_le()?;
        let button_timer3_ms = cur.read_u16_le()?;
        let a2b_debounce_ms = cur.read_u16_le()?;
        let tap_cutoff_ms = cur.read_u16_le()?;
        let double_tap_window_ms = cur.read_u16_le()?;
        debug_assert_eq!(
            cur.position(),
            1084,
            "axes_to_buttons[] should start at offset 1084"
        );

        let axes_to_buttons =
            decode_array::<AxisToButtons, MAX_AXIS_NUM>(&mut cur, AxisToButtons::decode)?;
        debug_assert_eq!(
            cur.position(),
            1196,
            "shift_registers[] should start at offset 1196"
        );

        let shift_registers =
            decode_array::<ShiftRegConfig, MAX_SHIFT_REG_NUM>(&mut cur, ShiftRegConfig::decode)?;
        debug_assert_eq!(
            cur.position(),
            1212,
            "shift_config should start at offset 1212"
        );

        let shift_config = read_i8_array::<MAX_SHIFTS_NUM>(&mut cur)?;
        let vid = cur.read_u16_le()?;
        let pid = cur.read_u16_le()?;
        debug_assert_eq!(
            cur.position(),
            1224,
            "led_pwm_config should start at offset 1224"
        );

        let led_pwm_config_raw = cur.read_array::<LED_PWM_CONFIG_TOTAL>()?;
        let leds_raw = cur.read_array::<LEDS_TOTAL>()?;
        let led_timer_ms_raw = cur.read_array::<LED_TIMER_MS_TOTAL>()?;
        debug_assert_eq!(
            cur.position(),
            1288,
            "encoders[] should start at offset 1288"
        );

        let encoders = cur.read_array::<MAX_ENCODERS_NUM>()?;
        let fast_encoders =
            decode_array::<FastEncoder, MAX_FAST_ENCODER_NUM>(&mut cur, FastEncoder::decode)?;
        debug_assert_eq!(
            cur.position(),
            1308,
            "button_polling_interval_ticks should start at offset 1308"
        );

        let button_polling_interval_ticks = cur.read_u8()?;
        let encoder_polling_interval_ticks = cur.read_u8()?;
        let rgb_effect = cur.read_u8()?;
        let rgb_count = cur.read_u8()?;
        let rgb_brightness = cur.read_u8()?;
        let rgb_pad = cur.read_u8()?;
        let rgb_delay_ms = cur.read_u16_le()?;
        debug_assert_eq!(
            cur.position(),
            1316,
            "rgb_leds[] should start at offset 1316"
        );

        let rgb_leds_raw = cur.read_array::<RGB_LEDS_TOTAL>()?;
        debug_assert_eq!(
            cur.position(),
            1566,
            "saved_breakdown should start at offset 1566"
        );

        let saved_breakdown = PhysBreakdown::decode(&mut cur)?;
        debug_assert_eq!(cur.position(), DEV_CONFIG_SIZE);

        Ok(Self {
            firmware_version,
            board_id,
            reserved_layout,
            device_name,
            button_debounce_ms,
            encoder_press_time_ms,
            exchange_period_ms,
            pins,
            axis_config,
            buttons,
            button_timer1_ms,
            button_timer2_ms,
            button_timer3_ms,
            a2b_debounce_ms,
            tap_cutoff_ms,
            double_tap_window_ms,
            axes_to_buttons,
            shift_registers,
            shift_config,
            vid,
            pid,
            led_pwm_config_raw,
            leds_raw,
            led_timer_ms_raw,
            encoders,
            fast_encoders,
            button_polling_interval_ticks,
            encoder_polling_interval_ticks,
            rgb_effect,
            rgb_count,
            rgb_brightness,
            rgb_pad,
            rgb_delay_ms,
            rgb_leds_raw,
            saved_breakdown,
        })
    }

    /// Encode this config back into 1580 bytes.
    #[must_use]
    pub fn encode(&self) -> [u8; DEV_CONFIG_SIZE] {
        let mut out = [0u8; DEV_CONFIG_SIZE];
        let mut w = Writer::new(&mut out);
        w.write_u16_le(self.firmware_version);
        w.write_u8(self.board_id);
        w.write_u8(self.reserved_layout);
        w.write_array(&self.device_name);
        w.write_u16_le(self.button_debounce_ms);
        w.write_u8(self.encoder_press_time_ms);
        w.write_u8(self.exchange_period_ms);
        write_i8_array(&mut w, &self.pins);
        debug_assert_eq!(w.position(), 64);

        for slot in &self.axis_config {
            slot.encode(&mut w);
        }
        debug_assert_eq!(w.position(), 304);

        for slot in &self.buttons {
            slot.encode(&mut w);
        }
        debug_assert_eq!(w.position(), 1072);

        w.write_u16_le(self.button_timer1_ms);
        w.write_u16_le(self.button_timer2_ms);
        w.write_u16_le(self.button_timer3_ms);
        w.write_u16_le(self.a2b_debounce_ms);
        w.write_u16_le(self.tap_cutoff_ms);
        w.write_u16_le(self.double_tap_window_ms);
        debug_assert_eq!(w.position(), 1084);

        for slot in &self.axes_to_buttons {
            slot.encode(&mut w);
        }
        debug_assert_eq!(w.position(), 1196);

        for slot in &self.shift_registers {
            slot.encode(&mut w);
        }
        debug_assert_eq!(w.position(), 1212);

        write_i8_array(&mut w, &self.shift_config);
        w.write_u16_le(self.vid);
        w.write_u16_le(self.pid);
        debug_assert_eq!(w.position(), 1224);

        w.write_array(&self.led_pwm_config_raw);
        w.write_array(&self.leds_raw);
        w.write_array(&self.led_timer_ms_raw);
        debug_assert_eq!(w.position(), 1288);

        w.write_array(&self.encoders);
        for slot in &self.fast_encoders {
            slot.encode(&mut w);
        }
        debug_assert_eq!(w.position(), 1308);

        w.write_u8(self.button_polling_interval_ticks);
        w.write_u8(self.encoder_polling_interval_ticks);
        w.write_u8(self.rgb_effect);
        w.write_u8(self.rgb_count);
        w.write_u8(self.rgb_brightness);
        w.write_u8(self.rgb_pad);
        w.write_u16_le(self.rgb_delay_ms);
        debug_assert_eq!(w.position(), 1316);

        w.write_array(&self.rgb_leds_raw);
        debug_assert_eq!(w.position(), 1566);

        self.saved_breakdown.encode(&mut w);
        debug_assert_eq!(w.position(), DEV_CONFIG_SIZE);

        out
    }
}

// =============================================================================
// Helpers
// =============================================================================

fn read_i8_array<const N: usize>(cur: &mut Cursor) -> Result<[i8; N], DecodeError> {
    let mut out = [0i8; N];
    for slot in &mut out {
        *slot = cur.read_i8()?;
    }
    Ok(out)
}

fn write_i8_array<const N: usize>(w: &mut Writer, src: &[i8; N]) {
    for v in src {
        w.write_i8(*v);
    }
}

fn decode_array<T, const N: usize>(
    cur: &mut Cursor,
    mut decoder: impl FnMut(&mut Cursor) -> Result<T, DecodeError>,
) -> Result<[T; N], DecodeError> {
    // Build via try_fold into a Vec, then convert. The std `array::try_from_fn`
    // is unstable; a Vec round-trip is fine for these small N (<= 128).
    let mut v: Vec<T> = Vec::with_capacity(N);
    for _ in 0..N {
        v.push(decoder(cur)?);
    }
    v.try_into().map_err(|_: Vec<T>| DecodeError::InvalidValue {
        field: "array length",
        value: N as u64,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Zero buffer round-trips. Tests structural correctness of the
    /// codec independently of any fixture's interpretation.
    #[test]
    fn roundtrip_zeros() {
        let zero = [0u8; DEV_CONFIG_SIZE];
        let cfg = DeviceConfig::decode(&zero).unwrap();
        assert_eq!(cfg.encode(), zero);
    }

    /// Walking pattern round-trips. Every byte is distinct so any
    /// off-by-one or swap surfaces.
    #[test]
    fn roundtrip_walking_pattern() {
        let mut bytes = [0u8; DEV_CONFIG_SIZE];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (i & 0xff) as u8;
        }
        let cfg = DeviceConfig::decode(&bytes).unwrap();
        assert_eq!(cfg.encode(), bytes);
    }

    /// Specific fields land at the expected offsets.
    #[test]
    fn known_field_offsets() {
        let mut bytes = [0u8; DEV_CONFIG_SIZE];
        // firmware_version = 0x0010 at offset 0
        bytes[0] = 0x10;
        bytes[1] = 0x00;
        // board_id at offset 2
        bytes[2] = 0x01;
        // vid at offset 1220 = 0x1234
        bytes[1220] = 0x34;
        bytes[1221] = 0x12;
        // pid at offset 1222 = 0xabcd
        bytes[1222] = 0xcd;
        bytes[1223] = 0xab;
        // rgb_delay_ms at offset 1314 = 0x4321
        bytes[1314] = 0x21;
        bytes[1315] = 0x43;
        // saved_breakdown.direct at offset 1579
        bytes[1579] = 0x42;

        let cfg = DeviceConfig::decode(&bytes).unwrap();
        assert_eq!(cfg.firmware_version, 0x0010);
        assert_eq!(cfg.board_id, 0x01);
        assert_eq!(cfg.vid, 0x1234);
        assert_eq!(cfg.pid, 0xabcd);
        assert_eq!(cfg.rgb_delay_ms, 0x4321);
        assert_eq!(cfg.saved_breakdown.direct, 0x42);
    }

    /// Sub-struct byte budgets sum to the wire total. Documents the
    /// per-field accounting that puts every byte of `dev_config_t`
    /// somewhere accountable.
    #[test]
    fn sub_struct_sizes_sum_to_total() {
        const AXIS_CONFIG_SIZE: usize = 30;
        const BUTTON_SIZE: usize = 6;
        const AXIS_TO_BUTTONS_SIZE: usize = 14;
        const SHIFT_REG_CONFIG_SIZE: usize = 4;
        const PHYS_BREAKDOWN_SIZE: usize = 14;

        let pre_axis = 64;
        let axis = AXIS_CONFIG_SIZE * MAX_AXIS_NUM; // 240
        let buttons = BUTTON_SIZE * MAX_BUTTONS_NUM; // 768
        let timers = 6 * 2; // 12
        let a2b = AXIS_TO_BUTTONS_SIZE * MAX_AXIS_NUM; // 112
        let sr = SHIFT_REG_CONFIG_SIZE * MAX_SHIFT_REG_NUM; // 16
        let shift_cfg = MAX_SHIFTS_NUM; // 8
        let vidpid = 4;
        let leds = LED_PWM_CONFIG_TOTAL + LEDS_TOTAL + LED_TIMER_MS_TOTAL; // 64
        let enc = MAX_ENCODERS_NUM + 2 * MAX_FAST_ENCODER_NUM; // 20
        let trailing = 5 + 1 + 2 + RGB_LEDS_TOTAL + PHYS_BREAKDOWN_SIZE; // 272
        let total = pre_axis
            + axis
            + buttons
            + timers
            + a2b
            + sr
            + shift_cfg
            + vidpid
            + leds
            + enc
            + trailing;
        assert_eq!(total, DEV_CONFIG_SIZE);
    }
}
