//! Owns the params-streaming subscription with the firmware.
//!
//! The firmware silently stops pushing `REPORT_ID_PARAMS` reports if it
//! hasn't seen a `request_params()` within ~5 s, and the subscription
//! is also disturbed by any config exchange (config and params share
//! the same EP on F103 and the firmware drains its request queue on
//! every state change). This module concentrates the four resulting
//! renewal moments so the pump loop stops repeating itself:
//!
//! 1. Initial subscribe after [`Device::open`].
//! 2. Periodic renewal every [`PARAMS_REQUEST_REFRESH`] while idle.
//! 3. Post-`ReadConfig` re-subscribe.
//! 4. Post-`WriteConfig` re-subscribe.
//!
//! All four call [`Transport::request_params`]; what differs is the
//! failure policy — see [`RenewalReason`].

use std::time::{Duration, Instant};

use tracing::debug;

use crate::transport::Transport;
use crate::TransportError;

/// Cadence for refreshing the firmware's params subscription. The
/// firmware stops pushing params reports if it doesn't see a renewal
/// within ~5 seconds. Matches `hiddevice.cpp:360` (the 5000 ms timer)
/// in the Qt configurator.
pub(crate) const PARAMS_REQUEST_REFRESH: Duration = Duration::from_secs(5);

/// Why the pump is renewing the subscription. Determines failure policy.
///
/// Every reason except [`AfterWrite`](RenewalReason::AfterWrite) treats
/// transport errors as device-lost. `AfterWrite` swallows them because
/// the firmware re-enumerates after a successful write and the renewal
/// races that re-enum; the next [`Periodic`](RenewalReason::Periodic)
/// tick retries cleanly if the device is still present.
#[derive(Debug, Clone, Copy)]
pub(crate) enum RenewalReason {
    /// 5 s periodic tick. Idempotent — no-op before the deadline.
    Periodic,
    /// A `ReadConfig` exchange just completed.
    AfterRead,
    /// A `WriteConfig` exchange just completed. Failures are expected.
    AfterWrite,
}

/// Result of a renewal attempt.
#[derive(Debug)]
#[must_use]
pub(crate) enum RenewalOutcome {
    /// Subscription is healthy (or the failure was benign per
    /// [`RenewalReason::AfterWrite`]).
    Active,
    /// Renewal failed in a way the caller should treat as disconnect.
    Lost(TransportError),
}

/// Owns the periodic-renewal deadline for the params subscription.
///
/// Construct via [`subscribe`](Self::subscribe), which sends the initial
/// `request_params()` and arms the deadline. The type system then
/// prevents a caller from forgetting the initial subscribe — there is
/// no other way to obtain an instance.
#[derive(Debug)]
pub(crate) struct ParamsSubscription {
    next_renewal: Instant,
}

impl ParamsSubscription {
    /// Subscribe at connect time. Sends the initial `request_params()`
    /// and returns the subscription with the periodic deadline armed.
    /// Caller treats `Err` here as "never connected successfully".
    ///
    /// # Errors
    ///
    /// Returns [`TransportError`] verbatim from
    /// [`Transport::request_params`].
    pub fn subscribe(
        now: Instant,
        transport: &mut dyn Transport,
    ) -> Result<Self, TransportError> {
        transport.request_params()?;
        Ok(Self {
            next_renewal: now + PARAMS_REQUEST_REFRESH,
        })
    }

    /// Renew the subscription. `Periodic` is a no-op before the
    /// deadline; every other reason always sends. The deadline resets
    /// on every successful send. `AfterWrite` swallows errors after a
    /// `tracing::debug!`.
    pub fn renew(
        &mut self,
        now: Instant,
        reason: RenewalReason,
        transport: &mut dyn Transport,
    ) -> RenewalOutcome {
        if matches!(reason, RenewalReason::Periodic) && now < self.next_renewal {
            return RenewalOutcome::Active;
        }
        match transport.request_params() {
            Ok(()) => {
                self.next_renewal = now + PARAMS_REQUEST_REFRESH;
                RenewalOutcome::Active
            }
            Err(e) => match reason {
                RenewalReason::AfterWrite => {
                    debug!("re-subscribe after write_config failed (expected on re-enum): {e}");
                    RenewalOutcome::Active
                }
                RenewalReason::Periodic | RenewalReason::AfterRead => RenewalOutcome::Lost(e),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::Transport;
    use freejoyx_core::wire::{DeviceConfig, ParamsReport};
    use std::cell::Cell;

    /// Minimal scriptable [`Transport`] for subscription tests. Records
    /// every `request_params` call and lets the test inject the next
    /// result. The read/config methods are unused here and panic if
    /// touched — keeps the fake honest about its scope.
    struct RecordingTransport {
        calls: Cell<u32>,
        next: Cell<Result<(), TransportError>>,
    }

    impl RecordingTransport {
        fn new() -> Self {
            Self {
                calls: Cell::new(0),
                next: Cell::new(Ok(())),
            }
        }
        fn fail_next(&self) {
            self.next.set(Err(TransportError::Timeout { ms: 0 }));
        }
        fn calls(&self) -> u32 {
            self.calls.get()
        }
    }

    impl Transport for RecordingTransport {
        fn request_params(&self) -> Result<(), TransportError> {
            self.calls.set(self.calls.get() + 1);
            self.next.replace(Ok(()))
        }
        fn read_params_blocking(
            &mut self,
            _timeout: Duration,
        ) -> Result<ParamsReport, TransportError> {
            unreachable!("subscription tests must not read params")
        }
        fn read_config(&self) -> Result<Box<DeviceConfig>, TransportError> {
            unreachable!("subscription tests must not touch config")
        }
        fn write_config(&self, _cfg: &DeviceConfig) -> Result<(), TransportError> {
            unreachable!("subscription tests must not touch config")
        }
    }

    #[test]
    fn subscribe_sends_request_and_returns_ok() {
        let mut t = RecordingTransport::new();
        let now = Instant::now();
        let _sub = ParamsSubscription::subscribe(now, &mut t).expect("ok");
        assert_eq!(t.calls(), 1);
    }

    #[test]
    fn subscribe_propagates_transport_error() {
        let mut t = RecordingTransport::new();
        t.fail_next();
        let now = Instant::now();
        let err = ParamsSubscription::subscribe(now, &mut t).expect_err("must fail");
        assert!(matches!(err, TransportError::Timeout { .. }));
        assert_eq!(t.calls(), 1);
    }

    #[test]
    fn periodic_before_deadline_is_no_op() {
        let mut t = RecordingTransport::new();
        let now = Instant::now();
        let mut sub = ParamsSubscription::subscribe(now, &mut t).unwrap();
        assert_eq!(t.calls(), 1);

        // Still well before the next deadline.
        let outcome = sub.renew(now + Duration::from_millis(100), RenewalReason::Periodic, &mut t);
        assert!(matches!(outcome, RenewalOutcome::Active));
        assert_eq!(t.calls(), 1, "no extra request_params before deadline");
    }

    #[test]
    fn periodic_after_deadline_sends_and_resets_timer() {
        let mut t = RecordingTransport::new();
        let now = Instant::now();
        let mut sub = ParamsSubscription::subscribe(now, &mut t).unwrap();

        let after = now + PARAMS_REQUEST_REFRESH + Duration::from_millis(1);
        let outcome = sub.renew(after, RenewalReason::Periodic, &mut t);
        assert!(matches!(outcome, RenewalOutcome::Active));
        assert_eq!(t.calls(), 2);

        // Immediately after the renewal, a fresh Periodic call must not
        // fire again — the deadline reset.
        let outcome = sub.renew(after + Duration::from_millis(1), RenewalReason::Periodic, &mut t);
        assert!(matches!(outcome, RenewalOutcome::Active));
        assert_eq!(t.calls(), 2);
    }

    #[test]
    fn periodic_failure_returns_lost_and_does_not_reset_timer() {
        let mut t = RecordingTransport::new();
        let now = Instant::now();
        let mut sub = ParamsSubscription::subscribe(now, &mut t).unwrap();

        let after = now + PARAMS_REQUEST_REFRESH + Duration::from_millis(1);
        t.fail_next();
        let outcome = sub.renew(after, RenewalReason::Periodic, &mut t);
        assert!(matches!(outcome, RenewalOutcome::Lost(_)));

        // Deadline should not have been pushed forward by a failed send,
        // so the next Periodic call (with the transport now healthy)
        // retries immediately.
        let outcome = sub.renew(after + Duration::from_millis(1), RenewalReason::Periodic, &mut t);
        assert!(matches!(outcome, RenewalOutcome::Active));
        assert_eq!(t.calls(), 3, "retry happens on the very next periodic tick");
    }

    #[test]
    fn after_read_always_sends() {
        let mut t = RecordingTransport::new();
        let now = Instant::now();
        let mut sub = ParamsSubscription::subscribe(now, &mut t).unwrap();

        // Well before deadline — Periodic would skip, AfterRead must not.
        let outcome = sub.renew(now + Duration::from_millis(50), RenewalReason::AfterRead, &mut t);
        assert!(matches!(outcome, RenewalOutcome::Active));
        assert_eq!(t.calls(), 2);
    }

    #[test]
    fn after_read_failure_returns_lost() {
        let mut t = RecordingTransport::new();
        let now = Instant::now();
        let mut sub = ParamsSubscription::subscribe(now, &mut t).unwrap();

        t.fail_next();
        let outcome = sub.renew(now + Duration::from_millis(50), RenewalReason::AfterRead, &mut t);
        assert!(matches!(outcome, RenewalOutcome::Lost(_)));
    }

    #[test]
    fn after_write_failure_returns_active() {
        let mut t = RecordingTransport::new();
        let now = Instant::now();
        let mut sub = ParamsSubscription::subscribe(now, &mut t).unwrap();

        t.fail_next();
        let outcome = sub.renew(now + Duration::from_millis(50), RenewalReason::AfterWrite, &mut t);
        assert!(
            matches!(outcome, RenewalOutcome::Active),
            "AfterWrite must swallow renewal failure during device re-enum",
        );
    }

    #[test]
    fn after_write_success_resets_timer() {
        let mut t = RecordingTransport::new();
        let now = Instant::now();
        let mut sub = ParamsSubscription::subscribe(now, &mut t).unwrap();

        // Push deadline forward via a successful AfterWrite at t=2s in.
        let at = now + Duration::from_secs(2);
        let outcome = sub.renew(at, RenewalReason::AfterWrite, &mut t);
        assert!(matches!(outcome, RenewalOutcome::Active));
        assert_eq!(t.calls(), 2);

        // A Periodic at t=2s+4s = 6s in must NOT send (deadline is at 2s + 5s = 7s).
        let outcome = sub.renew(at + Duration::from_secs(4), RenewalReason::Periodic, &mut t);
        assert!(matches!(outcome, RenewalOutcome::Active));
        assert_eq!(t.calls(), 2, "AfterWrite success rearmed the periodic deadline");
    }
}
