//! Phase11e — collection / index coverage audit.
//!
//! Compares the set of collections (Vectorizer) and indexes (Meili)
//! the orchestrator's routing matrix expects against what the live
//! servers actually host. Surfaces three operational signals:
//!
//! 1. **Boot-time `WARN` per missing collection** so the gap is
//!    visible at startup instead of only at query time.
//! 2. **`/v1/health/coverage`** so the dashboard's Health view can
//!    render the diff inline.
//! 3. **`cortex-ops doctor-coverage`** exit-code-mapped CLI.
//!
//! Decision: the canonical family list lives in
//! [`cortex_workers::fulltext::routing::FAMILIES`]; we re-state it
//! here because cortex-api does not depend on cortex-workers (would
//! pull in the entire worker binary tree). A `debug_assert!` in
//! [`expected_families`] documents the duplication so a future drift
//! between writer and auditor blows up in dev / test builds.
//!
//! Slugs come from whatever populated the cortex-api keyword lane
//! (the existing source of truth for `indexed_repos`) plus an
//! optional `CORTEX_COVERAGE_SLUGS` override that lets operators
//! pin extra slugs the keyword lane has not seen yet.

use std::collections::BTreeSet;

/// Canonical list of family suffixes the routing matrix uses. Pinned
/// to spec 08 §Routing matrix and mirrored in
/// [`cortex_workers::fulltext::routing::FAMILIES`]. The two MUST stay
/// aligned — see [`debug_assert_families_match`] for the in-tree
/// regression guard.
pub const EXPECTED_FAMILIES: &[&str] = &[
    "code",
    "docs",
    "decisions",
    "turns",
    "governance",
    "analyses",
    "knowledge",
    "learnings",
    "misc",
];

/// Build the canonical collection-name set for the given `slugs`,
/// crossed with [`EXPECTED_FAMILIES`].
///
/// Names follow the `cortex-{slug}-{family}` convention. `slugs` is
/// expected pre-normalised (lowercase, ASCII slug — same shape the
/// `cortex_storage::names::slug_for_repo` writer side produces).
pub fn expected_collections(slugs: &BTreeSet<String>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for slug in slugs {
        if slug.is_empty() {
            continue;
        }
        for family in EXPECTED_FAMILIES {
            out.insert(format!("cortex-{slug}-{family}"));
        }
    }
    out
}

/// Result of one diff. `present` and `missing` are subsets of
/// `expected`; `unexpected` lists anything the live backend hosts
/// that the routing matrix does not reference (stale collections /
/// indexes left over from migrations — phase4a-style debris).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoverageDiff {
    /// Set of collections / indexes the orchestrator expects.
    pub expected: BTreeSet<String>,
    /// Subset of `expected` that the live backend hosts.
    pub present: BTreeSet<String>,
    /// Subset of `expected` that the live backend is missing.
    pub missing: BTreeSet<String>,
    /// Live names that are NOT in `expected` (orphans / debris).
    pub unexpected: BTreeSet<String>,
}

impl CoverageDiff {
    /// Convenience: severity for the operator-facing surface.
    /// `Ok` when nothing is missing, `Warn` when at least one expected
    /// name is absent, `Critical` when nothing at all is hosted.
    pub fn severity(&self) -> CoverageSeverity {
        if self.expected.is_empty() {
            return CoverageSeverity::Ok;
        }
        if self.present.is_empty() {
            return CoverageSeverity::Critical;
        }
        if !self.missing.is_empty() {
            return CoverageSeverity::Warn;
        }
        CoverageSeverity::Ok
    }
}

/// Severity ordering the dashboard / CLI can map to exit codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CoverageSeverity {
    /// Every expected name is present.
    Ok,
    /// At least one expected name is missing.
    Warn,
    /// Nothing expected is hosted at all.
    Critical,
}

impl CoverageSeverity {
    /// Map to a process exit code: `0` ok, `1` warn, `2` critical.
    pub fn exit_code(self) -> i32 {
        match self {
            CoverageSeverity::Ok => 0,
            CoverageSeverity::Warn => 1,
            CoverageSeverity::Critical => 2,
        }
    }
}

/// Build the diff. `expected` and `live` may share none, all, or some
/// of their members; the diff is purely set-theoretic.
pub fn diff(expected: BTreeSet<String>, live: BTreeSet<String>) -> CoverageDiff {
    let present: BTreeSet<String> = expected.intersection(&live).cloned().collect();
    let missing: BTreeSet<String> = expected.difference(&live).cloned().collect();
    let unexpected: BTreeSet<String> = live.difference(&expected).cloned().collect();
    CoverageDiff {
        expected,
        present,
        missing,
        unexpected,
    }
}

/// Pull extra slugs out of `CORTEX_COVERAGE_SLUGS` (CSV). Empty / unset
/// returns an empty set. Operators use this to pin slugs the keyword
/// lane has not yet seen so the inventory diff catches missing
/// collections for new repos before the first envelope arrives.
pub fn slugs_from_env(env_value: Option<&str>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let Some(raw) = env_value else { return out };
    for piece in raw.split(',') {
        let trimmed = piece.trim();
        if !trimmed.is_empty() {
            out.insert(trimmed.to_ascii_lowercase());
        }
    }
    out
}

/// Compile-time-ish sanity check: cortex-api's [`EXPECTED_FAMILIES`]
/// must stay aligned with the writer-side family vocabulary in
/// `cortex-workers`. We can't `debug_assert!` against the workers
/// crate (no dep), but tests below pin every family the writer side
/// emits — drift is caught the moment the writer adds a family.
#[cfg(test)]
pub(crate) fn debug_assert_families_match(_writer_families: &[&str]) {
    debug_assert!(
        _writer_families.iter().all(|f| EXPECTED_FAMILIES.contains(f)),
        "writer ships family the auditor doesn't know about: {_writer_families:?} vs {EXPECTED_FAMILIES:?}"
    );
}

/// Fetch the live Vectorizer collection-name set via
/// `GET /collections`. Bearer token is optional (anon dev stack);
/// when present, sent as `Authorization: Bearer <jwt>`. Times out at
/// `timeout_ms` so a stuck server cannot block boot.
pub async fn fetch_vectorizer_collection_names(
    http: &reqwest::Client,
    base_url: &str,
    bearer: Option<&str>,
    timeout: std::time::Duration,
) -> Result<BTreeSet<String>, String> {
    let url = format!("{}/collections", base_url.trim_end_matches('/'));
    let mut req = http.get(&url).timeout(timeout);
    if let Some(token) = bearer {
        req = req.bearer_auth(token);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("vectorizer GET /collections {base_url}: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "vectorizer GET /collections {base_url}: HTTP {} {}",
            status.as_u16(),
            body.chars().take(200).collect::<String>()
        ));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("vectorizer GET /collections {base_url} parse: {e}"))?;
    // Accept both shapes: `{collections: [{name, ...}, ...]}` (live
    // Vectorizer 3.0) and a bare array `[{name, ...}, ...]` (legacy).
    let arr = body
        .get("collections")
        .and_then(|v| v.as_array())
        .or_else(|| body.as_array())
        .ok_or_else(|| {
            format!(
                "vectorizer GET /collections {base_url}: unexpected shape (no `collections` array)"
            )
        })?;
    let mut out = BTreeSet::new();
    for entry in arr {
        if let Some(name) = entry.get("name").and_then(|v| v.as_str()) {
            out.insert(name.to_string());
        }
    }
    Ok(out)
}

/// Phase11m §1 — fetch the set of repos that have at least one
/// `Repo` node in Nexus. Used by the coverage audit so the dashboard
/// can flag slugs the orchestrator expects (driven by the keyword
/// lane + `CORTEX_COVERAGE_SLUGS` env) but the graph backend has
/// never seen — typically a sign the graph-writer skipped the repo
/// or the bootstrap phase never ran.
///
/// Names are lower-cased to match the slug normalisation the rest of
/// the audit applies. `Repo.name` is the writer-side identity
/// property (see `cortex_workers::graph::schema::INIT_STATEMENTS`'s
/// `CREATE CONSTRAINT repo_name FOR (r:Repo) REQUIRE r.name IS UNIQUE`).
pub async fn fetch_nexus_repo_names(
    client: &nexus_sdk::NexusClient,
    timeout: std::time::Duration,
) -> Result<BTreeSet<String>, String> {
    let cypher = "MATCH (r:Repo) RETURN r.name AS name";
    let res = tokio::time::timeout(timeout, client.execute_cypher(cypher, None))
        .await
        .map_err(|_| format!("nexus query timeout after {:?}", timeout))?
        .map_err(|e| format!("nexus execute_cypher: {e}"))?;
    let mut out = BTreeSet::new();
    for row in &res.rows {
        let Some(cells) = row.as_array() else { continue };
        if let Some(name) = cells.first().and_then(|v| v.as_str()) {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                out.insert(trimmed.to_ascii_lowercase());
            }
        }
    }
    Ok(out)
}

/// Fetch the live Meilisearch index-uid set via `GET /indexes`. The
/// master key is optional (Meili can be unauthenticated in dev). Times
/// out at `timeout` so a stuck server cannot block boot.
pub async fn fetch_meili_index_uids(
    http: &reqwest::Client,
    base_url: &str,
    api_key: Option<&str>,
    timeout: std::time::Duration,
) -> Result<BTreeSet<String>, String> {
    let url = format!("{}/indexes?limit=1000", base_url.trim_end_matches('/'));
    let mut req = http.get(&url).timeout(timeout);
    if let Some(key) = api_key {
        req = req.bearer_auth(key);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("meili GET /indexes {base_url}: {e}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Err(format!(
            "meili GET /indexes {base_url}: HTTP {} {}",
            status.as_u16(),
            body.chars().take(200).collect::<String>()
        ));
    }
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("meili GET /indexes {base_url} parse: {e}"))?;
    // Meili surfaces results under `{results: [{uid, ...}, ...], ...}`.
    let arr = body
        .get("results")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            format!("meili GET /indexes {base_url}: unexpected shape (no `results` array)")
        })?;
    let mut out = BTreeSet::new();
    for entry in arr {
        if let Some(uid) = entry.get("uid").and_then(|v| v.as_str()) {
            out.insert(uid.to_string());
        }
    }
    Ok(out)
}

/// Emit a structured boot-time WARN per missing collection / index.
/// Centralised so the dashboard / CLI / boot path agree on the field
/// shape: `backend`, `collection`, `slug`, `family`. The diff also
/// logs a single INFO summary line so operators see the per-backend
/// coverage ratio without scraping individual WARNs.
pub fn log_diff(backend: &str, base_url: &str, diff: &CoverageDiff) {
    let total = diff.expected.len();
    let present = diff.present.len();
    let missing = diff.missing.len();
    let unexpected = diff.unexpected.len();
    tracing::info!(
        backend,
        base_url,
        expected = total,
        present,
        missing,
        unexpected,
        severity = ?diff.severity(),
        "coverage audit"
    );
    for name in &diff.missing {
        let (slug, family) = parse_canonical_name(name);
        tracing::warn!(
            backend,
            base_url,
            collection = %name,
            slug = %slug.unwrap_or("?"),
            family = %family.unwrap_or("?"),
            "coverage: expected collection / index missing"
        );
    }
    for name in &diff.unexpected {
        tracing::debug!(
            backend,
            base_url,
            collection = %name,
            "coverage: unexpected (orphan / debris) collection / index"
        );
    }
}

/// Per-backend audit result, suitable for serialisation to the
/// `/v1/health/coverage` endpoint.
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackendCoverage {
    /// Backend label (`vectorizer` or `meili`).
    pub backend: &'static str,
    /// Base URL probed (or `null` when env was unset).
    pub base_url: Option<String>,
    /// Overall severity for the backend.
    pub severity: &'static str,
    /// Number of expected names.
    pub expected_count: usize,
    /// Number of expected names that are present.
    pub present_count: usize,
    /// Number of expected names that are missing.
    pub missing_count: usize,
    /// Number of unexpected (orphan) live names.
    pub unexpected_count: usize,
    /// Sorted list of present names.
    pub present: Vec<String>,
    /// Sorted list of missing names.
    pub missing: Vec<String>,
    /// Sorted list of unexpected names.
    pub unexpected: Vec<String>,
    /// Last-error string when the diff could not run (e.g. server
    /// unreachable, JSON shape mismatch). `None` on a clean diff.
    pub error: Option<String>,
}

/// Aggregate response shape for `/v1/health/coverage`.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CoverageResponse {
    /// Slugs the auditor cross-referenced against [`EXPECTED_FAMILIES`].
    pub slugs: Vec<String>,
    /// Family suffixes the auditor crossed every slug with.
    pub families: Vec<&'static str>,
    /// Per-backend coverage diff.
    pub backends: Vec<BackendCoverage>,
    /// Overall severity (worst across backends).
    pub overall_severity: &'static str,
    /// Phase11i §5.2 — per-watcher health summary for every
    /// configured `cortex-claude-archive` instance. Empty when no
    /// `CORTEX_ARCHIVE_WATCHER_URLS` are configured.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub archive_watchers: Vec<ArchiveWatcherSummary>,
}

/// Phase11i §5.2 — one entry under
/// `CoverageResponse.archive_watchers`. Mirrors the JSON returned by
/// the watcher's `:17030/healthz` endpoint plus the probed URL and
/// (when the probe failed) a human-readable error string. The
/// watcher uses `cortex_claude_archive::tail::HealthSnapshot` as the
/// source of truth; this type re-shapes it into a cortex-api-owned
/// surface so the dashboard does not depend on the watcher crate.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ArchiveWatcherSummary {
    /// URL probed (matches one of `CORTEX_ARCHIVE_WATCHER_URLS`).
    pub url: String,
    /// `"healthy"` / `"degraded: <reason>"` / `"unreachable"`.
    pub status: String,
    /// Epoch ms of the watcher's most recent emitter flush.
    /// `0` when the watcher reports no flushes yet (cold start).
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub last_flush_ts: i64,
    /// `.jsonl` files the watcher's most recent walk surfaced.
    #[serde(default, skip_serializing_if = "is_zero_usize")]
    pub files_watched: usize,
    /// Envelopes per second over the watcher's most recent sample.
    #[serde(default, skip_serializing_if = "is_zero_f64")]
    pub envelope_rate: f64,
    /// Process RSS in bytes (sysinfo).
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub rss_bytes: u64,
    /// Cumulative envelopes emitted since the watcher started.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub envelopes_emitted: u64,
    /// Cumulative envelopes the watcher rejected.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub envelopes_dropped: u64,
    /// Wall-clock ms since the watcher started.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub uptime_ms: u64,
    /// Probe error, when the watcher could not be reached / parsed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn is_zero_i64(v: &i64) -> bool {
    *v == 0
}
fn is_zero_usize(v: &usize) -> bool {
    *v == 0
}
fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}
fn is_zero_f64(v: &f64) -> bool {
    *v == 0.0
}

#[derive(Debug, Clone, serde::Deserialize)]
struct WatcherHealthBody {
    #[serde(default)]
    last_flush_ts: i64,
    #[serde(default)]
    files_watched: usize,
    #[serde(default)]
    envelope_rate: f64,
    #[serde(default)]
    rss_bytes: u64,
    #[serde(default)]
    envelopes_emitted: u64,
    #[serde(default)]
    envelopes_dropped: u64,
    #[serde(default)]
    uptime_ms: u64,
    #[serde(default)]
    status: String,
}

/// Phase11i §5.2 — probe one watcher's `/healthz` endpoint and
/// return a summary suitable for the coverage payload. Surfaces
/// transport / parse errors as `status = "unreachable"` with the
/// upstream message in `error` so the dashboard can render the
/// degradation without crashing.
pub async fn fetch_archive_watcher_status(
    http: &reqwest::Client,
    url: &str,
    timeout: std::time::Duration,
) -> ArchiveWatcherSummary {
    let probe_url = format!("{}/healthz", url.trim_end_matches('/'));
    let send = http.get(&probe_url).timeout(timeout).send().await;
    let resp = match send {
        Ok(r) => r,
        Err(err) => {
            return ArchiveWatcherSummary {
                url: url.to_string(),
                status: "unreachable".into(),
                last_flush_ts: 0,
                files_watched: 0,
                envelope_rate: 0.0,
                rss_bytes: 0,
                envelopes_emitted: 0,
                envelopes_dropped: 0,
                uptime_ms: 0,
                error: Some(format!("GET {probe_url}: {err}")),
            };
        }
    };
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return ArchiveWatcherSummary {
            url: url.to_string(),
            status: "unreachable".into(),
            last_flush_ts: 0,
            files_watched: 0,
            envelope_rate: 0.0,
            rss_bytes: 0,
            envelopes_emitted: 0,
            envelopes_dropped: 0,
            uptime_ms: 0,
            error: Some(format!("{probe_url} returned {status}: {body}")),
        };
    }
    match resp.json::<WatcherHealthBody>().await {
        Ok(body) => ArchiveWatcherSummary {
            url: url.to_string(),
            status: if body.status.is_empty() {
                "healthy".into()
            } else {
                body.status
            },
            last_flush_ts: body.last_flush_ts,
            files_watched: body.files_watched,
            envelope_rate: body.envelope_rate,
            rss_bytes: body.rss_bytes,
            envelopes_emitted: body.envelopes_emitted,
            envelopes_dropped: body.envelopes_dropped,
            uptime_ms: body.uptime_ms,
            error: None,
        },
        Err(err) => ArchiveWatcherSummary {
            url: url.to_string(),
            status: "unreachable".into(),
            last_flush_ts: 0,
            files_watched: 0,
            envelope_rate: 0.0,
            rss_bytes: 0,
            envelopes_emitted: 0,
            envelopes_dropped: 0,
            uptime_ms: 0,
            error: Some(format!("parse {probe_url}: {err}")),
        },
    }
}

/// Phase11i §5.2 — resolve archive watcher URLs from the
/// `CORTEX_ARCHIVE_WATCHER_URLS` env var (comma-separated). Empty
/// / unset returns an empty Vec so the coverage path skips the
/// archive_watchers block entirely.
pub fn resolve_archive_watcher_urls() -> Vec<String> {
    std::env::var("CORTEX_ARCHIVE_WATCHER_URLS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

fn severity_label(s: CoverageSeverity) -> &'static str {
    match s {
        CoverageSeverity::Ok => "ok",
        CoverageSeverity::Warn => "warn",
        CoverageSeverity::Critical => "critical",
    }
}

fn into_backend_coverage(
    backend: &'static str,
    base_url: Option<String>,
    diff: CoverageDiff,
    error: Option<String>,
) -> BackendCoverage {
    BackendCoverage {
        backend,
        base_url,
        severity: severity_label(diff.severity()),
        expected_count: diff.expected.len(),
        present_count: diff.present.len(),
        missing_count: diff.missing.len(),
        unexpected_count: diff.unexpected.len(),
        present: diff.present.into_iter().collect(),
        missing: diff.missing.into_iter().collect(),
        unexpected: diff.unexpected.into_iter().collect(),
        error,
    }
}

/// Run a Vectorizer + Meili coverage diff for the given slug set and
/// return a serialisable [`CoverageResponse`]. Handles missing env
/// (skipped backend reports `severity: ok` with `error: None`),
/// transport errors (severity propagates from whatever names DID
/// come back; `error` carries the upstream message), and bearer
/// resolution.
pub async fn run_coverage_audit(
    http: &reqwest::Client,
    slugs: BTreeSet<String>,
    vectorizer_url: Option<&str>,
    vectorizer_bearer: Option<&str>,
    meili_url: Option<&str>,
    meili_api_key: Option<&str>,
    timeout: std::time::Duration,
) -> CoverageResponse {
    run_coverage_audit_with_archive_watchers(
        http,
        slugs,
        vectorizer_url,
        vectorizer_bearer,
        meili_url,
        meili_api_key,
        &[],
        timeout,
    )
    .await
}

/// Phase11i §5.2 — same as [`run_coverage_audit`] but also probes
/// every URL in `archive_watcher_urls` for its `:17030/healthz`
/// snapshot. Used by the live `/v1/health/coverage` handler so the
/// dashboard surfaces watcher health alongside the lane backends.
#[allow(clippy::too_many_arguments)]
pub async fn run_coverage_audit_with_archive_watchers(
    http: &reqwest::Client,
    slugs: BTreeSet<String>,
    vectorizer_url: Option<&str>,
    vectorizer_bearer: Option<&str>,
    meili_url: Option<&str>,
    meili_api_key: Option<&str>,
    archive_watcher_urls: &[String],
    timeout: std::time::Duration,
) -> CoverageResponse {
    run_coverage_audit_full(
        http,
        slugs,
        vectorizer_url,
        vectorizer_bearer,
        meili_url,
        meili_api_key,
        None,
        archive_watcher_urls,
        timeout,
    )
    .await
}

/// Phase11m §1 — full audit that, on top of Vectorizer + Meili +
/// archive watchers, also probes Nexus when a client is provided.
/// Per-slug "present" means the graph backend has at least one
/// `Repo` node with `r.name == <slug>`.
#[allow(clippy::too_many_arguments)]
pub async fn run_coverage_audit_full(
    http: &reqwest::Client,
    slugs: BTreeSet<String>,
    vectorizer_url: Option<&str>,
    vectorizer_bearer: Option<&str>,
    meili_url: Option<&str>,
    meili_api_key: Option<&str>,
    nexus_client: Option<&nexus_sdk::NexusClient>,
    archive_watcher_urls: &[String],
    timeout: std::time::Duration,
) -> CoverageResponse {
    let expected = expected_collections(&slugs);
    let mut backends: Vec<BackendCoverage> = Vec::with_capacity(2);
    let mut overall = CoverageSeverity::Ok;

    if let Some(url) = vectorizer_url {
        let (live, err) =
            match fetch_vectorizer_collection_names(http, url, vectorizer_bearer, timeout).await {
                Ok(set) => (set, None),
                Err(reason) => (BTreeSet::new(), Some(reason)),
            };
        let d = diff(expected.clone(), live);
        let sev = d.severity();
        if sev > overall {
            overall = sev;
        }
        backends.push(into_backend_coverage(
            "vectorizer",
            Some(url.to_string()),
            d,
            err,
        ));
    }

    if let Some(url) = meili_url {
        let (live, err) = match fetch_meili_index_uids(http, url, meili_api_key, timeout).await {
            Ok(set) => (set, None),
            Err(reason) => (BTreeSet::new(), Some(reason)),
        };
        let d = diff(expected.clone(), live);
        let sev = d.severity();
        if sev > overall {
            overall = sev;
        }
        backends.push(into_backend_coverage(
            "meili",
            Some(url.to_string()),
            d,
            err,
        ));
    }

    if let Some(client) = nexus_client {
        // Nexus uses the slug itself as the "name" because the graph
        // is global — there is no per-slug × family cross-product.
        // Expected = the slugs the orchestrator audits; live = slugs
        // with at least one `Repo` node in Nexus.
        let nexus_expected: BTreeSet<String> = slugs.clone();
        let (live, err) = match fetch_nexus_repo_names(client, timeout).await {
            Ok(set) => (set, None),
            Err(reason) => (BTreeSet::new(), Some(reason)),
        };
        let d = diff(nexus_expected, live);
        let sev = d.severity();
        if sev > overall {
            overall = sev;
        }
        backends.push(into_backend_coverage("nexus", None, d, err));
    }

    let mut archive_watchers: Vec<ArchiveWatcherSummary> = Vec::new();
    for url in archive_watcher_urls {
        archive_watchers.push(fetch_archive_watcher_status(http, url, timeout).await);
    }

    CoverageResponse {
        slugs: slugs.into_iter().collect(),
        families: EXPECTED_FAMILIES.to_vec(),
        backends,
        overall_severity: severity_label(overall),
        archive_watchers,
    }
}

/// Split a canonical `cortex-{slug}-{family}` name into its slug and
/// family parts. Returns `(None, None)` for non-canonical names.
fn parse_canonical_name(name: &str) -> (Option<&str>, Option<&str>) {
    let rest = match name.strip_prefix("cortex-") {
        Some(r) => r,
        None => return (None, None),
    };
    for family in EXPECTED_FAMILIES {
        let suffix = format!("-{family}");
        if let Some(slug) = rest.strip_suffix(&suffix) {
            if !slug.is_empty() {
                return (Some(slug), Some(*family));
            }
        }
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slugs(s: &[&str]) -> BTreeSet<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    fn live(s: &[&str]) -> BTreeSet<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn resolve_archive_watcher_urls_splits_csv_and_trims() {
        let prior = std::env::var("CORTEX_ARCHIVE_WATCHER_URLS").ok();
        std::env::set_var(
            "CORTEX_ARCHIVE_WATCHER_URLS",
            "http://a:17030 , http://b:17030,, http://c:17030",
        );
        let v = resolve_archive_watcher_urls();
        assert_eq!(
            v,
            vec![
                "http://a:17030".to_string(),
                "http://b:17030".to_string(),
                "http://c:17030".to_string()
            ]
        );
        std::env::remove_var("CORTEX_ARCHIVE_WATCHER_URLS");
        assert!(resolve_archive_watcher_urls().is_empty());
        if let Some(v) = prior {
            std::env::set_var("CORTEX_ARCHIVE_WATCHER_URLS", v);
        }
    }

    #[test]
    fn archive_watcher_summary_serialises_zero_fields_compactly() {
        // Fields default to 0 / 0.0 / empty get skipped via
        // `skip_serializing_if`, so an unreachable watcher payload
        // stays compact (just `url`, `status`, `error`).
        let s = ArchiveWatcherSummary {
            url: "http://x:17030".into(),
            status: "unreachable".into(),
            last_flush_ts: 0,
            files_watched: 0,
            envelope_rate: 0.0,
            rss_bytes: 0,
            envelopes_emitted: 0,
            envelopes_dropped: 0,
            uptime_ms: 0,
            error: Some("connection refused".into()),
        };
        let v = serde_json::to_value(&s).unwrap();
        let obj = v.as_object().unwrap();
        assert_eq!(obj.get("url").unwrap(), "http://x:17030");
        assert_eq!(obj.get("status").unwrap(), "unreachable");
        assert_eq!(obj.get("error").unwrap(), "connection refused");
        assert!(!obj.contains_key("last_flush_ts"));
        assert!(!obj.contains_key("envelope_rate"));
        assert!(!obj.contains_key("envelopes_emitted"));
    }

    #[test]
    fn coverage_response_omits_archive_watchers_when_empty() {
        let resp = CoverageResponse {
            slugs: vec!["cortex".into()],
            families: EXPECTED_FAMILIES.to_vec(),
            backends: vec![],
            overall_severity: "ok",
            archive_watchers: Vec::new(),
        };
        let v = serde_json::to_value(&resp).unwrap();
        assert!(!v.as_object().unwrap().contains_key("archive_watchers"));
    }

    #[test]
    fn expected_collections_crosses_slugs_with_families() {
        let got = expected_collections(&slugs(&["cortex"]));
        // 9 families × 1 slug = 9 names, all `cortex-cortex-*`.
        assert_eq!(got.len(), EXPECTED_FAMILIES.len());
        assert!(got.contains("cortex-cortex-decisions"));
        assert!(got.contains("cortex-cortex-turns"));
        assert!(got.contains("cortex-cortex-misc"));
        // Two slugs ⇒ 2 × families collections.
        let two = expected_collections(&slugs(&["cortex", "rulebook"]));
        assert_eq!(two.len(), EXPECTED_FAMILIES.len() * 2);
        assert!(two.contains("cortex-rulebook-decisions"));
    }

    #[test]
    fn empty_slug_is_ignored() {
        let got = expected_collections(&slugs(&[""]));
        assert!(got.is_empty());
    }

    #[test]
    fn diff_partitions_into_present_missing_unexpected() {
        let expected = expected_collections(&slugs(&["cortex"]));
        let live = live(&[
            "cortex-cortex-code",
            "cortex-cortex-docs",
            "cortex-cortex-misc",
            "cortex-cortex-governance",
            // ↑ the four collections phase11e probed live.
            "cortex-orphan-something", // unexpected debris
        ]);
        let d = diff(expected.clone(), live);
        assert_eq!(d.expected, expected);
        assert!(d.present.contains("cortex-cortex-code"));
        assert!(d.missing.contains("cortex-cortex-decisions"));
        assert!(d.missing.contains("cortex-cortex-turns"));
        assert!(d.missing.contains("cortex-cortex-analyses"));
        assert!(d.unexpected.contains("cortex-orphan-something"));
        assert_eq!(d.severity(), CoverageSeverity::Warn);
    }

    #[test]
    fn severity_critical_when_live_is_empty_but_expected_isnt() {
        let expected = expected_collections(&slugs(&["cortex"]));
        let d = diff(expected, BTreeSet::new());
        assert_eq!(d.severity(), CoverageSeverity::Critical);
        assert_eq!(d.severity().exit_code(), 2);
    }

    #[test]
    fn severity_ok_when_every_expected_is_present() {
        let expected = expected_collections(&slugs(&["cortex"]));
        let live = expected.clone();
        let d = diff(expected, live);
        assert_eq!(d.severity(), CoverageSeverity::Ok);
        assert_eq!(d.severity().exit_code(), 0);
        assert!(d.missing.is_empty());
    }

    #[test]
    fn severity_ok_when_expected_is_empty() {
        let d = diff(BTreeSet::new(), BTreeSet::new());
        assert_eq!(d.severity(), CoverageSeverity::Ok);
    }

    #[test]
    fn slugs_from_env_handles_empty_and_csv() {
        assert!(slugs_from_env(None).is_empty());
        assert!(slugs_from_env(Some("")).is_empty());
        assert!(slugs_from_env(Some("  ,  ,")).is_empty());
        let s = slugs_from_env(Some("Cortex, rulebook ,nexus"));
        assert!(s.contains("cortex"));
        assert!(s.contains("rulebook"));
        assert!(s.contains("nexus"));
        assert_eq!(s.len(), 3);
    }

    #[test]
    fn parse_canonical_name_extracts_slug_and_family() {
        assert_eq!(
            super::parse_canonical_name("cortex-cortex-decisions"),
            (Some("cortex"), Some("decisions"))
        );
        assert_eq!(
            super::parse_canonical_name("cortex-rulebook-knowledge"),
            (Some("rulebook"), Some("knowledge"))
        );
        assert_eq!(
            super::parse_canonical_name("cortex-cortex-mcp-code"),
            (Some("cortex-mcp"), Some("code"))
        );
        assert_eq!(
            super::parse_canonical_name("legacy-cortex-foo"),
            (None, None),
        );
        assert_eq!(
            super::parse_canonical_name("cortex-cortex-bogus"),
            (None, None)
        );
    }

    #[test]
    fn nexus_diff_treats_slugs_themselves_as_expected_names() {
        // Phase11m §1 — Nexus uses slugs (not slug × family) as the
        // expected set. Verify the diff partitions slugs the right
        // way when fed simulated `live = nexus_repo_names` output.
        let slugs: BTreeSet<String> =
            ["cortex", "rulebook", "vectorizer"].into_iter().map(String::from).collect();
        let live: BTreeSet<String> = ["cortex", "vectorizer"].into_iter().map(String::from).collect();
        let d = diff(slugs, live);
        assert_eq!(d.expected.len(), 3);
        assert_eq!(d.present.len(), 2);
        assert!(d.missing.contains("rulebook"));
        assert_eq!(d.severity(), CoverageSeverity::Warn);
    }

    #[test]
    fn families_pin_every_writer_side_family_used_today() {
        // Writer side (cortex-workers) ships these — keep this list
        // aligned with the FAMILIES constant in
        // `cortex_workers::fulltext::routing::FAMILIES` and the
        // routing match in `cortex_workers::embedder::routing`.
        let writer_families = [
            "code",
            "docs",
            "decisions",
            "turns",
            "governance",
            "analyses",
            "knowledge",
            "learnings",
            "misc",
        ];
        debug_assert_families_match(&writer_families);
        for f in writer_families {
            assert!(
                EXPECTED_FAMILIES.contains(&f),
                "writer family {f:?} missing from auditor's EXPECTED_FAMILIES"
            );
        }
    }
}
