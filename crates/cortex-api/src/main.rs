//! `cortex-api` binary — Axum daemon binding `POST /v1/query`.
//!
//! Spec 11 §HTTP endpoint: the daemon listens on
//! `127.0.0.1:17000` by default. Live lane wiring (Vectorizer,
//! Meilisearch, Nexus) drops into [`Orchestrator::new`]; the
//! current binary stands up the lane traits with
//! [`MemoryVectorLane`] / [`MemoryKeywordLane`] / [`MemoryGraphLane`]
//! defaults so the API surface is reachable end-to-end against a
//! cold dev stack and the spec-12 / spec-15 callers can integrate
//! before the live SDK wiring lands.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use clap::Parser;
use std::sync::Arc as StdArc;

use cortex_api::{
    GraphLane, KeywordLane, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane, MeiliKeywordLane,
    NexusGraphLane, Orchestrator, QueryService, VectorLane, VectorizerLane,
};
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Debug, Clone, Parser)]
#[command(name = "cortex-api", version, about)]
struct Cli {
    /// Bind address. Matches `.env` `CORTEX_API_PORT` (17000) so a
    /// supervisor booting from env settings stays in sync with the
    /// GUI's hardcoded `BASE_URL`.
    #[arg(long, default_value = "127.0.0.1:17000")]
    bind: SocketAddr,
    /// Verbose tracing output.
    #[arg(long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let vector_memory = Arc::new(MemoryVectorLane::new());
    let keyword_memory = Arc::new(MemoryKeywordLane::new());
    let graph_memory = Arc::new(MemoryGraphLane::new());

    // Live vector lane: when CORTEX_VECTORIZER_URL is set and the
    // SDK's `health_check` succeeds, swap the in-memory double for
    // a `VectorizerLane` that runs KNN against the same per-project
    // collections the spec-06 embedder-worker upserts to. Without
    // this swap, `debug.lanes.vector_ms` stays at 0 on every query
    // and the snippets surfaced under `source = "vector"` are
    // actually keyword-lane hits the orchestrator's lane_label
    // fallback mislabelled (the 2026-04-27 audit caught this).
    //
    // Fallback to the in-memory lane on probe failure (env unset,
    // server unreachable, build error) keeps cold-stack dev working;
    // failures log WARN with the URL + reason so misconfigurations
    // are spotted at boot.
    let vector: StdArc<dyn VectorLane> = if let Ok(vectorizer_url) =
        std::env::var("CORTEX_VECTORIZER_URL")
            .or_else(|_| std::env::var("VECTORIZER_URL"))
    {
        // Auth selection mirrors the embedder-worker boot flow:
        // - explicit JWT / api-key wins (`*_API_KEY` env)
        // - otherwise, if user+password are both set, run /auth/login
        //   once and use the minted JWT
        // - otherwise, no auth (dev-stack default)
        let api_key = std::env::var("CORTEX_VECTORIZER_API_KEY")
            .or_else(|_| std::env::var("VECTORIZER_API_KEY"))
            .ok();
        let username = std::env::var("CORTEX_VECTORIZER_USER")
            .or_else(|_| std::env::var("CORTEX_EMBEDDER_VECTORIZER_USER"))
            .ok();
        let password = std::env::var("CORTEX_VECTORIZER_PASSWORD")
            .or_else(|_| std::env::var("CORTEX_EMBEDDER_VECTORIZER_PASSWORD"))
            .ok();
        let lane_result = if api_key.is_some() {
            VectorizerLane::new(&vectorizer_url, api_key)
        } else if let (Some(u), Some(p)) = (username.as_deref(), password.as_deref()) {
            tracing::info!(
                vectorizer_url = %vectorizer_url,
                user = %u,
                "vector lane: running /auth/login to mint JWT"
            );
            VectorizerLane::with_login(&vectorizer_url, u, p).await
        } else {
            VectorizerLane::new(&vectorizer_url, None)
        };
        match lane_result {
            Ok(live) => match live.probe().await {
                Ok(()) => {
                    tracing::info!(
                        vectorizer_url = %vectorizer_url,
                        "live vector lane: VectorizerLane wired"
                    );
                    StdArc::new(live)
                }
                Err(reason) => {
                    tracing::warn!(
                        vectorizer_url = %vectorizer_url,
                        reason = %reason,
                        "live vector lane probe failed; falling back to MemoryVectorLane"
                    );
                    vector_memory.clone()
                }
            },
            Err(reason) => {
                tracing::warn!(
                    vectorizer_url = %vectorizer_url,
                    reason = %reason,
                    "live vector lane build failed; falling back to MemoryVectorLane"
                );
                vector_memory.clone()
            }
        }
    } else {
        tracing::info!(
            "CORTEX_VECTORIZER_URL unset; vector lane stays on MemoryVectorLane"
        );
        vector_memory.clone()
    };

    // Live keyword lane: when CORTEX_FULLTEXT_MEILI_URL is set and
    // the server answers /health within the probe timeout, we hand
    // the orchestrator a Meili-backed lane that filters by the
    // request's actual `query` string. Without this swap, every
    // /v1/query call returns the same archive snapshot regardless
    // of input — the 2026-04-27 audit confirmed three consecutive
    // pre-thinking turns yielded the same five smoke-test envelopes.
    //
    // On probe failure (env unset, server unreachable, timeout) we
    // fall back to the in-memory lane so cold-stack dev keeps
    // working without a Meili dependency. Failure logs WARN with
    // the URL + reason so the operator can spot the misconfiguration.
    let keyword: StdArc<dyn KeywordLane> = if let Ok(meili_url) =
        std::env::var("CORTEX_FULLTEXT_MEILI_URL")
    {
        let api_key = std::env::var("CORTEX_FULLTEXT_MEILI_API_KEY").ok();
        match MeiliKeywordLane::new(&meili_url, api_key) {
            Ok(live) => match live.probe().await {
                Ok(()) => {
                    tracing::info!(
                        meili_url = %meili_url,
                        "live keyword lane: MeiliKeywordLane wired"
                    );
                    StdArc::new(live)
                }
                Err(reason) => {
                    tracing::warn!(
                        meili_url = %meili_url,
                        reason = %reason,
                        "live keyword lane probe failed; falling back to MemoryKeywordLane"
                    );
                    keyword_memory.clone()
                }
            },
            Err(reason) => {
                tracing::warn!(
                    meili_url = %meili_url,
                    reason = %reason,
                    "live keyword lane build failed; falling back to MemoryKeywordLane"
                );
                keyword_memory.clone()
            }
        }
    } else {
        tracing::info!(
            "CORTEX_FULLTEXT_MEILI_URL unset; keyword lane stays on MemoryKeywordLane"
        );
        keyword_memory.clone()
    };

    // Pragmatic boot-time seed + periodic re-scan: when
    // CORTEX_ARCHIVE_ROOT is set, walk the cortex-ingestion archive
    // and pre-populate the keyword lane with every captured turn /
    // tool_call / agent_call envelope. The same scan re-runs on the
    // CORTEX_ARCHIVE_REFRESH_SECS interval (default 30 s) so a
    // freshly-captured prompt becomes queryable without a daemon
    // restart. Closes the "captured events are queryable" gap until
    // the live spec-06 / spec-07 / spec-08 indexers ship.
    if let Ok(root) = std::env::var("CORTEX_ARCHIVE_ROOT") {
        let archive_root = PathBuf::from(&root);
        let initial = cortex_api::load_into_keyword_lane(&archive_root, &keyword_memory);
        tracing::info!(
            archive_root = %root,
            files_visited = initial.files_visited,
            envelopes_parsed = initial.envelopes_parsed,
            hits_seeded = initial.hits_seeded,
            lines_dropped = initial.lines_dropped,
            "archive loader: keyword lane seeded (boot)"
        );

        let refresh_secs = std::env::var("CORTEX_ARCHIVE_REFRESH_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(30)
            .max(1);
        let lane = keyword_memory.clone();
        tokio::spawn(async move {
            let interval = Duration::from_secs(refresh_secs);
            loop {
                tokio::time::sleep(interval).await;
                let report = cortex_api::load_into_keyword_lane(&archive_root, &lane);
                tracing::debug!(
                    archive_root = %archive_root.display(),
                    files_visited = report.files_visited,
                    hits_seeded = report.hits_seeded,
                    lines_dropped = report.lines_dropped,
                    "archive loader: keyword lane refreshed"
                );
            }
        });
    }

    // Pragmatic boot-time seed + periodic re-scan for non-Turn /
    // ToolCall / AgentCall envelopes. The cortex-ingestion archive
    // does not carry decisions, law violations, memories, or
    // analyses — those flow exclusively through the bootstrap
    // pipeline and live in Meili. The loader pulls them once at
    // boot + every CORTEX_MEILI_REFRESH_SECS so the dashboard's
    // /v1/dashboard/decisions, /violations, /memory and /analyses
    // endpoints stop returning empty. Skips silently when the env
    // var is absent — cold-stack dev keeps working.
    if let Ok(meili_url) = std::env::var("CORTEX_FULLTEXT_MEILI_URL") {
        let meili_api_key = std::env::var("CORTEX_FULLTEXT_MEILI_API_KEY").ok();
        match cortex_api::load_meili_into_keyword_lane(
            &meili_url,
            meili_api_key.as_deref(),
            &keyword_memory,
        )
        .await
        {
            Ok(report) => {
                tracing::info!(
                    meili_url = %meili_url,
                    indexes_visited = report.indexes_visited,
                    decisions = report.decisions_seeded,
                    violations = report.violations_seeded,
                    memories = report.memories_seeded,
                    analyses = report.analyses_seeded,
                    turns = report.turns_seeded,
                    skipped = report.hits_skipped,
                    "meili loader: keyword lane seeded (boot)"
                );
            }
            Err(e) => {
                tracing::warn!(
                    meili_url = %meili_url,
                    error = %e,
                    "meili loader: skipped — dashboard decisions/violations stay empty until next refresh",
                );
            }
        }

        let refresh_secs = std::env::var("CORTEX_MEILI_REFRESH_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(60)
            .max(5);
        let lane = keyword_memory.clone();
        let url = meili_url.clone();
        let key = meili_api_key.clone();
        tokio::spawn(async move {
            let interval = Duration::from_secs(refresh_secs);
            loop {
                tokio::time::sleep(interval).await;
                match cortex_api::load_meili_into_keyword_lane(&url, key.as_deref(), &lane)
                    .await
                {
                    Ok(report) => tracing::debug!(
                        meili_url = %url,
                        total = report.total_seeded(),
                        "meili loader: refreshed"
                    ),
                    Err(e) => tracing::warn!(meili_url = %url, error = %e, "meili loader: refresh failed"),
                }
            }
        });
    }

    let nexus_client = build_nexus_client().await;
    // Live graph lane: when the Nexus client is reachable, share
    // the same `Arc<NexusClient>` between `DashboardState` and the
    // orchestrator's `GraphLane`. Single TCP session, two consumers.
    // The 2026-04-27 audit caught the asymmetry — dashboard graph
    // view ran live Cypher while `/v1/query`'s graph lane stayed on
    // the empty `MemoryGraphLane` test double, so `graph_neighbors`
    // returned empty across every probed query. Fallback to the
    // memory lane preserves cold-stack dev when Nexus is unreachable.
    let graph: StdArc<dyn GraphLane> = match &nexus_client {
        Some(client) => {
            tracing::info!("live graph lane: NexusGraphLane wired");
            StdArc::new(NexusGraphLane::new(client.clone()))
        }
        None => {
            tracing::info!("nexus client unavailable; graph lane stays on MemoryGraphLane");
            graph_memory.clone()
        }
    };
    let analyzer = StdArc::new(cortex_api::analyzer::Analyzer::from_env());
    // Rulebook task loader. `CORTEX_RULEBOOK_ROOT` points at the
    // `.rulebook/` directory whose `tasks/` + `archive/` subtrees the
    // dashboard surfaces under `/v1/dashboard/tasks*`. Falls back to
    // `<cwd>/.rulebook` so a `cargo run` from the repo root just works;
    // when the path is unreachable the loader yields empty results.
    let rulebook_root: PathBuf = std::env::var("CORTEX_RULEBOOK_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::current_dir()
                .map(|p| p.join(".rulebook"))
                .unwrap_or_else(|_| PathBuf::from(".rulebook"))
        });
    tracing::info!(
        rulebook_root = %rulebook_root.display(),
        "tasks loader: rooted at .rulebook/"
    );
    let tasks = StdArc::new(cortex_api::TaskLoader::new(rulebook_root));

    // SQLite metadata store powering `series.classifier_cost_usd_today`.
    // Opened best-effort — when the file is unreachable (first boot
    // before `cortex-classifier-worker` ran, permission issues, etc.)
    // the dashboard ribbon stays "—" until the worker creates the DB.
    let metadata_path = resolve_metadata_db_path();
    let metadata = match cortex_storage::MetadataStore::open(&metadata_path) {
        Ok(store) => {
            tracing::info!(
                metadata_db = %metadata_path.display(),
                "metadata store opened — classifier cost ribbon enabled"
            );
            Some(StdArc::new(std::sync::Mutex::new(store)))
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                metadata_db = %metadata_path.display(),
                "metadata store unavailable — classifier cost ribbon stays empty"
            );
            None
        }
    };

    let dashboard_state = cortex_api::DashboardState {
        lane: keyword_memory.clone(),
        nexus: nexus_client,
        analyzer,
        tasks,
        metadata,
    };

    let orchestrator = Orchestrator::new(vector, keyword.clone(), graph);
    // Wire the `keyword_memory` snapshot into the service so
    // `/v1/status.indexed_repos` and `notice.repo_not_indexed` (issue
    // hivellm/cortex#1) read from the same source the dashboard does.
    let service = Arc::new(
        QueryService::with_memory_defaults(orchestrator)
            .with_indexed_repos(keyword_memory.clone()),
    );

    tracing::info!(bind = %cli.bind, "cortex-api starting");
    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    let app = cortex_api::build_router_with(service, Some(dashboard_state));
    axum::serve(listener, app).await?;
    Ok(())
}

/// Resolve the SQLite metadata database path. Precedence:
/// 1. `CORTEX_METADATA_DB` (full path override).
/// 2. `${CORTEX_HOME}/metadata.sqlite` when `CORTEX_HOME` is set.
/// 3. `<home>/.cortex/metadata.sqlite` (cross-platform default).
///
/// Centralised so the API and the classifier-worker resolve the same
/// file — they share the database (worker writes, API reads).
fn resolve_metadata_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("CORTEX_METADATA_DB") {
        return PathBuf::from(p);
    }
    if let Ok(home) = std::env::var("CORTEX_HOME") {
        return PathBuf::from(home).join("metadata.sqlite");
    }
    home_dir().join(".cortex").join("metadata.sqlite")
}

fn home_dir() -> PathBuf {
    if let Ok(h) = std::env::var("HOME") {
        return PathBuf::from(h);
    }
    if let Ok(h) = std::env::var("USERPROFILE") {
        return PathBuf::from(h);
    }
    PathBuf::from(".")
}

/// Build a Nexus client from `CORTEX_NEXUS_URL` (or `NEXUS_URL` as
/// fallback) when set. Returns `None` when neither variable is set
/// or when the URL is unreachable — the dashboard graph endpoint
/// then degrades to the synthetic-from-lane fallback.
async fn build_nexus_client() -> Option<Arc<nexus_sdk::NexusClient>> {
    let url = std::env::var("CORTEX_NEXUS_URL")
        .or_else(|_| std::env::var("NEXUS_URL"))
        .ok()?;
    let cfg = nexus_sdk::ClientConfig {
        base_url: url.clone(),
        api_key: std::env::var("CORTEX_NEXUS_API_KEY").ok(),
        ..Default::default()
    };
    match nexus_sdk::NexusClient::with_config(cfg) {
        Ok(client) => {
            tracing::info!(url = %url, "nexus client connected");
            Some(Arc::new(client))
        }
        Err(e) => {
            tracing::warn!(
                url = %url,
                error = %e,
                "nexus client unreachable; graph endpoint will return synthetic data",
            );
            None
        }
    }
}

fn init_tracing(verbose: bool) {
    let level = if verbose { "debug" } else { "info" };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("{level},cortex_api={level}")));
    fmt().with_env_filter(filter).with_target(true).init();
}
