//! Shared health-report types for the Cortex stack (phase8a).
//!
//! Every long-running Cortex binary exposes a `GET /healthz` returning
//! its own [`SubsystemStatus`]; cortex-api fans out across them and
//! returns the aggregated [`HealthReport`] at `GET /v1/health`. The
//! types live in their own crate so the per-binary handlers and the
//! aggregator can speak the same wire shape without dragging in
//! cortex-api's full dependency surface.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "server")]
pub mod server;

#[cfg(feature = "client")]
pub mod client;

use serde::{Deserialize, Serialize};

/// Health state of one subsystem.
///
/// Aggregator rule (in [`HealthReport::aggregate`]): `Down` wins over
/// `Degraded`, which wins over `Ok`. The CLI maps `Ok` → exit 0,
/// `Degraded` → exit 1, `Down` → exit 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthState {
    /// All probes within tolerance.
    Ok,
    /// Subsystem reachable but a soft signal is degraded
    /// (publisher stalled, archive refresh stale, queue lag, etc.).
    Degraded,
    /// Subsystem unreachable, panicked, or hard-fault.
    Down,
}

impl HealthState {
    /// Stable string identifier (matches the JSON enum tag).
    pub fn as_str(self) -> &'static str {
        match self {
            HealthState::Ok => "ok",
            HealthState::Degraded => "degraded",
            HealthState::Down => "down",
        }
    }

    /// Exit code mapping for `scripts/doctor/health.sh` / `scripts/doctor/health.bat`.
    pub fn exit_code(self) -> i32 {
        match self {
            HealthState::Ok => 0,
            HealthState::Degraded => 1,
            HealthState::Down => 2,
        }
    }

    /// `Ok < Degraded < Down`. Used by the aggregator's `max` reducer.
    pub fn rank(self) -> u8 {
        match self {
            HealthState::Ok => 0,
            HealthState::Degraded => 1,
            HealthState::Down => 2,
        }
    }
}

/// Per-subsystem health snapshot.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubsystemStatus {
    /// Subsystem identifier — matches the binary name (`cortex-api`,
    /// `cortex-adapter-claude`, `cortex-ingestion`, …) or the
    /// external dependency tag (`vectorizer`, `nexus`, `meili`,
    /// `synap`).
    pub name: String,
    /// Current health state.
    pub state: HealthState,
    /// Wall-clock latency of the most recent probe in milliseconds.
    /// `0` for self-reports; the aggregator stamps the actual probe
    /// latency.
    pub latency_ms: u64,
    /// Free-text reason when `state != Ok`. `None` on healthy
    /// reports keeps the wire shape lean.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Crate version (`env!("CARGO_PKG_VERSION")`).
    pub version: String,
    /// RFC-3339 timestamp the subsystem started serving.
    pub since: String,
    /// Crate-specific signals (publisher queue depth, WAL bytes,
    /// last-publish timestamp, archive-loader freshness, …). The
    /// aggregator passes this through verbatim so per-subsystem
    /// dashboards can read whatever the producer chose to emit.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    pub extras: serde_json::Map<String, serde_json::Value>,
}

impl SubsystemStatus {
    /// Build a healthy `Ok` status with the given name and version.
    /// Caller fills in `extras` and `since`.
    pub fn ok(
        name: impl Into<String>,
        version: impl Into<String>,
        since: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            state: HealthState::Ok,
            latency_ms: 0,
            last_error: None,
            version: version.into(),
            since: since.into(),
            extras: serde_json::Map::new(),
        }
    }

    /// Build a `Down` status carrying a one-line error reason. Used
    /// by the aggregator when a probe times out or returns a non-2xx.
    pub fn down(name: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            state: HealthState::Down,
            latency_ms: 0,
            last_error: Some(error.into()),
            version: String::new(),
            since: String::new(),
            extras: serde_json::Map::new(),
        }
    }
}

/// Aggregated health report — what `GET /v1/health` returns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthReport {
    /// Worst state across `subsystems`.
    pub overall: HealthState,
    /// Per-subsystem snapshots, sorted by `name` for stable output.
    pub subsystems: Vec<SubsystemStatus>,
    /// RFC-3339 timestamp the aggregation ran.
    pub checked_at: String,
}

impl HealthReport {
    /// Aggregate a list of subsystem snapshots into a single report.
    /// `overall` is the worst state; `checked_at` is RFC-3339 now.
    pub fn aggregate(mut subsystems: Vec<SubsystemStatus>) -> Self {
        subsystems.sort_by(|a, b| a.name.cmp(&b.name));
        let overall = subsystems
            .iter()
            .map(|s| s.state)
            .max_by_key(|s| s.rank())
            .unwrap_or(HealthState::Ok);
        Self {
            overall,
            subsystems,
            checked_at: chrono::Utc::now().to_rfc3339(),
        }
    }
}

/// Default admin/health port for the cortex-adapter-claude-code
/// daemon. Override via `CORTEX_ADAPTER_ADMIN_PORT` env var.
pub const DEFAULT_ADAPTER_ADMIN_PORT: u16 = 17011;

/// Threshold above which a freshness-driven subsystem is marked
/// `Degraded`. Any timestamp older than this and the subsystem is
/// reporting stale signals (last publish, last refresh, last batch).
///
/// 600 s (10 min) is the operator-friendly idle window: a worker
/// that just drained the bootstrap stream sits idle until the next
/// burst arrives, and reporting `Degraded` after 60 s painted the
/// dashboard red on every refresh after a successful drain. 10 min
/// is long enough to span typical idle gaps without masking a
/// genuinely stuck worker.
pub const DEFAULT_FRESHNESS_DEGRADED_SECS: u64 = 600;

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(name: &str) -> SubsystemStatus {
        SubsystemStatus::ok(name, "0.1.0", "2026-04-28T00:00:00Z")
    }

    fn degraded(name: &str) -> SubsystemStatus {
        let mut s = ok(name);
        s.state = HealthState::Degraded;
        s.last_error = Some("stalled".into());
        s
    }

    fn down(name: &str) -> SubsystemStatus {
        SubsystemStatus::down(name, "unreachable")
    }

    #[test]
    fn aggregate_ok_when_all_ok() {
        let r = HealthReport::aggregate(vec![ok("a"), ok("b")]);
        assert_eq!(r.overall, HealthState::Ok);
    }

    #[test]
    fn aggregate_degraded_when_any_degraded() {
        let r = HealthReport::aggregate(vec![ok("a"), degraded("b")]);
        assert_eq!(r.overall, HealthState::Degraded);
    }

    #[test]
    fn aggregate_down_wins_over_degraded() {
        let r = HealthReport::aggregate(vec![ok("a"), degraded("b"), down("c")]);
        assert_eq!(r.overall, HealthState::Down);
    }

    #[test]
    fn aggregate_empty_is_ok() {
        let r = HealthReport::aggregate(vec![]);
        assert_eq!(r.overall, HealthState::Ok);
    }

    #[test]
    fn aggregate_sorts_by_name() {
        let r = HealthReport::aggregate(vec![ok("z"), ok("a"), ok("m")]);
        let names: Vec<&str> = r.subsystems.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["a", "m", "z"]);
    }

    #[test]
    fn exit_codes_mapping() {
        assert_eq!(HealthState::Ok.exit_code(), 0);
        assert_eq!(HealthState::Degraded.exit_code(), 1);
        assert_eq!(HealthState::Down.exit_code(), 2);
    }

    #[test]
    fn rank_order_is_total() {
        assert!(HealthState::Ok.rank() < HealthState::Degraded.rank());
        assert!(HealthState::Degraded.rank() < HealthState::Down.rank());
    }

    #[test]
    fn aggregate_two_degraded_stays_degraded() {
        let r = HealthReport::aggregate(vec![degraded("a"), degraded("b")]);
        assert_eq!(r.overall, HealthState::Degraded);
    }

    #[test]
    fn aggregate_two_down_stays_down() {
        let r = HealthReport::aggregate(vec![down("a"), down("b")]);
        assert_eq!(r.overall, HealthState::Down);
    }

    #[test]
    fn aggregate_carries_subsystems_through() {
        let r = HealthReport::aggregate(vec![ok("a"), down("b")]);
        assert_eq!(r.subsystems.len(), 2);
        assert_eq!(r.subsystems[0].name, "a");
        assert_eq!(r.subsystems[1].state, HealthState::Down);
    }

    #[test]
    fn json_roundtrip_keeps_wire_shape() {
        let mut s = ok("x");
        s.extras.insert("queue_depth".into(), serde_json::json!(42));
        let serialized = serde_json::to_string(&s).unwrap();
        let parsed: SubsystemStatus = serde_json::from_str(&serialized).unwrap();
        assert_eq!(parsed.name, "x");
        assert_eq!(
            parsed.extras.get("queue_depth"),
            Some(&serde_json::json!(42))
        );
    }
}
