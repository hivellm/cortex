use super::helpers::home_dir;
use std::process::ExitCode;

/// Global decisions index name (mirrors `cortex_storage::names::INDEX_DECISIONS`).
const INDEX_DECISIONS: &str = "cortex_decisions";

pub(super) fn doctor(
    vectorizer: Option<String>,
    nexus: Option<String>,
    synap: Option<String>,
    meili: Option<String>,
) -> ExitCode {
    // We intentionally do not pull in reqwest here: this binary should stay
    // dependency-light. Doctor delegates to `curl` which is present on every
    // Unix + Windows-with-modern-powershell host.
    let vectorizer = vectorizer.or_else(|| std::env::var("VECTORIZER_URL").ok());
    let nexus = nexus.or_else(|| std::env::var("NEXUS_URL").ok());
    let synap = synap.or_else(|| std::env::var("SYNAP_URL").ok());
    let meili = meili.or_else(|| std::env::var("MEILI_URL").ok());

    let checks: &[(&str, Option<String>, &str)] = &[
        ("vectorizer", vectorizer, "/health"),
        ("nexus", nexus, "/health"),
        ("synap", synap, "/health"),
        ("meilisearch", meili, "/health"),
    ];

    let mut any_failure = false;
    for (name, base, path) in checks {
        match base {
            Some(b) => {
                let url = format!("{}{}", b.trim_end_matches('/'), path);
                let ok = std::process::Command::new("curl")
                    .args(["-fsS", "--max-time", "3", "-o", "/dev/null", &url])
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false);
                if ok {
                    println!("ok     {:<12} {url}", name);
                } else {
                    println!("fail   {:<12} {url}", name);
                    any_failure = true;
                }
            }
            None => {
                println!("skip   {:<12} (no URL configured)", name);
            }
        }
    }

    // phase10g — mounted-route smoke check. The audit caught the
    // GUI's Health tab returning empty bodies on every
    // `/v1/health/*` call against the live daemon because a
    // future refactor could drop the merge() that mounts them.
    // Doctor probes the five canonical routes against
    // `CORTEX_API_URL` so a missed registration shows up as a
    // doctor red BEFORE the operator opens the GUI.
    let cfg = cortex_config::Config::load().unwrap_or_default();
    if let Some(api) = cfg.dashboard.api_url.clone().filter(|s| !s.is_empty()) {
        for path in [
            "/v1/health",
            "/v1/health/freshness",
            "/v1/health/divergence",
            "/v1/health/versions",
            "/v1/health/config",
        ] {
            let url = format!("{}{}", api.trim_end_matches('/'), path);
            let ok = std::process::Command::new("curl")
                .args(["-fsS", "--max-time", "3", "-o", "/dev/null", &url])
                .status()
                .map(|s| s.success())
                .unwrap_or(false);
            let label = format!("api{path}");
            if ok {
                println!("ok     {label:<28} {url}");
            } else {
                println!("fail   {label:<28} {url}");
                any_failure = true;
            }
        }
    } else {
        println!(
            "skip   {label:<28} (set CORTEX_API_URL to probe the daemon's /v1/health/* routes)",
            label = "api/v1/health/*"
        );
    }

    // Phase11s §1.4 — classifier-worker liveness probe. The
    // 2026-05-02 incident showed the worker can stay
    // `/healthz`-green while its consume loop is dead for hours.
    // Read the worker's `/healthz.extras.last_consume_ts_ms` and
    // flag the worker as `degraded` when the timestamp is older
    // than `CORTEX_CLASSIFIER_STALENESS_MS` (default 60_000).
    if let Some(classifier) = cfg.classifier.health_url.clone().filter(|s| !s.is_empty()) {
        let staleness_ms: u64 = cfg.classifier.staleness_ms.unwrap_or(60_000);
        let url = format!("{}/healthz", classifier.trim_end_matches('/'));
        let body = std::process::Command::new("curl")
            .args(["-fsS", "--max-time", "3", &url])
            .output()
            .ok()
            .and_then(|o| {
                if o.status.success() {
                    Some(o.stdout)
                } else {
                    None
                }
            })
            .and_then(|b| String::from_utf8(b).ok());
        match body {
            Some(b) => {
                let parsed: serde_json::Value = serde_json::from_str(&b).unwrap_or_default();
                let last_consume_ts_ms = parsed
                    .pointer("/extras/last_consume_ts_ms")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let consecutive = parsed
                    .pointer("/extras/consume_errors_consecutive")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let now_ms = chrono::Utc::now().timestamp_millis().max(0) as u64;
                let age_ms = now_ms.saturating_sub(last_consume_ts_ms);
                if last_consume_ts_ms == 0 {
                    println!(
                        "warn   {:<28} {url} (last_consume_ts_ms not reported — pre-§1.3 build?)",
                        "classifier-worker"
                    );
                } else if age_ms > staleness_ms {
                    println!(
                        "fail   {:<28} {url} (degraded: classifier-worker stuck — \
                         last consume {age_ms} ms ago, threshold {staleness_ms} ms, \
                         consecutive_errors={consecutive})",
                        "classifier-worker"
                    );
                    any_failure = true;
                } else {
                    println!(
                        "ok     {:<28} {url} (last_consume_ts {age_ms} ms ago, \
                         consecutive_errors={consecutive})",
                        "classifier-worker"
                    );
                }
            }
            None => {
                println!("fail   {:<28} {url} (unreachable)", "classifier-worker");
                any_failure = true;
            }
        }
    } else {
        println!(
            "skip   {:<28} (set CORTEX_CLASSIFIER_HEALTH_URL to probe the worker's /healthz)",
            "classifier-worker"
        );
    }

    if any_failure {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Phase8d — `cortex-ops doctor-config`. Runs the cortex-api config
/// audit (read-only, static analysis) and renders either a plain-text
/// table or JSON. Exit codes match `Severity`: 0=ok, 1=warn, 2=critical.
pub(super) fn doctor_config(
    workspace: Option<String>,
    adapter_toml: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_api::config_audit::{run_audit_with, AuditOptions, AuditPaths, Severity};

    let workspace = workspace
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let mut paths = AuditPaths::default_for_workspace(&workspace);
    if let Some(p) = adapter_toml {
        paths.adapter_toml = std::path::PathBuf::from(p);
    }
    // Phase8d — `full()` adds live-port + cargo-tree -d scans on
    // top of the file-only static analysis so the CLI surfaces the
    // 2026-04-28 incident class (config says :17010 but daemon
    // bound :15010).
    let audit = run_audit_with(&paths, AuditOptions::full());
    if json {
        match serde_json::to_string_pretty(&audit) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize audit: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops doctor-config");
        println!("workspace: {}", workspace.display());
        println!("surfaces read: {}\n", audit.surfaces_read);
        for f in &audit.findings {
            let marker = match f.severity {
                Severity::Ok => "ok      ",
                Severity::Warn => "WARN    ",
                Severity::Critical => "CRITICAL",
            };
            println!("{marker}  [{}] {}", f.source, f.message);
        }
        println!("\nworst severity: {:?}", audit.worst_severity());
    }
    match audit.worst_severity() {
        Severity::Ok => ExitCode::SUCCESS,
        Severity::Warn => ExitCode::from(1),
        Severity::Critical => ExitCode::from(2),
    }
}

/// Phase8e — `cortex-ops doctor-alerts`. Lists every persisted
/// silent-drop alert under `~/.cortex/alerts/<pair>.json` (or the
/// `--state-dir` override). Exit codes: `0` no Critical alerts
/// active, `2` at least one Critical.
pub(super) fn doctor_alerts(state_dir: Option<String>, json: bool) -> ExitCode {
    use cortex_api::silent_drop::{AlertState, PairState};

    let dir: std::path::PathBuf = match state_dir {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home)
                .join(".cortex")
                .join("alerts")
        }
    };

    let mut rows: Vec<(String, PairState)> = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
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
            let pair = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("?")
                .to_string();
            rows.push((pair, state));
        }
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let any_critical = rows
        .iter()
        .any(|(_, s)| matches!(s.alert, AlertState::Critical));

    if json {
        let payload = serde_json::json!({
            "state_dir": dir.display().to_string(),
            "any_critical": any_critical,
            "alerts": rows
                .iter()
                .map(|(p, s)| serde_json::json!({
                    "pair": p,
                    "state": &s.alert,
                    "recovery_streak": s.recovery_streak,
                }))
                .collect::<Vec<_>>(),
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize alerts: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops doctor-alerts");
        println!("state_dir:    {}", dir.display());
        println!("any_critical: {any_critical}\n");
        if rows.is_empty() {
            println!(
                "(no persisted alert state — silent-drop watcher idle or no alerts since boot)"
            );
        } else {
            for (pair, state) in &rows {
                let label = match state.alert {
                    AlertState::Ok => "ok      ",
                    AlertState::Warn { .. } => "WARN    ",
                    AlertState::Critical => "CRITICAL",
                };
                println!(
                    "{label}  {} (recovery_streak={})",
                    pair, state.recovery_streak
                );
            }
        }
    }
    if any_critical {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

/// Phase11e §2.3 — `cortex-ops doctor-coverage`. Hits the
/// `cortex-api` daemon's `/v1/health/coverage` endpoint and
/// renders the per-backend collection / index inventory diff.
///
/// Exit codes mirror the audit's `overall_severity`:
/// - `0` — every expected name present (severity = ok)
/// - `1` — at least one missing (severity = warn)
/// - `2` — nothing expected is present at all (severity =
///   critical), or the daemon is unreachable
pub(super) fn doctor_coverage(api_url: Option<String>, json: bool) -> ExitCode {
    let url = api_url
        .or_else(|| {
            cortex_config::Config::load()
                .ok()
                .and_then(|c| c.dashboard.api_url)
        })
        .unwrap_or_else(|| "http://127.0.0.1:17000".to_string());
    let endpoint = format!("{}/v1/health/coverage", url.trim_end_matches('/'));

    // The workspace's `reqwest` feature set excludes `blocking`, so
    // build a local single-thread tokio runtime for this one
    // call rather than asking the whole binary to be async.
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("doctor-coverage: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };
    let payload: serde_json::Value = match rt.block_on(async {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let resp = http.get(&endpoint).send().await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "{endpoint} returned HTTP {} {}",
                status.as_u16(),
                body.chars().take(200).collect::<String>()
            ));
        }
        let parsed: serde_json::Value = resp.json().await?;
        Ok::<_, anyhow::Error>(parsed)
    }) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("doctor-coverage: {e}");
            return ExitCode::from(2);
        }
    };

    let overall = payload
        .get("overall_severity")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    if json {
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("doctor-coverage: serialize: {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        let slugs = payload
            .get("slugs")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        let families = payload
            .get("families")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        println!("cortex-ops doctor-coverage");
        println!("api_url:         {url}");
        println!("overall:         {overall}");
        println!(
            "expected:        {slugs} slugs × {families} families = {} names",
            slugs * families
        );
        println!();
        if let Some(backends) = payload.get("backends").and_then(|v| v.as_array()) {
            for backend in backends {
                let name = backend
                    .get("backend")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let sev = backend
                    .get("severity")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                let expected = backend
                    .get("expected_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let present = backend
                    .get("present_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let missing = backend
                    .get("missing_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let unexpected = backend
                    .get("unexpected_count")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let base_url = backend
                    .get("base_url")
                    .and_then(|v| v.as_str())
                    .unwrap_or("(not configured)");
                let ratio = if expected > 0 {
                    (present as f64 / expected as f64) * 100.0
                } else {
                    0.0
                };
                println!("[{sev:>4}] {name} @ {base_url}");
                println!(
                    "       {present}/{expected} present ({ratio:.0}%) · {missing} missing · {unexpected} orphan"
                );
                if let Some(missing_names) = backend.get("missing").and_then(|v| v.as_array()) {
                    let take = missing_names.iter().take(10);
                    for name in take {
                        if let Some(n) = name.as_str() {
                            println!("         missing: {n}");
                        }
                    }
                    if missing_names.len() > 10 {
                        println!(
                            "         …{} more missing — see /v1/health/coverage for full list",
                            missing_names.len() - 10
                        );
                    }
                }
                if let Some(err) = backend.get("error").and_then(|v| v.as_str()) {
                    println!("         error: {err}");
                }
                println!();
            }
        }
    }

    match overall {
        "ok" => ExitCode::SUCCESS,
        "warn" => ExitCode::from(1),
        _ => ExitCode::from(2),
    }
}

/// Wire the phase4d doctor: scan the archive, probe Meili, render
/// the report. Read-only end-to-end. Spins up a one-shot Tokio
/// runtime so the surrounding `main` stays sync (the rest of
/// `cortex-ops` does not need an async runtime).
#[allow(clippy::too_many_arguments)]
pub(super) fn doctor_consistency(
    archive_root: Option<String>,
    meili: Option<String>,
    meili_key: Option<String>,
    vectorizer: Option<String>,
    vectorizer_user: Option<String>,
    vectorizer_password: Option<String>,
    nexus: Option<String>,
    queries: Vec<String>,
    probe_k: usize,
    min_overlap_jaccard: f64,
    json: bool,
) -> ExitCode {
    let cfg = cortex_config::Config::load().unwrap_or_default();
    let archive_root = archive_root
        .or_else(|| cfg.ingestion.archive_root.clone())
        .unwrap_or_else(|| {
            home_dir()
                .map(|h| h.join(".cortex/archive").display().to_string())
                .unwrap_or_else(|| ".cortex/archive".to_string())
        });
    let meili_url = match meili.or_else(|| cfg.meili.meili_url.clone()) {
        Some(u) if !u.is_empty() => u,
        _ => {
            eprintln!("doctor consistency: --meili (or $CORTEX_FULLTEXT_MEILI_URL) is required");
            return ExitCode::FAILURE;
        }
    };
    let meili_key = meili_key.or_else(|| cfg.meili.meili_api_key.clone());
    let vectorizer_url = vectorizer
        .or_else(|| cfg.embedder.vectorizer_url.clone())
        .filter(|u| !u.is_empty());
    let vectorizer_user = vectorizer_user
        .or_else(|| {
            if cfg.embedder.vectorizer_user.is_empty() {
                None
            } else {
                Some(cfg.embedder.vectorizer_user.clone())
            }
        })
        .filter(|u| !u.is_empty());
    let vectorizer_password = vectorizer_password
        .or_else(|| cfg.embedder.vectorizer_password.clone())
        .filter(|u| !u.is_empty());
    let nexus_url = nexus
        .or_else(|| cfg.nexus.nexus_url.clone())
        .filter(|u| !u.is_empty());

    let archive = match cortex_cli::ops::ArchiveProbe::new(&archive_root).scan() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("archive scan failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Phase3 — `tool_call_hash_coverage` probe: walks the same archive
    // root and asserts ≥99% of `tool_call` envelopes captured in the
    // last 24 h carry a non-empty `content_hash`. The probe never
    // fails the run when the window is empty — that's a fresh-stack
    // skip rather than a regression.
    let hash_coverage = cortex_cli::ops::scan_hash_coverage(
        std::path::Path::new(&archive_root),
        chrono::Utc::now().timestamp_millis(),
        cortex_cli::ops::HASH_COVERAGE_WINDOW_HOURS,
        cortex_cli::ops::HASH_COVERAGE_THRESHOLD,
    );

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    let meili_result = runtime.block_on(probe_meili(&meili_url, meili_key.as_deref()));
    let (meili_partitions, non_canonical) = match meili_result {
        Ok(v) => v,
        Err(e) => {
            eprintln!("meili probe failed: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Vectorizer probe — only runs when both URL and credentials
    // are present. A missing-cred deployment falls back to the v1
    // archive ↔ Meili report.
    let (vec_partitions, non_canonical_vec) = if let (Some(url), Some(user), Some(pwd)) =
        (vectorizer_url, vectorizer_user, vectorizer_password)
    {
        match runtime.block_on(probe_vectorizer(&url, &user, &pwd)) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("vectorizer probe failed: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        (Default::default(), Vec::new())
    };

    let nexus_repo_counts = if let Some(url) = nexus_url {
        match runtime.block_on(probe_nexus(&url)) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("nexus probe failed: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        Default::default()
    };

    let mut report = cortex_cli::ops::coverage_report_full(
        archive,
        meili_partitions,
        non_canonical,
        vec_partitions,
        non_canonical_vec,
        nexus_repo_counts,
        cortex_cli::ops::CoverageOptions::default(),
    );
    let hash_failed = hash_coverage.failed;
    report.hash_coverage = Some(hash_coverage);
    if hash_failed {
        report.failed = true;
    }

    // Phase4i — query-overlap probes against the three live lanes.
    // Each lane fans out across its canonical partition list (Meili
    // indexes, Vectorizer collections) or runs a single Cypher
    // query (Nexus repo-grain), then dedupes the result paths into
    // a single top-K set per lane.
    if !queries.is_empty() {
        let meili_indexes: Vec<String> = report
            .rows
            .iter()
            .map(|r| r.partition.meili_index())
            .collect();
        let live_meili = LiveMeiliQueryProbe {
            base_url: meili_url.clone(),
            api_key: meili_key.clone(),
            indexes: meili_indexes.clone(),
        };
        // Vectorizer collection naming mirrors Meili index naming.
        let live_vec = match runtime.block_on(build_live_vec_query_probe(
            &report,
            cfg.embedder.vectorizer_url.clone(),
            if cfg.embedder.vectorizer_user.is_empty() {
                None
            } else {
                Some(cfg.embedder.vectorizer_user.clone())
            },
            cfg.embedder.vectorizer_password.clone(),
        )) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("vectorizer query probe init failed: {e}");
                return ExitCode::FAILURE;
            }
        };
        let live_nexus =
            match runtime.block_on(build_live_nexus_query_probe(cfg.nexus.nexus_url.clone())) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!("nexus query probe init failed: {e}");
                    return ExitCode::FAILURE;
                }
            };
        let q_reports = runtime.block_on(cortex_cli::ops::run_query_probes(
            &queries,
            probe_k,
            &live_meili,
            &live_vec,
            &live_nexus,
            min_overlap_jaccard,
        ));
        let any_below = q_reports.iter().any(|r| r.below_threshold);
        report.queries = q_reports;
        if any_below {
            report.failed = true;
        }
    }
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize report: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print!("{}", cortex_cli::ops::render_coverage_markdown(&report));
    }
    if report.failed {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

async fn probe_meili(
    url: &str,
    api_key: Option<&str>,
) -> anyhow::Result<(
    std::collections::BTreeMap<cortex_cli::ops::PartitionKey, u64>,
    Vec<String>,
)> {
    use cortex_workers::fulltext::{FulltextConfig, LiveMeiliClient};
    let config = FulltextConfig {
        meili_url: url.to_string(),
        meili_api_key: api_key.map(String::from),
        ..FulltextConfig::default()
    };
    let client = LiveMeiliClient::new(&config).map_err(|e| anyhow::anyhow!("meili client: {e}"))?;
    cortex_cli::ops::doctor::meili_partition_counts(&client).await
}

async fn probe_vectorizer(
    url: &str,
    user: &str,
    password: &str,
) -> anyhow::Result<(
    std::collections::BTreeMap<cortex_cli::ops::PartitionKey, u64>,
    Vec<String>,
)> {
    use cortex_cli::ops::{LiveVectorizerCoverageProbe, VectorizerCoverageScan};
    let probe = LiveVectorizerCoverageProbe::new(url, user, password).await?;
    probe.scan().await
}

async fn probe_nexus(url: &str) -> anyhow::Result<cortex_cli::ops::NexusCounts> {
    use cortex_cli::ops::{LiveNexusCoverageProbe, NexusCoverageScan};
    use cortex_workers::graph::GraphConfig;
    // GraphConfig::from_env reads the rest of the auth / transport
    // knobs (CORTEX_NEXUS_USER / _PASSWORD, transport selection, …)
    // so we let the operator set them through the same env vars the
    // streaming worker already honours.
    let mut config = GraphConfig::from_env();
    config.nexus_url = url.to_string();
    let probe = LiveNexusCoverageProbe::new(config)?;
    probe.scan().await
}

// ----- Phase4i live query probes ------------------------------------

/// Live Meili query probe — POSTs `/indexes/{uid}/search` to every
/// canonical index discovered by the coverage probe and dedupes the
/// hit `path` fields into a single top-K list. Empty when no index
/// returned anything (transport failure, missing auth) — by
/// contract the per-lane probe never propagates errors so a single
/// bad lane doesn't poison the whole probe run.
struct LiveMeiliQueryProbe {
    base_url: String,
    api_key: Option<String>,
    indexes: Vec<String>,
}

#[async_trait::async_trait]
impl cortex_cli::ops::QueryProbe for LiveMeiliQueryProbe {
    async fn search(&self, query: &str, k: usize) -> Vec<String> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .ok();
        let client = match client {
            Some(c) => c,
            None => return Vec::new(),
        };
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        for uid in &self.indexes {
            let url = format!(
                "{}/indexes/{}/search",
                self.base_url.trim_end_matches('/'),
                uid
            );
            let mut req = client
                .post(&url)
                .json(&serde_json::json!({ "q": query, "limit": k }));
            if let Some(key) = &self.api_key {
                req = req.bearer_auth(key);
            }
            let body: serde_json::Value = match req.send().await {
                Ok(r) => match r.json().await {
                    Ok(v) => v,
                    Err(_) => continue,
                },
                Err(_) => continue,
            };
            let hits = match body.get("hits").and_then(|v| v.as_array()) {
                Some(arr) => arr,
                None => continue,
            };
            for hit in hits {
                if let Some(path) = hit.get("path").and_then(|v| v.as_str()) {
                    seen.insert(path.to_string());
                } else if let Some(id) = hit.get("id").and_then(|v| v.as_str()) {
                    // Fall back to id when no `path` field — the
                    // Meili schema stamps `id` as the canonical
                    // dedup key, so it works as a stand-in for the
                    // overlap check.
                    seen.insert(id.to_string());
                }
            }
        }
        let mut out: Vec<String> = seen.into_iter().collect();
        out.truncate(k);
        out
    }
}

/// Live Vectorizer query probe — calls `search_vectors(...)` against
/// every canonical collection discovered by the coverage probe.
/// Result paths come from the per-hit `metadata.path` slot.
struct LiveVectorizerQueryProbe {
    client: vectorizer_sdk::VectorizerClient,
    collections: Vec<String>,
}

#[async_trait::async_trait]
impl cortex_cli::ops::QueryProbe for LiveVectorizerQueryProbe {
    async fn search(&self, query: &str, k: usize) -> Vec<String> {
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        for col in &self.collections {
            let resp = match self.client.search_vectors(col, query, Some(k), None).await {
                Ok(r) => r,
                Err(_) => continue,
            };
            for hit in resp.results {
                let path = hit
                    .metadata
                    .as_ref()
                    .and_then(|m| m.get("path"))
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
                    .unwrap_or(hit.id);
                seen.insert(path);
            }
        }
        let mut out: Vec<String> = seen.into_iter().collect();
        out.truncate(k);
        out
    }
}

async fn build_live_vec_query_probe(
    report: &cortex_cli::ops::DoctorReport,
    base_url: Option<String>,
    user: Option<String>,
    password: Option<String>,
) -> anyhow::Result<LiveVectorizerQueryProbe> {
    let url = base_url.ok_or_else(|| {
        anyhow::anyhow!("CORTEX_EMBEDDER_VECTORIZER_URL is required for --query probes")
    })?;
    let user = user.ok_or_else(|| {
        anyhow::anyhow!("CORTEX_EMBEDDER_VECTORIZER_USER is required for --query probes")
    })?;
    let password = password.ok_or_else(|| {
        anyhow::anyhow!("CORTEX_EMBEDDER_VECTORIZER_PASSWORD is required for --query probes")
    })?;
    let pre_auth = vectorizer_sdk::ClientConfig {
        base_url: Some(url.clone()),
        api_key: None,
        timeout_secs: Some(30),
        ..vectorizer_sdk::ClientConfig::default()
    };
    let auth_client = vectorizer_sdk::VectorizerClient::new(pre_auth)
        .map_err(|e| anyhow::anyhow!("vectorizer client: {e}"))?;
    let token = auth_client
        .login(&user, &password)
        .await
        .map_err(|e| anyhow::anyhow!("vectorizer login: {e}"))?;
    let bearer = vectorizer_sdk::ClientConfig {
        base_url: Some(url),
        api_key: Some(token.access_token),
        timeout_secs: Some(30),
        ..vectorizer_sdk::ClientConfig::default()
    };
    let client = vectorizer_sdk::VectorizerClient::new(bearer)
        .map_err(|e| anyhow::anyhow!("vectorizer authenticated client: {e}"))?;
    // Use the same canonical naming as the coverage probe rows —
    // every populated `(repo, family)` row maps to one collection.
    let collections: Vec<String> = report
        .rows
        .iter()
        .filter(|r| r.vec_vectors.unwrap_or(0) > 0)
        .map(|r| r.partition.meili_index())
        .collect();
    Ok(LiveVectorizerQueryProbe {
        client,
        collections,
    })
}

/// Live Nexus query probe — substring match on `Artifact.body`.
/// Returns `a.path` projections, deduped + truncated to `k`.
struct LiveNexusQueryProbe {
    client: cortex_workers::graph::LiveNexusClient,
}

#[async_trait::async_trait]
impl cortex_cli::ops::QueryProbe for LiveNexusQueryProbe {
    async fn search(&self, query: &str, k: usize) -> Vec<String> {
        // Bind the query as a literal Cypher string — the Cypher
        // CONTAINS operator does substring match. Escape the few
        // characters that can break the literal; everything else
        // passes through verbatim.
        let safe = query.replace('\\', "\\\\").replace('"', "\\\"");
        let cypher = format!(
            "MATCH (a:Artifact) \
             WHERE toLower(a.body) CONTAINS toLower(\"{safe}\") \
             RETURN a.path AS path LIMIT {k}",
        );
        let result = match self.client.execute_with_retry(&cypher, None).await {
            Ok(r) => r,
            Err(_) => return Vec::new(),
        };
        let mut seen: std::collections::BTreeSet<String> = Default::default();
        for row in &result.rows {
            if let Some(arr) = row.as_array() {
                if let Some(p) = arr.first().and_then(|v| v.as_str()) {
                    seen.insert(p.to_string());
                }
            }
        }
        let mut out: Vec<String> = seen.into_iter().collect();
        out.truncate(k);
        out
    }
}

/// Phase12d §3 — Meili index settings drift checker. Compares the
/// declared `searchableAttributes` / `filterableAttributes` /
/// `sortableAttributes` from `cortex_storage::fulltext::INDEXES`
/// against the live Meili settings for every index.
///
/// Exit codes:
/// - `0` — all indexes match the declared settings.
/// - `2` — any drift OR any HTTP failure (network, auth, missing
///   index, unparseable response). `--json` always emits the
///   structured report regardless of exit code.
pub(super) fn doctor_meili_indexes(
    meili_url: Option<String>,
    master_key: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_storage::fulltext::INDEXES;

    let url = meili_url
        .or_else(|| std::env::var("MEILI_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:17004".to_string());
    let key = master_key
        .or_else(|| std::env::var("MEILI_MASTER_KEY").ok())
        .filter(|s| !s.is_empty());

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("doctor-meili-indexes: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };

    let report = rt.block_on(async {
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                return MeiliDriftReport::transport_error(&url, format!("client build: {e}"));
            }
        };
        let mut entries: Vec<MeiliIndexCheck> = Vec::with_capacity(INDEXES.len());
        for idx in INDEXES {
            let declared: serde_json::Value = match serde_json::from_str(idx.settings_json) {
                Ok(v) => v,
                Err(e) => {
                    entries.push(MeiliIndexCheck::error(
                        idx.name,
                        format!("declared settings parse: {e}"),
                    ));
                    continue;
                }
            };
            let endpoint = format!(
                "{}/indexes/{}/settings",
                url.trim_end_matches('/'),
                idx.name
            );
            let mut req = client.get(&endpoint);
            if let Some(k) = key.as_deref() {
                req = req.bearer_auth(k);
            }
            let resp = match req.send().await {
                Ok(r) => r,
                Err(e) => {
                    entries.push(MeiliIndexCheck::error(idx.name, format!("network: {e}")));
                    continue;
                }
            };
            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                let reason = if status.as_u16() == 404 {
                    "missing".to_string()
                } else {
                    format!(
                        "HTTP {} {}",
                        status.as_u16(),
                        body.chars().take(120).collect::<String>()
                    )
                };
                entries.push(MeiliIndexCheck::error(idx.name, reason));
                continue;
            }
            let live: serde_json::Value = match resp.json().await {
                Ok(v) => v,
                Err(e) => {
                    entries.push(MeiliIndexCheck::error(idx.name, format!("parse: {e}")));
                    continue;
                }
            };
            entries.push(MeiliIndexCheck::diff(idx.name, &declared, &live));
        }
        MeiliDriftReport {
            meili_url: url.clone(),
            entries,
            transport_error: None,
        }
    });

    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("doctor-meili-indexes: serialize: {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        report.render_text();
    }
    if report.has_drift() {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

#[derive(Debug, serde::Serialize)]
struct MeiliDriftReport {
    meili_url: String,
    entries: Vec<MeiliIndexCheck>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transport_error: Option<String>,
}

impl MeiliDriftReport {
    fn transport_error(meili_url: &str, reason: String) -> Self {
        Self {
            meili_url: meili_url.to_string(),
            entries: Vec::new(),
            transport_error: Some(reason),
        }
    }

    fn has_drift(&self) -> bool {
        if self.transport_error.is_some() {
            return true;
        }
        self.entries.iter().any(|e| e.status != "ok")
    }

    fn render_text(&self) {
        println!("cortex-ops doctor-meili-indexes");
        println!("meili_url: {}", self.meili_url);
        if let Some(err) = &self.transport_error {
            println!("transport_error: {err}");
            return;
        }
        println!();
        for entry in &self.entries {
            println!("{:<6} {:<24} {}", entry.status, entry.index, entry.detail);
        }
    }
}

#[derive(Debug, serde::Serialize)]
struct MeiliIndexCheck {
    index: &'static str,
    /// `ok` (no drift) | `drift` (live diverges from declared) |
    /// `missing` (404 from Meili) | `error` (network, parse, etc.).
    status: &'static str,
    detail: String,
    /// Per-attribute diff, populated only when `status = "drift"`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    diffs: Vec<MeiliAttrDiff>,
}

#[derive(Debug, serde::Serialize)]
struct MeiliAttrDiff {
    attribute: &'static str,
    declared: Vec<String>,
    live: Vec<String>,
}

impl MeiliIndexCheck {
    fn error(index: &'static str, reason: String) -> Self {
        let status = if reason == "missing" {
            "missing"
        } else {
            "error"
        };
        Self {
            index,
            status,
            detail: reason,
            diffs: Vec::new(),
        }
    }

    fn diff(index: &'static str, declared: &serde_json::Value, live: &serde_json::Value) -> Self {
        let mut diffs = Vec::new();
        for attr in [
            "searchableAttributes",
            "filterableAttributes",
            "sortableAttributes",
        ] {
            let dec = string_array(declared.get(attr));
            let liv = string_array(live.get(attr));
            // Compare as sorted sets — Meili does not preserve the
            // declared order on read, and the order does not affect
            // ranking for these three keys.
            let mut dec_sorted = dec.clone();
            dec_sorted.sort();
            let mut liv_sorted = liv.clone();
            liv_sorted.sort();
            if dec_sorted != liv_sorted {
                diffs.push(MeiliAttrDiff {
                    attribute: attr,
                    declared: dec,
                    live: liv,
                });
            }
        }
        if diffs.is_empty() {
            Self {
                index,
                status: "ok",
                detail: "settings match".to_string(),
                diffs,
            }
        } else {
            let names: Vec<&str> = diffs.iter().map(|d| d.attribute).collect();
            Self {
                index,
                status: "drift",
                detail: format!("drift on: {}", names.join(", ")),
                diffs,
            }
        }
    }
}

async fn build_live_nexus_query_probe(
    nexus_url: Option<String>,
) -> anyhow::Result<LiveNexusQueryProbe> {
    let url = nexus_url
        .ok_or_else(|| anyhow::anyhow!("CORTEX_NEXUS_URL is required for --query probes"))?;
    let config = cortex_workers::graph::GraphConfig {
        nexus_url: url,
        ..cortex_workers::graph::GraphConfig::from_env()
    };
    let client = cortex_workers::graph::LiveNexusClient::new(config)
        .map_err(|e| anyhow::anyhow!("nexus client: {e}"))?;
    Ok(LiveNexusQueryProbe { client })
}

fn string_array(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// `cortex-ops doctor-decisions` — scan `cortex_decisions` for malformed
/// orphan docs whose `title == id` (the signature of the `01KQNYF4J*`
/// early buggy emit batch).
///
/// Exit codes:
/// - `0` — no malformed docs found.
/// - `2` — at least one doc has `title == id`, or the index is unreachable.
///
/// Use `cortex-ops decisions-reindex` to fix the malformed docs.
pub(super) fn doctor_decisions(
    meili_url: Option<String>,
    master_key: Option<String>,
    json: bool,
) -> ExitCode {
    let cfg = cortex_config::Config::load().unwrap_or_default();
    let url = meili_url
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.meili.meili_url.clone())
        .unwrap_or_else(|| "http://127.0.0.1:7700".to_string());
    let key = master_key
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.meili.meili_api_key.clone());

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("doctor-decisions: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };

    let result = rt.block_on(scan_decisions_index(&url, key.as_deref()));

    match result {
        Ok(malformed) => {
            let any_malformed = !malformed.is_empty();
            if json {
                let payload = serde_json::json!({
                    "meili_url": url,
                    "index": INDEX_DECISIONS,
                    "malformed_count": malformed.len(),
                    "malformed": malformed,
                    "status": if any_malformed { "fail" } else { "ok" },
                    "fix": "cortex-ops decisions-reindex --dry-run",
                });
                match serde_json::to_string_pretty(&payload) {
                    Ok(s) => println!("{s}"),
                    Err(e) => eprintln!("doctor-decisions: serialize: {e}"),
                }
            } else {
                println!("cortex-ops doctor-decisions");
                println!("index: {INDEX_DECISIONS} @ {url}");
                println!();
                if malformed.is_empty() {
                    println!("ok     no malformed docs (title == id)");
                } else {
                    for id in &malformed {
                        println!("fail   malformed doc title == id: {id}");
                    }
                    println!();
                    println!(
                        "FAIL   {} malformed doc(s) found — run \
                         `cortex-ops decisions-reindex` to repair",
                        malformed.len()
                    );
                }
            }
            if any_malformed {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            if json {
                let payload = serde_json::json!({
                    "meili_url": url,
                    "index": INDEX_DECISIONS,
                    "status": "error",
                    "error": e.to_string(),
                });
                match serde_json::to_string_pretty(&payload) {
                    Ok(s) => println!("{s}"),
                    Err(se) => eprintln!("doctor-decisions: serialize error report: {se}"),
                }
            } else {
                eprintln!("doctor-decisions: {e}");
            }
            ExitCode::from(2)
        }
    }
}

/// Fetch up to 1 000 docs from `cortex_decisions` and return the ids of any
/// whose `title == id`.  A 404 (index missing) is treated as zero docs rather
/// than an error — the index may not exist yet on a fresh stack.
async fn scan_decisions_index(
    meili_url: &str,
    api_key: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(key) = api_key {
        let bearer = format!("Bearer {key}");
        let val = HeaderValue::from_str(&bearer)
            .map_err(|e| anyhow::anyhow!("invalid api key: {e}"))?;
        headers.insert(AUTHORIZATION, val);
    }
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| anyhow::anyhow!("reqwest: {e}"))?;

    let endpoint = format!(
        "{}/indexes/{}/search",
        meili_url.trim_end_matches('/'),
        INDEX_DECISIONS,
    );
    let body = serde_json::json!({
        "q": "",
        "limit": 1000,
        "attributesToRetrieve": ["id", "title"],
    });
    let resp = client.post(&endpoint).json(&body).send().await?;
    let status = resp.status();
    // 404 means the index doesn't exist yet — not an error.
    if status.as_u16() == 404 {
        return Ok(Vec::new());
    }
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "HTTP {}: {}",
            status.as_u16(),
            text.chars().take(200).collect::<String>()
        ));
    }
    let payload: serde_json::Value = resp.json().await?;
    let hits = payload
        .get("hits")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);

    let malformed: Vec<String> = hits
        .iter()
        .filter_map(|hit| {
            let id = hit.get("id")?.as_str()?;
            let title = hit.get("title")?.as_str()?;
            if title == id {
                Some(id.to_string())
            } else {
                None
            }
        })
        .collect();

    Ok(malformed)
}

/// `cortex-ops doctor-content-addressable --index <idx>` — scan any
/// content-addressable index for docs NOT keyed by the Meili-safe
/// `bootstrap-` scheme (legacy random-ULID residue) and the malformed
/// `title == id` subset.
///
/// Exit codes: `0` clean, `2` at least one legacy doc (or unreachable).
/// Repair: `cortex-ops decisions-reindex` (file-backed kinds) or
/// `cortex-ops meili-rekey` (in-place migration for the rest).
pub(super) fn doctor_content_addressable(
    index: String,
    meili_url: Option<String>,
    master_key: Option<String>,
    json: bool,
) -> ExitCode {
    if index.trim().is_empty() {
        eprintln!("doctor-content-addressable: --index is required");
        return ExitCode::from(2);
    }
    let cfg = cortex_config::Config::load().unwrap_or_default();
    let url = meili_url
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.meili.meili_url.clone())
        .unwrap_or_else(|| "http://127.0.0.1:7700".to_string());
    let key = master_key
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.meili.meili_api_key.clone());

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("doctor-content-addressable: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };

    match rt.block_on(scan_content_addressable(&url, key.as_deref(), &index)) {
        Ok((total, fixable, title_id, no_triple)) => {
            let fail = fixable > 0;
            if json {
                let payload = serde_json::json!({
                    "meili_url": url,
                    "index": index,
                    "total": total,
                    "fixable_legacy_count": fixable,
                    "title_eq_id_count": title_id,
                    "no_triple_count": no_triple,
                    "status": if fail { "fail" } else { "ok" },
                    "fix": "cortex-ops decisions-reindex (file-backed) or cortex-ops meili-rekey (in-place)",
                });
                match serde_json::to_string_pretty(&payload) {
                    Ok(s) => println!("{s}"),
                    Err(e) => eprintln!("doctor-content-addressable: serialize: {e}"),
                }
            } else {
                println!("cortex-ops doctor-content-addressable");
                println!("index: {index} @ {url}");
                println!();
                println!("total docs:              {total}");
                println!("fixable legacy:          {fixable} (non-bootstrap- WITH repo+path+content_hash)");
                println!("  of which title==id:    {title_id}");
                println!("no-triple (live-keyed):  {no_triple} (no path — legitimately ULID-keyed)");
                println!();
                if fail {
                    println!(
                        "FAIL   {fixable} fixable legacy doc(s) — run `cortex-ops meili-rekey \
                         --index {index}` (or decisions-reindex for file-backed kinds)"
                    );
                } else {
                    println!("ok     no fixable legacy residue (all content-addressable docs are bootstrap--keyed)");
                }
            }
            if fail {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            if json {
                let payload = serde_json::json!({
                    "meili_url": url, "index": index, "status": "error", "error": e.to_string(),
                });
                let _ = serde_json::to_string_pretty(&payload).map(|s| println!("{s}"));
            } else {
                eprintln!("doctor-content-addressable: {e}");
            }
            ExitCode::from(2)
        }
    }
}

/// Scan `index` and return `(total, fixable_legacy, title_eq_id, no_triple)`.
///
/// - `fixable_legacy` = id is NOT `bootstrap-`-keyed AND the doc carries
///   the full `(repo, path, content_hash)` identity triple — genuine
///   residue that `meili-rekey`/`decisions-reindex` can repair (drives
///   the non-zero exit).
/// - `no_triple` = id is NOT `bootstrap-`-keyed but the doc lacks `path`
///   (e.g. live `cortex_capture_memory` entries) — these are legitimately
///   ULID-keyed forever (the builder only uses the `bootstrap-` key when
///   a path is present), so they are reported but do NOT fail the check.
///
/// A 404 (missing index) is treated as zero docs rather than an error.
async fn scan_content_addressable(
    meili_url: &str,
    api_key: Option<&str>,
    index: &str,
) -> anyhow::Result<(usize, usize, usize, usize)> {
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};

    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(key) = api_key {
        let bearer = format!("Bearer {key}");
        let val = HeaderValue::from_str(&bearer)
            .map_err(|e| anyhow::anyhow!("invalid api key: {e}"))?;
        headers.insert(AUTHORIZATION, val);
    }
    let client = reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| anyhow::anyhow!("reqwest: {e}"))?;

    let endpoint = format!("{}/indexes/{}/search", meili_url.trim_end_matches('/'), index);
    let mut total = 0usize;
    let mut fixable = 0usize;
    let mut title_id = 0usize;
    let mut no_triple = 0usize;
    let page = 1000usize;
    let mut offset = 0usize;
    loop {
        let body = serde_json::json!({
            "q": "", "limit": page, "offset": offset,
            "attributesToRetrieve": ["id", "title", "repo", "path", "content_hash"],
        });
        let resp = client.post(&endpoint).json(&body).send().await?;
        let status = resp.status();
        if status.as_u16() == 404 {
            return Ok((0, 0, 0, 0));
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(anyhow::anyhow!(
                "HTTP {}: {}",
                status.as_u16(),
                text.chars().take(200).collect::<String>()
            ));
        }
        let payload: serde_json::Value = resp.json().await?;
        let hits = payload
            .get("hits")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let got = hits.len();
        for hit in &hits {
            total += 1;
            let id = hit.get("id").and_then(|v| v.as_str()).unwrap_or_default();
            if id.starts_with("bootstrap-") {
                continue;
            }
            let has_triple = hit.get("repo").and_then(|v| v.as_str()).is_some()
                && hit.get("path").and_then(|v| v.as_str()).is_some()
                && hit.get("content_hash").and_then(|v| v.as_str()).is_some();
            if has_triple {
                fixable += 1;
                if hit.get("title").and_then(|v| v.as_str()) == Some(id) {
                    title_id += 1;
                }
            } else {
                no_triple += 1;
            }
        }
        if got < page {
            break;
        }
        offset += page;
    }
    Ok((total, fixable, title_id, no_triple))
}

#[cfg(test)]
mod meili_drift_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diff_reports_ok_when_attributes_match_in_any_order() {
        let declared = json!({
            "searchableAttributes": ["title", "body"],
            "filterableAttributes": ["repo", "tags"],
            "sortableAttributes": ["occurred_at"]
        });
        let live = json!({
            // Order swapped — Meili-side is allowed to permute these.
            "searchableAttributes": ["body", "title"],
            "filterableAttributes": ["tags", "repo"],
            "sortableAttributes": ["occurred_at"]
        });
        let check = MeiliIndexCheck::diff("cortex_test", &declared, &live);
        assert_eq!(check.status, "ok");
        assert!(check.diffs.is_empty());
    }

    #[test]
    fn diff_flags_drift_on_missing_searchable_attribute() {
        let declared = json!({
            "searchableAttributes": ["title", "body", "topics"],
            "filterableAttributes": ["repo"],
            "sortableAttributes": ["occurred_at"]
        });
        let live = json!({
            "searchableAttributes": ["title", "body"],
            "filterableAttributes": ["repo"],
            "sortableAttributes": ["occurred_at"]
        });
        let check = MeiliIndexCheck::diff("cortex_test", &declared, &live);
        assert_eq!(check.status, "drift");
        assert_eq!(check.diffs.len(), 1);
        assert_eq!(check.diffs[0].attribute, "searchableAttributes");
        assert!(check.diffs[0].declared.contains(&"topics".to_string()));
        assert!(!check.diffs[0].live.contains(&"topics".to_string()));
    }

    #[test]
    fn diff_flags_drift_across_multiple_attributes() {
        let declared = json!({
            "searchableAttributes": ["title"],
            "filterableAttributes": ["repo", "topics"],
            "sortableAttributes": ["occurred_at"]
        });
        let live = json!({
            "searchableAttributes": ["title"],
            "filterableAttributes": ["repo"],
            "sortableAttributes": []
        });
        let check = MeiliIndexCheck::diff("cortex_test", &declared, &live);
        assert_eq!(check.status, "drift");
        assert_eq!(check.diffs.len(), 2);
    }

    #[test]
    fn missing_attribute_in_declared_but_present_live_is_drift() {
        let declared = json!({
            "searchableAttributes": ["title"]
            // No filterableAttributes / sortableAttributes declared.
        });
        let live = json!({
            "searchableAttributes": ["title"],
            "filterableAttributes": ["repo"],
            "sortableAttributes": []
        });
        let check = MeiliIndexCheck::diff("cortex_test", &declared, &live);
        // Declared has no filterableAttributes → empty set; live has
        // ["repo"] → drift.
        assert_eq!(check.status, "drift");
        assert!(check
            .diffs
            .iter()
            .any(|d| d.attribute == "filterableAttributes"));
    }

    #[test]
    fn error_constructor_classifies_missing_vs_error() {
        let m = MeiliIndexCheck::error("cortex_test", "missing".to_string());
        assert_eq!(m.status, "missing");
        let e = MeiliIndexCheck::error("cortex_test", "HTTP 502 ...".to_string());
        assert_eq!(e.status, "error");
    }

    #[test]
    fn report_has_drift_returns_true_when_any_entry_is_not_ok() {
        let report = MeiliDriftReport {
            meili_url: "http://localhost:7700".to_string(),
            entries: vec![
                MeiliIndexCheck {
                    index: "cortex_a",
                    status: "ok",
                    detail: "match".into(),
                    diffs: Vec::new(),
                },
                MeiliIndexCheck {
                    index: "cortex_b",
                    status: "drift",
                    detail: "drift".into(),
                    diffs: Vec::new(),
                },
            ],
            transport_error: None,
        };
        assert!(report.has_drift());
    }

    #[test]
    fn report_has_drift_returns_false_when_all_ok() {
        let report = MeiliDriftReport {
            meili_url: "http://localhost:7700".to_string(),
            entries: vec![
                MeiliIndexCheck {
                    index: "cortex_a",
                    status: "ok",
                    detail: "match".into(),
                    diffs: Vec::new(),
                },
                MeiliIndexCheck {
                    index: "cortex_b",
                    status: "ok",
                    detail: "match".into(),
                    diffs: Vec::new(),
                },
            ],
            transport_error: None,
        };
        assert!(!report.has_drift());
    }
}
