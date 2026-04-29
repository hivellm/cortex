//! `cortex-ops` — local-stack operator CLI.
//!
//! Subcommands:
//! - `plan` — prints the expected Vectorizer collections, Nexus Cypher,
//!   Meilisearch indexes, Synap streams/KV namespaces in JSON. Useful as
//!   the source the seed scripts consume.
//! - `doctor` — pokes every backend and reports liveness.
//!
//! This CLI never mutates external state directly in v1 — that job belongs
//! to `cortex-api` (spec 04) and the workers (specs 05–08). `cortex-ops
//! plan` is what `bin/cortex-init.sh` feeds into each backend's native
//! create API.

#![forbid(unsafe_code)]

use clap::{Parser, Subcommand};
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "cortex-ops", version, about = "Cortex operator CLI")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Emit the bootstrap plan (collections / Cypher / indexes / streams) as JSON.
    Plan {
        /// Pretty-print the JSON (default: compact).
        #[arg(long)]
        pretty: bool,
        /// Which slice to emit. Default: all.
        #[arg(long, value_enum, default_value_t = PlanSlice::All)]
        slice: PlanSlice,
    },
    /// Poke every backend at the configured URL and report status.
    Doctor {
        /// Vectorizer base URL (defaults to `$VECTORIZER_URL`).
        #[arg(long)]
        vectorizer: Option<String>,
        /// Nexus base URL (defaults to `$NEXUS_URL`).
        #[arg(long)]
        nexus: Option<String>,
        /// Synap base URL (defaults to `$SYNAP_URL`).
        #[arg(long)]
        synap: Option<String>,
        /// Meilisearch base URL (defaults to `$MEILI_URL`).
        #[arg(long)]
        meili: Option<String>,
    },
    /// Phase9b — archive rollup compactor. Merges hourly Parquet
    /// files older than 90 d into one daily file per `day=`, daily
    /// files older than 365 d into one monthly file per `month=`,
    /// and drops monthly files older than 3 y unless `kind ∈
    /// {decision, analysis, law_violation}` or
    /// `redactions[].pii_risk = "low"`. Atomic + crash-safe.
    Rollup {
        /// Override "now" so the 90 / 365 / 1095-day boundaries
        /// are deterministic.
        #[arg(long, value_name = "RFC3339")]
        time_travel: Option<String>,
        /// Print the plan + per-partition counts without mutating
        /// any file.
        #[arg(long)]
        dry_run: bool,
        /// Limit the run to one granularity. `all` runs the full
        /// pipeline (hourly→daily, daily→monthly, three-year drop)
        /// in order.
        #[arg(long, value_enum, default_value_t = RollupGranularityArg::All)]
        granularity: RollupGranularityArg,
        /// Override `CORTEX_ARCHIVE_ROOT` for one-shot CI runs.
        #[arg(long)]
        archive_root: Option<String>,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase9a — Vectorizer tier-transition sweep. Re-encodes
    /// records FP32 → PQ at 30 d and PQ → Binary at 365 d per spec
    /// 02 §quantization. Idempotent + concurrency-safe via the
    /// `retention_sweeps` SQLite table; emits one bookkeeping row
    /// per invocation.
    ///
    /// `--dry-run` runs the plan against an empty in-memory store
    /// so the operator can see the canonical plan layout without
    /// touching the live Vectorizer.
    RetentionSweep {
        /// Override "now" so the 30 d / 365 d boundaries are
        /// deterministic for tests + scheduled CI runs.
        #[arg(long, value_name = "RFC3339")]
        time_travel: Option<String>,
        /// Print the plan + transitions without mutating the
        /// destination collections.
        #[arg(long)]
        dry_run: bool,
        /// Maximum records per Vectorizer batch list call.
        #[arg(long, default_value_t = 256)]
        batch_size: u32,
        /// SQLite metadata DB path. Defaults to
        /// `$CORTEX_METADATA_DB` then `<home>/.cortex/metadata.sqlite`.
        #[arg(long)]
        metadata_db: Option<String>,
        /// Emit JSON instead of the plain-text summary.
        #[arg(long)]
        json: bool,
    },
    /// Phase8f — fire a synthetic end-to-end canary frame through
    /// the live IPC pipe and assert it lands in the archive within
    /// the deadline. Detects regressions in the adapter → ingestion
    /// → archive path that latent unit tests miss.
    Canary {
        /// Hook flavour to fire (default `PostToolUse`).
        #[arg(long, default_value = "PostToolUse")]
        hook: String,
        /// Override the IPC binding (named pipe / unix socket path).
        #[arg(long)]
        ipc: Option<String>,
        /// Override `cortex-api` URL (defaults to `CORTEX_API_URL` /
        /// `http://127.0.0.1:17000`).
        #[arg(long)]
        api_url: Option<String>,
        /// Deadline before the canary is declared a `Timeout`.
        /// Default 10 s.
        #[arg(long, default_value_t = 10)]
        deadline_secs: u64,
        /// Emit JSON instead of the plain-text outcome line.
        #[arg(long)]
        json: bool,
    },
    /// Phase8e — list active silent-drop alerts. Walks
    /// `~/.cortex/alerts/*.json` produced by the cortex-api
    /// silent-drop watcher and renders one row per pair. Exit `0`
    /// when no Critical alerts are active, `2` when any are.
    DoctorAlerts {
        /// Override the alerts directory (defaults to
        /// `~/.cortex/alerts`).
        #[arg(long)]
        state_dir: Option<String>,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Phase8d — config-coherence audit. Read-only static analysis
    /// of every config surface (`.env`, `~/.cortex/adapter.toml`,
    /// `cortex-plugin/.mcp.json`, `cortex-plugin/hooks/hooks.json`)
    /// + cross-checks (e.g. adapter.endpoint must match
    /// CORTEX_INGESTION_URL). Exit codes: `0` all ok, `1` any warn,
    /// `2` any critical.
    DoctorConfig {
        /// Workspace root (defaults to current dir). The audit
        /// expects `.env` and `cortex-plugin/` under this path.
        #[arg(long)]
        workspace: Option<String>,
        /// Override `~/.cortex/adapter.toml` location (CI / fixtures).
        #[arg(long)]
        adapter_toml: Option<String>,
        /// Emit JSON instead of the plain-text table.
        #[arg(long)]
        json: bool,
    },
    /// Cross-backend consistency checker. v1 (phase4d) covered the
    /// archive ↔ Meili axis; phase4h widens it to Vectorizer +
    /// Nexus. Each probe is opt-in via its own flag / env var so
    /// the doctor still runs against partial backends.
    DoctorConsistency {
        /// Archive root (defaults to `$CORTEX_ARCHIVE_ROOT` then
        /// `~/.cortex/archive`).
        #[arg(long)]
        archive_root: Option<String>,
        /// Meilisearch base URL (defaults to
        /// `$CORTEX_FULLTEXT_MEILI_URL`).
        #[arg(long)]
        meili: Option<String>,
        /// Meilisearch master key (defaults to
        /// `$CORTEX_FULLTEXT_MEILI_API_KEY`).
        #[arg(long)]
        meili_key: Option<String>,
        /// Vectorizer base URL (defaults to
        /// `$CORTEX_EMBEDDER_VECTORIZER_URL`). Probe runs only
        /// when both URL and credentials are present.
        #[arg(long)]
        vectorizer: Option<String>,
        /// Vectorizer admin username (defaults to
        /// `$CORTEX_EMBEDDER_VECTORIZER_USER`).
        #[arg(long)]
        vectorizer_user: Option<String>,
        /// Vectorizer admin password (defaults to
        /// `$CORTEX_EMBEDDER_VECTORIZER_PASSWORD`).
        #[arg(long)]
        vectorizer_password: Option<String>,
        /// Nexus base URL (defaults to `$CORTEX_NEXUS_URL`). Probe
        /// runs only when this is set; the rest of the
        /// `cortex-graph` env vars (auth, transport) are picked up
        /// via `GraphConfig::from_env`.
        #[arg(long)]
        nexus: Option<String>,
        /// Repeatable query-overlap probe (phase4i). Each value
        /// runs against the three lanes; the report renders one
        /// per-query block with pairwise Jaccards. The run fails
        /// when any pair falls below `--min-overlap-jaccard`.
        #[arg(long = "query")]
        queries: Vec<String>,
        /// Top-K cap for query-overlap probes (default 10).
        #[arg(long, default_value_t = 10)]
        probe_k: usize,
        /// Pairwise Jaccard threshold below which the probe fails
        /// (default 0.2).
        #[arg(long, default_value_t = 0.2)]
        min_overlap_jaccard: f64,
        /// Emit JSON instead of the markdown table.
        #[arg(long)]
        json: bool,
    },
}

/// Phase9b — granularity selector for `cortex-ops rollup`.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
#[allow(non_camel_case_types)]
enum RollupGranularityArg {
    /// Run all three granularities in order.
    All,
    HourlyToDaily,
    DailyToMonthly,
    ThreeYearDrop,
}

#[derive(Debug, Clone, Copy, clap::ValueEnum)]
enum PlanSlice {
    All,
    Collections,
    Cypher,
    Indexes,
    Streams,
    Kv,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Plan { pretty, slice } => match emit_plan(pretty, slice) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::FAILURE
            }
        },
        Command::Doctor {
            vectorizer,
            nexus,
            synap,
            meili,
        } => doctor(vectorizer, nexus, synap, meili),
        Command::DoctorConfig {
            workspace,
            adapter_toml,
            json,
        } => doctor_config(workspace, adapter_toml, json),
        Command::DoctorAlerts { state_dir, json } => doctor_alerts(state_dir, json),
        Command::Rollup {
            time_travel,
            dry_run,
            granularity,
            archive_root,
            json,
        } => rollup(time_travel, dry_run, granularity, archive_root, json),
        Command::RetentionSweep {
            time_travel,
            dry_run,
            batch_size,
            metadata_db,
            json,
        } => retention_sweep(time_travel, dry_run, batch_size, metadata_db, json),
        Command::Canary {
            hook,
            ipc,
            api_url,
            deadline_secs,
            json,
        } => canary(hook, ipc, api_url, deadline_secs, json),
        Command::DoctorConsistency {
            archive_root,
            meili,
            meili_key,
            vectorizer,
            vectorizer_user,
            vectorizer_password,
            nexus,
            queries,
            probe_k,
            min_overlap_jaccard,
            json,
        } => doctor_consistency(
            archive_root,
            meili,
            meili_key,
            vectorizer,
            vectorizer_user,
            vectorizer_password,
            nexus,
            queries,
            probe_k,
            min_overlap_jaccard,
            json,
        ),
    }
}

/// Wire the phase4d doctor: scan the archive, probe Meili, render
/// the report. Read-only end-to-end. Spins up a one-shot Tokio
/// runtime so the surrounding `main` stays sync (the rest of
/// `cortex-ops` does not need an async runtime).
#[allow(clippy::too_many_arguments)]
fn doctor_consistency(
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
    let archive_root = archive_root
        .or_else(|| std::env::var("CORTEX_ARCHIVE_ROOT").ok())
        .unwrap_or_else(|| {
            home_dir()
                .map(|h| h.join(".cortex/archive").display().to_string())
                .unwrap_or_else(|| ".cortex/archive".to_string())
        });
    let meili_url = match meili.or_else(|| std::env::var("CORTEX_FULLTEXT_MEILI_URL").ok()) {
        Some(u) if !u.is_empty() => u,
        _ => {
            eprintln!("doctor consistency: --meili (or $CORTEX_FULLTEXT_MEILI_URL) is required");
            return ExitCode::FAILURE;
        }
    };
    let meili_key = meili_key.or_else(|| std::env::var("CORTEX_FULLTEXT_MEILI_API_KEY").ok());
    let vectorizer_url = vectorizer
        .or_else(|| std::env::var("CORTEX_EMBEDDER_VECTORIZER_URL").ok())
        .filter(|u| !u.is_empty());
    let vectorizer_user = vectorizer_user
        .or_else(|| std::env::var("CORTEX_EMBEDDER_VECTORIZER_USER").ok())
        .filter(|u| !u.is_empty());
    let vectorizer_password = vectorizer_password
        .or_else(|| std::env::var("CORTEX_EMBEDDER_VECTORIZER_PASSWORD").ok())
        .filter(|u| !u.is_empty());
    let nexus_url = nexus
        .or_else(|| std::env::var("CORTEX_NEXUS_URL").ok())
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
            std::env::var("CORTEX_EMBEDDER_VECTORIZER_URL").ok(),
            std::env::var("CORTEX_EMBEDDER_VECTORIZER_USER").ok(),
            std::env::var("CORTEX_EMBEDDER_VECTORIZER_PASSWORD").ok(),
        )) {
            Ok(p) => p,
            Err(e) => {
                eprintln!("vectorizer query probe init failed: {e}");
                return ExitCode::FAILURE;
            }
        };
        let live_nexus = match runtime.block_on(build_live_nexus_query_probe(
            std::env::var("CORTEX_NEXUS_URL").ok(),
        )) {
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
    let client = LiveMeiliClient::new(&config)
        .map_err(|e| anyhow::anyhow!("meili client: {e}"))?;
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
    use cortex_workers::graph::GraphConfig;
    use cortex_cli::ops::{LiveNexusCoverageProbe, NexusCoverageScan};
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
            let resp = match self
                .client
                .search_vectors(col, query, Some(k), None)
                .await
            {
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

async fn build_live_nexus_query_probe(
    nexus_url: Option<String>,
) -> anyhow::Result<LiveNexusQueryProbe> {
    let url = nexus_url
        .ok_or_else(|| anyhow::anyhow!("CORTEX_NEXUS_URL is required for --query probes"))?;
    let mut config = cortex_workers::graph::GraphConfig::from_env();
    config.nexus_url = url;
    let client = cortex_workers::graph::LiveNexusClient::new(config)
        .map_err(|e| anyhow::anyhow!("nexus client: {e}"))?;
    Ok(LiveNexusQueryProbe { client })
}

fn home_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(std::path::PathBuf::from)
}

fn emit_plan(pretty: bool, slice: PlanSlice) -> anyhow::Result<()> {
    use cortex_storage::{
        collections::COLLECTIONS,
        fulltext::INDEXES,
        graph::BOOTSTRAP_STATEMENTS,
        streams::{KV_NAMESPACES, STREAMS},
    };

    let mut out = serde_json::Map::new();
    if matches!(slice, PlanSlice::All | PlanSlice::Collections) {
        out.insert("collections".into(), serde_json::to_value(COLLECTIONS)?);
    }
    if matches!(slice, PlanSlice::All | PlanSlice::Cypher) {
        out.insert(
            "cypher".into(),
            serde_json::to_value(BOOTSTRAP_STATEMENTS)?,
        );
    }
    if matches!(slice, PlanSlice::All | PlanSlice::Indexes) {
        let rows: Vec<_> = INDEXES
            .iter()
            .map(|idx| {
                serde_json::json!({
                    "name": idx.name,
                    "primary_key": idx.primary_key,
                    "settings": serde_json::from_str::<serde_json::Value>(idx.settings_json).unwrap_or_default()
                })
            })
            .collect();
        out.insert("indexes".into(), serde_json::Value::Array(rows));
    }
    if matches!(slice, PlanSlice::All | PlanSlice::Streams) {
        out.insert("streams".into(), serde_json::to_value(STREAMS)?);
    }
    if matches!(slice, PlanSlice::All | PlanSlice::Kv) {
        out.insert("kv_namespaces".into(), serde_json::to_value(KV_NAMESPACES)?);
    }
    let value = serde_json::Value::Object(out);
    let rendered = if pretty {
        serde_json::to_string_pretty(&value)?
    } else {
        serde_json::to_string(&value)?
    };
    println!("{rendered}");
    Ok(())
}

/// Phase8d — `cortex-ops doctor-config`. Runs the cortex-api config
/// audit (read-only, static analysis) and renders either a plain-text
/// table or JSON. Exit codes match `Severity`: 0=ok, 1=warn, 2=critical.
fn doctor_config(
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
fn doctor_alerts(state_dir: Option<String>, json: bool) -> ExitCode {
    use cortex_api::silent_drop::{AlertState, PairState};

    let dir: std::path::PathBuf = match state_dir {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home).join(".cortex").join("alerts")
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
            println!("(no persisted alert state — silent-drop watcher idle or no alerts since boot)");
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

/// Phase9b — `cortex-ops rollup`. Compacts the archive's hourly /
/// daily / monthly Parquet partitions per spec 19. Quarantines
/// `*.corrupted*` and orphan `*.tmp` files on entry so the working
/// tree is clean before compaction starts.
fn rollup(
    time_travel: Option<String>,
    dry_run: bool,
    granularity: RollupGranularityArg,
    archive_root: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_retention::parquet_rollup::{
        apply_three_year_drop, compact_partition, enumerate_compactable, quarantine_pre_existing,
        Granularity, RollupCounts,
    };

    let now = match time_travel {
        Some(s) => match chrono::DateTime::parse_from_rfc3339(&s) {
            Ok(t) => t.with_timezone(&chrono::Utc),
            Err(e) => {
                eprintln!("--time-travel parse error: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => chrono::Utc::now(),
    };

    let archive = archive_root
        .or_else(|| std::env::var("CORTEX_ARCHIVE_ROOT").ok())
        .unwrap_or_else(|| {
            home_dir()
                .map(|h| h.join(".cortex/archive").display().to_string())
                .unwrap_or_else(|| ".cortex/archive".to_string())
        });
    let archive_path = std::path::PathBuf::from(&archive);

    let granularities: Vec<Granularity> = match granularity {
        RollupGranularityArg::All => vec![
            Granularity::HourlyToDaily,
            Granularity::DailyToMonthly,
            Granularity::ThreeYearDrop,
        ],
        RollupGranularityArg::HourlyToDaily => vec![Granularity::HourlyToDaily],
        RollupGranularityArg::DailyToMonthly => vec![Granularity::DailyToMonthly],
        RollupGranularityArg::ThreeYearDrop => vec![Granularity::ThreeYearDrop],
    };

    let mut totals = RollupCounts::default();
    // Pre-flight: quarantine corrupted + orphan tmp files.
    let qcounts = if dry_run {
        RollupCounts::default()
    } else {
        quarantine_pre_existing(&archive_path)
    };
    totals.merge(&qcounts);

    let mut per_granularity: Vec<(Granularity, RollupCounts, usize)> = Vec::new();
    let mut had_error = false;
    for g in granularities {
        let plans = enumerate_compactable(&archive_path, now, g);
        let plan_count = plans.len();
        let mut sub = RollupCounts::default();
        if !dry_run {
            for plan in &plans {
                let result = match g {
                    Granularity::ThreeYearDrop => apply_three_year_drop(&archive_path, plan),
                    _ => compact_partition(&archive_path, plan),
                };
                match result {
                    Ok(c) => sub.merge(&c),
                    Err(e) => {
                        had_error = true;
                        tracing::warn!(error = %e, "rollup: partition compaction failed");
                    }
                }
            }
        }
        per_granularity.push((g, sub.clone(), plan_count));
        totals.merge(&sub);
    }

    if json {
        let payload = serde_json::json!({
            "now": now.to_rfc3339(),
            "archive_root": archive,
            "dry_run": dry_run,
            "totals": totals,
            "per_granularity": per_granularity
                .iter()
                .map(|(g, c, n)| serde_json::json!({
                    "granularity": g.as_str(),
                    "plans": *n,
                    "counts": c,
                }))
                .collect::<Vec<_>>(),
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops rollup");
        println!("now:           {}", now.to_rfc3339());
        println!("archive_root:  {archive}");
        println!("dry_run:       {dry_run}");
        println!(
            "totals:        files_in={} files_out={} reclaimed={}B quarantined={} preserved={} dropped={}",
            totals.files_in,
            totals.files_out,
            totals.bytes_reclaimed,
            totals.quarantined,
            totals.records_preserved,
            totals.records_dropped,
        );
        for (g, c, plans) in &per_granularity {
            println!(
                "  {:<18} plans={plans} files_in={} files_out={} reclaimed={}B preserved={} dropped={}",
                g.as_str(),
                c.files_in,
                c.files_out,
                c.bytes_reclaimed,
                c.records_preserved,
                c.records_dropped,
            );
        }
    }

    if had_error {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// Phase9a — `cortex-ops retention-sweep`. Runs one tier-transition
/// pass and exits. Idempotent + concurrency-safe via the
/// `retention_sweeps` table.
///
/// Production Vectorizer integration is wired through the
/// `VectorizerOps` trait; this CLI surface uses the in-memory ops
/// (`MemoryVectorizerOps`) so the dry-run path works without a
/// running Vectorizer server. Live ops integration ships in a
/// follow-up that adds the SDK adapter — keeping the trait surface
/// stable now means that switch is one line.
fn retention_sweep(
    time_travel: Option<String>,
    dry_run: bool,
    batch_size: u32,
    metadata_db: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_retention::{run_sweep, MemoryVectorizerOps, SweepError, SweepPlan};
    use cortex_storage::MetadataStore;

    let now = match time_travel {
        Some(s) => match chrono::DateTime::parse_from_rfc3339(&s) {
            Ok(t) => t.with_timezone(&chrono::Utc),
            Err(e) => {
                eprintln!("--time-travel parse error: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => chrono::Utc::now(),
    };

    let mut plan = SweepPlan::default_for(now);
    plan.batch_size = batch_size;
    plan.dry_run = dry_run;

    let metadata_path = metadata_db
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var("CORTEX_METADATA_DB").ok().map(std::path::PathBuf::from))
        .unwrap_or_else(|| {
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .unwrap_or_else(|_| ".".to_string());
            std::path::PathBuf::from(home).join(".cortex").join("metadata.sqlite")
        });

    let store = match MetadataStore::open(&metadata_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("metadata open ({}): {e}", metadata_path.display());
            return ExitCode::FAILURE;
        }
    };

    let sweep_id = cortex_retention::new_sweep_id();
    if let Err(e) = store.start_retention_sweep(&sweep_id, now, 3600) {
        eprintln!("retention-sweep: {e}");
        // Code 2 — another sweep in flight (per spec).
        return ExitCode::from(2);
    }

    // The MemoryVectorizerOps holds an empty store on a fresh CI
    // run, so the dry-run path emits a `0 demoted / 0 dropped` row
    // plus the canonical plan summary. Live Vectorizer integration
    // swaps `ops` for the SDK adapter.
    let ops = MemoryVectorizerOps::new();

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

    let outcome = runtime.block_on(run_sweep(&plan, &ops));
    let finished_at = chrono::Utc::now();
    let (status, exit) = match &outcome {
        Ok(_) => ("success", ExitCode::SUCCESS),
        Err(SweepError::ErrorRateExceeded { .. }) => ("failed", ExitCode::FAILURE),
        Err(SweepError::Vectorizer(_)) => ("failed", ExitCode::FAILURE),
    };

    let report = outcome.unwrap_or_default();
    if let Err(e) = store.finish_retention_sweep(
        &sweep_id,
        finished_at,
        report.records_demoted,
        report.records_dropped,
        &report.tier_transitions_json(),
        status,
    ) {
        eprintln!("retention-sweep: bookkeeping write failed: {e}");
        return ExitCode::FAILURE;
    }

    if json {
        let payload = serde_json::json!({
            "sweep_id": sweep_id,
            "started_at": now.to_rfc3339(),
            "finished_at": finished_at.to_rfc3339(),
            "status": status,
            "dry_run": dry_run,
            "records_demoted": report.records_demoted,
            "records_dropped": report.records_dropped,
            "tier_transitions": report.tier_transitions,
            "transitions": report.transitions,
        });
        match serde_json::to_string_pretty(&payload) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops retention-sweep");
        println!("sweep_id:   {sweep_id}");
        println!("now:        {}", now.to_rfc3339());
        println!("dry_run:    {dry_run}");
        println!("status:     {status}");
        println!(
            "demoted:    {}    dropped: {}",
            report.records_demoted, report.records_dropped
        );
        if report.tier_transitions.is_empty() {
            println!("transitions: (none — every collection within thresholds)");
        } else {
            println!("transitions:");
            for (key, count) in &report.tier_transitions {
                println!("  {key}: {count}");
            }
        }
    }
    exit
}

/// Phase8f — `cortex-ops canary`. Sends a synthetic frame through
/// the daemon's IPC and polls the archive for the marker. Exit
/// codes match `CanaryOutcome::exit_code()`.
fn canary(
    hook: String,
    ipc: Option<String>,
    api_url: Option<String>,
    deadline_secs: u64,
    json: bool,
) -> ExitCode {
    use cortex_api::canary::{run_canary_once, CanaryConfig};
    let mut cfg = CanaryConfig::default();
    cfg.deadline_secs = deadline_secs;
    cfg.ipc_path = ipc;
    if let Some(url) = api_url {
        cfg.api_url = url;
    }
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
    let outcome = runtime.block_on(run_canary_once(&cfg, &hook));
    if json {
        match serde_json::to_string_pretty(&outcome) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize outcome: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("cortex-ops canary --hook={hook}");
        println!("{}", outcome.describe());
    }
    match outcome.exit_code() {
        0 => ExitCode::SUCCESS,
        2 => ExitCode::from(2),
        _ => ExitCode::FAILURE,
    }
}

fn doctor(
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
    if any_failure {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}
