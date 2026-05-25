//! Aggregator client — fans out probes across `/healthz` URLs and
//! folds the results through [`crate::HealthReport::aggregate`].
//!
//! Used by `cortex-api`'s `/v1/health` route to assemble the stack-
//! wide report. Lives in this crate so the wire shape and the probe
//! contract stay co-located with the types they use.

use std::time::{Duration, Instant};

use crate::{HealthReport, HealthState, SubsystemStatus};

/// One probe target the aggregator should hit. `name` is the
/// subsystem identifier the aggregator stamps onto a `Down` status
/// when the probe fails (so the operator sees which subsystem
/// triggered the degradation, not just an opaque URL).
#[derive(Debug, Clone)]
pub struct ProbeTarget {
    /// Subsystem identifier (matches the binary name).
    pub name: &'static str,
    /// Full URL of the `/healthz` endpoint.
    pub url: String,
}

/// Tunables for the aggregator. Defaults match the spec
/// (1.5s per probe, 8 max concurrent, identifies via `cortex-aggregator`).
#[derive(Debug, Clone)]
pub struct AggregatorConfig {
    /// Per-probe timeout. Spec: 1.5s.
    pub probe_timeout: Duration,
    /// User-Agent header the aggregator sends so downstream logs
    /// can attribute the calls.
    pub user_agent: &'static str,
}

impl Default for AggregatorConfig {
    fn default() -> Self {
        Self {
            probe_timeout: Duration::from_millis(1_500),
            user_agent: "cortex-aggregator/1",
        }
    }
}

/// Probe one target. On any failure (timeout, transport, non-2xx,
/// JSON parse) returns `SubsystemStatus::down(name, reason)` so
/// the aggregator can fold it into the report instead of bailing
/// the whole call.
pub async fn probe_one(
    client: &reqwest::Client,
    target: &ProbeTarget,
    timeout: Duration,
) -> SubsystemStatus {
    let started = Instant::now();
    let result = tokio::time::timeout(timeout, async {
        let resp = client
            .get(&target.url)
            .send()
            .await
            .map_err(|e| format!("transport: {e}"))?;
        let status_code = resp.status();
        let body = resp.text().await.map_err(|e| format!("read body: {e}"))?;
        if !status_code.is_success() && status_code.as_u16() != 503 {
            // 503 carries a `Down` body — accept it. Other 4xx /
            // 5xx are surfaced as a probe failure so the aggregator
            // can mark the subsystem `Down` with a clear reason.
            return Err(format!("http {status_code}"));
        }
        let parsed: SubsystemStatus =
            serde_json::from_str(&body).map_err(|e| format!("json: {e}"))?;
        Ok::<SubsystemStatus, String>(parsed)
    })
    .await;
    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match result {
        Ok(Ok(mut status)) => {
            status.latency_ms = latency_ms;
            // Defensive: if the upstream forgot to stamp `name`,
            // fall back to the configured target name so the
            // dashboard never shows a blank row.
            if status.name.is_empty() {
                status.name = target.name.to_string();
            }
            status
        }
        Ok(Err(reason)) => {
            let mut s = SubsystemStatus::down(target.name, reason);
            s.latency_ms = latency_ms;
            s
        }
        Err(_) => {
            let mut s =
                SubsystemStatus::down(target.name, format!("timeout {}ms", timeout.as_millis()));
            s.latency_ms = latency_ms;
            s
        }
    }
}

/// Probe every target in parallel, aggregate the results into a
/// single [`HealthReport`]. Per-probe failures degrade the
/// individual subsystem but never fail the whole call.
pub async fn aggregate(
    client: &reqwest::Client,
    targets: &[ProbeTarget],
    config: &AggregatorConfig,
) -> HealthReport {
    if targets.is_empty() {
        return HealthReport::aggregate(Vec::new());
    }
    // Spawn each probe on the runtime — `reqwest::Client` and
    // `ProbeTarget` are cheap to clone (the client is internally an
    // `Arc`; the target carries owned strings) so the per-target
    // owned snapshot keeps each spawned task `'static`.
    let timeout = config.probe_timeout;
    let mut set = tokio::task::JoinSet::new();
    for (idx, target) in targets.iter().enumerate() {
        let client = client.clone();
        let target = target.clone();
        set.spawn(async move { (idx, probe_one(&client, &target, timeout).await) });
    }
    let mut indexed: Vec<(usize, SubsystemStatus)> = Vec::with_capacity(targets.len());
    while let Some(joined) = set.join_next().await {
        if let Ok((idx, snapshot)) = joined {
            indexed.push((idx, snapshot));
        }
    }
    indexed.sort_by_key(|(idx, _)| *idx);
    let snapshots: Vec<SubsystemStatus> = indexed.into_iter().map(|(_, s)| s).collect();
    HealthReport::aggregate(snapshots)
}

/// Build a reqwest client tuned for aggregator use (no per-request
/// timeout — the per-probe timeout in `probe_one` enforces it).
pub fn build_client(config: &AggregatorConfig) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .user_agent(config.user_agent)
        .build()
}

/// Trivial degraded-only aggregate path used when the aggregator
/// itself can't enumerate targets (e.g. all env vars unset). Emits
/// a single `Degraded` row so the operator sees an explicit "I
/// don't know who to ask" rather than a confusing "everything OK".
pub fn unknown_targets_report(name: &'static str, reason: impl Into<String>) -> HealthReport {
    let mut s = SubsystemStatus::ok(
        name,
        env!("CARGO_PKG_VERSION"),
        chrono::Utc::now().to_rfc3339(),
    );
    s.state = HealthState::Degraded;
    s.last_error = Some(reason.into());
    HealthReport::aggregate(vec![s])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aggregator_config_default_carries_spec_timeout() {
        let cfg = AggregatorConfig::default();
        assert_eq!(cfg.probe_timeout, Duration::from_millis(1_500));
        assert!(cfg.user_agent.starts_with("cortex-aggregator"));
    }

    #[tokio::test]
    async fn aggregate_with_no_targets_returns_empty_ok_report() {
        let client = build_client(&AggregatorConfig::default()).unwrap();
        let report = aggregate(&client, &[], &AggregatorConfig::default()).await;
        assert_eq!(report.overall, HealthState::Ok);
        assert!(report.subsystems.is_empty());
    }

    #[test]
    fn unknown_targets_report_emits_one_degraded_row() {
        let r = unknown_targets_report("cortex-aggregator", "no targets configured");
        assert_eq!(r.overall, HealthState::Degraded);
        assert_eq!(r.subsystems.len(), 1);
        assert_eq!(r.subsystems[0].state, HealthState::Degraded);
        assert_eq!(
            r.subsystems[0].last_error.as_deref(),
            Some("no targets configured")
        );
    }
}
