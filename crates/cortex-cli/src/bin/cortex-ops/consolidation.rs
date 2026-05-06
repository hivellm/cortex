use std::path::PathBuf;
use std::process::ExitCode;

/// Run the consolidation pruner once. Lists every doc in
/// `cortex_consolidations` (paginated by `page_size`), bucketises
/// each into a tier, and runs the
/// [`cortex_workers::pruner::engine::run_sweep`] cascade against
/// the live Vectorizer + Meili.
pub(super) fn consolidation_prune(
    time_travel: Option<String>,
    dry_run: bool,
    vectorizer_url: Option<String>,
    meili_url: Option<String>,
    meili_key: Option<String>,
    page_size: u32,
    json: bool,
) -> ExitCode {
    // phase11v §6 — bookkeeping anchor.
    let started_at = chrono::Utc::now();

    let now = match time_travel {
        Some(s) => match chrono::DateTime::parse_from_rfc3339(&s) {
            Ok(t) => t.with_timezone(&chrono::Utc),
            Err(e) => {
                eprintln!("consolidation-prune: invalid --time-travel: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => chrono::Utc::now(),
    };

    // phase11v §2.1 — env resolution moved into pure helpers so the
    // precedence is unit-testable without process env mutation.
    let vec_url = consolidation_prune_vectorizer_url(
        vectorizer_url.clone(),
        ConsolidationPruneEnv::from_env(),
    );
    let vec_user = consolidation_prune_vectorizer_user(ConsolidationPruneEnv::from_env());
    let vec_password = consolidation_prune_vectorizer_password(ConsolidationPruneEnv::from_env());

    let meili_url_v = meili_url
        .or_else(|| std::env::var("CORTEX_FULLTEXT_MEILI_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:7700".to_string());
    let meili_key_v = consolidation_prune_meili_key(
        meili_key.clone(),
        ConsolidationPruneEnv::from_env(),
    );

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("consolidation-prune: tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    let outcome = rt.block_on(async move {
        // Build the Vectorizer client. JWT minted via `login` so the
        // SDK 3.3 calls (`move_vectors`, `delete_vectors`) ride on
        // the real bearer token, not the dev fallback password.
        let token = match cortex_workers::embedder::vectorizer_client::LiveVectorizerClient::login(
            &vec_url,
            &vec_user,
            &vec_password,
        )
        .await
        {
            Ok(t) => t.access_token,
            Err(e) => return Err(format!("vectorizer login: {e}")),
        };
        let mut embed_cfg = cortex_workers::embedder::EmbedderConfig::default();
        embed_cfg.vectorizer_url = vec_url.clone();
        embed_cfg.vectorizer_password = Some(token);
        let live_vec =
            match cortex_workers::embedder::vectorizer_client::LiveVectorizerClient::new(
                embed_cfg,
            ) {
                Ok(c) => c,
                Err(e) => return Err(format!("vectorizer client: {e}")),
            };

        // Build the Meili client through the existing fulltext
        // worker config — the `MeiliPruneOps` impl on
        // `LiveMeiliClient` (phase11o §2.3) is what the engine
        // needs.
        let mut meili_cfg = cortex_workers::fulltext::FulltextConfig::default();
        meili_cfg.meili_url = meili_url_v.clone();
        meili_cfg.meili_api_key = meili_key_v.clone();
        let live_meili =
            match cortex_workers::fulltext::meili_client::LiveMeiliClient::new(&meili_cfg) {
                Ok(c) => c,
                Err(e) => return Err(format!("meili client: {e}")),
            };

        // Pull every consolidation doc. Meili's `GET /indexes/{uid}/documents?limit=N&offset=K`
        // returns `{results: [...], offset, limit, total}`. We paginate
        // until we've seen everything.
        let docs = match fetch_all_consolidations(&meili_url_v, meili_key_v.as_deref(), page_size)
            .await
        {
            Ok(d) => d,
            Err(e) => return Err(format!("meili list: {e}")),
        };

        if dry_run {
            // Bucket without touching either backend.
            let mut counts: std::collections::BTreeMap<String, u64> =
                std::collections::BTreeMap::new();
            for d in &docs {
                let action = cortex_workers::pruner::plan_demotion(
                    d.event_id.clone(),
                    d.occurred_at,
                    now,
                    d.vector_ids.clone(),
                );
                let key = match action.as_ref() {
                    None => "hot".to_string(),
                    Some(a) => cortex_workers::pruner::tier_pair_key(a.from, a.to),
                };
                *counts.entry(key).or_insert(0) += 1;
            }
            return Ok(SweepOutcome {
                consolidations_seen: docs.len() as u64,
                events_demoted_per_tier: counts,
                events_purged: 0,
                last_run_duration_ms: 0,
                last_error: None,
                events_failed: 0,
                dry_run: true,
            });
        }

        let report = match cortex_workers::pruner::engine::run_sweep(
            &docs,
            now,
            &live_vec,
            &live_meili,
            cortex_storage::names::INDEX_CONSOLIDATIONS,
        )
        .await
        {
            Ok(r) => r,
            Err(e) => return Err(format!("sweep: {e}")),
        };
        Ok(SweepOutcome {
            consolidations_seen: report.consolidations_seen,
            events_demoted_per_tier: report.events_demoted_per_tier,
            events_purged: report.events_purged,
            last_run_duration_ms: report.last_run_duration_ms,
            last_error: report.last_error,
            events_failed: report.events_failed,
            dry_run: false,
        })
    });

    match outcome {
        Ok(report) => {
            // Phase11o §3.1 — persist the run summary so
            // `/v1/health/coverage` can surface it on the next probe.
            // Skipped on dry-run (the file would mask the live run).
            if !report.dry_run {
                if let Err(e) = persist_pruner_status(&report) {
                    eprintln!("consolidation-prune: persist status: {e}");
                }
            }
            if json {
                let body = serde_json::json!({
                    "consolidations_seen": report.consolidations_seen,
                    "events_demoted_per_tier": report.events_demoted_per_tier,
                    "events_purged": report.events_purged,
                    "last_run_duration_ms": report.last_run_duration_ms,
                    "events_failed": report.events_failed,
                    "dry_run": report.dry_run,
                });
                println!("{}", serde_json::to_string_pretty(&body).unwrap());
            } else {
                println!(
                    "consolidation-prune: seen={} purged={} failed={} duration_ms={} dry_run={}",
                    report.consolidations_seen,
                    report.events_purged,
                    report.events_failed,
                    report.last_run_duration_ms,
                    report.dry_run,
                );
                for (k, v) in &report.events_demoted_per_tier {
                    println!("  {k:<24} {v}");
                }
            }
            let mut extras = serde_json::Map::new();
            extras.insert(
                "consolidations_seen".into(),
                report.consolidations_seen.into(),
            );
            extras.insert(
                "events_demoted_per_tier".into(),
                serde_json::to_value(&report.events_demoted_per_tier)
                    .unwrap_or(serde_json::Value::Null),
            );
            extras.insert("events_failed".into(), report.events_failed.into());
            extras.insert(
                "last_run_duration_ms".into(),
                report.last_run_duration_ms.into(),
            );
            extras.insert("dry_run".into(), report.dry_run.into());
            super::record_sweep_run(
                "consolidation_prune",
                started_at,
                "success",
                cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
                    records_demoted: report
                        .events_demoted_per_tier
                        .values()
                        .sum::<u64>(),
                    records_dropped: report.events_purged,
                    extras,
                    ..Default::default()
                },
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("consolidation-prune: {e}");
            super::record_sweep_run(
                "consolidation_prune",
                started_at,
                "failed",
                cortex_cli::ops::sweep_bookkeeping::SweepStageStats {
                    last_error: Some(e.chars().take(256).collect()),
                    ..Default::default()
                },
            );
            ExitCode::FAILURE
        }
    }
}

/// Phase11o §3.1 — write the pruner-status JSON to the path
/// `cortex-api`'s coverage handler reads on every probe.
fn persist_pruner_status(report: &SweepOutcome) -> std::io::Result<()> {
    use std::io::Write;
    let path = match cortex_api::coverage::pruner_status_path() {
        Some(p) => p,
        None => {
            // No HOME / USERPROFILE — skip persistence rather than
            // failing the whole run.
            return Ok(());
        }
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let last_run_ts = chrono::Utc::now().timestamp_millis();
    let body = cortex_api::coverage::PrunerStatus {
        last_run_ts,
        events_demoted_per_tier: report.events_demoted_per_tier.clone(),
        events_purged: report.events_purged,
        last_run_duration_ms: report.last_run_duration_ms,
        last_error: report.last_error.clone(),
    };
    let raw = serde_json::to_vec_pretty(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let tmp = path.with_extension("json.tmp");
    {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(&raw)?;
        f.sync_all().ok();
    }
    std::fs::rename(&tmp, &path)?;
    Ok(())
}

#[derive(Debug)]
pub(super) struct SweepOutcome {
    pub(super) consolidations_seen: u64,
    pub(super) events_demoted_per_tier: std::collections::BTreeMap<String, u64>,
    pub(super) events_purged: u64,
    pub(super) last_run_duration_ms: u64,
    #[allow(dead_code)]
    pub(super) last_error: Option<String>,
    pub(super) events_failed: u64,
    pub(super) dry_run: bool,
}

/// Walk Meili's `cortex_consolidations` index, paginated, building
/// a [`cortex_workers::pruner::engine::ConsolidationDoc`] per row.
/// Every doc's `event_id` is also threaded through as the matching
/// Vectorizer primary key (the consolidator writes the vector under
/// the same id, see `crates/cortex-workers/src/consolidator/`).
pub(super) async fn fetch_all_consolidations(
    meili_url: &str,
    meili_key: Option<&str>,
    page_size: u32,
) -> anyhow::Result<Vec<cortex_workers::pruner::engine::ConsolidationDoc>> {
    use anyhow::Context;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("reqwest client")?;
    let mut out: Vec<cortex_workers::pruner::engine::ConsolidationDoc> = Vec::new();
    let mut offset: u32 = 0;
    let limit = page_size.max(1);
    let base = meili_url.trim_end_matches('/');
    loop {
        let url = format!(
            "{base}/indexes/{}/documents?limit={limit}&offset={offset}",
            cortex_storage::names::INDEX_CONSOLIDATIONS
        );
        let mut req = client.get(&url);
        if let Some(k) = meili_key {
            req = req.bearer_auth(k);
        }
        let resp = req.send().await.with_context(|| format!("GET {url}"))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            // Index does not exist yet — nothing to prune.
            return Ok(Vec::new());
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("GET {url}: {status} — {body}");
        }
        #[derive(serde::Deserialize)]
        struct Page {
            results: Vec<serde_json::Value>,
            #[allow(dead_code)]
            offset: u32,
            #[allow(dead_code)]
            limit: u32,
            total: u32,
        }
        let page: Page = resp.json().await.context("decode meili page")?;
        let count = page.results.len() as u32;
        for v in page.results {
            // The doc shape matches the consolidator's writer: at
            // minimum a `event_id` (primary key) + an
            // `occurred_at` RFC3339 timestamp. `source_event_ids`
            // is an array of raw-event ids; we resolve them to the
            // canonical Vectorizer primary key by treating each
            // entry as the dedup key (the embedder writes
            // `metadata.dedup_key = event_id` so the round-trip
            // stays stable). Missing fields cause the row to be
            // skipped with a stderr note rather than aborting the
            // whole run.
            let event_id = match v.get("event_id").and_then(|x| x.as_str()) {
                Some(s) => s.to_string(),
                None => {
                    eprintln!("consolidation-prune: skip doc without event_id");
                    continue;
                }
            };
            let occurred_at_str = match v
                .get("occurred_at")
                .and_then(|x| x.as_str())
                .or_else(|| v.get("ts").and_then(|x| x.as_str()))
            {
                Some(s) => s,
                None => {
                    eprintln!(
                        "consolidation-prune: skip {event_id} without occurred_at/ts"
                    );
                    continue;
                }
            };
            let occurred_at = match chrono::DateTime::parse_from_rfc3339(occurred_at_str) {
                Ok(t) => t.with_timezone(&chrono::Utc),
                Err(_) => {
                    eprintln!(
                        "consolidation-prune: skip {event_id} — bad occurred_at {occurred_at_str:?}"
                    );
                    continue;
                }
            };
            let mut vector_ids: Vec<String> = Vec::new();
            // Prefer an explicit `vector_ids` list when present;
            // fall back to `source_event_ids` (the canonical writer
            // shape) so we still operate on older docs.
            if let Some(arr) = v.get("vector_ids").and_then(|x| x.as_array()) {
                for entry in arr {
                    if let Some(s) = entry.as_str() {
                        vector_ids.push(s.to_string());
                    }
                }
            } else if let Some(arr) = v.get("source_event_ids").and_then(|x| x.as_array()) {
                for entry in arr {
                    if let Some(s) = entry.as_str() {
                        vector_ids.push(s.to_string());
                    }
                }
            } else {
                // No source list — operate on the consolidation's
                // own vector (consolidator writes one fp32 vector
                // per consolidation, primary key = event_id).
                vector_ids.push(event_id.clone());
            }
            out.push(cortex_workers::pruner::engine::ConsolidationDoc {
                event_id,
                occurred_at,
                vector_ids,
            });
        }
        offset += count;
        if offset >= page.total || count == 0 {
            break;
        }
    }
    Ok(out)
}

/// Snapshot of every env var the consolidation-prune resolver
/// consults. `from_env()` reads the live process env once; tests
/// build the struct directly to drive precedence cases.
#[derive(Default, Clone)]
pub(super) struct ConsolidationPruneEnv {
    pub(super) vectorizer_url: Option<String>,
    pub(super) embedder_vectorizer_url: Option<String>,
    pub(super) vectorizer_user: Option<String>,
    pub(super) embedder_vectorizer_user: Option<String>,
    pub(super) vectorizer_password: Option<String>,
    pub(super) embedder_vectorizer_password: Option<String>,
    pub(super) fulltext_meili_api_key: Option<String>,
    pub(super) fulltext_meili_key: Option<String>,
    pub(super) meili_master_key: Option<String>,
}

impl ConsolidationPruneEnv {
    pub(super) fn from_env() -> Self {
        Self {
            vectorizer_url: std::env::var("CORTEX_VECTORIZER_URL").ok(),
            embedder_vectorizer_url: std::env::var("CORTEX_EMBEDDER_VECTORIZER_URL").ok(),
            vectorizer_user: std::env::var("CORTEX_VECTORIZER_USER").ok(),
            embedder_vectorizer_user: std::env::var("CORTEX_EMBEDDER_VECTORIZER_USER").ok(),
            vectorizer_password: std::env::var("CORTEX_VECTORIZER_PASSWORD").ok(),
            embedder_vectorizer_password: std::env::var("CORTEX_EMBEDDER_VECTORIZER_PASSWORD").ok(),
            fulltext_meili_api_key: std::env::var("CORTEX_FULLTEXT_MEILI_API_KEY").ok(),
            fulltext_meili_key: std::env::var("CORTEX_FULLTEXT_MEILI_KEY").ok(),
            meili_master_key: std::env::var("MEILI_MASTER_KEY").ok(),
        }
    }
}

/// CLI flag → unprefixed env → embedder-prefixed env → loopback.
pub(super) fn consolidation_prune_vectorizer_url(
    cli: Option<String>,
    env: ConsolidationPruneEnv,
) -> String {
    cli.or(env.vectorizer_url)
        .or(env.embedder_vectorizer_url)
        .unwrap_or_else(|| "http://127.0.0.1:17001".to_string())
}

/// Unprefixed env → embedder-prefixed env → `admin`.
pub(super) fn consolidation_prune_vectorizer_user(env: ConsolidationPruneEnv) -> String {
    env.vectorizer_user
        .or(env.embedder_vectorizer_user)
        .unwrap_or_else(|| "admin".to_string())
}

/// Unprefixed env → embedder-prefixed env → `cortex-dev-admin`.
pub(super) fn consolidation_prune_vectorizer_password(env: ConsolidationPruneEnv) -> String {
    env.vectorizer_password
        .or(env.embedder_vectorizer_password)
        .unwrap_or_else(|| "cortex-dev-admin".to_string())
}

/// CLI flag → `_API_KEY` → `_KEY` → upstream master key → `None`.
/// Returning `Option` because `meilisearch` accepts requests
/// without a key when it was started in `--no-master-key` mode;
/// callers must not synthesise a default that authenticates against
/// a key-protected instance with the wrong header.
pub(super) fn consolidation_prune_meili_key(
    cli: Option<String>,
    env: ConsolidationPruneEnv,
) -> Option<String> {
    cli.or(env.fulltext_meili_api_key)
        .or(env.fulltext_meili_key)
        .or(env.meili_master_key)
}

// ===========================================================================
// Phase 12a §4 — consolidations-replay
// ===========================================================================

/// Resolve the JSONL fallback path the consolidator wrote to.
/// Mirrors the precedence in `crates/cortex-workers/src/bin/cortex-consolidator.rs::fallback_path`.
pub(super) fn consolidations_replay_path(cli_from: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(p) = cli_from {
        return Some(p);
    }
    if let Ok(p) = std::env::var("CORTEX_CONSOLIDATIONS_FALLBACK_FILE") {
        if !p.trim().is_empty() {
            return Some(PathBuf::from(p));
        }
    }
    if let Ok(p) = std::env::var("CORTEX_HOME") {
        if !p.trim().is_empty() {
            return Some(PathBuf::from(p).join("consolidations.jsonl"));
        }
    }
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    Some(
        PathBuf::from(home)
            .join(".cortex")
            .join("consolidations.jsonl"),
    )
}

/// CLI flag → `CORTEX_INGESTION_URL` → loopback default. Returns the
/// URL trimmed of trailing slash so callers can append `/v1/events`.
pub(super) fn consolidations_replay_ingest_url(cli: Option<String>) -> String {
    let raw = cli
        .or_else(|| std::env::var("CORTEX_INGESTION_URL").ok())
        .unwrap_or_else(|| "http://127.0.0.1:17010".to_string());
    raw.trim().trim_end_matches('/').to_string()
}

/// One outcome row per JSONL line.
#[derive(Debug, Default, serde::Serialize)]
struct ReplayOutcome {
    total_lines: usize,
    sent: usize,
    skipped_dry_run: usize,
    parse_failed: usize,
    network_failed: usize,
    non_2xx: usize,
    accepted_event_ids: Vec<String>,
}

/// Run the replay. The function is callable by tests with a custom
/// path / URL; the CLI handler below is a thin wrapper that resolves
/// args from env + flags, builds an HTTP client, and prints the
/// outcome.
pub(super) fn consolidations_replay(
    from: Option<PathBuf>,
    ingest_url: Option<String>,
    dry_run: bool,
    limit: Option<usize>,
    json: bool,
) -> ExitCode {
    let path = match consolidations_replay_path(from) {
        Some(p) => p,
        None => {
            eprintln!(
                "consolidations-replay: no fallback path resolvable (set CORTEX_CONSOLIDATIONS_FALLBACK_FILE, CORTEX_HOME, or HOME/USERPROFILE)"
            );
            return ExitCode::FAILURE;
        }
    };

    let body = match std::fs::read_to_string(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // No fallback file means nothing to replay — that is the
            // healthy steady state, not a failure.
            let outcome = ReplayOutcome::default();
            print_replay(&outcome, &path, json);
            return ExitCode::SUCCESS;
        }
        Err(e) => {
            eprintln!("consolidations-replay: read {}: {e}", path.display());
            return ExitCode::FAILURE;
        }
    };

    let base = consolidations_replay_ingest_url(ingest_url);
    let url = format!("{base}/v1/events");

    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("consolidations-replay: tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("consolidations-replay: client build: {e}");
            return ExitCode::FAILURE;
        }
    };

    let outcome = rt.block_on(async {
        let mut outcome = ReplayOutcome::default();
        for line in body.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            outcome.total_lines += 1;
            if let Some(cap) = limit {
                if outcome.sent + outcome.skipped_dry_run >= cap {
                    break;
                }
            }
            let parsed: serde_json::Value = match serde_json::from_str(trimmed) {
                Ok(v) => v,
                Err(_) => {
                    outcome.parse_failed += 1;
                    continue;
                }
            };
            let envelope = match parsed.get("envelope") {
                Some(e) => e,
                None => {
                    outcome.parse_failed += 1;
                    continue;
                }
            };
            let event_id = envelope
                .get("event_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if dry_run {
                outcome.skipped_dry_run += 1;
                if !event_id.is_empty() {
                    outcome.accepted_event_ids.push(event_id);
                }
                continue;
            }

            match client
                .post(&url)
                .header("content-type", "application/json")
                .json(envelope)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    outcome.sent += 1;
                    if !event_id.is_empty() {
                        outcome.accepted_event_ids.push(event_id);
                    }
                }
                Ok(_) => {
                    outcome.non_2xx += 1;
                }
                Err(_) => {
                    outcome.network_failed += 1;
                }
            }
        }
        outcome
    });

    print_replay(&outcome, &path, json);

    if outcome.network_failed + outcome.non_2xx + outcome.parse_failed > 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

fn print_replay(outcome: &ReplayOutcome, path: &std::path::Path, json: bool) {
    if json {
        let body = serde_json::json!({
            "fallback_file": path.display().to_string(),
            "total_lines": outcome.total_lines,
            "sent": outcome.sent,
            "skipped_dry_run": outcome.skipped_dry_run,
            "parse_failed": outcome.parse_failed,
            "network_failed": outcome.network_failed,
            "non_2xx": outcome.non_2xx,
            "accepted_event_ids": outcome.accepted_event_ids,
        });
        println!("{}", body);
    } else {
        println!("consolidations-replay file={}", path.display());
        println!("  total_lines     : {}", outcome.total_lines);
        println!("  sent            : {}", outcome.sent);
        println!("  skipped_dry_run : {}", outcome.skipped_dry_run);
        println!("  parse_failed    : {}", outcome.parse_failed);
        println!("  network_failed  : {}", outcome.network_failed);
        println!("  non_2xx         : {}", outcome.non_2xx);
    }
}

#[cfg(test)]
mod consolidation_prune_env_tests {
    use super::*;

    fn empty() -> ConsolidationPruneEnv {
        ConsolidationPruneEnv::default()
    }

    #[test]
    fn vectorizer_url_prefers_cli_over_env() {
        let env = ConsolidationPruneEnv {
            vectorizer_url: Some("http://env-unprefixed:1".into()),
            embedder_vectorizer_url: Some("http://env-embedder:2".into()),
            ..empty()
        };
        assert_eq!(
            consolidation_prune_vectorizer_url(Some("http://cli:0".into()), env),
            "http://cli:0"
        );
    }

    #[test]
    fn vectorizer_url_prefers_unprefixed_over_embedder_env() {
        let env = ConsolidationPruneEnv {
            vectorizer_url: Some("http://vectorizer:15002".into()),
            embedder_vectorizer_url: Some("http://embedder-fallback:1".into()),
            ..empty()
        };
        assert_eq!(
            consolidation_prune_vectorizer_url(None, env),
            "http://vectorizer:15002"
        );
    }

    #[test]
    fn vectorizer_url_falls_through_to_embedder_when_unprefixed_absent() {
        let env = ConsolidationPruneEnv {
            embedder_vectorizer_url: Some("http://embedder-only:1".into()),
            ..empty()
        };
        assert_eq!(
            consolidation_prune_vectorizer_url(None, env),
            "http://embedder-only:1"
        );
    }

    #[test]
    fn vectorizer_url_falls_back_to_loopback_when_all_absent() {
        assert_eq!(
            consolidation_prune_vectorizer_url(None, empty()),
            "http://127.0.0.1:17001"
        );
    }

    #[test]
    fn vectorizer_user_prefers_unprefixed_over_embedder_env() {
        let env = ConsolidationPruneEnv {
            vectorizer_user: Some("alice".into()),
            embedder_vectorizer_user: Some("bob".into()),
            ..empty()
        };
        assert_eq!(consolidation_prune_vectorizer_user(env), "alice");
    }

    #[test]
    fn vectorizer_password_falls_back_to_default() {
        assert_eq!(
            consolidation_prune_vectorizer_password(empty()),
            "cortex-dev-admin"
        );
    }

    #[test]
    fn meili_key_walks_three_env_names_in_priority() {
        // Priority: API_KEY → KEY (legacy) → MEILI_MASTER_KEY (upstream).
        let env = ConsolidationPruneEnv {
            fulltext_meili_api_key: Some("api".into()),
            fulltext_meili_key: Some("legacy".into()),
            meili_master_key: Some("master".into()),
            ..empty()
        };
        assert_eq!(
            consolidation_prune_meili_key(None, env.clone()),
            Some("api".to_string())
        );

        let env = ConsolidationPruneEnv {
            fulltext_meili_key: Some("legacy".into()),
            meili_master_key: Some("master".into()),
            ..empty()
        };
        assert_eq!(
            consolidation_prune_meili_key(None, env),
            Some("legacy".to_string())
        );

        let env = ConsolidationPruneEnv {
            meili_master_key: Some("master".into()),
            ..empty()
        };
        assert_eq!(
            consolidation_prune_meili_key(None, env),
            Some("master".to_string())
        );

        assert_eq!(consolidation_prune_meili_key(None, empty()), None);
    }

    #[test]
    fn meili_key_cli_beats_every_env() {
        let env = ConsolidationPruneEnv {
            fulltext_meili_api_key: Some("api".into()),
            ..empty()
        };
        assert_eq!(
            consolidation_prune_meili_key(Some("cli".into()), env),
            Some("cli".to_string())
        );
    }

    // -- Phase 12a §4 — consolidations-replay -------------------------

    #[test]
    fn replay_path_honours_cli_flag_first() {
        let saved = std::env::var("CORTEX_CONSOLIDATIONS_FALLBACK_FILE").ok();
        std::env::set_var(
            "CORTEX_CONSOLIDATIONS_FALLBACK_FILE",
            "D:/from-env/fallback.jsonl",
        );
        let p = consolidations_replay_path(Some(PathBuf::from("D:/explicit/x.jsonl")));
        match saved {
            Some(v) => std::env::set_var("CORTEX_CONSOLIDATIONS_FALLBACK_FILE", v),
            None => std::env::remove_var("CORTEX_CONSOLIDATIONS_FALLBACK_FILE"),
        }
        assert_eq!(p, Some(PathBuf::from("D:/explicit/x.jsonl")));
    }

    #[test]
    fn replay_path_falls_through_to_cortex_home() {
        let saved_override = std::env::var("CORTEX_CONSOLIDATIONS_FALLBACK_FILE").ok();
        let saved_home = std::env::var("CORTEX_HOME").ok();
        std::env::remove_var("CORTEX_CONSOLIDATIONS_FALLBACK_FILE");
        std::env::set_var("CORTEX_HOME", "D:/cortex-root");
        let p = consolidations_replay_path(None);
        match saved_override {
            Some(v) => std::env::set_var("CORTEX_CONSOLIDATIONS_FALLBACK_FILE", v),
            None => std::env::remove_var("CORTEX_CONSOLIDATIONS_FALLBACK_FILE"),
        }
        match saved_home {
            Some(v) => std::env::set_var("CORTEX_HOME", v),
            None => std::env::remove_var("CORTEX_HOME"),
        }
        assert_eq!(p, Some(PathBuf::from("D:/cortex-root/consolidations.jsonl")));
    }

    #[test]
    fn replay_ingest_url_strips_trailing_slash() {
        let saved = std::env::var("CORTEX_INGESTION_URL").ok();
        std::env::remove_var("CORTEX_INGESTION_URL");
        assert_eq!(
            consolidations_replay_ingest_url(Some("http://localhost:9999/".to_string())),
            "http://localhost:9999"
        );
        assert_eq!(
            consolidations_replay_ingest_url(None),
            "http://127.0.0.1:17010"
        );
        match saved {
            Some(v) => std::env::set_var("CORTEX_INGESTION_URL", v),
            None => std::env::remove_var("CORTEX_INGESTION_URL"),
        }
    }

    #[test]
    fn replay_dry_run_against_jsonl_counts_every_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("consolidations.jsonl");
        // Two well-formed lines + one parse-failure line so the
        // outcome counters surface every branch.
        std::fs::write(
            &path,
            r#"{"reason":"env_unset","envelope":{"event_id":"01ULID0000000000000001","kind":"consolidation"}}
{"reason":"non_2xx","envelope":{"event_id":"01ULID0000000000000002","kind":"consolidation"}}
not-json-and-not-empty
"#,
        )
        .unwrap();
        let exit = consolidations_replay(Some(path), None, true, None, true);
        // Parse failure ⇒ exit 2.
        assert_eq!(exit, ExitCode::from(2));
    }

    #[test]
    fn replay_returns_success_when_fallback_file_missing() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("absent.jsonl");
        let exit = consolidations_replay(Some(missing), None, true, None, true);
        assert_eq!(exit, ExitCode::SUCCESS);
    }
}
