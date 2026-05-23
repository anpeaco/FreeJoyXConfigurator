//! Axis-calibration state machine.
//!
//! When the user clicks the "Calibrate" button on an axis row, that
//! slot enters calibration mode. On every subsequent `ParamsReport`,
//! we watch the slot's raw value and widen `calib_min` / `calib_max`
//! whenever the value moves past the current extremes. Mirrors the Qt
//! configurator's `Axes::calibrationStarted` behaviour.
//!
//! Only one slot can calibrate at a time. Clicking "Stop & Save" (or
//! Calibrate on a different slot) ends the mode; the widened bounds
//! are already written to the config in place.

use crate::wire::config::DeviceConfig;
use crate::wire::params::ParamsReport;

/// Result of one [`AxisCalibration::on_params_tick`] call.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AxisCalibrationOutcome {
    /// True iff `calib_min` / `calib_max` widened (or `calib_center`
    /// recomputed) this tick. Caller flips dirty flags.
    pub config_changed: bool,
}

/// Axis-calibration state machine.
#[derive(Debug, Default, Clone)]
pub struct AxisCalibration {
    armed_slot: Option<usize>,
}

impl AxisCalibration {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Toggle calibration on `slot`. If `slot` was already calibrating,
    /// disarm; otherwise arm it (swapping out any other armed slot).
    pub fn toggle(&mut self, slot: usize) {
        self.armed_slot = if self.armed_slot == Some(slot) {
            None
        } else {
            Some(slot)
        };
    }

    /// Disarm whatever's currently armed. No-op if idle.
    pub fn disarm(&mut self) {
        self.armed_slot = None;
    }

    /// Clear the arm bit without doing anything else. Same as `disarm`
    /// today; kept distinct for symmetry with [`super::ButtonCapture`]
    /// where the two semantics differ.
    pub fn clear(&mut self) {
        self.armed_slot = None;
    }

    #[must_use]
    pub fn armed_slot(&self) -> Option<usize> {
        self.armed_slot
    }

    /// Watch the armed slot's raw value and widen its calibration
    /// bounds if the value escaped the current `[calib_min, calib_max]`
    /// range. If the slot is not in "centered" mode, also recompute
    /// `calib_center` as the midpoint — matches Qt's
    /// `calibMinMaxValueChanged`.
    pub fn on_params_tick(
        &mut self,
        params: &ParamsReport,
        cfg: &mut DeviceConfig,
    ) -> AxisCalibrationOutcome {
        let Some(slot) = self.armed_slot else {
            return AxisCalibrationOutcome::default();
        };
        let raw = params.raw_axis_data[slot];
        let a = &mut cfg.axis_config[slot];
        let mut changed = false;
        if raw > a.calib_max {
            a.calib_max = raw;
            changed = true;
        }
        if raw < a.calib_min {
            a.calib_min = raw;
            changed = true;
        }
        if !a.is_centered() {
            // Midpoint of two i16s — sum fits in i17.
            let mid = (i32::from(a.calib_min) + i32::from(a.calib_max)) / 2;
            let new_center = i16::try_from(mid).unwrap_or(0);
            if new_center != a.calib_center {
                a.calib_center = new_center;
                changed = true;
            }
        }
        AxisCalibrationOutcome {
            config_changed: changed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::config::DEV_CONFIG_SIZE;
    use crate::wire::params::PARAMS_REPORT_SIZE;

    fn empty_config() -> DeviceConfig {
        DeviceConfig::decode(&[0u8; DEV_CONFIG_SIZE]).unwrap()
    }

    fn empty_params() -> ParamsReport {
        ParamsReport::decode(&[0u8; PARAMS_REPORT_SIZE]).unwrap()
    }

    fn params_with_raw(slot: usize, raw: i16) -> ParamsReport {
        let mut p = empty_params();
        p.raw_axis_data[slot] = raw;
        p
    }

    #[test]
    fn idle_does_nothing() {
        let mut cal = AxisCalibration::new();
        let mut cfg = empty_config();
        cfg.axis_config[0].calib_min = 0;
        cfg.axis_config[0].calib_max = 0;

        let out = cal.on_params_tick(&params_with_raw(0, 5000), &mut cfg);
        assert!(!out.config_changed);
        assert_eq!(cfg.axis_config[0].calib_max, 0, "no calibration → no change");
    }

    #[test]
    fn widens_max_when_raw_exceeds() {
        let mut cal = AxisCalibration::new();
        let mut cfg = empty_config();
        cfg.axis_config[2].calib_max = 1000;
        cal.toggle(2);

        let out = cal.on_params_tick(&params_with_raw(2, 5000), &mut cfg);
        assert!(out.config_changed);
        assert_eq!(cfg.axis_config[2].calib_max, 5000);
    }

    #[test]
    fn widens_min_when_raw_below() {
        let mut cal = AxisCalibration::new();
        let mut cfg = empty_config();
        cfg.axis_config[1].calib_min = -1000;
        cal.toggle(1);

        let out = cal.on_params_tick(&params_with_raw(1, -8000), &mut cfg);
        assert!(out.config_changed);
        assert_eq!(cfg.axis_config[1].calib_min, -8000);
    }

    #[test]
    fn recomputes_center_when_not_centered() {
        let mut cal = AxisCalibration::new();
        let mut cfg = empty_config();
        cfg.axis_config[0].set_is_centered(false);
        cfg.axis_config[0].calib_min = -1000;
        cfg.axis_config[0].calib_max = 1000;
        cfg.axis_config[0].calib_center = 0;
        cal.toggle(0);

        // Push max out → center recomputes to midpoint.
        cal.on_params_tick(&params_with_raw(0, 3000), &mut cfg);
        assert_eq!(cfg.axis_config[0].calib_max, 3000);
        assert_eq!(cfg.axis_config[0].calib_center, 1000); // (-1000 + 3000) / 2
    }

    #[test]
    fn preserves_user_center_when_centered_flag_set() {
        let mut cal = AxisCalibration::new();
        let mut cfg = empty_config();
        cfg.axis_config[0].set_is_centered(true);
        cfg.axis_config[0].calib_min = -1000;
        cfg.axis_config[0].calib_max = 1000;
        cfg.axis_config[0].calib_center = 250;
        cal.toggle(0);

        cal.on_params_tick(&params_with_raw(0, 3000), &mut cfg);
        assert_eq!(cfg.axis_config[0].calib_center, 250, "user value preserved");
    }

    #[test]
    fn toggle_disarms_when_same_slot_clicked_twice() {
        let mut cal = AxisCalibration::new();
        cal.toggle(3);
        assert_eq!(cal.armed_slot(), Some(3));
        cal.toggle(3);
        assert_eq!(cal.armed_slot(), None);
    }

    #[test]
    fn toggle_switches_when_different_slot_clicked() {
        let mut cal = AxisCalibration::new();
        cal.toggle(3);
        cal.toggle(5);
        assert_eq!(cal.armed_slot(), Some(5));
    }

    #[test]
    fn raw_inside_bounds_reports_no_change() {
        let mut cal = AxisCalibration::new();
        let mut cfg = empty_config();
        cfg.axis_config[0].set_is_centered(true); // freeze center
        cfg.axis_config[0].calib_min = -10000;
        cfg.axis_config[0].calib_max = 10000;
        cal.toggle(0);

        let out = cal.on_params_tick(&params_with_raw(0, 5000), &mut cfg);
        assert!(!out.config_changed);
    }
}
