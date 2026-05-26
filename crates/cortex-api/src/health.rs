//! Phase8b — pipeline-stage freshness + divergence aggregators.
//!
//! Two new endpoints under `cortex-api`:
//!
//! - `GET /v1/health/freshness` — fans out the same probes
//!   `/v1/health` uses, parses the per-stage `last_*_ts` extras, and
//!   returns a flat map keyed by `<stage>.<kind>` (or `<stage>`)
//!   carrying `{ last_event_ts_ms, gap_seconds, severity }` rows.
//!
//! - `GET /v1/health/divergence` — pairs adjacent-stage counters
//!   from `/healthz` extras and reports per-pair `(upstream,
//!   downstream, delta, severity)` rows so a silent drop between
//!   two stages localises the moment it lands.
//!
//! Both endpoints share the same `gather_subsystem_extras` helper so
//! a single fan-out feeds both paths — the cortex-api freshness +
//! divergence views never disagree about what the underlying
//! `/healthz` extras said.
//!
//! Severity rules — both endpoints:
//! - `gap_seconds > 60`  → `warn`
//! - `gap_seconds > 300` → `critical`
//! - `delta_growth_60s > 10` → `warn`
//! - `delta_growth_60s > 50` → `critical`
//!
//! `delta_growth_60s` requires a one-sample-back state which the
//! aggregator caches in process memory; the first probe always
//! returns growth = 0 (we don't have anything to compare against
//! yet) and the row is therefore `ok` until the second probe lands.
//!
//! Sibling modules live under `crates/cortex-api/src/health/*.rs`
//! (currently `coverage` for the `/v1/health/coverage` endpoint).

pub mod consolidator;
pub mod coverage;

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::Json;
use cortex_health::client::{aggregate, build_client, AggregatorConfig, ProbeTarget};
use cortex_health::SubsystemStatus;
use futures::stream::Stream;
use serde::Serialize;
use serde_json::Value;

/// Severity bucket the GUI colour-codes on.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Within tolerance.
    Ok,
    /// Stale or growing — action recommended.
    Warn,
    /// Hard stall or persistent silent drop.
    Critical,
}

impl Severity {
    fn from_gap_seconds(gap_seconds: i64) -> Self {
        if gap_seconds > 300 {
            Severity::Critical
        } else if gap_seconds > 60 {
            Severity::Warn
        } else {
            Severity::Ok
        }
    }
    fn from_growth(growth: i64) -> Self {
        if growth > 50 {
            Severity::Critical
        } else if growth > 10 {
            Severity::Warn
        } else {
            Severity::Ok
        }
    }
    /// Phase8e — same growth thresholds the silent-drop watcher
    /// uses to map a row to a severity bucket. Exposed so the
    /// watcher and the HTTP endpoint share one classification.
    pub(crate) fn from_growth_for_silent_drop(growth: i64) -> Self {
        Self::from_growth(growth)
    }
}

/// One row in the `/v1/health/freshness` table.
#[derive(Debug, Clone, Serialize)]
pub struct FreshnessRow {
    /// Stage + kind/hook key (e.g. `adapter.publisher.tool_call`).
    pub key: String,
    /// Most recent activity timestamp in Unix-epoch ms (`0` when the
    /// stage has never emitted that signal).
    pub last_event_ts_ms: u64,
    /// Now - last_event_ts in whole seconds. `-1` when
    /// `last_event_ts_ms == 0` so the GUI can render `—`.
    pub gap_seconds: i64,
    /// Bucket the GUI colour-codes on.
    pub severity: Severity,
}

/// One row in the `/v1/health/divergence` table.
#[derive(Debug, Clone, Serialize)]
pub struct DivergenceRow {
    /// Pair name (e.g. `adapter.envelopes_built.tool_call ->
    /// adapter.envelopes_publish_ok.tool_call`).
    pub pair: String,
    /// Upstream counter value at probe time.
    pub upstream: u64,
    /// Downstream counter value at probe time.
    pub downstream: u64,
    /// `upstream - downstream`. Saturating: never negative; if the
    /// downstream beat the upstream (counter wrap on a restart) the
    /// delta clamps to zero.
    pub delta: u64,
    /// Change in `delta` since the last probe (~60s ago, depending on
    /// scrape cadence). `0` on the first probe — there is nothing to
    /// compare against. A *positive* growth means upstream pulled
    /// further ahead — the smoking-gun signal.
    pub delta_growth: i64,
    /// Bucket the GUI colour-codes on.
    pub severity: Severity,
}

/// Per-pair history slot the divergence aggregator keeps in memory
/// to compute `delta_growth` between probes.
#[derive(Debug, Clone, Default)]
pub(crate) struct DivergenceSample {
    pub(crate) delta: u64,
    pub(crate) captured_at: Option<Instant>,
}

/// In-process state shared by both endpoints — the per-pair history
/// table the divergence aggregator consults to produce `delta_growth`.
#[derive(Debug, Default)]
pub struct HealthAggregatorState {
    pub(crate) history: Mutex<BTreeMap<String, DivergenceSample>>,
}

impl HealthAggregatorState {
    /// Fresh aggregator state.
    pub fn new() -> Self {
        Self::default()
    }
}

/// Handler state — the shared aggregator history plus an Arc so the
/// freshness handler can read the cortex-api-local LoaderMetrics
/// without re-probing itself across the network.
#[derive(Clone)]
pub struct HealthState {
    /// Per-pair sample history.
    pub aggregator: Arc<HealthAggregatorState>,
    /// LoaderMetrics shared with the dashboard. Read directly so a
    /// fan-out probe to the cortex-api self URL is unnecessary.
    pub loader_metrics: Arc<crate::LoaderMetrics>,
    /// Phase8c — workspace HEAD captured once at boot. The drift
    /// aggregator compares each running binary's `git_sha` against
    /// this value. `None` means the daemon couldn't run `git
    /// rev-parse HEAD` at boot (no git on PATH, no working tree) —
    /// drift detection degrades gracefully to "head unknown".
    pub head_sha: Option<Arc<HeadSha>>,
}

/// Cached snapshot of `git rev-parse HEAD` taken once at boot.
#[derive(Debug, Clone)]
pub struct HeadSha {
    /// Full 40-char SHA.
    pub full: String,
    /// First 7 chars.
    pub short: String,
}

impl HeadSha {
    /// Resolve `git rev-parse HEAD` once. Returns `None` when git is
    /// unavailable or the command failed.
    pub fn resolve_now() -> Option<Self> {
        use std::process::Command;
        let out = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let full = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if full.is_empty() {
            return None;
        }
        let short = if full.len() >= 7 {
            full[..7].to_string()
        } else {
            full.clone()
        };
        Some(Self { full, short })
    }
}

/// Probe every subsystem (same target list as `/v1/health`) and
/// return the `(name → SubsystemStatus)` map. Network failures and
/// 5xx still appear in the map as `Down` rows so the freshness /
/// divergence endpoints can flag them honestly instead of dropping
/// the row silently.
pub(crate) async fn gather_subsystem_extras() -> BTreeMap<String, SubsystemStatus> {
    let candidates: &[(&'static str, &str, &str)] = &[
        (
            "cortex-adapter",
            "CORTEX_ADAPTER_ADMIN_URL",
            "http://127.0.0.1:17011/healthz",
        ),
        (
            "cortex-ingestion",
            "CORTEX_INGESTION_URL",
            "http://127.0.0.1:17010/v1/healthz",
        ),
        (
            "cortex-classifier-worker",
            "CORTEX_CLASSIFIER_WORKER_URL",
            "http://127.0.0.1:17021/healthz",
        ),
        (
            "cortex-embedder-worker",
            "CORTEX_EMBEDDER_WORKER_URL",
            "http://127.0.0.1:17022/healthz",
        ),
        (
            "cortex-fulltext-worker",
            "CORTEX_FULLTEXT_WORKER_URL",
            "http://127.0.0.1:17023/healthz",
        ),
        (
            "cortex-graph-worker",
            "CORTEX_GRAPH_WORKER_URL",
            "http://127.0.0.1:17024/healthz",
        ),
    ];
    let mut targets: Vec<ProbeTarget> = Vec::with_capacity(candidates.len());
    for (name, key, default) in candidates {
        let url = std::env::var(key)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .unwrap_or_else(|| (*default).to_string());
        targets.push(ProbeTarget { name, url });
    }
    let client = match build_client(&AggregatorConfig::default()) {
        Ok(c) => c,
        Err(_) => return BTreeMap::new(),
    };
    let report = aggregate(&client, &targets, &AggregatorConfig::default()).await;
    let mut by_name = BTreeMap::new();
    for sub in report.subsystems {
        by_name.insert(sub.name.clone(), sub);
    }
    by_name
}

/// One row in the `/v1/health/versions` table — per running binary.
#[derive(Debug, Clone, Serialize)]
pub struct RunningBinaryVersion {
    /// Subsystem name (matches `/healthz`'s `name` field).
    pub name: String,
    /// Full git SHA the binary was built from. `"unknown"` when the
    /// binary was built outside a git working tree.
    pub git_sha: String,
    /// 7-char short form of `git_sha`.
    pub git_sha_short: String,
    /// RFC-3339 build timestamp.
    pub build_ts: String,
    /// `true` when the binary was built from a dirty working tree.
    pub git_dirty: bool,
    /// `"debug"` or `"release"`.
    pub profile: String,
    /// Cargo crate version (`CARGO_PKG_VERSION`).
    pub crate_version: String,
    /// `true` when the running SHA matches workspace HEAD.
    pub matches_head: bool,
}

/// One row in the `/v1/health/versions.drift[]` table — present only
/// when a binary's `git_sha` differs from workspace HEAD.
#[derive(Debug, Clone, Serialize)]
pub struct DriftRow {
    /// Subsystem name.
    pub binary: String,
    /// SHA the running binary was built from.
    pub running_sha: String,
    /// Workspace HEAD at the moment cortex-api booted.
    pub expected_sha: String,
    /// `git rev-list <running_sha>..HEAD --count`. `None` when the
    /// running SHA is unreachable from HEAD (force-pushed branch).
    pub behind_by_commits: Option<u64>,
    /// Free-text explanation when `behind_by_commits` is `None`.
    pub note: Option<String>,
}

/// Top-level body returned by `/v1/health/versions`.
#[derive(Debug, Clone, Serialize)]
pub struct VersionsReport {
    /// Workspace HEAD captured once at boot. `"unknown"` when git
    /// was unavailable.
    pub head_sha: String,
    /// 7-char short form of [`Self::head_sha`].
    pub head_sha_short: String,
    /// Per-running-binary version rows.
    pub running_binaries: Vec<RunningBinaryVersion>,
    /// Drift rows — only populated when a binary's SHA != head.
    pub drift: Vec<DriftRow>,
    /// `true` iff `drift` is empty.
    pub all_in_sync: bool,
}

/// `GET /v1/health/versions` handler.
pub async fn versions_handler(State(state): State<HealthState>) -> Response {
    let by_name = gather_subsystem_extras().await;
    let head = state
        .head_sha
        .as_ref()
        .map(|h| (h.full.clone(), h.short.clone()))
        .unwrap_or_else(|| ("unknown".to_string(), "unknown".to_string()));
    let head_sha = head.0;
    let head_sha_short = head.1;

    // Self-row: cortex-api's own version block, read in-process.
    let self_version = crate::self_version_info();
    let self_running = RunningBinaryVersion {
        name: "cortex-api".to_string(),
        matches_head: head_sha != "unknown" && self_version.git_sha == head_sha,
        git_sha: self_version.git_sha.clone(),
        git_sha_short: self_version.git_sha_short.clone(),
        build_ts: self_version.build_ts.clone(),
        git_dirty: self_version.git_dirty,
        profile: self_version.profile.clone(),
        crate_version: self_version.crate_version.clone(),
    };

    let mut running_binaries: Vec<RunningBinaryVersion> = Vec::new();
    running_binaries.push(self_running);
    for (name, status) in &by_name {
        if let Some(version_value) = status.extras.get("version") {
            if let Some(row) = version_row_from_json(name, version_value, &head_sha) {
                running_binaries.push(row);
            }
        }
    }
    running_binaries.sort_by(|a, b| a.name.cmp(&b.name));

    // Drift rows — only when SHA != head AND head is known.
    let mut drift: Vec<DriftRow> = Vec::new();
    if head_sha != "unknown" {
        for row in &running_binaries {
            if row.git_sha == "unknown" || row.matches_head {
                continue;
            }
            let (behind, note) = behind_by_commits(&row.git_sha, &head_sha);
            drift.push(DriftRow {
                binary: row.name.clone(),
                running_sha: row.git_sha.clone(),
                expected_sha: head_sha.clone(),
                behind_by_commits: behind,
                note,
            });
        }
    }

    let all_in_sync = drift.is_empty() && head_sha != "unknown";
    let report = VersionsReport {
        head_sha,
        head_sha_short,
        running_binaries,
        drift,
        all_in_sync,
    };
    (StatusCode::OK, Json(report)).into_response()
}

fn version_row_from_json(name: &str, raw: &Value, head_sha: &str) -> Option<RunningBinaryVersion> {
    let obj = raw.as_object()?;
    let git_sha = obj
        .get("git_sha")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let git_sha_short = obj
        .get("git_sha_short")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let build_ts = obj
        .get("build_ts")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let git_dirty = obj
        .get("git_dirty")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let profile = obj
        .get("profile")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let crate_version = obj
        .get("crate_version")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();
    let matches_head = head_sha != "unknown" && git_sha == head_sha;
    Some(RunningBinaryVersion {
        name: name.to_string(),
        git_sha,
        git_sha_short,
        build_ts,
        git_dirty,
        profile,
        crate_version,
        matches_head,
    })
}

/// Run `git rev-list <running_sha>..HEAD --count`. Returns
/// `(Some(count), None)` on success, `(None, Some(reason))` when
/// the running SHA isn't reachable from HEAD (typical on a force-
/// pushed branch or a tarball build).
fn behind_by_commits(running_sha: &str, head_sha: &str) -> (Option<u64>, Option<String>) {
    use std::process::Command;
    if running_sha == head_sha {
        return (Some(0), None);
    }
    if running_sha == "unknown" {
        return (
            None,
            Some("running sha unknown (binary built outside git)".to_string()),
        );
    }
    let out = match Command::new("git")
        .args(["rev-list", &format!("{running_sha}..HEAD"), "--count"])
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            return (
                None,
                Some("git rev-list failed (no git binary)".to_string()),
            );
        }
    };
    if !out.status.success() {
        return (None, Some("running sha unreachable from HEAD".to_string()));
    }
    let trimmed = String::from_utf8_lossy(&out.stdout).trim().to_string();
    match trimmed.parse::<u64>() {
        Ok(n) => (Some(n), None),
        Err(_) => (
            None,
            Some(format!("git rev-list emitted unexpected output: {trimmed}")),
        ),
    }
}

/// `GET /v1/health/freshness` handler.
pub async fn freshness_handler(State(state): State<HealthState>) -> Response {
    let by_name = gather_subsystem_extras().await;
    let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
    let mut rows: Vec<FreshnessRow> = Vec::new();

    // ---- adapter.* ---------------------------------------------------------
    if let Some(adapter) = by_name.get("cortex-adapter") {
        // Phase14c — heartbeat floor. Adapter bumps
        // `last_heartbeat_ts_ms` every 30 s; freshness rows for
        // every adapter.* metric use it as a max() floor so an idle
        // adapter (no real Claude Code activity) reads OK as long
        // as the heartbeat is fresh. Per-kind gaps still reflect
        // the true age in the raw extras JSON for debugging.
        let heartbeat = adapter
            .extras
            .get("last_heartbeat_ts_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        // Per-hook last_frame_ts.
        push_per_label_ts_with_floor(
            &mut rows,
            now_ms,
            "adapter.last_frame",
            adapter.extras.get("last_frame_ts_ms"),
            heartbeat,
        );
        // Per-kind last_envelope_ts (built).
        push_per_label_ts_with_floor(
            &mut rows,
            now_ms,
            "adapter.last_envelope",
            adapter.extras.get("last_envelope_ts_ms"),
            heartbeat,
        );
        // Per-kind last_publish_ok_ts.
        push_per_label_ts_with_floor(
            &mut rows,
            now_ms,
            "adapter.last_publish_ok",
            adapter.extras.get("last_publish_ok_ts_ms_by_kind"),
            heartbeat,
        );
        // Aggregate (any-kind) publish-ok timestamp from phase8a.
        if let Some(ms) = adapter
            .extras
            .get("last_publish_ok_ts_ms")
            .and_then(|v| v.as_u64())
        {
            rows.push(freshness_row_with_floor(
                "adapter.last_publish_ok",
                ms,
                now_ms,
                heartbeat,
            ));
        }
    }

    // ---- ingestion.* -------------------------------------------------------
    if let Some(ingest) = by_name.get("cortex-ingestion") {
        push_per_label_ts(
            &mut rows,
            now_ms,
            "ingestion.last_archive_write",
            ingest.extras.get("last_archive_write_ts_ms"),
        );
        if let Some(ms) = ingest
            .extras
            .get("last_batch_accepted_ts_ms")
            .and_then(|v| v.as_u64())
        {
            rows.push(freshness_row("ingestion.last_batch_accepted", ms, now_ms));
        }
    }

    // ---- cortex-api loader stages (read directly from in-process LoaderMetrics) -----
    let arch_ts = state.loader_metrics.archive_last_refresh_ts_ms();
    rows.push(freshness_row(
        "api.archive_loader.last_refresh",
        arch_ts,
        now_ms,
    ));
    let meili_ts = state.loader_metrics.meili_last_refresh_ts_ms();
    rows.push(freshness_row(
        "api.meili_loader.last_refresh",
        meili_ts,
        now_ms,
    ));

    // ---- workers — use last_job_ts_ms exposed in extras --------------------
    for worker in [
        "cortex-classifier-worker",
        "cortex-embedder-worker",
        "cortex-fulltext-worker",
        "cortex-graph-worker",
    ] {
        if let Some(s) = by_name.get(worker) {
            if let Some(ms) = s.extras.get("last_job_ts_ms").and_then(|v| v.as_u64()) {
                let key = format!("{}.last_job", short_worker_name(worker));
                rows.push(freshness_row(&key, ms, now_ms));
            }
        }
    }

    rows.sort_by(|a, b| a.key.cmp(&b.key));
    let _ = state; // aggregator history not needed here, kept for API symmetry.
    (StatusCode::OK, Json(rows)).into_response()
}

/// `GET /v1/health/divergence` handler.
pub async fn divergence_handler(State(state): State<HealthState>) -> Response {
    let by_name = gather_subsystem_extras().await;
    let now = Instant::now();
    let mut rows: Vec<DivergenceRow> = Vec::new();

    // The divergence pairs span adjacent pipeline stages. Each pair
    // is `(pair_key, upstream_lookup, downstream_lookup)` where
    // each lookup is a small closure that pulls the relevant counter
    // out of `by_name`.
    let pairs: Vec<(String, u64, u64)> = build_divergence_pairs(&by_name);

    let mut history = state
        .aggregator
        .history
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();

    for (key, up, down) in pairs {
        let delta = up.saturating_sub(down);
        let prev = history.get(&key).cloned().unwrap_or_default();
        let delta_growth = match prev.captured_at {
            Some(when) if now.saturating_duration_since(when) >= Duration::from_secs(30) => {
                i64::try_from(delta).unwrap_or(0) - i64::try_from(prev.delta).unwrap_or(0)
            }
            // First probe or sub-30s second probe — no growth signal yet.
            _ => 0,
        };
        let severity = if delta_growth > 0 {
            Severity::from_growth(delta_growth)
        } else {
            Severity::Ok
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
            DivergenceSample {
                delta,
                captured_at: Some(now),
            },
        );
    }

    if let Ok(mut guard) = state.aggregator.history.lock() {
        *guard = history;
    }

    rows.sort_by(|a, b| a.pair.cmp(&b.pair));
    (StatusCode::OK, Json(rows)).into_response()
}

/// Pull the canonical divergence pairs from the gathered extras.
/// Returns `(pair_key, upstream_total, downstream_total)` rows.
pub(crate) fn build_divergence_pairs(
    by_name: &BTreeMap<String, SubsystemStatus>,
) -> Vec<(String, u64, u64)> {
    let mut out: Vec<(String, u64, u64)> = Vec::new();

    let adapter = by_name.get("cortex-adapter");
    let ingestion = by_name.get("cortex-ingestion");

    // Pair 1 — IPC parsed vs envelopes built (per kind we don't pivot;
    // the aggregate suffices because they should match envelope
    // production rate).
    if let Some(a) = adapter {
        let parsed_total = sum_u64_map(a.extras.get("frames_parsed_total"));
        let built_total = sum_u64_map(a.extras.get("envelopes_built_total"));
        out.push((
            "adapter.frames_parsed -> adapter.envelopes_built".to_string(),
            parsed_total,
            built_total,
        ));
        // Pair 2 — built vs publish_ok (queue drops + WAL spills).
        let publish_ok_total = sum_u64_map(a.extras.get("envelopes_publish_ok_total"));
        out.push((
            "adapter.envelopes_built -> adapter.envelopes_publish_ok".to_string(),
            built_total,
            publish_ok_total,
        ));
    }
    // Pair 3 — adapter publish_ok vs ingestion archived.
    if let (Some(a), Some(ing)) = (adapter, ingestion) {
        let pub_ok_map = a.extras.get("envelopes_publish_ok_total");
        let archived_map = ing.extras.get("events_archived_total");
        // Per-kind pairs so a divergence localises which kind dropped.
        let pub_ok_by_kind = parse_u64_map(pub_ok_map);
        let archived_by_kind = parse_u64_map(archived_map);
        let mut keys: std::collections::BTreeSet<&String> = pub_ok_by_kind.keys().collect();
        keys.extend(archived_by_kind.keys());
        for k in keys {
            let up = pub_ok_by_kind.get(k).copied().unwrap_or(0);
            let down = archived_by_kind.get(k).copied().unwrap_or(0);
            out.push((
                format!("adapter.publish_ok.{k} -> ingestion.archived.{k}"),
                up,
                down,
            ));
        }
    }
    out
}

fn short_worker_name(name: &str) -> &str {
    match name {
        "cortex-classifier-worker" => "classifier",
        "cortex-embedder-worker" => "embedder",
        "cortex-fulltext-worker" => "fulltext",
        "cortex-graph-worker" => "graph",
        other => other,
    }
}

fn freshness_row(key: &str, ts_ms: u64, now_ms: u64) -> FreshnessRow {
    let gap_seconds = if ts_ms == 0 {
        -1
    } else {
        ((now_ms.saturating_sub(ts_ms)) / 1000) as i64
    };
    let severity = if ts_ms == 0 {
        Severity::Warn
    } else {
        Severity::from_gap_seconds(gap_seconds)
    };
    FreshnessRow {
        key: key.to_string(),
        last_event_ts_ms: ts_ms,
        gap_seconds,
        severity,
    }
}

/// Phase14c — `freshness_row` variant that floors the effective
/// timestamp at `floor_ts_ms` (typically the adapter's heartbeat).
/// `gap_seconds` and `severity` use `max(ts_ms, floor_ts_ms)`, but
/// `last_event_ts_ms` on the wire preserves the original `ts_ms`
/// so the raw row stays honest about per-kind activity.
fn freshness_row_with_floor(
    key: &str,
    ts_ms: u64,
    now_ms: u64,
    floor_ts_ms: u64,
) -> FreshnessRow {
    let effective = ts_ms.max(floor_ts_ms);
    let gap_seconds = if effective == 0 {
        -1
    } else {
        ((now_ms.saturating_sub(effective)) / 1000) as i64
    };
    let severity = if effective == 0 {
        Severity::Warn
    } else {
        Severity::from_gap_seconds(gap_seconds)
    };
    FreshnessRow {
        key: key.to_string(),
        last_event_ts_ms: ts_ms,
        gap_seconds,
        severity,
    }
}

/// Iterate over a `Map<String, u64>` extras value and emit one row per
/// entry, using `<prefix>.<label>` as the row key.
fn push_per_label_ts(
    rows: &mut Vec<FreshnessRow>,
    now_ms: u64,
    prefix: &str,
    extras_value: Option<&Value>,
) {
    let map = match extras_value {
        Some(v) => v,
        None => return,
    };
    if let Some(obj) = map.as_object() {
        for (label, raw) in obj {
            let ms = raw.as_u64().unwrap_or(0);
            let key = format!("{prefix}.{label}");
            rows.push(freshness_row(&key, ms, now_ms));
        }
    }
}

/// Phase14c — `push_per_label_ts` variant that floors every per-
/// label gap at `floor_ts_ms`. Used for `adapter.*` rows so the
/// adapter's heartbeat lifts per-kind severities to OK while keeping
/// `last_event_ts_ms` honest in the wire shape.
fn push_per_label_ts_with_floor(
    rows: &mut Vec<FreshnessRow>,
    now_ms: u64,
    prefix: &str,
    extras_value: Option<&Value>,
    floor_ts_ms: u64,
) {
    let map = match extras_value {
        Some(v) => v,
        None => return,
    };
    if let Some(obj) = map.as_object() {
        for (label, raw) in obj {
            let ms = raw.as_u64().unwrap_or(0);
            let key = format!("{prefix}.{label}");
            rows.push(freshness_row_with_floor(&key, ms, now_ms, floor_ts_ms));
        }
    }
}

fn parse_u64_map(extras_value: Option<&Value>) -> BTreeMap<String, u64> {
    let mut out = BTreeMap::new();
    if let Some(Value::Object(obj)) = extras_value {
        for (k, v) in obj {
            if let Some(n) = v.as_u64() {
                out.insert(k.clone(), n);
            }
        }
    }
    out
}

fn sum_u64_map(extras_value: Option<&Value>) -> u64 {
    parse_u64_map(extras_value).values().copied().sum()
}

/// Phase8g — combined snapshot the `/v1/health/stream` SSE endpoint
/// emits every 5 seconds. Bundles the four phase8 sub-aggregators
/// into one payload so the GUI subscribes once instead of polling
/// five endpoints in lockstep.
#[derive(Debug, Clone, Serialize)]
pub struct HealthSnapshot {
    /// RFC-3339 timestamp the snapshot was assembled.
    pub generated_at: String,
    /// Overall traffic-light label (`ok` / `degraded` / `down`).
    pub overall: String,
    /// Per-stage `<stage>.<kind>` rows from the freshness aggregator.
    pub freshness: Vec<FreshnessRow>,
    /// Adjacent-stage divergence pairs.
    pub divergence: Vec<DivergenceRow>,
    /// `true` when at least one row is included; flipped to `false`
    /// after the byte budget truncates.
    pub truncated: bool,
}

/// `GET /v1/health/stream` — SSE handler emitting one `health` event
/// every 5 s + a `heartbeat` event every 15 s. Reuses the same
/// per-subscriber loop pattern Live Timeline uses (one `tokio::spawn`
/// per connection, no shared broadcast channel).
pub async fn stream_handler(
    State(state): State<HealthState>,
) -> Sse<impl Stream<Item = Result<SseEvent, Infallible>>> {
    let stream = async_stream::stream! {
        let mut snapshot = tokio::time::interval(Duration::from_secs(5));
        snapshot.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut heartbeat = tokio::time::interval(Duration::from_secs(15));
        heartbeat.tick().await;
        loop {
            tokio::select! {
                _ = snapshot.tick() => {
                    let bundle = build_snapshot(&state).await;
                    let json = serde_json::to_string(&bundle).unwrap_or_else(|_| "{}".into());
                    yield Ok::<SseEvent, Infallible>(
                        SseEvent::default().event("health").data(json)
                    );
                }
                _ = heartbeat.tick() => {
                    yield Ok::<SseEvent, Infallible>(
                        SseEvent::default()
                            .event("heartbeat")
                            .data(r#"{"ok":true}"#)
                    );
                }
            }
        }
    };
    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// Pure helper that assembles one [`HealthSnapshot`]. Re-runs every
/// SSE tick — the constituent endpoints are cheap (file reads + a
/// fan-out probe each) so a fresh snapshot per subscriber is fine
/// at single-digit-Hz cadence.
async fn build_snapshot(state: &HealthState) -> HealthSnapshot {
    let by_name = gather_subsystem_extras().await;
    let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;

    // Freshness rows — same logic as `freshness_handler` but in-line
    // so the SSE handler doesn't fan out twice per tick.
    let mut freshness: Vec<FreshnessRow> = Vec::new();
    if let Some(adapter) = by_name.get("cortex-adapter") {
        push_per_label_ts(
            &mut freshness,
            now_ms,
            "adapter.last_frame",
            adapter.extras.get("last_frame_ts_ms"),
        );
        push_per_label_ts(
            &mut freshness,
            now_ms,
            "adapter.last_envelope",
            adapter.extras.get("last_envelope_ts_ms"),
        );
        push_per_label_ts(
            &mut freshness,
            now_ms,
            "adapter.last_publish_ok",
            adapter.extras.get("last_publish_ok_ts_ms_by_kind"),
        );
        if let Some(ms) = adapter
            .extras
            .get("last_publish_ok_ts_ms")
            .and_then(|v| v.as_u64())
        {
            freshness.push(freshness_row("adapter.last_publish_ok", ms, now_ms));
        }
    }
    if let Some(ingest) = by_name.get("cortex-ingestion") {
        push_per_label_ts(
            &mut freshness,
            now_ms,
            "ingestion.last_archive_write",
            ingest.extras.get("last_archive_write_ts_ms"),
        );
        if let Some(ms) = ingest
            .extras
            .get("last_batch_accepted_ts_ms")
            .and_then(|v| v.as_u64())
        {
            freshness.push(freshness_row("ingestion.last_batch_accepted", ms, now_ms));
        }
    }
    let arch_ts = state.loader_metrics.archive_last_refresh_ts_ms();
    freshness.push(freshness_row(
        "api.archive_loader.last_refresh",
        arch_ts,
        now_ms,
    ));
    let meili_ts = state.loader_metrics.meili_last_refresh_ts_ms();
    freshness.push(freshness_row(
        "api.meili_loader.last_refresh",
        meili_ts,
        now_ms,
    ));
    for worker in [
        "cortex-classifier-worker",
        "cortex-embedder-worker",
        "cortex-fulltext-worker",
        "cortex-graph-worker",
    ] {
        if let Some(s) = by_name.get(worker) {
            if let Some(ms) = s.extras.get("last_job_ts_ms").and_then(|v| v.as_u64()) {
                let key = format!("{}.last_job", short_worker_name(worker));
                freshness.push(freshness_row(&key, ms, now_ms));
            }
        }
    }
    freshness.sort_by(|a, b| a.key.cmp(&b.key));

    // Divergence rows — same shape as `divergence_handler`, sharing
    // the aggregator history Mutex.
    let now = Instant::now();
    let pairs = build_divergence_pairs(&by_name);
    let mut history = state
        .aggregator
        .history
        .lock()
        .map(|g| g.clone())
        .unwrap_or_default();
    let mut divergence: Vec<DivergenceRow> = Vec::new();
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
            Severity::from_growth(delta_growth)
        } else {
            Severity::Ok
        };
        divergence.push(DivergenceRow {
            pair: key.clone(),
            upstream: up,
            downstream: down,
            delta,
            delta_growth,
            severity,
        });
        history.insert(
            key,
            DivergenceSample {
                delta,
                captured_at: Some(now),
            },
        );
    }
    if let Ok(mut guard) = state.aggregator.history.lock() {
        *guard = history;
    }
    divergence.sort_by(|a, b| a.pair.cmp(&b.pair));

    // Overall — Down beats Degraded beats Ok across all subsystems.
    let overall = if by_name
        .values()
        .any(|s| s.state == cortex_health::HealthState::Down)
    {
        "down"
    } else if by_name
        .values()
        .any(|s| s.state == cortex_health::HealthState::Degraded)
    {
        "degraded"
    } else {
        "ok"
    };

    // Byte-cap the snapshot at 64 KiB. Truncate freshness rows from
    // the bottom (the table is sorted by gap_seconds desc inside the
    // GUI; we sorted alphabetically here so truncation drops the
    // alphabetic tail, not the most-stale rows). When truncated,
    // flip the flag.
    let mut snapshot = HealthSnapshot {
        generated_at: chrono::Utc::now().to_rfc3339(),
        overall: overall.to_string(),
        freshness,
        divergence,
        truncated: false,
    };
    let serialized = serde_json::to_string(&snapshot).unwrap_or_default();
    if serialized.len() > 64 * 1024 {
        // Halve the freshness vec until the snapshot fits.
        while serde_json::to_string(&snapshot)
            .map(|s| s.len() > 64 * 1024)
            .unwrap_or(false)
            && !snapshot.freshness.is_empty()
        {
            let new_len = snapshot.freshness.len() / 2;
            snapshot.freshness.truncate(new_len.max(1));
            snapshot.truncated = true;
        }
    }
    snapshot
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_buckets_match_spec_thresholds() {
        assert_eq!(Severity::from_gap_seconds(0), Severity::Ok);
        assert_eq!(Severity::from_gap_seconds(60), Severity::Ok);
        assert_eq!(Severity::from_gap_seconds(61), Severity::Warn);
        assert_eq!(Severity::from_gap_seconds(300), Severity::Warn);
        assert_eq!(Severity::from_gap_seconds(301), Severity::Critical);

        assert_eq!(Severity::from_growth(0), Severity::Ok);
        assert_eq!(Severity::from_growth(10), Severity::Ok);
        assert_eq!(Severity::from_growth(11), Severity::Warn);
        assert_eq!(Severity::from_growth(50), Severity::Warn);
        assert_eq!(Severity::from_growth(51), Severity::Critical);
    }

    #[test]
    fn freshness_row_with_zero_ts_is_warn_with_neg_gap() {
        let row = freshness_row("x.y", 0, 1_000_000);
        assert_eq!(row.gap_seconds, -1);
        assert_eq!(row.severity, Severity::Warn);
    }

    #[test]
    fn freshness_row_recent_ts_is_ok() {
        let now_ms = 1_000_000u64;
        let row = freshness_row("x.y", now_ms - 30_000, now_ms);
        assert_eq!(row.gap_seconds, 30);
        assert_eq!(row.severity, Severity::Ok);
    }

    #[test]
    fn freshness_row_stale_ts_is_critical_past_5_minutes() {
        let now_ms = 1_000_000u64;
        // 350 seconds in the past.
        let row = freshness_row("x.y", now_ms - 350_000, now_ms);
        assert_eq!(row.gap_seconds, 350);
        assert_eq!(row.severity, Severity::Critical);
    }

    #[test]
    fn parse_u64_map_extracts_numeric_values() {
        let raw = serde_json::json!({"turn": 5, "tool_call": 3});
        let m = parse_u64_map(Some(&raw));
        assert_eq!(m.get("turn"), Some(&5));
        assert_eq!(m.get("tool_call"), Some(&3));
    }

    #[test]
    fn sum_u64_map_adds_values() {
        let raw = serde_json::json!({"a": 1, "b": 2, "c": 7});
        assert_eq!(sum_u64_map(Some(&raw)), 10);
    }

    #[test]
    fn push_per_label_ts_emits_one_row_per_entry() {
        let raw = serde_json::json!({"PostToolUse": 1_000_000u64, "UserPromptSubmit": 0});
        let mut rows = Vec::new();
        let now_ms = 1_500_000u64;
        push_per_label_ts(&mut rows, now_ms, "adapter.last_frame", Some(&raw));
        assert_eq!(rows.len(), 2);
        let by_key: BTreeMap<_, _> = rows.iter().map(|r| (r.key.as_str(), r)).collect();
        assert_eq!(by_key["adapter.last_frame.PostToolUse"].gap_seconds, 500);
        // Zero ts → -1 gap, Warn severity.
        assert_eq!(
            by_key["adapter.last_frame.UserPromptSubmit"].gap_seconds,
            -1
        );
        assert_eq!(
            by_key["adapter.last_frame.UserPromptSubmit"].severity,
            Severity::Warn
        );
    }

    #[test]
    fn build_divergence_pairs_handles_missing_subsystems() {
        // No subsystems at all — no pairs.
        let empty = BTreeMap::new();
        assert!(build_divergence_pairs(&empty).is_empty());

        // Adapter-only → frames_parsed and built pairs but no
        // ingestion-side rows.
        let mut adapter = SubsystemStatus::ok("cortex-adapter", "0.1.0", "now");
        adapter.extras.insert(
            "frames_parsed_total".into(),
            serde_json::json!({"PostToolUse": 5, "UserPromptSubmit": 2}),
        );
        adapter.extras.insert(
            "envelopes_built_total".into(),
            serde_json::json!({"tool_call": 5, "turn": 2}),
        );
        adapter.extras.insert(
            "envelopes_publish_ok_total".into(),
            serde_json::json!({"tool_call": 3, "turn": 2}),
        );
        let mut by_name = BTreeMap::new();
        by_name.insert("cortex-adapter".to_string(), adapter);
        let rows = build_divergence_pairs(&by_name);
        // Two intra-adapter pairs.
        assert!(rows
            .iter()
            .any(|(k, _, _)| k == "adapter.frames_parsed -> adapter.envelopes_built"));
        assert!(rows
            .iter()
            .any(|(k, _, _)| k == "adapter.envelopes_built -> adapter.envelopes_publish_ok"));
        // No ingestion pairs because `cortex-ingestion` is missing.
        assert!(!rows
            .iter()
            .any(|(k, _, _)| k.starts_with("adapter.publish_ok.")));
    }

    #[test]
    fn build_divergence_pairs_localises_per_kind_drop_to_ingestion() {
        let mut adapter = SubsystemStatus::ok("cortex-adapter", "0.1.0", "now");
        adapter.extras.insert(
            "envelopes_publish_ok_total".into(),
            serde_json::json!({"tool_call": 100, "turn": 50}),
        );
        let mut ingest = SubsystemStatus::ok("cortex-ingestion", "0.1.0", "now");
        ingest.extras.insert(
            "events_archived_total".into(),
            serde_json::json!({"tool_call": 100, "turn": 0}), // turn dropped silently
        );
        let mut by_name = BTreeMap::new();
        by_name.insert("cortex-adapter".to_string(), adapter);
        by_name.insert("cortex-ingestion".to_string(), ingest);
        let rows = build_divergence_pairs(&by_name);
        let turn_row = rows
            .iter()
            .find(|(k, _, _)| k == "adapter.publish_ok.turn -> ingestion.archived.turn")
            .expect("turn pair must exist");
        assert_eq!(turn_row.1, 50, "upstream is the adapter publish_ok value");
        assert_eq!(turn_row.2, 0, "downstream is ingestion archived");
        let tool_row = rows
            .iter()
            .find(|(k, _, _)| k == "adapter.publish_ok.tool_call -> ingestion.archived.tool_call")
            .expect("tool_call pair must exist");
        assert_eq!(tool_row.1, 100);
        assert_eq!(tool_row.2, 100);
    }

    #[test]
    fn version_row_marks_match_when_sha_equals_head() {
        let raw = serde_json::json!({
            "git_sha": "abc1234567890abc1234567890abc1234567890a",
            "git_sha_short": "abc1234",
            "build_ts": "2026-04-29T12:00:00Z",
            "git_dirty": false,
            "profile": "release",
            "crate_version": "0.1.0"
        });
        let row =
            version_row_from_json("cortex-x", &raw, "abc1234567890abc1234567890abc1234567890a")
                .unwrap();
        assert!(row.matches_head);
        assert_eq!(row.git_sha_short, "abc1234");
        assert!(!row.git_dirty);
    }

    #[test]
    fn version_row_marks_drift_when_sha_differs_from_head() {
        let raw = serde_json::json!({
            "git_sha": "abc1234567890abc1234567890abc1234567890a",
            "git_sha_short": "abc1234",
            "build_ts": "2026-04-29T12:00:00Z",
            "git_dirty": true,
            "profile": "debug",
            "crate_version": "0.1.0"
        });
        let row = version_row_from_json("cortex-x", &raw, "def5678").unwrap();
        assert!(!row.matches_head);
        assert!(row.git_dirty);
        assert_eq!(row.profile, "debug");
    }

    #[test]
    fn version_row_handles_unknown_sentinel_fields() {
        let raw = serde_json::json!({
            "git_sha": "unknown",
            "git_sha_short": "unknown",
            "build_ts": "unknown",
            "git_dirty": false,
            "profile": "unknown",
            "crate_version": "0.1.0"
        });
        let row = version_row_from_json("cortex-x", &raw, "head").unwrap();
        // matches_head MUST be false when SHA is unknown — the
        // aggregator can't validate freshness without a real SHA.
        assert!(!row.matches_head);
        assert_eq!(row.git_sha, "unknown");
    }

    #[test]
    fn version_row_returns_none_for_non_object_input() {
        assert!(version_row_from_json("x", &Value::Null, "h").is_none());
        assert!(version_row_from_json("x", &Value::String("nope".into()), "h").is_none());
    }

    #[test]
    fn behind_by_commits_returns_zero_when_running_eq_head() {
        let (n, note) = behind_by_commits("abc", "abc");
        assert_eq!(n, Some(0));
        assert!(note.is_none());
    }

    #[test]
    fn behind_by_commits_explains_when_running_unknown() {
        let (n, note) = behind_by_commits("unknown", "abc");
        assert!(n.is_none());
        assert!(note.unwrap().contains("running sha unknown"));
    }
}
