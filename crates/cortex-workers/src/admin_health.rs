//! Tiny shared helper that spawns the `/healthz` admin listener
//! every worker binary opens. Centralises the "name + port +
//! provider" wiring so each `bin/*.rs` is a one-liner.
//!
//! Used by the four worker binaries (classifier / embedder /
//! fulltext / graph). Each bin builds its own provider closure
//! (read from its metric registry) and calls
//! [`spawn_health_listener`] right before entering `run_pool`.
//!
//! Phase8a §2 — every long-running Cortex binary exposes a
//! `/healthz` returning [`cortex_health::SubsystemStatus`]. The
//! aggregator on cortex-api fans out across them at `/v1/health`.

use cortex_health::server::{
    serve_standalone, serve_standalone_with_metrics, MetricsRenderer, SnapshotProvider,
};

/// Spawn the admin `/healthz` listener on `port` for the given
/// `subsystem_name`. Failures (port already bound, OS error)
/// produce a `tracing::warn!` and the worker keeps running with
/// degraded observability — the CI relevance gate considers loss
/// of `/healthz` non-fatal so a crashed listener never takes down
/// the whole worker.
pub fn spawn_health_listener(
    port: u16,
    subsystem_name: &'static str,
    crate_version: &'static str,
    provider: SnapshotProvider,
) {
    let started_at = chrono::Utc::now().to_rfc3339();
    tokio::spawn(async move {
        if let Err(e) = serve_standalone(port, subsystem_name, crate_version, started_at, provider)
            .await
        {
            tracing::warn!(
                subsystem = subsystem_name,
                port,
                error = %e,
                "admin /healthz listener failed; worker continues without health endpoint"
            );
        }
    });
    tracing::info!(
        subsystem = subsystem_name,
        port,
        "admin /healthz listening on http://0.0.0.0:{port}/healthz"
    );
}

/// Phase8b — same as [`spawn_health_listener`] but also mounts a
/// Prometheus-text `/metrics` endpoint on the same port. Workers
/// pass their own renderer so the per-stage counters land in a
/// uniform scrape surface.
pub fn spawn_health_listener_with_metrics(
    port: u16,
    subsystem_name: &'static str,
    crate_version: &'static str,
    provider: SnapshotProvider,
    metrics: Option<MetricsRenderer>,
) {
    let started_at = chrono::Utc::now().to_rfc3339();
    let has_metrics = metrics.is_some();
    tokio::spawn(async move {
        if let Err(e) = serve_standalone_with_metrics(
            port,
            subsystem_name,
            crate_version,
            started_at,
            provider,
            metrics,
        )
        .await
        {
            tracing::warn!(
                subsystem = subsystem_name,
                port,
                error = %e,
                "admin /healthz listener failed; worker continues without health endpoint"
            );
        }
    });
    tracing::info!(
        subsystem = subsystem_name,
        port,
        metrics = has_metrics,
        "admin /healthz listening on http://0.0.0.0:{port}/healthz"
    );
}

/// Resolve the admin port from `env_var`, falling back to
/// `default_port` when unset / non-numeric / zero. The defaults
/// are chosen to land in the `1701x..1702x` block alongside the
/// ingestion + adapter ports so a single firewall rule covers
/// the whole stack.
pub fn resolve_port_from_env(env_var: &str, default_port: u16) -> u16 {
    std::env::var(env_var)
        .ok()
        .and_then(|s| s.trim().parse::<u16>().ok())
        .filter(|n| *n > 0)
        .unwrap_or(default_port)
}

/// Default admin port for the classifier worker.
pub const DEFAULT_CLASSIFIER_PORT: u16 = 17021;
/// Default admin port for the embedder worker.
pub const DEFAULT_EMBEDDER_PORT: u16 = 17022;
/// Default admin port for the fulltext worker.
pub const DEFAULT_FULLTEXT_PORT: u16 = 17023;
/// Default admin port for the graph worker.
pub const DEFAULT_GRAPH_PORT: u16 = 17024;

/// Trivial health helpers exposed for test use — surface the
/// `(state, last_error)` rule the workers share so the four
/// binaries don't reinvent the threshold check.
pub mod rules {
    use cortex_health::{HealthState, DEFAULT_FRESHNESS_DEGRADED_SECS};

    /// Compute `(state, last_error)` from a "last successful
    /// activity" Unix-epoch ms timestamp + an optional explicit
    /// hard-down condition. Returns `Down` when `down_reason` is
    /// `Some(_)`, `Degraded` when the timestamp is older than
    /// [`cortex_health::DEFAULT_FRESHNESS_DEGRADED_SECS`], and
    /// `Ok` otherwise.
    pub fn freshness_state(
        last_activity_ms: u64,
        down_reason: Option<String>,
    ) -> (HealthState, Option<String>) {
        if let Some(reason) = down_reason {
            return (HealthState::Down, Some(reason));
        }
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        if last_activity_ms == 0 {
            // No activity yet — soft degraded with an explicit
            // "warming up" message so operators can tell apart
            // "never started" from "stalled".
            return (
                HealthState::Degraded,
                Some("no activity recorded yet (warming up)".into()),
            );
        }
        if now_ms.saturating_sub(last_activity_ms)
            > DEFAULT_FRESHNESS_DEGRADED_SECS * 1_000
        {
            return (
                HealthState::Degraded,
                Some(format!(
                    "no activity in last {} secs",
                    DEFAULT_FRESHNESS_DEGRADED_SECS
                )),
            );
        }
        (HealthState::Ok, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_port_falls_back_when_env_missing() {
        std::env::remove_var("CORTEX_TEST_HEALTH_PORT_AAA");
        assert_eq!(resolve_port_from_env("CORTEX_TEST_HEALTH_PORT_AAA", 17099), 17099);
    }

    #[test]
    fn resolve_port_honours_valid_env_override() {
        std::env::set_var("CORTEX_TEST_HEALTH_PORT_BBB", "17777");
        assert_eq!(resolve_port_from_env("CORTEX_TEST_HEALTH_PORT_BBB", 17000), 17777);
        std::env::remove_var("CORTEX_TEST_HEALTH_PORT_BBB");
    }

    #[test]
    fn resolve_port_falls_back_on_garbage() {
        std::env::set_var("CORTEX_TEST_HEALTH_PORT_CCC", "abc");
        assert_eq!(resolve_port_from_env("CORTEX_TEST_HEALTH_PORT_CCC", 17000), 17000);
        std::env::remove_var("CORTEX_TEST_HEALTH_PORT_CCC");
    }

    #[test]
    fn resolve_port_falls_back_on_zero() {
        std::env::set_var("CORTEX_TEST_HEALTH_PORT_DDD", "0");
        assert_eq!(resolve_port_from_env("CORTEX_TEST_HEALTH_PORT_DDD", 17000), 17000);
        std::env::remove_var("CORTEX_TEST_HEALTH_PORT_DDD");
    }

    #[test]
    fn freshness_state_down_wins_over_freshness_check() {
        let (state, err) = rules::freshness_state(0, Some("backend gone".into()));
        assert_eq!(state, cortex_health::HealthState::Down);
        assert_eq!(err.as_deref(), Some("backend gone"));
    }

    #[test]
    fn freshness_state_warming_up_when_no_activity_yet() {
        let (state, err) = rules::freshness_state(0, None);
        assert_eq!(state, cortex_health::HealthState::Degraded);
        assert!(err.unwrap().contains("warming up"));
    }

    #[test]
    fn freshness_state_ok_when_recent_activity() {
        let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
        let (state, err) = rules::freshness_state(now_ms, None);
        assert_eq!(state, cortex_health::HealthState::Ok);
        assert!(err.is_none());
    }

    #[test]
    fn freshness_state_degraded_when_stale() {
        let stale = chrono::Utc::now()
            .checked_sub_signed(chrono::Duration::seconds(120))
            .unwrap()
            .timestamp_millis()
            .max(0) as u64;
        let (state, err) = rules::freshness_state(stale, None);
        assert_eq!(state, cortex_health::HealthState::Degraded);
        assert!(err.unwrap().contains("no activity"));
    }
}
