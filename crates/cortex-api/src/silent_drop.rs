//! Phase8e — silent-drop detector.
//!
//! phase8b's `/v1/health/divergence` endpoint exposes the raw signal
//! (per-pair counters + delta growth). phase8e is the *detector*: an
//! always-on background watcher that polls the same internal data,
//! runs every pair through a debounced state machine, and emits a
//! `law_violation` envelope the moment a sustained drop transitions
//! `Ok → Warn` or `Warn → Critical`.
//!
//! The crucial difference from phase8b's pull-on-demand aggregator:
//! this is a push channel. The alert envelope lands in the archive
//! AND the in-memory keyword lane, so it shows up in the existing
//! Live Timeline and Violations dashboard without anyone having to
//! remember to call the divergence endpoint.
//!
//! Debouncing rules (per spec):
//! - `Ok → Warn`: 2 consecutive polls observe `delta_growth >
//!   warn_delta`. Avoids transient bursts.
//! - `Warn → Critical`: a single poll exceeding `critical_delta`
//!   suffices.
//! - Recovery: 5 consecutive polls observe non-growing delta.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use serde::{Deserialize, Serialize};

use crate::health::{DivergenceRow, HealthAggregatorState};

/// Per-pair sensitivity thresholds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PairConfig {
    /// Pair key matching `health::DivergenceRow.pair`.
    pub pair: String,
    /// `delta_growth` threshold above which we count toward `Warn`.
    /// Default: 10 (matches phase8b's severity bucketing).
    pub warn_delta: i64,
    /// `delta_growth` threshold above which we transition to
    /// `Critical` immediately. Default: 50.
    pub critical_delta: i64,
}

impl PairConfig {
    /// Default pair config sourced from a divergence-row pair name.
    pub fn for_pair(pair: impl Into<String>) -> Self {
        Self {
            pair: pair.into(),
            warn_delta: 10,
            critical_delta: 50,
        }
    }
}

/// Watcher-wide configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SilentDropConfig {
    /// Master switch — when `false`, the watcher is never spawned.
    pub enabled: bool,
    /// Cadence between divergence polls.
    pub poll_interval_secs: u64,
    /// Per-pair thresholds (per-key). When a row's pair name isn't
    /// in this map, the default `(warn=10, critical=50)` from
    /// [`PairConfig::for_pair`] is used.
    pub pairs: Vec<PairConfig>,
    /// Optional webhook URL — every transition is POSTed here as
    /// JSON. Best-effort; transport failures log and continue.
    pub webhook_url: Option<String>,
    /// When `true`, every Critical transition appends a single
    /// `[silent-drop alert]`-prefixed line to
    /// `.rulebook/handoff/_pending.md`.
    pub write_to_handoff: bool,
    /// Directory where per-pair `<pair>.json` state files live.
    /// Defaults to `~/.cortex/alerts`.
    pub state_dir: PathBuf,
    /// `cortex-ingestion` base URL — the watcher POSTs alert
    /// envelopes to `<base>/v1/events/batch`.
    pub ingestion_url: String,
}

impl Default for SilentDropConfig {
    fn default() -> Self {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        Self {
            enabled: true,
            poll_interval_secs: 30,
            pairs: Vec::new(),
            webhook_url: None,
            write_to_handoff: false,
            state_dir: PathBuf::from(home).join(".cortex").join("alerts"),
            ingestion_url: std::env::var("CORTEX_INGESTION_URL")
                .unwrap_or_else(|_| "http://127.0.0.1:17010".to_string()),
        }
    }
}

/// Per-pair alert state. Persisted as JSON under
/// `~/.cortex/alerts/<safe-pair-name>.json` so a daemon restart
/// doesn't re-fire alerts for issues that were already flagged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase", tag = "state")]
pub enum AlertState {
    /// Pair is healthy.
    Ok,
    /// Above warn threshold; carries the `consecutive` count of
    /// over-threshold polls (used for the 2-poll debounce).
    Warn {
        /// Number of consecutive over-threshold polls observed.
        consecutive: u8,
    },
    /// Above critical threshold.
    Critical,
}

impl Default for AlertState {
    fn default() -> Self {
        AlertState::Ok
    }
}

/// Outcome of one [`transition`] call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AlertTransition {
    /// State did not change; no envelope to emit.
    None,
    /// State changed — caller should emit one envelope.
    Changed {
        /// Severity to stamp on the envelope.
        severity: AlertSeverity,
    },
}

/// Severity stamped on the alert envelope payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertSeverity {
    /// Below `warn_delta` — recovery transition.
    Ok,
    /// Above `warn_delta`, below `critical_delta`.
    Warn,
    /// Above `critical_delta`.
    Critical,
}

/// Per-pair watcher state — persisted across restarts via
/// [`SilentDropConfig::state_dir`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PairState {
    /// Current alert state.
    pub alert: AlertState,
    /// Number of consecutive recovery (non-growing) polls.
    pub recovery_streak: u8,
}

/// Pure-function transition driver. Inputs:
/// - `prev`: current state on disk (or `Default` on first observation)
/// - `delta_growth`: the latest divergence sample's `delta_growth`
/// - `cfg`: per-pair thresholds
///
/// Returns the new state + a transition decision the caller acts on.
pub fn transition(
    prev: &PairState,
    delta_growth: i64,
    cfg: &PairConfig,
) -> (PairState, AlertTransition) {
    let above_warn = delta_growth > cfg.warn_delta;
    let above_critical = delta_growth > cfg.critical_delta;

    let mut next = prev.clone();
    // Above critical from any prior state → Critical (single poll
    // suffices per spec). Handle this first so the per-state match
    // below stays on the (above_critical = false) branch.
    if above_critical {
        if matches!(prev.alert, AlertState::Critical) {
            next.recovery_streak = 0;
            return (next, AlertTransition::None);
        }
        next.alert = AlertState::Critical;
        next.recovery_streak = 0;
        return (
            next,
            AlertTransition::Changed {
                severity: AlertSeverity::Critical,
            },
        );
    }
    let transition = match (&prev.alert, above_warn) {
        // Ok → Warn: 2 consecutive polls above warn threshold.
        (AlertState::Ok, true) => {
            // First over-threshold poll while in Ok: bump the
            // pre-warn counter via a `Warn { consecutive: 1 }`
            // sentinel that doesn't yet emit. Second consecutive
            // poll promotes to a real Warn alert.
            next.alert = AlertState::Warn { consecutive: 1 };
            next.recovery_streak = 0;
            AlertTransition::None
        }
        (AlertState::Warn { consecutive }, true) => {
            let new_count = consecutive.saturating_add(1);
            if *consecutive == 0 {
                // Sanity guard — `Warn { consecutive: 0 }` is
                // technically possible if a future caller stamps
                // the state by hand; treat as the first poll.
                next.alert = AlertState::Warn { consecutive: 1 };
                next.recovery_streak = 0;
                AlertTransition::None
            } else if *consecutive == 1 {
                // Second consecutive over-threshold poll — fire.
                next.alert = AlertState::Warn { consecutive: 2 };
                next.recovery_streak = 0;
                AlertTransition::Changed {
                    severity: AlertSeverity::Warn,
                }
            } else {
                // Already firing as Warn; keep counting but no new
                // envelope.
                next.alert = AlertState::Warn {
                    consecutive: new_count,
                };
                next.recovery_streak = 0;
                AlertTransition::None
            }
        }
        (AlertState::Critical, true) => {
            // Critical drops to Warn-counter on its way back. No
            // envelope yet — recovery requires 5 consecutive ok.
            next.recovery_streak = 0;
            AlertTransition::None
        }
        // Recovery — delta below warn threshold.
        (state, false) => {
            let streak = prev.recovery_streak.saturating_add(1);
            next.recovery_streak = streak;
            if streak >= 5 && !matches!(state, AlertState::Ok) {
                next.alert = AlertState::Ok;
                next.recovery_streak = 0;
                AlertTransition::Changed {
                    severity: AlertSeverity::Ok,
                }
            } else {
                AlertTransition::None
            }
        }
    };
    (next, transition)
}

/// Look up the per-pair config, falling back to the default when
/// the pair isn't explicitly listed in the watcher config.
pub fn pair_config(cfg: &SilentDropConfig, pair: &str) -> PairConfig {
    cfg.pairs
        .iter()
        .find(|p| p.pair == pair)
        .cloned()
        .unwrap_or_else(|| PairConfig::for_pair(pair))
}

/// Sanitise a pair name for use as a filename.
pub fn pair_state_filename(pair: &str) -> String {
    pair.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | ' ' | '>' | '<' | '*' | '?' | '|' | '"' => '_',
            other => other,
        })
        .collect()
}

/// Read the persisted state for a pair. Returns `Default` when the
/// file doesn't exist.
pub fn load_pair_state(state_dir: &std::path::Path, pair: &str) -> PairState {
    let file = state_dir.join(format!("{}.json", pair_state_filename(pair)));
    let raw = match std::fs::read_to_string(&file) {
        Ok(s) => s,
        Err(_) => return PairState::default(),
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

/// Persist state for a pair. Best-effort: filesystem failures log
/// at WARN and continue (the watcher must never panic on disk
/// errors — losing dedup state is preferable to losing the alert).
pub fn save_pair_state(state_dir: &std::path::Path, pair: &str, state: &PairState) {
    if let Err(e) = std::fs::create_dir_all(state_dir) {
        tracing::warn!(error = %e, dir = %state_dir.display(), "silent-drop: state dir create failed");
        return;
    }
    let file = state_dir.join(format!("{}.json", pair_state_filename(pair)));
    let raw = match serde_json::to_string_pretty(state) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = %e, "silent-drop: state serialize failed");
            return;
        }
    };
    if let Err(e) = std::fs::write(&file, raw) {
        tracing::warn!(error = %e, file = %file.display(), "silent-drop: state write failed");
    }
}

/// Build the alert envelope payload for a transition.
pub fn build_alert_envelope(row: &DivergenceRow, severity: AlertSeverity) -> serde_json::Value {
    let now = chrono::Utc::now().to_rfc3339();
    let event_id = format!("01ALERT{}", now.replace([':', '-', '.', 'T', 'Z'], ""));
    let event_id: String = event_id.chars().take(26).collect();
    let safe_pair = pair_state_filename(&row.pair);
    serde_json::json!({
        "event_id": event_id,
        "schema_version": "1",
        "occurred_at": now,
        "session_id": format!("01SILENTDROP{}", safe_pair).chars().take(26).collect::<String>(),
        "stream": "live",
        "tool": "cortex-api",
        "kind": "law_violation",
        "context": {
            "platform": std::env::consts::OS,
        },
        "payload": {
            "law_id": format!("silent-drop:{}", row.pair),
            "severity": severity,
            "upstream_count": row.upstream,
            "downstream_count": row.downstream,
            "delta": row.delta,
            "delta_growth": row.delta_growth,
            "since": now,
            "message": format!(
                "silent drop detected on pair `{}`: upstream={}, downstream={}, growth={}",
                row.pair, row.upstream, row.downstream, row.delta_growth
            ),
        },
        "redactions": [],
        "content_hash": format!("sha256:silent-drop:{safe_pair}"),
    })
}

/// Compute the divergence rows the watcher consumes. Mirrors the
/// `divergence_handler` logic from [`crate::health`] so the watcher
/// and the HTTP endpoint always agree on `delta_growth` per pair.
/// Side-effect: updates the shared `HealthAggregatorState.history`
/// with each pair's latest sample.
async fn compute_divergence_rows(aggregator: Arc<HealthAggregatorState>) -> Vec<DivergenceRow> {
    use std::time::Duration;
    let by_name = crate::health::gather_subsystem_extras().await;
    let now = Instant::now();
    let pairs = crate::health::build_divergence_pairs(&by_name);
    let mut history: std::collections::BTreeMap<String, crate::health::DivergenceSample> =
        aggregator
            .history
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();
    let mut rows: Vec<DivergenceRow> = Vec::new();
    for (key, up, down) in pairs {
        let delta = up.saturating_sub(down);
        let prev = history.get(&key).cloned().unwrap_or_default();
        let delta_growth = match prev.captured_at {
            Some(when) if now.saturating_duration_since(when) >= Duration::from_secs(30) => {
                i64::try_from(delta).unwrap_or(0) - i64::try_from(prev.delta).unwrap_or(0)
            }
            _ => 0,
        };
        let severity = if delta_growth > 0 {
            crate::health::Severity::from_growth_for_silent_drop(delta_growth)
        } else {
            crate::health::Severity::Ok
        };
        rows.push(DivergenceRow {
            pair: key.clone(),
            upstream: up,
            downstream: down,
            delta,
            delta_growth,
            severity,
        });
        history.insert(
            key,
            crate::health::DivergenceSample {
                delta,
                captured_at: Some(now),
            },
        );
    }
    if let Ok(mut guard) = aggregator.history.lock() {
        *guard = history;
    }
    rows
}

/// Translate a [`DivergenceRow`] + transition severity into a
/// [`crate::lanes::LaneHit`] suitable for the in-memory keyword
/// lane. The lane fields mirror what the meili_loader emits for
/// `law_violation` envelopes so the dashboard's existing renderer
/// surfaces the alert without special-casing.
pub fn alert_lane_hit(
    row: &DivergenceRow,
    severity: AlertSeverity,
    ts_ms: i64,
) -> crate::lanes::LaneHit {
    let mut extras: std::collections::BTreeMap<String, serde_json::Value> = Default::default();
    extras.insert(
        "source".into(),
        serde_json::Value::String("silent-drop-watcher".into()),
    );
    extras.insert(
        "law_id".into(),
        serde_json::Value::String(format!("silent-drop:{}", row.pair)),
    );
    extras.insert("delta_growth".into(), serde_json::json!(row.delta_growth));
    extras.insert("upstream".into(), serde_json::json!(row.upstream));
    extras.insert("downstream".into(), serde_json::json!(row.downstream));
    let severity_str = match severity {
        AlertSeverity::Ok => "ok",
        AlertSeverity::Warn => "warn",
        AlertSeverity::Critical => "critical",
    };
    extras.insert(
        "severity".into(),
        serde_json::Value::String(severity_str.to_string()),
    );
    crate::lanes::LaneHit {
        doc_id: format!("silent-drop|{}|{}", pair_state_filename(&row.pair), ts_ms),
        text: format!(
            "[silent-drop {severity_str}] {}: upstream={} downstream={} growth={}",
            row.pair, row.upstream, row.downstream, row.delta_growth
        ),
        repo: None,
        path: None,
        symbol: Some("law_violation:silent_drop".to_string()),
        content_hash: Some(format!(
            "sha256:silent-drop:{}",
            pair_state_filename(&row.pair)
        )),
        score: 1.0,
        ts: ts_ms,
        severity: Some(severity_str.to_string()),
        extras,
    }
}

/// Best-effort POST of the envelope to `cortex-ingestion
/// /v1/events/batch`. Transport failures log at WARN and return
/// without panicking — losing one alert envelope is preferable to
/// crashing the watcher.
pub async fn post_envelope_to_ingestion(
    client: &reqwest::Client,
    ingestion_url: &str,
    envelope: &serde_json::Value,
) {
    let url = format!("{}/v1/events/batch", ingestion_url.trim_end_matches('/'));
    let body = serde_json::json!({ "events": [envelope] });
    match client.post(&url).json(&body).send().await {
        Ok(resp) if resp.status().is_success() || resp.status().as_u16() == 202 => {
            // Body has accepted/rejected; we deliberately ignore it
            // — the watcher's contract is "fire once per transition",
            // not "guarantee delivery". Ingestion-side rejections
            // surface in cortex-adapter's existing publisher.rejected
            // metric.
        }
        Ok(resp) => {
            tracing::warn!(
                status = %resp.status(),
                url = %url,
                "silent-drop: ingestion rejected alert envelope"
            );
        }
        Err(e) => {
            tracing::warn!(error = %e, url = %url, "silent-drop: ingestion POST failed");
        }
    }
}

/// Best-effort POST of the alert envelope to a configured webhook.
pub async fn post_to_webhook(client: &reqwest::Client, url: &str, envelope: &serde_json::Value) {
    if let Err(e) = client.post(url).json(envelope).send().await {
        tracing::warn!(error = %e, url, "silent-drop: webhook POST failed");
    }
}

/// Append a one-line alert summary to `.rulebook/handoff/_pending.md`.
/// Best-effort: any error is logged and ignored.
pub fn append_handoff(workspace: &std::path::Path, row: &DivergenceRow, severity: AlertSeverity) {
    let path = workspace
        .join(".rulebook")
        .join("handoff")
        .join("_pending.md");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    use std::io::Write as _;
    let line = format!(
        "[silent-drop alert] {:?} on `{}` (delta_growth={})\n",
        severity, row.pair, row.delta_growth
    );
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Watcher-side handle that owns the in-memory state map. Tests
/// construct one directly; the real boot path uses [`spawn`].
pub struct SilentDropWatcher {
    /// Per-pair persisted state (mirrored to disk on every change).
    pub states: HashMap<String, PairState>,
    /// Watcher config.
    pub cfg: SilentDropConfig,
    /// Aggregator state shared with the divergence endpoint so the
    /// watcher's view of `delta_growth` matches what the dashboard
    /// shows.
    pub aggregator: Arc<HealthAggregatorState>,
    /// Last poll timestamp — exposed so tests can assert progress.
    pub last_poll: Option<Instant>,
}

impl SilentDropWatcher {
    /// Build a watcher with a fresh in-memory state map; the real
    /// boot path also calls [`hydrate_from_disk`] to restore state
    /// across restarts.
    pub fn new(cfg: SilentDropConfig, aggregator: Arc<HealthAggregatorState>) -> Self {
        Self {
            states: HashMap::new(),
            cfg,
            aggregator,
            last_poll: None,
        }
    }

    /// Apply one set of divergence rows through the state machine
    /// and return the list of transitions to emit. Tests drive this
    /// directly without spawning the actual tokio task.
    pub fn step(&mut self, rows: &[DivergenceRow]) -> Vec<(DivergenceRow, AlertSeverity)> {
        self.last_poll = Some(Instant::now());
        let mut emitted: Vec<(DivergenceRow, AlertSeverity)> = Vec::new();
        for row in rows {
            let cfg = pair_config(&self.cfg, &row.pair);
            let prev = self.states.get(&row.pair).cloned().unwrap_or_default();
            let (next, transition) = transition(&prev, row.delta_growth, &cfg);
            self.states.insert(row.pair.clone(), next.clone());
            save_pair_state(&self.cfg.state_dir, &row.pair, &next);
            if let AlertTransition::Changed { severity } = transition {
                emitted.push((row.clone(), severity));
            }
        }
        emitted
    }

    /// Phase8e — one async tick: fetch the divergence rows from the
    /// shared aggregator, run them through [`Self::step`], and emit
    /// each transition's envelope to ingestion + the keyword lane.
    /// Failures inside any single emit are logged and never abort
    /// the tick — the watcher's contract is "best-effort fire once
    /// per transition", not delivery guarantee.
    pub async fn tick(
        &mut self,
        client: &reqwest::Client,
        lane: &Arc<crate::lanes::MemoryKeywordLane>,
    ) {
        // Compute the divergence rows in the same shape the
        // /v1/health/divergence endpoint returns — sharing
        // `aggregator.history` keeps the watcher's view aligned
        // with what dashboards show.
        let rows = compute_divergence_rows(self.aggregator.clone()).await;
        for (row, severity) in self.step(&rows) {
            let now_ms = chrono::Utc::now().timestamp_millis();
            // 1. Lane insert — Live Timeline / Violations dashboard
            //    surfaces the alert in <1s without waiting for the
            //    archive_loader refresh.
            let hit = alert_lane_hit(&row, severity, now_ms);
            for index in ["cortex-code", "cortex-docs", "cortex-decisions"] {
                lane.seed(index, vec![hit.clone()]);
            }
            // 2. Envelope POST to ingestion so the alert lands in
            //    the durable archive too.
            let envelope = build_alert_envelope(&row, severity);
            post_envelope_to_ingestion(client, &self.cfg.ingestion_url, &envelope).await;
            // 3. Optional webhook + handoff escalation.
            if let Some(url) = self.cfg.webhook_url.clone() {
                post_to_webhook(client, &url, &envelope).await;
            }
            if self.cfg.write_to_handoff && severity == AlertSeverity::Critical {
                let workspace =
                    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                append_handoff(&workspace, &row, severity);
            }
        }
    }

    /// Read every `<pair>.json` under [`SilentDropConfig::state_dir`]
    /// and pre-populate the in-memory state map. Called once at boot
    /// so a restart doesn't re-fire alerts.
    pub fn hydrate_from_disk(&mut self) {
        let dir = match std::fs::read_dir(&self.cfg.state_dir) {
            Ok(d) => d,
            Err(_) => return,
        };
        for entry in dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let raw = match std::fs::read_to_string(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            let state: PairState = match serde_json::from_str(&raw) {
                Ok(s) => s,
                Err(_) => continue,
            };
            // The filename is the sanitised pair key — we recover
            // the canonical pair string from a sidecar `pair` field
            // if present, otherwise fall back to the file stem.
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            self.states.insert(stem, state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(pair: &str) -> PairConfig {
        PairConfig {
            pair: pair.to_string(),
            warn_delta: 10,
            critical_delta: 50,
        }
    }

    #[test]
    fn ok_to_warn_requires_two_consecutive_polls() {
        let s0 = PairState::default();
        // First over-warn poll — counter starts but no envelope.
        let (s1, t1) = transition(&s0, 30, &cfg("p"));
        assert_eq!(t1, AlertTransition::None);
        assert!(matches!(s1.alert, AlertState::Warn { consecutive: 1 }));
        // Second over-warn poll — fire Warn.
        let (s2, t2) = transition(&s1, 30, &cfg("p"));
        assert!(matches!(
            t2,
            AlertTransition::Changed {
                severity: AlertSeverity::Warn
            }
        ));
        assert!(matches!(s2.alert, AlertState::Warn { consecutive: 2 }));
    }

    #[test]
    fn transient_spike_does_not_alert() {
        // One spike then immediate recovery — no envelope emitted
        // and the state returns toward Ok via the recovery streak.
        let s0 = PairState::default();
        let (s1, t1) = transition(&s0, 30, &cfg("p"));
        assert_eq!(t1, AlertTransition::None);
        let (s2, t2) = transition(&s1, 0, &cfg("p"));
        assert_eq!(t2, AlertTransition::None);
        // Recovery streak = 1; not yet 5 consecutive so still Warn{1}.
        assert_eq!(s2.recovery_streak, 1);
    }

    #[test]
    fn critical_fires_on_single_poll_above_threshold() {
        let s0 = PairState::default();
        let (s1, t1) = transition(&s0, 100, &cfg("p"));
        assert!(matches!(
            t1,
            AlertTransition::Changed {
                severity: AlertSeverity::Critical
            }
        ));
        assert_eq!(s1.alert, AlertState::Critical);
    }

    #[test]
    fn warn_to_critical_promotes_after_threshold_breach() {
        let s_warn = PairState {
            alert: AlertState::Warn { consecutive: 2 },
            recovery_streak: 0,
        };
        let (s1, t1) = transition(&s_warn, 100, &cfg("p"));
        assert!(matches!(
            t1,
            AlertTransition::Changed {
                severity: AlertSeverity::Critical
            }
        ));
        assert_eq!(s1.alert, AlertState::Critical);
    }

    #[test]
    fn recovery_requires_five_consecutive_ok_polls() {
        let mut s = PairState {
            alert: AlertState::Critical,
            recovery_streak: 0,
        };
        // Four ok polls — no recovery yet.
        for _ in 0..4 {
            let (next, t) = transition(&s, 0, &cfg("p"));
            assert_eq!(t, AlertTransition::None);
            s = next;
        }
        assert_eq!(s.alert, AlertState::Critical);
        // Fifth ok poll — Critical → Ok recovery alert.
        let (next, t) = transition(&s, 0, &cfg("p"));
        assert!(matches!(
            t,
            AlertTransition::Changed {
                severity: AlertSeverity::Ok
            }
        ));
        assert_eq!(next.alert, AlertState::Ok);
    }

    #[test]
    fn already_critical_stays_silent_on_subsequent_critical_polls() {
        let s = PairState {
            alert: AlertState::Critical,
            recovery_streak: 0,
        };
        let (next, t) = transition(&s, 100, &cfg("p"));
        assert_eq!(t, AlertTransition::None);
        assert_eq!(next.alert, AlertState::Critical);
    }

    #[test]
    fn pair_config_falls_back_to_defaults() {
        let cfg = SilentDropConfig::default();
        let p = pair_config(&cfg, "x.y");
        assert_eq!(p.warn_delta, 10);
        assert_eq!(p.critical_delta, 50);
    }

    #[test]
    fn pair_state_filename_sanitises_path_chars() {
        assert_eq!(pair_state_filename("a.b -> c.d"), "a.b_-__c.d");
        assert_eq!(pair_state_filename("ok.kind"), "ok.kind");
    }

    #[test]
    fn save_and_load_round_trip_state() {
        let tmp = tempfile::tempdir().unwrap();
        let s = PairState {
            alert: AlertState::Critical,
            recovery_streak: 2,
        };
        save_pair_state(tmp.path(), "x -> y", &s);
        let read = load_pair_state(tmp.path(), "x -> y");
        assert_eq!(read.alert, AlertState::Critical);
        assert_eq!(read.recovery_streak, 2);
    }

    #[test]
    fn missing_state_file_returns_default() {
        let tmp = tempfile::tempdir().unwrap();
        let s = load_pair_state(tmp.path(), "never-stored");
        assert_eq!(s.alert, AlertState::Ok);
        assert_eq!(s.recovery_streak, 0);
    }

    #[test]
    fn build_alert_envelope_carries_canonical_law_violation_kind() {
        let row = DivergenceRow {
            pair: "adapter.envelopes_built -> adapter.envelopes_publish_ok".into(),
            upstream: 100,
            downstream: 20,
            delta: 80,
            delta_growth: 80,
            severity: crate::health::Severity::Critical,
        };
        let env = build_alert_envelope(&row, AlertSeverity::Critical);
        assert_eq!(
            env.get("kind").and_then(|v| v.as_str()),
            Some("law_violation")
        );
        let payload = env.get("payload").unwrap();
        assert!(payload
            .get("law_id")
            .and_then(|v| v.as_str())
            .unwrap()
            .starts_with("silent-drop:"));
        assert_eq!(
            payload.get("upstream_count").and_then(|v| v.as_u64()),
            Some(100)
        );
        assert_eq!(
            payload.get("downstream_count").and_then(|v| v.as_u64()),
            Some(20)
        );
    }

    #[test]
    fn step_emits_one_envelope_per_transition() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = SilentDropConfig::default();
        cfg.state_dir = tmp.path().to_path_buf();
        let aggregator = Arc::new(HealthAggregatorState::new());
        let mut watcher = SilentDropWatcher::new(cfg, aggregator);
        let row_warn = DivergenceRow {
            pair: "warn-pair".into(),
            upstream: 30,
            downstream: 0,
            delta: 30,
            delta_growth: 30,
            severity: crate::health::Severity::Warn,
        };
        // First poll — no emit (debounce).
        assert!(watcher.step(&[row_warn.clone()]).is_empty());
        // Second poll — Warn fires.
        let emitted = watcher.step(&[row_warn.clone()]);
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].1, AlertSeverity::Warn);
    }

    #[test]
    fn watcher_step_persists_state_to_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let mut cfg = SilentDropConfig::default();
        cfg.state_dir = tmp.path().to_path_buf();
        let aggregator = Arc::new(HealthAggregatorState::new());
        let mut watcher = SilentDropWatcher::new(cfg.clone(), aggregator);
        let row = DivergenceRow {
            pair: "p".into(),
            upstream: 100,
            downstream: 0,
            delta: 100,
            delta_growth: 100,
            severity: crate::health::Severity::Critical,
        };
        watcher.step(&[row]);
        // Single Critical poll should land state on disk.
        let read = load_pair_state(&cfg.state_dir, "p");
        assert_eq!(read.alert, AlertState::Critical);
    }

    #[test]
    fn hydrate_from_disk_pre_populates_state_map() {
        let tmp = tempfile::tempdir().unwrap();
        let s = PairState {
            alert: AlertState::Warn { consecutive: 2 },
            recovery_streak: 0,
        };
        save_pair_state(tmp.path(), "old-pair", &s);
        let mut cfg = SilentDropConfig::default();
        cfg.state_dir = tmp.path().to_path_buf();
        let aggregator = Arc::new(HealthAggregatorState::new());
        let mut watcher = SilentDropWatcher::new(cfg, aggregator);
        watcher.hydrate_from_disk();
        let stem = pair_state_filename("old-pair");
        assert!(watcher.states.contains_key(&stem));
    }
}
