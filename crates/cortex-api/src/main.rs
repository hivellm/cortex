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
    GraphLane, KeywordLane, LoaderMetrics, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane,
    MeiliKeywordLane, NexusGraphLane, Orchestrator, QueryService, VectorLane, VectorizerLane,
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
    // Phase8b — single LoaderMetrics shared by archive_loader,
    // meili_loader, the dashboard freshness handler, and the
    // /metrics renderer. Each refresh task gets a clone of the Arc.
    let loader_metrics = Arc::new(LoaderMetrics::new());

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
        let has_login_creds = username.is_some() && password.is_some();
        let creds_configured = api_key.is_some() || has_login_creds;
        // Phase11a — when the URL is reachable but no credentials are
        // present, the SDK's unauthenticated `/health` probe succeeds
        // and the live lane wires up successfully, but every real
        // `search_vectors` call returns 401 with no recovery path.
        // Surface this loudly at boot so operators don't have to
        // reverse-engineer it from `debug.errors.vector` later.
        if !creds_configured {
            tracing::warn!(
                vectorizer_url = %vectorizer_url,
                checked_env = "CORTEX_VECTORIZER_API_KEY, VECTORIZER_API_KEY, \
                               CORTEX_VECTORIZER_USER + _PASSWORD, \
                               CORTEX_EMBEDDER_VECTORIZER_USER + _PASSWORD",
                "vector lane: URL set but no credentials configured — every \
                 authenticated search_vectors call will return 401. \
                 See docs/operations/vectorizer-auth.md."
            );
        }
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
            Ok(live) => {
                // Phase11a — when credentials are configured, run an
                // authenticated probe (`list_collections`) so we
                // catch a misconfigured-creds stack at boot instead
                // of on the first `/v1/query` call. Anonymous boots
                // fall back to the unauthenticated `/health` probe
                // since `list_collections` would 401 every time and
                // mask the (intended) anonymous behaviour.
                let probe_result = if creds_configured {
                    live.probe_authenticated().await
                } else {
                    live.probe().await
                };
                match probe_result {
                    Ok(()) => {
                        tracing::info!(
                            vectorizer_url = %vectorizer_url,
                            authenticated = creds_configured,
                            "live vector lane: VectorizerLane wired"
                        );
                        // Phase11a — optional periodic JWT warmup.
                        // Reactive refresh on 401 already exists in
                        // `vectorizer_lane.rs::search`; this loop
                        // keeps the cached JWT fresh proactively for
                        // deployments where 401-then-retry pairs add
                        // measurable tail latency. Disabled by
                        // default (0 / unset).
                        if has_login_creds {
                            let warmup_secs =
                                std::env::var("CORTEX_VECTORIZER_JWT_WARMUP_SECS")
                                    .ok()
                                    .and_then(|s| s.trim().parse::<u64>().ok())
                                    .unwrap_or(0);
                            if warmup_secs > 0 {
                                let lane = live.clone();
                                let url_for_log = vectorizer_url.clone();
                                tokio::spawn(async move {
                                    let mut ticker = tokio::time::interval(
                                        Duration::from_secs(warmup_secs),
                                    );
                                    // Skip the immediate first tick;
                                    // we just minted a JWT.
                                    ticker.tick().await;
                                    loop {
                                        ticker.tick().await;
                                        match lane.refresh_token().await {
                                            Ok(()) => tracing::debug!(
                                                vectorizer_url = %url_for_log,
                                                interval_secs = warmup_secs,
                                                "vector lane: JWT warmup refresh ok"
                                            ),
                                            Err(reason) => tracing::warn!(
                                                vectorizer_url = %url_for_log,
                                                reason = %reason,
                                                "vector lane: JWT warmup refresh failed"
                                            ),
                                        }
                                    }
                                });
                                tracing::info!(
                                    vectorizer_url = %vectorizer_url,
                                    interval_secs = warmup_secs,
                                    "vector lane: JWT warmup loop spawned"
                                );
                            }
                        }
                        StdArc::new(live)
                    }
                    Err(reason) => {
                        if creds_configured {
                            tracing::error!(
                                vectorizer_url = %vectorizer_url,
                                reason = %reason,
                                "live vector lane authenticated probe failed; \
                                 falling back to MemoryVectorLane. \
                                 Check CORTEX_VECTORIZER_USER / _PASSWORD."
                            );
                        } else {
                            tracing::warn!(
                                vectorizer_url = %vectorizer_url,
                                reason = %reason,
                                "live vector lane probe failed; falling back to MemoryVectorLane"
                            );
                        }
                        vector_memory.clone()
                    }
                }
            }
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
        let initial = cortex_api::archive_loader::load_into_keyword_lane_with_metrics(
            &archive_root,
            &keyword_memory,
            Some(loader_metrics.as_ref()),
        );
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
        let metrics_for_loop = loader_metrics.clone();
        tokio::spawn(async move {
            let interval = Duration::from_secs(refresh_secs);
            loop {
                tokio::time::sleep(interval).await;
                let report = cortex_api::archive_loader::load_into_keyword_lane_with_metrics(
                    &archive_root,
                    &lane,
                    Some(metrics_for_loop.as_ref()),
                );
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
        match cortex_api::meili_loader::load_meili_into_keyword_lane_with_metrics(
            &meili_url,
            meili_api_key.as_deref(),
            &keyword_memory,
            Some(loader_metrics.as_ref()),
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
        let metrics_for_loop = loader_metrics.clone();
        tokio::spawn(async move {
            let interval = Duration::from_secs(refresh_secs);
            loop {
                tokio::time::sleep(interval).await;
                match cortex_api::meili_loader::load_meili_into_keyword_lane_with_metrics(
                    &url,
                    key.as_deref(),
                    &lane,
                    Some(metrics_for_loop.as_ref()),
                )
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
    // Rulebook task loader. Two configuration paths:
    //
    // - **Single project** (legacy): `CORTEX_RULEBOOK_ROOT=/path/.rulebook`
    //   — same behaviour as before phase5b. The repo slug stamped on
    //   each row is `None`; the dashboard renders one anonymous list.
    // - **Multi project** (phase5b): `CORTEX_RULEBOOK_ROOTS=/path/A,/path/B,...`
    //   semicolon- or comma-separated `.rulebook/` directories. The
    //   repo slug is auto-derived from each directory's parent name
    //   (lowercased), so `/work/Cortex/.rulebook` becomes `cortex`.
    //   Both env vars may coexist; `_ROOTS` wins when present.
    //
    // Falls back to `<cwd>/.rulebook` so `cargo run` from the repo
    // root just works on a single-project deployment.
    let task_loaders: Vec<cortex_api::TaskLoader> =
        if let Ok(roots) = std::env::var("CORTEX_RULEBOOK_ROOTS") {
            let mut out = Vec::new();
            for raw in roots.split(|c: char| c == ',' || c == ';') {
                let p = PathBuf::from(raw.trim());
                if p.as_os_str().is_empty() {
                    continue;
                }
                let slug = p
                    .parent()
                    .and_then(|parent| parent.file_name())
                    .and_then(|n| n.to_str())
                    .map(|s| s.to_ascii_lowercase());
                tracing::info!(
                    rulebook_root = %p.display(),
                    repo_slug = ?slug,
                    "tasks loader: registered project"
                );
                let mut loader = cortex_api::TaskLoader::new(p);
                if let Some(s) = slug {
                    loader = loader.with_repo(s);
                }
                out.push(loader);
            }
            if out.is_empty() {
                tracing::warn!(
                    "CORTEX_RULEBOOK_ROOTS was set but parsed empty; falling back to single-root resolution"
                );
            }
            out
        } else {
            Vec::new()
        };
    let task_loaders = if task_loaders.is_empty() {
        let rulebook_root: PathBuf = std::env::var("CORTEX_RULEBOOK_ROOT")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                std::env::current_dir()
                    .map(|p| p.join(".rulebook"))
                    .unwrap_or_else(|_| PathBuf::from(".rulebook"))
            });
        tracing::info!(
            rulebook_root = %rulebook_root.display(),
            "tasks loader: rooted at .rulebook/ (single project)"
        );
        let slug = rulebook_root
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
            .map(|s| s.to_ascii_lowercase());
        let mut loader = cortex_api::TaskLoader::new(rulebook_root);
        if let Some(s) = slug {
            loader = loader.with_repo(s);
        }
        vec![loader]
    } else {
        task_loaders
    };
    let tasks = StdArc::new(cortex_api::MultiTaskLoader::new(task_loaders));

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
        loader_metrics: loader_metrics.clone(),
    };

    let fusion = resolve_fusion_config_from_env();
    tracing::info!(
        alpha = fusion.alpha,
        k = fusion.k,
        "fusion config resolved (CORTEX_RRF_ALPHA / CORTEX_RRF_K)"
    );
    let rewriter = resolve_query_rewriter_from_env();
    let orchestrator = Orchestrator::new(vector, keyword.clone(), graph)
        .with_fusion(fusion)
        .with_rewriter(rewriter);
    // Wire the `keyword_memory` snapshot into the service so
    // `/v1/status.indexed_repos` and `notice.repo_not_indexed` (issue
    // hivellm/cortex#1) read from the same source the dashboard does.
    let service = Arc::new(
        QueryService::with_memory_defaults(orchestrator)
            .with_indexed_repos(keyword_memory.clone()),
    );

    // Phase8e — silent-drop watcher. Polls the same divergence
    // pairs the /v1/health/divergence endpoint surfaces and emits
    // `law_violation` envelopes on sustained drops. The watcher
    // owns its own aggregator history (independent from the HTTP
    // endpoint's) so the polling cadence stays predictable.
    let silent_drop_cfg = cortex_api::silent_drop::SilentDropConfig::default();
    if silent_drop_cfg.enabled {
        let watcher_lane = keyword_memory.clone();
        let interval = std::time::Duration::from_secs(silent_drop_cfg.poll_interval_secs.max(5));
        let watcher_aggregator =
            std::sync::Arc::new(cortex_api::health::HealthAggregatorState::new());
        let mut watcher = cortex_api::silent_drop::SilentDropWatcher::new(
            silent_drop_cfg,
            watcher_aggregator,
        );
        watcher.hydrate_from_disk();
        tokio::spawn(async move {
            let client = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(5))
                .build()
                .unwrap_or_default();
            tracing::info!(
                interval_secs = interval.as_secs(),
                "silent-drop watcher started"
            );
            let mut ticker = tokio::time::interval(interval);
            // Skip the immediate first tick so the watcher gives
            // every subsystem time to bind /healthz before probing.
            ticker.tick().await;
            loop {
                ticker.tick().await;
                watcher.tick(&client, &watcher_lane).await;
            }
        });
    } else {
        tracing::info!("silent-drop watcher disabled by config");
    }

    // Phase10k — always-on retention scheduler. Spawns a 30-s tick
    // loop that calls into `cortex-retention::scheduler::tick` so
    // the eight default sweeps (FP32→PQ→Binary tier transitions,
    // meili-prune, CAS vacuum, metadata reap, PII enforce, turn
    // digest, memory consolidate) actually fire on schedule. The
    // daemon opens its own metadata connection so the dashboard
    // mutex (`std::sync::Mutex`) and the daemon mutex
    // (`tokio::sync::Mutex`) don't fight over the same handle —
    // SQLite WAL handles the on-disk concurrency. Skipped when the
    // metadata file isn't reachable (same precondition as the
    // dashboard ribbon) or when the operator opted out via
    // `CORTEX_RETENTION_DAEMON=disabled`.
    cortex_api::retention_daemon::spawn(
        metadata_path.clone(),
        cortex_api::retention_daemon::SpawnOptions::default(),
    );

    // Phase8f — opt-in synthetic canary runner. When
    // `CORTEX_CANARY_ENABLED=1`, fire a fake hook frame through the
    // real IPC pipe every `interval_secs` (default 300) and assert
    // it lands in the archive within `deadline_secs` (default 10).
    // On failure, emit a `law_violation` envelope via the same
    // path phase8e uses. Off by default — operators flip the env
    // var when they want quiet-hours regression coverage.
    if std::env::var("CORTEX_CANARY_ENABLED")
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false)
    {
        let mut canary_cfg = cortex_api::canary::CanaryConfig::default();
        canary_cfg.enabled = true;
        if let Ok(secs) = std::env::var("CORTEX_CANARY_INTERVAL_SECS").map(|s| s.parse::<u64>()) {
            if let Ok(s) = secs {
                canary_cfg.interval_secs = s.max(10);
            }
        }
        if let Ok(secs) = std::env::var("CORTEX_CANARY_DEADLINE_SECS").map(|s| s.parse::<u64>()) {
            if let Ok(s) = secs {
                canary_cfg.deadline_secs = s.max(1);
            }
        }
        tokio::spawn(cortex_api::canary::run_canary_loop(canary_cfg));
    } else {
        tracing::info!("canary runner disabled (set CORTEX_CANARY_ENABLED=1 to enable)");
    }

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

/// Phase6c — read the score-aware RRF knobs from the environment.
///
/// `CORTEX_RRF_ALPHA` (`f32`, default
/// [`cortex_api::fusion::DEFAULT_RRF_ALPHA`] = 0.7) is the blend
/// weight between positional RRF and the lane-native score.
/// `CORTEX_RRF_K` (`u32`, default 60) is the RRF stabilisation
/// constant. Out-of-range values log `WARN` and fall back to the
/// default — `FusionConfig::new` clamps anyway, but logging at
/// the boundary tells the operator the env var was visible.
fn resolve_fusion_config_from_env() -> cortex_api::fusion::FusionConfig {
    let default = cortex_api::fusion::FusionConfig::default();
    let alpha = match std::env::var("CORTEX_RRF_ALPHA") {
        Ok(raw) => match raw.trim().parse::<f32>() {
            Ok(v) if (0.0..=1.0).contains(&v) => v,
            Ok(v) => {
                tracing::warn!(
                    raw = %raw,
                    parsed = v,
                    "CORTEX_RRF_ALPHA out of [0.0, 1.0]; using default"
                );
                default.alpha
            }
            Err(e) => {
                tracing::warn!(
                    raw = %raw,
                    error = %e,
                    "CORTEX_RRF_ALPHA not a float; using default"
                );
                default.alpha
            }
        },
        Err(_) => default.alpha,
    };
    let k = match std::env::var("CORTEX_RRF_K") {
        Ok(raw) => match raw.trim().parse::<u32>() {
            Ok(v) if v >= 1 => v,
            Ok(v) => {
                tracing::warn!(raw = %raw, parsed = v, "CORTEX_RRF_K must be >= 1; using default");
                default.k
            }
            Err(e) => {
                tracing::warn!(raw = %raw, error = %e, "CORTEX_RRF_K not a u32; using default");
                default.k
            }
        },
        Err(_) => default.k,
    };
    cortex_api::fusion::FusionConfig::new(alpha, k)
}

/// Phase6f — pick the [`cortex_api::query_rewrite::QueryRewriter`]
/// implementation from `CORTEX_QUERY_REWRITER`.
///
/// Recognised values:
/// - `noun_phrase` (default) — deterministic noun-phrase strip, no
///   network call.
/// - `sonnet` — Anthropic Sonnet rewrite per cache miss; falls back
///   to noun-phrase on timeout / upstream error so a flaky upstream
///   never fails the user-facing call.
/// - `passthrough` — kill-switch reproducing the pre-phase6f
///   behaviour (prompt copied verbatim to every lane).
///
/// Unknown values log a `WARN` and fall back to `noun_phrase`.
fn resolve_query_rewriter_from_env() -> Arc<dyn cortex_api::query_rewrite::QueryRewriter> {
    use cortex_api::query_rewrite::{
        NounPhraseRewriter, PassthroughRewriter, SonnetRewriter,
    };
    let raw = std::env::var("CORTEX_QUERY_REWRITER")
        .ok()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty());
    let strategy = match raw.as_deref() {
        None | Some("noun_phrase") => "noun_phrase",
        Some("sonnet") => "sonnet",
        Some("passthrough") => "passthrough",
        Some(other) => {
            tracing::warn!(
                raw = %other,
                "CORTEX_QUERY_REWRITER unknown; falling back to noun_phrase"
            );
            "noun_phrase"
        }
    };
    tracing::info!(
        rewriter = strategy,
        "query rewriter resolved (CORTEX_QUERY_REWRITER)"
    );
    match strategy {
        "noun_phrase" => Arc::new(NounPhraseRewriter::new()),
        "sonnet" => Arc::new(SonnetRewriter::from_env()),
        "passthrough" => Arc::new(PassthroughRewriter),
        _ => unreachable!("strategy normalised above"),
    }
}
