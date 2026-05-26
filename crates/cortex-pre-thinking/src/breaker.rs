//! Phase14e — pre-thinking circuit breaker.
//!
//! Pre-thinking's fail-open path returns `{ bundle: "", fail_open:
//! true }` whenever `cortex-api` is down or slow. Without a
//! breaker every call paid the full `total_budget` timeout before
//! falling open — burning latency the agent could not recover.
//! With a breaker, `threshold` consecutive failures inside
//! `window` flip the state to **Open**; subsequent calls
//! short-circuit instantly without waiting for the timeout. After
//! `cooldown` the breaker transitions to **HalfOpen**: the next
//! call gets a single probe budget; success closes the breaker,
//! another failure flips it back to Open.
//!
//! Defaults match the F-003 spec: 5 fails / 60 s window / 30 s
//! cooldown. Operator overrides land via [`BreakerConfig`].

use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// Reasons the pipeline tags a fail-open with. Mirrors the
/// `cortex_pre_thinking_fail_open_total{reason}` label set.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FailReason {
    /// Total budget elapsed before `cortex-api` answered.
    Timeout,
    /// Transport-layer error talking to `cortex-api`.
    Network,
    /// 401/403 from `cortex-api`.
    Unauthorised,
    /// Any other internal error path the pipeline classifies as
    /// non-recoverable (panic, schema mismatch, etc.).
    Internal,
    /// Breaker was Open at call time — request short-circuited
    /// without an upstream attempt. Recorded so the operator can
    /// see the breaker is doing its job.
    BreakerOpen,
}

impl FailReason {
    /// Stable lower-case label used in the metric + tracing fields.
    pub fn as_str(self) -> &'static str {
        match self {
            FailReason::Timeout => "timeout",
            FailReason::Network => "network",
            FailReason::Unauthorised => "unauthorised",
            FailReason::Internal => "internal",
            FailReason::BreakerOpen => "breaker_open",
        }
    }
}

/// Breaker state. Mirrors the canonical Closed → Open → HalfOpen →
/// {Closed | Open} cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BreakerState {
    /// Healthy — calls flow through normally.
    Closed,
    /// Tripped — calls short-circuit to fail-open with reason
    /// `breaker_open` until `cooldown` elapses.
    Open,
    /// Probe window — the next call is allowed through. Success
    /// closes; failure re-opens.
    HalfOpen,
}

/// Operator-tunable breaker thresholds.
#[derive(Debug, Clone, Copy)]
pub struct BreakerConfig {
    /// Number of failures within `window` that trip the breaker.
    pub threshold: u32,
    /// Sliding window the failure count is bucketed in.
    pub window: Duration,
    /// Time the breaker stays Open before transitioning to
    /// HalfOpen.
    pub cooldown: Duration,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        Self {
            threshold: 5,
            window: Duration::from_secs(60),
            cooldown: Duration::from_secs(30),
        }
    }
}

/// Permit returned by [`Breaker::guard`]. Callers MUST report the
/// outcome via [`Permit::record_success`] or
/// [`Permit::record_failure`] so the breaker stays consistent.
/// Dropping a permit without reporting leaks the in-flight tally
/// (the breaker treats the drop as a no-op success).
pub struct Permit<'a> {
    breaker: &'a Breaker,
    half_open_probe: bool,
}

impl Permit<'_> {
    /// Mark the call successful. Closes the breaker when in
    /// HalfOpen state.
    pub fn record_success(self) {
        if self.half_open_probe {
            self.breaker.close();
        }
    }
    /// Mark the call failed. Bumps the failure count + re-opens
    /// the breaker if the threshold trips (or if this was a
    /// HalfOpen probe).
    pub fn record_failure(self) -> Option<BreakerState> {
        let new = self.breaker.on_fail_internal(self.half_open_probe);
        Some(new)
    }
}

/// Error returned by [`Breaker::guard`] when the breaker is Open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("circuit breaker is open")]
pub struct BreakerOpen;

/// Pre-thinking circuit breaker. Cloneable via `Arc` — the inner
/// `Mutex` makes single-instance state cheap to share across the
/// pipeline + the doctor + the health endpoint.
#[derive(Debug)]
pub struct Breaker {
    config: BreakerConfig,
    inner: Mutex<Inner>,
}

#[derive(Debug)]
struct Inner {
    state: BreakerState,
    failures: u32,
    window_start: Instant,
    opened_at: Option<Instant>,
}

impl Breaker {
    /// Build a breaker with the default thresholds.
    pub fn new() -> Self {
        Self::with_config(BreakerConfig::default())
    }

    /// Build a breaker with operator-supplied thresholds.
    pub fn with_config(config: BreakerConfig) -> Self {
        Self {
            config,
            inner: Mutex::new(Inner {
                state: BreakerState::Closed,
                failures: 0,
                window_start: Instant::now(),
                opened_at: None,
            }),
        }
    }

    /// Snapshot the current breaker state.
    pub fn state(&self) -> BreakerState {
        self.refresh().state
    }

    /// Snapshot of the breaker's current configuration.
    pub fn config(&self) -> BreakerConfig {
        self.config
    }

    /// Snapshot the failures observed in the current window.
    /// Useful for the doctor command.
    pub fn failures_in_window(&self) -> u32 {
        self.refresh().failures
    }

    /// Try to acquire a permit. Returns [`BreakerOpen`] when the
    /// breaker is Open (cooldown not yet elapsed). When the
    /// breaker is HalfOpen, the returned permit carries the
    /// `half_open_probe` flag so the outcome closes or re-opens
    /// the breaker.
    pub fn guard(&self) -> Result<Permit<'_>, BreakerOpen> {
        let snapshot = self.refresh();
        match snapshot.state {
            BreakerState::Open => Err(BreakerOpen),
            BreakerState::HalfOpen => Ok(Permit {
                breaker: self,
                half_open_probe: true,
            }),
            BreakerState::Closed => Ok(Permit {
                breaker: self,
                half_open_probe: false,
            }),
        }
    }

    /// Test-only accessor: returns the current `Inner` snapshot
    /// for diagnostic logging.
    #[doc(hidden)]
    pub fn snapshot_inner(&self) -> (BreakerState, u32) {
        let snap = self.refresh();
        (snap.state, snap.failures)
    }

    fn refresh(&self) -> InnerSnap {
        let mut guard = self.inner.lock().expect("breaker mutex poisoned");
        let now = Instant::now();
        // Open → HalfOpen transition when cooldown has elapsed.
        if guard.state == BreakerState::Open {
            if let Some(at) = guard.opened_at {
                if now.saturating_duration_since(at) >= self.config.cooldown {
                    guard.state = BreakerState::HalfOpen;
                    tracing::warn!(
                        breaker = "pre-thinking",
                        from = "open",
                        to = "half_open",
                        "circuit breaker transitioned"
                    );
                }
            }
        }
        // Window roll-over while Closed resets the failure tally.
        if guard.state == BreakerState::Closed
            && now.saturating_duration_since(guard.window_start) >= self.config.window
        {
            guard.window_start = now;
            guard.failures = 0;
        }
        InnerSnap {
            state: guard.state,
            failures: guard.failures,
        }
    }

    fn on_fail_internal(&self, half_open_probe: bool) -> BreakerState {
        let mut guard = self.inner.lock().expect("breaker mutex poisoned");
        let now = Instant::now();
        if half_open_probe {
            // Half-open probe failed — straight back to Open.
            guard.state = BreakerState::Open;
            guard.opened_at = Some(now);
            tracing::warn!(
                breaker = "pre-thinking",
                from = "half_open",
                to = "open",
                "circuit breaker re-opened after half-open probe failed"
            );
            return BreakerState::Open;
        }
        // Roll the window if we crossed it.
        if now.saturating_duration_since(guard.window_start) >= self.config.window {
            guard.window_start = now;
            guard.failures = 0;
        }
        guard.failures = guard.failures.saturating_add(1);
        if guard.failures >= self.config.threshold && guard.state == BreakerState::Closed {
            guard.state = BreakerState::Open;
            guard.opened_at = Some(now);
            tracing::warn!(
                breaker = "pre-thinking",
                from = "closed",
                to = "open",
                failures = guard.failures,
                threshold = self.config.threshold,
                "circuit breaker tripped"
            );
        }
        guard.state
    }

    fn close(&self) {
        let mut guard = self.inner.lock().expect("breaker mutex poisoned");
        guard.state = BreakerState::Closed;
        guard.failures = 0;
        guard.window_start = Instant::now();
        guard.opened_at = None;
        tracing::info!(
            breaker = "pre-thinking",
            to = "closed",
            "circuit breaker closed after successful half-open probe"
        );
    }
}

impl Default for Breaker {
    fn default() -> Self {
        Self::new()
    }
}

struct InnerSnap {
    state: BreakerState,
    failures: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn fast_cfg() -> BreakerConfig {
        BreakerConfig {
            threshold: 3,
            window: Duration::from_millis(200),
            cooldown: Duration::from_millis(50),
        }
    }

    #[test]
    fn closed_breaker_short_circuits_after_threshold_burst() {
        let b = Breaker::with_config(fast_cfg());
        assert_eq!(b.state(), BreakerState::Closed);
        // 3 quick failures = threshold.
        for _ in 0..3 {
            let p = b.guard().expect("guard");
            p.record_failure();
        }
        assert_eq!(b.state(), BreakerState::Open);
        // 4th call short-circuits.
        assert!(b.guard().is_err());
    }

    #[test]
    fn open_breaker_short_circuits_without_calling_through() {
        let b = Breaker::with_config(fast_cfg());
        for _ in 0..3 {
            b.guard().unwrap().record_failure();
        }
        let r = b.guard();
        assert!(r.is_err());
        assert_eq!(b.state(), BreakerState::Open);
    }

    #[test]
    fn half_open_success_closes_breaker() {
        let cfg = fast_cfg();
        let b = Breaker::with_config(cfg);
        for _ in 0..3 {
            b.guard().unwrap().record_failure();
        }
        assert_eq!(b.state(), BreakerState::Open);
        std::thread::sleep(cfg.cooldown + Duration::from_millis(10));
        let p = b.guard().expect("half-open probe granted");
        p.record_success();
        assert_eq!(b.state(), BreakerState::Closed);
        assert_eq!(b.failures_in_window(), 0);
    }

    #[test]
    fn half_open_failure_re_opens_breaker() {
        let cfg = fast_cfg();
        let b = Breaker::with_config(cfg);
        for _ in 0..3 {
            b.guard().unwrap().record_failure();
        }
        std::thread::sleep(cfg.cooldown + Duration::from_millis(10));
        let p = b.guard().expect("half-open probe granted");
        let new_state = p.record_failure().unwrap();
        assert_eq!(new_state, BreakerState::Open);
        assert!(b.guard().is_err());
    }

    #[test]
    fn window_roll_over_resets_failure_count() {
        let cfg = fast_cfg();
        let b = Breaker::with_config(cfg);
        for _ in 0..2 {
            b.guard().unwrap().record_failure();
        }
        assert_eq!(b.failures_in_window(), 2);
        std::thread::sleep(cfg.window + Duration::from_millis(20));
        // refresh() rolls the window first.
        assert_eq!(b.failures_in_window(), 0);
        // 3 more failures DON'T trip yet because the window
        // just reset.
        for _ in 0..2 {
            b.guard().unwrap().record_failure();
        }
        assert_eq!(b.state(), BreakerState::Closed);
    }

    #[test]
    fn threshold_parametric_honoured() {
        let mut cfg = fast_cfg();
        cfg.threshold = 7;
        let b = Breaker::with_config(cfg);
        for _ in 0..6 {
            b.guard().unwrap().record_failure();
        }
        assert_eq!(b.state(), BreakerState::Closed);
        b.guard().unwrap().record_failure();
        assert_eq!(b.state(), BreakerState::Open);
    }

    #[test]
    fn fail_reason_labels_are_stable() {
        assert_eq!(FailReason::Timeout.as_str(), "timeout");
        assert_eq!(FailReason::Network.as_str(), "network");
        assert_eq!(FailReason::Unauthorised.as_str(), "unauthorised");
        assert_eq!(FailReason::Internal.as_str(), "internal");
        assert_eq!(FailReason::BreakerOpen.as_str(), "breaker_open");
    }

    #[test]
    fn breaker_state_serialises_lowercase() {
        let j = serde_json::to_string(&BreakerState::HalfOpen).unwrap();
        assert_eq!(j, "\"halfopen\"");
        let parsed: BreakerState = serde_json::from_str("\"open\"").unwrap();
        assert_eq!(parsed, BreakerState::Open);
    }

    #[test]
    fn arc_shared_breaker_is_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Arc<Breaker>>();
    }
}
