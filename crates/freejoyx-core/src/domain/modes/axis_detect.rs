//! Axis auto-detect state machine.
//!
//! When the user clicks the Detect button on an axis row, that slot
//! arms. We snapshot every axis's current raw value as the baseline,
//! then on each `ParamsReport` watch every *other* axis for a
//! |delta| > [`AXIS_DETECT_THRESHOLD`]. The axis that moved most past
//! the threshold "claims" the source — its `source_main` is copied onto
//! the armed row. Mirrors the Qt configurator's
//! `AxesConfig::m_armedAxisIdx` + `m_baselineRaw` pair.
//!
//! Disarms on a successful bind or on the [`AXIS_DETECT_TIMEOUT`].
//! Caller provides `now: Instant` per tick so tests can advance time
//! deterministically.

use std::time::{Duration, Instant};

use crate::domain::axes::AxisSource;
use crate::wire::config::DeviceConfig;
use crate::wire::params::{ParamsReport, MAX_AXIS_NUM};

/// Auto-detect threshold in raw ADC LSBs. Matches the Qt
/// configurator's `AxesConfig::m_kDetectThresh`: ~6% of full scale,
/// low enough that a normal pot sweep crosses it within a single
/// tick, high enough not to trigger on idle noise.
pub const AXIS_DETECT_THRESHOLD: i32 = 4000;

/// How long the armed slot waits for any other axis to cross the
/// threshold before timing out and disarming.
pub const AXIS_DETECT_TIMEOUT: Duration = Duration::from_secs(5);

/// Result of one [`AxisDetect::on_params_tick`] call.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AxisDetectOutcome {
    /// `Some(slot)` if the armed slot bound to a source this tick.
    pub bound_to_slot: Option<usize>,
    /// True if the bound landed and changed the config.
    pub config_changed: bool,
    /// True if [`AXIS_DETECT_TIMEOUT`] elapsed and we disarmed without
    /// binding. The UI uses this to bump its disarm-tick so the cell
    /// drops out of armed visual state.
    pub timed_out: bool,
}

/// Axis-detect state machine.
#[derive(Debug, Default)]
pub struct AxisDetect {
    armed: Option<DetectArm>,
    /// Per-slot disarm-tick. UI watches `changed disarm-tick` on each
    /// row's IconButton to drop its armed visual state.
    disarm_ticks: [i32; MAX_AXIS_NUM],
}

#[derive(Debug, Clone, Copy)]
struct DetectArm {
    slot: usize,
    baseline: [i16; MAX_AXIS_NUM],
    armed_at: Instant,
}

impl AxisDetect {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Arm `slot`. Snapshots the current raw values from `params` as
    /// the baseline so subsequent ticks can detect what moved. If
    /// another slot was already armed, that one's disarm-tick is
    /// bumped and the new slot takes over.
    pub fn arm(&mut self, slot: usize, params: &ParamsReport, now: Instant) {
        if let Some(prev) = self.armed.take() {
            if prev.slot != slot {
                self.bump_tick(prev.slot);
            }
        }
        let mut baseline = [0i16; MAX_AXIS_NUM];
        baseline[..MAX_AXIS_NUM].copy_from_slice(&params.raw_axis_data[..MAX_AXIS_NUM]);
        self.armed = Some(DetectArm {
            slot,
            baseline,
            armed_at: now,
        });
    }

    /// Disarm whatever's currently armed and bump that slot's tick so
    /// the UI tears down its armed state. No-op if idle.
    pub fn disarm(&mut self) {
        if let Some(prev) = self.armed.take() {
            self.bump_tick(prev.slot);
        }
    }

    /// Same as `disarm` but doesn't bump the tick — call this when the
    /// UI itself is going away (disconnect, tab destroyed).
    pub fn clear(&mut self) {
        self.armed = None;
    }

    #[must_use]
    pub fn armed_slot(&self) -> Option<usize> {
        self.armed.map(|a| a.slot)
    }

    #[must_use]
    pub fn disarm_tick(&self, slot: usize) -> i32 {
        self.disarm_ticks[slot]
    }

    /// Check the live raw values against the baseline. If any axis
    /// (other than the armed one) moved more than
    /// [`AXIS_DETECT_THRESHOLD`], the axis whose delta is largest
    /// claims the source: its `source_main` is copied onto the armed
    /// row. Also handles the [`AXIS_DETECT_TIMEOUT`] disarm.
    pub fn on_params_tick(
        &mut self,
        params: &ParamsReport,
        cfg: &mut DeviceConfig,
        now: Instant,
    ) -> AxisDetectOutcome {
        let Some(arm) = self.armed else {
            return AxisDetectOutcome::default();
        };

        // Timeout disarm — nothing moved enough within the window.
        if now.duration_since(arm.armed_at) > AXIS_DETECT_TIMEOUT {
            self.armed = None;
            self.bump_tick(arm.slot);
            return AxisDetectOutcome {
                bound_to_slot: None,
                config_changed: false,
                timed_out: true,
            };
        }

        // Find the axis with the largest qualifying delta.
        let mut best: Option<(usize, i32)> = None;
        for (slot, base) in arm.baseline.iter().enumerate().take(MAX_AXIS_NUM) {
            if slot == arm.slot {
                continue;
            }
            let delta = (i32::from(params.raw_axis_data[slot]) - i32::from(*base)).abs();
            if delta >= AXIS_DETECT_THRESHOLD && best.is_none_or(|(_, b)| delta > b) {
                best = Some((slot, delta));
            }
        }
        let Some((moved_slot, _)) = best else {
            return AxisDetectOutcome::default();
        };

        // Copy the moved axis's source onto the armed row.
        let new_source = cfg.axis_config[moved_slot].source();
        cfg.axis_config[arm.slot].set_source(new_source);
        // No-source-no-output guard: if the source we copied is None,
        // make sure the armed row's output is off too.
        if matches!(new_source, AxisSource::None) {
            cfg.axis_config[arm.slot].set_out_enabled(false);
        }

        self.armed = None;
        self.bump_tick(arm.slot);
        AxisDetectOutcome {
            bound_to_slot: Some(arm.slot),
            config_changed: true,
            timed_out: false,
        }
    }

    fn bump_tick(&mut self, slot: usize) {
        self.disarm_ticks[slot] = self.disarm_ticks[slot].wrapping_add(1);
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

    fn params_with(raw: [i16; MAX_AXIS_NUM]) -> ParamsReport {
        let mut p = empty_params();
        p.raw_axis_data = raw;
        p
    }

    fn now_at(secs: u64) -> Instant {
        // Anchor: same Instant + advancement keeps tests deterministic.
        // We don't expose the anchor — it's only relative deltas that
        // matter for the timeout test.
        Instant::now() + Duration::from_secs(secs)
    }

    #[test]
    fn idle_returns_default() {
        let mut det = AxisDetect::new();
        let mut cfg = empty_config();
        let out = det.on_params_tick(&empty_params(), &mut cfg, Instant::now());
        assert_eq!(out, AxisDetectOutcome::default());
    }

    #[test]
    fn arming_then_moving_another_axis_binds_source() {
        let mut det = AxisDetect::new();
        let mut cfg = empty_config();
        // Pre-set axis 5's source so we can verify it's copied onto axis 2.
        cfg.axis_config[5].set_source(AxisSource::Pin(7));
        let t0 = Instant::now();
        det.arm(2, &empty_params(), t0);

        let moved = params_with({
            let mut a = [0i16; MAX_AXIS_NUM];
            a[5] = (AXIS_DETECT_THRESHOLD + 100) as i16;
            a
        });
        let out = det.on_params_tick(&moved, &mut cfg, t0 + Duration::from_millis(50));

        assert_eq!(out.bound_to_slot, Some(2));
        assert!(out.config_changed);
        assert!(!out.timed_out);
        assert_eq!(cfg.axis_config[2].source(), AxisSource::Pin(7));
        assert!(det.armed_slot().is_none());
    }

    #[test]
    fn movement_below_threshold_does_not_bind() {
        let mut det = AxisDetect::new();
        let mut cfg = empty_config();
        det.arm(0, &empty_params(), Instant::now());

        let small = params_with({
            let mut a = [0i16; MAX_AXIS_NUM];
            a[3] = (AXIS_DETECT_THRESHOLD - 1) as i16;
            a
        });
        let out = det.on_params_tick(&small, &mut cfg, Instant::now());
        assert_eq!(out.bound_to_slot, None);
        assert!(det.armed_slot() == Some(0), "still armed");
    }

    #[test]
    fn movement_on_armed_axis_itself_is_ignored() {
        let mut det = AxisDetect::new();
        let mut cfg = empty_config();
        det.arm(3, &empty_params(), Instant::now());

        let self_move = params_with({
            let mut a = [0i16; MAX_AXIS_NUM];
            a[3] = (AXIS_DETECT_THRESHOLD + 5000) as i16;
            a
        });
        let out = det.on_params_tick(&self_move, &mut cfg, Instant::now());
        assert_eq!(out.bound_to_slot, None);
        assert!(det.armed_slot() == Some(3));
    }

    #[test]
    fn largest_delta_wins_when_multiple_axes_move() {
        let mut det = AxisDetect::new();
        let mut cfg = empty_config();
        cfg.axis_config[4].set_source(AxisSource::Pin(3));
        cfg.axis_config[6].set_source(AxisSource::Pin(8));
        det.arm(0, &empty_params(), Instant::now());

        let moved = params_with({
            let mut a = [0i16; MAX_AXIS_NUM];
            a[4] = (AXIS_DETECT_THRESHOLD + 100) as i16; // smaller
            a[6] = (AXIS_DETECT_THRESHOLD + 9000) as i16; // bigger
            a
        });
        det.on_params_tick(&moved, &mut cfg, Instant::now());
        assert_eq!(cfg.axis_config[0].source(), AxisSource::Pin(8));
    }

    #[test]
    fn timeout_disarms_without_binding() {
        let mut det = AxisDetect::new();
        let mut cfg = empty_config();
        let t0 = now_at(0);
        det.arm(1, &empty_params(), t0);

        let out = det.on_params_tick(&empty_params(), &mut cfg, t0 + Duration::from_secs(6));
        assert!(out.timed_out);
        assert!(!out.config_changed);
        assert!(det.armed_slot().is_none());
    }

    #[test]
    fn binding_to_none_source_disables_output_on_armed_row() {
        let mut det = AxisDetect::new();
        let mut cfg = empty_config();
        cfg.axis_config[0].set_out_enabled(true);
        // Empty config decodes source byte 0 as Pin(0), so we have to
        // explicitly set the donor axis's source to None to exercise
        // the no-source-no-output guard.
        cfg.axis_config[5].set_source(AxisSource::None);
        det.arm(0, &empty_params(), Instant::now());

        let moved = params_with({
            let mut a = [0i16; MAX_AXIS_NUM];
            a[5] = (AXIS_DETECT_THRESHOLD + 100) as i16;
            a
        });
        det.on_params_tick(&moved, &mut cfg, Instant::now());
        assert_eq!(cfg.axis_config[0].source(), AxisSource::None);
        assert!(!cfg.axis_config[0].out_enabled());
    }

    #[test]
    fn bind_and_timeout_both_bump_disarm_tick() {
        // Bind path.
        let mut det = AxisDetect::new();
        let mut cfg = empty_config();
        cfg.axis_config[4].set_source(AxisSource::Pin(2));
        let before = det.disarm_tick(0);
        det.arm(0, &empty_params(), Instant::now());
        let moved = params_with({
            let mut a = [0i16; MAX_AXIS_NUM];
            a[4] = (AXIS_DETECT_THRESHOLD + 100) as i16;
            a
        });
        det.on_params_tick(&moved, &mut cfg, Instant::now());
        assert_ne!(det.disarm_tick(0), before, "bind bumps tick");

        // Timeout path.
        let t0 = Instant::now();
        let before = det.disarm_tick(1);
        det.arm(1, &empty_params(), t0);
        det.on_params_tick(&empty_params(), &mut cfg, t0 + Duration::from_secs(6));
        assert_ne!(det.disarm_tick(1), before, "timeout bumps tick");
    }

    #[test]
    fn manual_disarm_bumps_tick_clear_does_not() {
        let mut det = AxisDetect::new();
        let before = det.disarm_tick(2);
        det.arm(2, &empty_params(), Instant::now());
        det.disarm();
        assert_ne!(det.disarm_tick(2), before, "disarm bumps");

        let before = det.disarm_tick(3);
        det.arm(3, &empty_params(), Instant::now());
        det.clear();
        assert_eq!(det.disarm_tick(3), before, "clear does not bump");
    }
}
