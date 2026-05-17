//! Human-readable formatter for [`DeviceConfig`].
//!
//! Used by the CLI debug subcommands and the worker's diagnostic
//! tracing. Renders a multi-line dump with only the fields that would
//! actually mean something to a user (skips default / unused entries
//! for buttons, axes, encoders, shift registers, LEDs).
//!
//! The output is grep-able by section header (`-- pins`, `-- axes`,
//! `-- buttons`, etc) so a maintainer can pipe `freejoyx-app
//! read-config` into a file and search for the relevant table when
//! debugging a wire-format hypothesis.

use std::fmt::Write;

use super::config::{
    AxisConfig, Button, DeviceConfig, FastEncoder, MAX_BUTTONS_NUM, MAX_ENCODERS_NUM,
    MAX_FAST_ENCODER_NUM, MAX_SHIFT_REG_NUM, USED_PINS_NUM,
};

/// Render `cfg` as a multi-line human-readable dump.
///
/// Skips entries whose meaning is "unused / default" to keep the
/// output scannable on a real device with ~10 buttons configured out
/// of 128 slots.
#[must_use]
pub fn format_config(cfg: &DeviceConfig) -> String {
    let mut s = String::with_capacity(4096);

    let _ = writeln!(s, "DeviceConfig (1580 bytes):");
    let _ = writeln!(s, "  firmware_version: 0x{:04x}", cfg.firmware_version);
    let _ = writeln!(
        s,
        "  board_id:         {} ({})",
        cfg.board_id,
        board_name(cfg.board_id)
    );
    let _ = writeln!(s, "  reserved_layout:  {}", cfg.reserved_layout);
    let _ = writeln!(s, "  device_name:      {:?}", name_string(&cfg.device_name));
    let _ = writeln!(
        s,
        "  vid/pid:          0x{:04x} / 0x{:04x}",
        cfg.vid, cfg.pid
    );
    let _ = writeln!(s, "  exchange_period:  {} ms", cfg.exchange_period_ms);
    let _ = writeln!(s, "  encoder_press:    {} ms", cfg.encoder_press_time_ms);
    let _ = writeln!(
        s,
        "  polling_ticks:    button={} encoder={}",
        cfg.button_polling_interval_ticks, cfg.encoder_polling_interval_ticks
    );

    let _ = writeln!(s, "\n  -- timers --");
    let _ = writeln!(s, "  button_debounce_ms:   {}", cfg.button_debounce_ms);
    let _ = writeln!(s, "  button_timer1_ms:     {}", cfg.button_timer1_ms);
    let _ = writeln!(s, "  button_timer2_ms:     {}", cfg.button_timer2_ms);
    let _ = writeln!(s, "  button_timer3_ms:     {}", cfg.button_timer3_ms);
    let _ = writeln!(s, "  a2b_debounce_ms:      {}", cfg.a2b_debounce_ms);
    let _ = writeln!(s, "  tap_cutoff_ms:        {}", cfg.tap_cutoff_ms);
    let _ = writeln!(s, "  double_tap_window_ms: {}", cfg.double_tap_window_ms);

    let _ = writeln!(s, "\n  -- pins (USED_PINS_NUM = {}) --", USED_PINS_NUM);
    for (i, &p) in cfg.pins.iter().enumerate() {
        if p == 0 {
            continue;
        }
        let _ = writeln!(s, "    [{:>2}] {:>4} = {}", i, p, pin_function_name(p));
    }
    let used = cfg.pins.iter().filter(|&&p| p != 0).count();
    let _ = writeln!(s, "    ({}/{} pins assigned)", used, USED_PINS_NUM);

    let _ = writeln!(s, "\n  -- axes (8 slots, non-default only) --");
    let mut any_axis = false;
    for (i, a) in cfg.axis_config.iter().enumerate() {
        if is_default_axis(a) {
            continue;
        }
        any_axis = true;
        let _ = writeln!(
            s,
            "    [{}] enabled={} inverted={} centered={} func={} filter={} \
             calib=[{},{},{}] res={} chan={} deadband={}{} \
             src_main={} src_sec={} offset_angle={} \
             buttons=({},{},{}) btn_types=({},{},{}) \
             prescaler={} divider={} i2c=0x{:02x}",
            i,
            a.out_enabled(),
            a.inverted(),
            a.is_centered(),
            a.function(),
            a.filter(),
            a.calib_min,
            a.calib_center,
            a.calib_max,
            a.resolution(),
            a.channel(),
            a.deadband_size(),
            if a.is_dynamic_deadband() {
                " (dynamic)"
            } else {
                ""
            },
            a.source_main,
            (a.flags4 & 0x07),
            ((a.flags4 >> 3) & 0x1f),
            a.button1,
            a.button2,
            a.button3,
            (a.flags5 & 0x07),
            ((a.flags5 >> 3) & 0x03),
            ((a.flags5 >> 5) & 0x07),
            a.prescaler,
            a.divider,
            a.i2c_address,
        );
    }
    if !any_axis {
        let _ = writeln!(s, "    (all axes default)");
    }

    let _ = writeln!(s, "\n  -- buttons (128 slots, non-default only) --");
    let mut button_lines = 0;
    for (i, b) in cfg.buttons.iter().enumerate() {
        if is_default_button(b) {
            continue;
        }
        button_lines += 1;
        let _ = writeln!(
            s,
            "    [{:>3}] phys={:>3} type={} shift={} op={} src_b={} delay_t={} press_t={} inv={} dis={}",
            i,
            b.physical_num,
            button_type_name(b.button_type),
            b.shift_modificator(),
            logic_op_name(b.op()),
            b.src_b,
            b.delay_timer(),
            b.press_timer(),
            b.is_inverted(),
            b.is_disabled(),
        );
    }
    if button_lines == 0 {
        let _ = writeln!(s, "    (all buttons default)");
    } else {
        let _ = writeln!(
            s,
            "    ({} of {} button slots configured)",
            button_lines, MAX_BUTTONS_NUM
        );
    }

    let _ = writeln!(s, "\n  -- encoders (16 soft slots, non-default only) --");
    let mut any_enc = false;
    for (i, &e) in cfg.encoders.iter().enumerate() {
        if e == 0 {
            continue;
        }
        any_enc = true;
        let _ = writeln!(s, "    [{:>2}] type = {} ({})", i, e, encoder_type_name(e));
    }
    if !any_enc {
        let _ = writeln!(s, "    (all {} slots default)", MAX_ENCODERS_NUM);
    }

    let _ = writeln!(s, "\n  -- fast_encoders (2 slots) --");
    for (i, f) in cfg.fast_encoders.iter().enumerate() {
        let _ = writeln!(
            s,
            "    [{}] enabled={} mode={} ({})",
            i,
            f.enabled,
            f.mode,
            encoder_type_name(f.mode),
        );
    }
    if cfg.fast_encoders.len() != MAX_FAST_ENCODER_NUM {
        let _ = writeln!(
            s,
            "    (unexpected array len {} != {})",
            cfg.fast_encoders.len(),
            MAX_FAST_ENCODER_NUM
        );
    }

    let _ = writeln!(s, "\n  -- shift_registers (4 slots, non-default only) --");
    let mut any_sr = false;
    for (i, sr) in cfg.shift_registers.iter().enumerate() {
        if sr.reg_type == 0 && sr.button_cnt == 0 {
            continue;
        }
        any_sr = true;
        let _ = writeln!(
            s,
            "    [{}] type={} ({}) count={}",
            i,
            sr.reg_type,
            sr_type_name(sr.reg_type),
            sr.button_cnt,
        );
    }
    if !any_sr {
        let _ = writeln!(s, "    (all {} slots default)", MAX_SHIFT_REG_NUM);
    }

    let _ = writeln!(
        s,
        "\n  -- shift_config (8 modifier slots, non-default only) --"
    );
    let mut any_sm = false;
    for (i, &btn) in cfg.shift_config.iter().enumerate() {
        if btn < 0 {
            continue;
        }
        any_sm = true;
        let _ = writeln!(s, "    [{}] modifier button = {}", i, btn);
    }
    if !any_sm {
        let _ = writeln!(s, "    (all modifiers cleared)");
    }

    let _ = writeln!(s, "\n  -- axes_to_buttons (8 slots, non-default only) --");
    let mut any_a2b = false;
    for (i, a2b) in cfg.axes_to_buttons.iter().enumerate() {
        if a2b.buttons_cnt == 0 {
            continue;
        }
        any_a2b = true;
        let pts = a2b
            .points
            .iter()
            .take(a2b.buttons_cnt as usize + 1)
            .map(u8::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let _ = writeln!(s, "    [{}] count={} points=[{}]", i, a2b.buttons_cnt, pts);
    }
    if !any_a2b {
        let _ = writeln!(s, "    (all axes-to-buttons default)");
    }

    let _ = writeln!(s, "\n  -- led / rgb summary --");
    let _ = writeln!(
        s,
        "    led_pwm_config_raw: {} bytes, nonzero={}",
        cfg.led_pwm_config_raw.len(),
        cfg.led_pwm_config_raw.iter().filter(|&&b| b != 0).count()
    );
    let _ = writeln!(
        s,
        "    leds_raw:           {} bytes, nonzero={}",
        cfg.leds_raw.len(),
        cfg.leds_raw.iter().filter(|&&b| b != 0).count()
    );
    let _ = writeln!(
        s,
        "    led_timer_ms_raw:   {} bytes, nonzero={}",
        cfg.led_timer_ms_raw.len(),
        cfg.led_timer_ms_raw.iter().filter(|&&b| b != 0).count()
    );
    let _ = writeln!(
        s,
        "    rgb: effect={} count={} brightness={} delay={} ms",
        cfg.rgb_effect, cfg.rgb_count, cfg.rgb_brightness, cfg.rgb_delay_ms
    );
    let _ = writeln!(
        s,
        "    rgb_leds_raw:       {} bytes, nonzero={}",
        cfg.rgb_leds_raw.len(),
        cfg.rgb_leds_raw.iter().filter(|&&b| b != 0).count()
    );

    let _ = writeln!(s, "\n  -- saved_breakdown (configurator metadata) --");
    let pb = &cfg.saved_breakdown;
    let _ = writeln!(
        s,
        "    matrix={} per_sr={:?} per_a2b={:?} direct={}",
        pb.matrix, pb.per_sr, pb.per_a2b, pb.direct
    );

    s
}

fn name_string(name: &[u8; 26]) -> String {
    let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
    String::from_utf8_lossy(&name[..end]).into_owned()
}

fn board_name(id: u8) -> &'static str {
    match id {
        0 => "unset / legacy",
        1 => "F103 BluePill",
        2 => "F411 BlackPill",
        _ => "unknown",
    }
}

fn pin_function_name(v: i8) -> &'static str {
    // pin_t enum from vendored/common_types.h. Negative values aren't
    // assigned by the enum but the type is int8_t for size reasons.
    match v {
        0 => "NOT_USED",
        1 => "BUTTON_GND",
        2 => "BUTTON_VCC",
        3 => "BUTTON_ROW",
        4 => "BUTTON_COLUMN",
        5 => "AXIS_ANALOG",
        6 => "FAST_ENCODER",
        7 => "SPI_SCK",
        8 => "SPI_MOSI",
        9 => "SPI_MISO",
        10 => "TLE5011_GEN",
        11 => "TLE5011_CS",
        12 => "TLE5012_CS",
        13 => "MCP3201_CS",
        14 => "MCP3202_CS",
        15 => "MCP3204_CS",
        16 => "MCP3208_CS",
        17 => "MLX90393_CS",
        18 => "AS5048A_CS",
        19 => "SHIFT_REG_LATCH",
        20 => "SHIFT_REG_DATA",
        21 => "LED_PWM",
        22 => "LED_SINGLE",
        23 => "LED_ROW",
        24 => "LED_COLUMN",
        25 => "I2C_SCL",
        26 => "I2C_SDA",
        27 => "MLX90363_CS",
        28 => "SHIFT_REG_CLK",
        29 => "LED_RGB_WS2812B",
        30 => "LED_RGB_PL9823",
        31 => "UART_TX",
        _ => "<unknown>",
    }
}

fn button_type_name(v: u8) -> &'static str {
    match v {
        0 => "NORMAL",
        1 => "TOGGLE",
        2 => "TOGGLE_SWITCH",
        3 => "TOGGLE_SWITCH_ON",
        4 => "TOGGLE_SWITCH_OFF",
        5 => "POV1_UP",
        6 => "POV1_RIGHT",
        7 => "POV1_DOWN",
        8 => "POV1_LEFT",
        9 => "POV1_CENTER",
        10 => "POV2_UP",
        11 => "POV2_RIGHT",
        12 => "POV2_DOWN",
        13 => "POV2_LEFT",
        14 => "POV2_CENTER",
        15 => "POV3_UP",
        16 => "POV3_RIGHT",
        17 => "POV3_DOWN",
        18 => "POV3_LEFT",
        19 => "POV4_UP",
        20 => "POV4_RIGHT",
        21 => "POV4_DOWN",
        22 => "POV4_LEFT",
        23 => "ENCODER_INPUT_A",
        24 => "ENCODER_INPUT_B",
        25 => "RADIO_BUTTON1",
        26 => "RADIO_BUTTON2",
        27 => "RADIO_BUTTON3",
        28 => "RADIO_BUTTON4",
        29 => "SEQUENTIAL_TOGGLE",
        30 => "SEQUENTIAL_BUTTON",
        31 => "POV3_CENTER",
        32 => "POV4_CENTER",
        33 => "LOGIC",
        34 => "TAP",
        35 => "DOUBLE_TAP",
        _ => "<unknown>",
    }
}

fn logic_op_name(v: u8) -> &'static str {
    match v {
        0 => "AND",
        1 => "OR",
        2 => "NOT",
        3 => "NOR",
        4 => "NAND",
        5 => "XOR",
        6 => "A_AND_NOT_B",
        _ => "<unknown>",
    }
}

fn encoder_type_name(v: u8) -> &'static str {
    match v {
        0 => "1x",
        1 => "2x",
        2 => "4x",
        _ => "<unknown>",
    }
}

fn sr_type_name(v: u8) -> &'static str {
    match v {
        0 => "HC165_PULL_DOWN",
        1 => "CD4021_PULL_DOWN",
        2 => "HC165_PULL_UP",
        3 => "CD4021_PULL_UP",
        _ => "<unknown>",
    }
}

fn is_default_axis(a: &AxisConfig) -> bool {
    a.flags1 == 0
        && a.flags2 == 0
        && a.flags3 == 0
        && a.flags4 == 0
        && a.flags5 == 0
        && a.calib_min == 0
        && a.calib_center == 0
        && a.calib_max == 0
        && a.button1 == 0
        && a.button2 == 0
        && a.button3 == 0
        && a.source_main == 0
        && a.curve_shape.iter().all(|&v| v == 0)
}

fn is_default_button(b: &Button) -> bool {
    b.physical_num == -1
        && b.button_type == 0
        && b.src_b == -1
        && b.flags_a == 0
        && b.flags_b == 0
        && b.flags_c == 0
}

#[allow(dead_code)]
fn _is_default_fast_encoder(f: &FastEncoder) -> bool {
    f.enabled == 0 && f.mode == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::DEV_CONFIG_SIZE;

    #[test]
    fn zero_config_formats_without_panic() {
        let zero = [0u8; DEV_CONFIG_SIZE];
        let cfg = DeviceConfig::decode(&zero).unwrap();
        let s = format_config(&cfg);
        assert!(s.contains("DeviceConfig (1580 bytes)"));
        assert!(s.contains("-- pins"));
        assert!(s.contains("-- buttons"));
        assert!(s.contains("-- timers"));
    }
}
