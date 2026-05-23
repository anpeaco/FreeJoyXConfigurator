//! Button-capture state machine.
//!
//! When the user clicks a *Physical* or *Source B* number cell on the
//! Buttons tab, the cell flips into "press a button to capture" mode.
//! On the next `ParamsReport`, the first rising edge in
//! `phy_button_data` writes its 0-based physical index into the
//! corresponding `Button` slot and disarms the cell.
//!
//! This module owns:
//!
//! - The currently-armed target (one cell at most).
//! - A per-slot `(physical_tick, src_b_tick)` disarm counter the UI
//!   bumps to force a Slint `NumberCell` out of capture mode.
//!
//! It does **not** own:
//!
//! - The previous-tick `phy_button_data` bitmap. The caller passes
//!   `prev_phy` to [`Self::on_params_tick`] because the structured-event
//!   logger (`log_button_bitmap_edges`) also wants that bitmap and we
//!   don't want two copies racing each other.

use crate::wire::config::DeviceConfig;
use crate::wire::params::ParamsReport;
use crate::wire::{BUTTON_BITMAP_BYTES, MAX_BUTTONS_NUM};

/// Which Buttons-tab field is currently capturing physical-button
/// presses. `slot` is the wire slot (0..[`MAX_BUTTONS_NUM`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureTarget {
    /// `physical_num` cell on row `slot`.
    Physical(usize),
    /// `src_b` cell on the LOGIC sub-row of row `slot`.
    SrcB(usize),
}

impl CaptureTarget {
    /// The button slot this target is editing, regardless of which
    /// cell within the row.
    #[must_use]
    pub fn slot(self) -> usize {
        match self {
            Self::Physical(s) | Self::SrcB(s) => s,
        }
    }
}

/// Result of one [`ButtonCapture::on_params_tick`] call.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ButtonCaptureOutcome {
    /// `Some(slot)` if a press resolved the capture this tick — caller
    /// should refresh the button model for that row.
    pub captured_to_slot: Option<usize>,
    /// True if the captured press changed a `physical_num` / `src_b`
    /// value (not just confirmed the existing value). Caller flips
    /// dirty flags accordingly.
    pub config_changed: bool,
}

/// Button-capture state machine. Holds the arm bit + the per-slot
/// disarm-tick counters; everything else is passed in per call.
#[derive(Debug)]
pub struct ButtonCapture {
    armed: Option<CaptureTarget>,
    /// Per-slot disarm-tick pair: `.0` = physical cell tick, `.1` =
    /// Source B cell tick. The UI observes `changed disarm-tick` on
    /// the Slint side, so we bump these whenever the cell should
    /// drop out of capture mode.
    disarm_ticks: [(i32, i32); MAX_BUTTONS_NUM],
}

impl Default for ButtonCapture {
    fn default() -> Self {
        Self {
            armed: None,
            // Array literal — `Default::default()` only auto-derives
            // for arrays of length <= 32, but MAX_BUTTONS_NUM = 128.
            disarm_ticks: [(0, 0); MAX_BUTTONS_NUM],
        }
    }
}

impl ButtonCapture {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Arm a cell. Bumps the *previous* armed cell's disarm tick first
    /// so the UI tears down its capture state — only one cell can be
    /// armed at a time.
    pub fn arm(&mut self, target: CaptureTarget) {
        if let Some(prev) = self.armed.replace(target) {
            if prev != target {
                self.bump_disarm_tick(prev);
            }
        }
    }

    /// Disarm without writing anything. Bumps the disarm tick so the
    /// UI tears down its capture state. No-op if nothing was armed.
    pub fn disarm(&mut self) {
        if let Some(prev) = self.armed.take() {
            self.bump_disarm_tick(prev);
        }
    }

    /// Disarm without bumping the tick. Used on device disconnect
    /// where the cell is going away anyway — bumping would write to a
    /// dead UI handle.
    pub fn clear(&mut self) {
        self.armed = None;
    }

    #[must_use]
    pub fn armed(&self) -> Option<CaptureTarget> {
        self.armed
    }

    /// Disarm-tick pair for `slot`: `(physical_tick, src_b_tick)`.
    /// Fed directly into the Slint `NumberCell.disarm-tick` properties
    /// on each refresh.
    #[must_use]
    pub fn disarm_ticks(&self, slot: usize) -> (i32, i32) {
        self.disarm_ticks[slot]
    }

    /// Look at `params.phy_button_data` against `prev_phy`. If a rising
    /// edge appears and we're armed, write the pressed physical index
    /// into the armed cell and disarm.
    ///
    /// Returns the outcome — `captured_to_slot.is_some()` iff a press
    /// resolved capture this tick, regardless of whether the value
    /// actually changed (pressing the same button that's already
    /// configured still ends capture mode — it's a valid "yes, confirm
    /// this slot" gesture).
    pub fn on_params_tick(
        &mut self,
        params: &ParamsReport,
        prev_phy: &[u8; BUTTON_BITMAP_BYTES],
        cfg: &mut DeviceConfig,
    ) -> ButtonCaptureOutcome {
        let Some(target) = self.armed else {
            return ButtonCaptureOutcome::default();
        };
        let Some(phy) = first_rising_edge(params, prev_phy) else {
            return ButtonCaptureOutcome::default();
        };
        let phy_i8 = i8::try_from(phy).unwrap_or(i8::MAX);
        let config_changed = write_capture(cfg, target, phy_i8);

        // Rule (b): a press was detected → disarm regardless of whether
        // the value actually changed.
        self.armed = None;
        self.bump_disarm_tick(target);

        ButtonCaptureOutcome {
            captured_to_slot: Some(target.slot()),
            config_changed,
        }
    }

    fn bump_disarm_tick(&mut self, target: CaptureTarget) {
        let slot = target.slot();
        let entry = &mut self.disarm_ticks[slot];
        match target {
            CaptureTarget::Physical(_) => entry.0 = entry.0.wrapping_add(1),
            CaptureTarget::SrcB(_) => entry.1 = entry.1.wrapping_add(1),
        }
    }
}

/// Find the lowest-numbered physical slot whose bit is set in
/// `params.phy_button_data` but was unset in `prev_phy`. Returns the
/// 0-based physical index (= byte_idx * 8 + bit). Returns `None` if no
/// new presses this tick.
fn first_rising_edge(
    params: &ParamsReport,
    prev_phy: &[u8; BUTTON_BITMAP_BYTES],
) -> Option<usize> {
    for (byte_idx, (new, old)) in params
        .phy_button_data
        .iter()
        .zip(prev_phy.iter())
        .enumerate()
    {
        let edge = new & !old;
        if edge == 0 {
            continue;
        }
        for bit in 0..8u32 {
            if edge & (1u8 << bit) != 0 {
                let phy = byte_idx * 8 + bit as usize;
                if phy < MAX_BUTTONS_NUM {
                    return Some(phy);
                }
            }
        }
    }
    None
}

/// Write `phy_i8` into the cell `target` names. Returns `true` if the
/// value actually changed (not just confirmed the existing value).
fn write_capture(cfg: &mut DeviceConfig, target: CaptureTarget, phy_i8: i8) -> bool {
    match target {
        CaptureTarget::Physical(slot) => match cfg.buttons.get_mut(slot) {
            Some(b) if b.physical_num != phy_i8 => {
                b.physical_num = phy_i8;
                true
            }
            _ => false,
        },
        CaptureTarget::SrcB(slot) => match cfg.buttons.get_mut(slot) {
            Some(b) if b.src_b != phy_i8 => {
                b.src_b = phy_i8;
                true
            }
            _ => false,
        },
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

    /// Build a `ParamsReport` whose `phy_button_data` has only the
    /// given physical index set.
    fn press(phy: usize) -> ParamsReport {
        let mut p = empty_params();
        p.phy_button_data[phy / 8] = 1u8 << (phy % 8);
        p
    }

    const ZERO: [u8; BUTTON_BITMAP_BYTES] = [0u8; BUTTON_BITMAP_BYTES];

    #[test]
    fn idle_capture_does_nothing() {
        let mut cap = ButtonCapture::new();
        let mut cfg = empty_config();
        let out = cap.on_params_tick(&press(5), &ZERO, &mut cfg);
        assert_eq!(out, ButtonCaptureOutcome::default());
        assert_eq!(cfg.buttons[0].physical_num, 0);
    }

    #[test]
    fn arming_writes_physical_on_next_press() {
        let mut cap = ButtonCapture::new();
        let mut cfg = empty_config();
        cap.arm(CaptureTarget::Physical(3));

        let out = cap.on_params_tick(&press(7), &ZERO, &mut cfg);
        assert_eq!(out.captured_to_slot, Some(3));
        assert!(out.config_changed);
        assert_eq!(cfg.buttons[3].physical_num, 7);
        assert!(cap.armed().is_none(), "arm should clear on capture");
    }

    #[test]
    fn arming_writes_src_b_on_next_press() {
        let mut cap = ButtonCapture::new();
        let mut cfg = empty_config();
        cap.arm(CaptureTarget::SrcB(11));

        let out = cap.on_params_tick(&press(4), &ZERO, &mut cfg);
        assert_eq!(out.captured_to_slot, Some(11));
        assert!(out.config_changed);
        assert_eq!(cfg.buttons[11].src_b, 4);
    }

    #[test]
    fn same_value_press_still_disarms_but_reports_no_change() {
        let mut cap = ButtonCapture::new();
        let mut cfg = empty_config();
        cfg.buttons[2].physical_num = 9;
        cap.arm(CaptureTarget::Physical(2));

        let out = cap.on_params_tick(&press(9), &ZERO, &mut cfg);
        assert_eq!(out.captured_to_slot, Some(2));
        assert!(!out.config_changed);
        assert!(cap.armed().is_none());
    }

    #[test]
    fn no_edge_means_no_capture() {
        // A button held from the previous tick — not a rising edge.
        let mut cap = ButtonCapture::new();
        let mut cfg = empty_config();
        cap.arm(CaptureTarget::Physical(0));

        let mut prev = ZERO;
        prev[0] = 0b0000_0001; // bit 0 already set
        let out = cap.on_params_tick(&press(0), &prev, &mut cfg);
        assert_eq!(out, ButtonCaptureOutcome::default());
        assert!(cap.armed().is_some(), "still armed; no edge to consume");
    }

    #[test]
    fn arming_a_new_target_bumps_previous_disarm_tick() {
        let mut cap = ButtonCapture::new();
        cap.arm(CaptureTarget::Physical(5));
        let (before, _) = cap.disarm_ticks(5);

        cap.arm(CaptureTarget::Physical(8));
        let (after, _) = cap.disarm_ticks(5);
        assert_ne!(before, after, "previous cell's tick must bump");
    }

    #[test]
    fn disarm_bumps_tick_clear_does_not() {
        let mut cap = ButtonCapture::new();
        cap.arm(CaptureTarget::Physical(2));
        let (before, _) = cap.disarm_ticks(2);

        cap.disarm();
        let (after_disarm, _) = cap.disarm_ticks(2);
        assert_ne!(before, after_disarm, "disarm bumps tick");
        assert!(cap.armed().is_none());

        cap.arm(CaptureTarget::SrcB(2));
        let (_, before_src_b) = cap.disarm_ticks(2);
        cap.clear();
        let (_, after_clear) = cap.disarm_ticks(2);
        assert_eq!(
            before_src_b, after_clear,
            "clear must not bump tick (cell is gone)"
        );
        assert!(cap.armed().is_none());
    }

    #[test]
    fn capture_picks_lowest_edge_when_multiple_press() {
        let mut cap = ButtonCapture::new();
        let mut cfg = empty_config();
        cap.arm(CaptureTarget::Physical(0));

        let mut p = empty_params();
        p.phy_button_data[0] = 0b0000_0100; // bit 2 → physical 2
        p.phy_button_data[1] = 0b0000_0001; // bit 0 → physical 8
        let out = cap.on_params_tick(&p, &ZERO, &mut cfg);
        assert_eq!(out.captured_to_slot, Some(0));
        assert_eq!(cfg.buttons[0].physical_num, 2, "lowest physical wins");
    }
}
