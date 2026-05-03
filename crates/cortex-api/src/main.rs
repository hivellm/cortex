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
use clap::{Parser, Subcommand};
use std::sync::Arc as StdArc;

use cortex_api::{
    GraphLane, KeywordLane, LoaderMetrics, MeiliKeywordLane, MemoryGraphLane, MemoryKeywordLane,
    MemoryVectorLane, NexusGraphLane, Orchestrator, QueryService, VectorLane, VectorizerLane,
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
    /// Phase3 §7.6-7.8 — admin subcommands manage the SQLite
    /// `api_keys` table the dashboard auth middleware reads from.
    /// When `Some(..)`, the daemon does not start; the subcommand
    /// runs and the process exits.
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Clone, Subcommand)]
enum Command {
    /// Manage dashboard API keys backing the
    /// `CORTEX_DASHBOARD_AUTH=1` middleware.
    Admin {
        #[command(subcommand)]
        op: AdminOp,
    },
}

#[derive(Debug, Clone, Subcommand)]
enum AdminOp {
    /// Mint a new API key. Prints the cleartext exactly once;
    /// only the Argon2id hash persists.
    IssueApiKey {
        /// Scope tag stored on the row. Today only `dashboard`
        /// is honoured by the middleware; future scopes can carve
        /// per-route auth (e.g. `query`, `ingest`).
        #[arg(long, default_value = "dashboard")]
        scope: String,
        /// Human-readable label, e.g. `local-gui`, `staging-readonly`.
        #[arg(long)]
        label: String,
    },
    /// Print every key in the table — id, scope, label, timestamps.
    /// Hashes are never printed.
    ListApiKeys,
    /// Soft-revoke a key by id. The middleware blocks the next
    /// request that presents it.
    RevokeApiKey {
        /// ULID id of the key to revoke (from `list-api-keys`).
        id: String,
    },
}

/// Resolve the path of the `api_keys.sqlite` file. Honours the
/// `CORTEX_API_KEYS_DB` env override; otherwise falls back to
/// `~/.cortex/api_keys.sqlite` (or `/tmp/cortex-api-keys.sqlite`
/// when `HOME` is unset, which only happens in the rare daemon-as-
/// root config the dev-stack does not exercise).
fn api_keys_db_path() -> PathBuf {
    if let Ok(p) = std::env::var("CORTEX_API_KEYS_DB") {
        return PathBuf::from(p);
    }
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| "/tmp".to_string());
    let dir = PathBuf::from(home).join(".cortex");
    std::fs::create_dir_all(&dir).ok();
    dir.join("api_keys.sqlite")
}

fn run_admin_op(op: AdminOp) -> Result<()> {
    let path = api_keys_db_path();
    let store = cortex_api::storage::api_keys::ApiKeyStore::open(&path)
        .map_err(|e| anyhow::anyhow!("open api_keys store at {}: {e}", path.display()))?;
    match op {
        AdminOp::IssueApiKey { scope, label } => {
            let issued = store
                .issue(&scope, &label)
                .map_err(|e| anyhow::anyhow!("issue: {e}"))?;
            // Print the cleartext exactly once. Operators copy
            // this value into the renderer's connection bearer
            // field; no second chance.
            println!("id:         {}", issued.id);
            println!("scope:      {}", issued.scope);
            println!("label:      {}", issued.label);
            println!("created_at: {} (epoch ms)", issued.created_at_ms);
            println!("key:        {}", issued.cleartext);
            println!();
            println!("This is the ONLY time the cleartext key is shown.");
            println!("Persist it now; the daemon stores only the Argon2id hash.");
        }
        AdminOp::ListApiKeys => {
            let rows = store.list().map_err(|e| anyhow::anyhow!("list: {e}"))?;
            if rows.is_empty() {
                println!("(no keys issued)");
                return Ok(());
            }
            println!(
                "{:<26} {:<12} {:<24} {:<14} {:<14} {:<14}",
                "id", "scope", "label", "created_at", "last_used_at", "revoked_at",
            );
            for r in rows {
                println!(
                    "{:<26} {:<12} {:<24} {:<14} {:<14} {:<14}",
                    r.id,
                    r.scope,
                    r.label,
                    r.created_at_ms,
                    r.last_used_at_ms
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    r.revoked_at_ms
                        .map(|v| v.to_string())
                        .unwrap_or_else(|| "-".to_string()),
                );
            }
        }
        AdminOp::RevokeApiKey { id } => {
            store
                .revoke(&id)
                .map_err(|e| anyhow::anyhow!("revoke: {e}"))?;
            println!("revoked: {id}");
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    if let Some(Command::Admin { op }) = cli.command {
        // Admin subcommands run synchronously and exit; never start
        // the HTTP listener. Keeps the surface narrow — operators
        // running `cortex-api admin issue-api-key` should never
        // accidentally bind a port.
        return tokio::task::spawn_blocking(move || run_admin_op(op))
            .await
            .map_err(|e| anyhow::anyhow!("admin task join: {e}"))?;
    }

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
        std::env::var("CORTEX_VECTORIZER_URL").or_else(|_| std::env::var("VECTORIZER_URL"))
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
                            let warmup_secs = std::env::var("CORTEX_VECTORIZER_JWT_WARMUP_SECS")
                                .ok()
                                .and_then(|s| s.trim().parse::<u64>().ok())
                                .unwrap_or(0);
                            if warmup_secs > 0 {
                                let lane = live.clone();
                                let url_for_log = vectorizer_url.clone();
                                tokio::spawn(async move {
                                    let mut ticker =
                                        tokio::time::interval(Duration::from_secs(warmup_secs));
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
        tracing::info!("CORTEX_VECTORIZER_URL unset; vector lane stays on MemoryVectorLane");
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
        tracing::info!("CORTEX_FULLTEXT_MEILI_URL unset; keyword lane stays on MemoryKeywordLane");
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
                    Err(e) => {
                        tracing::warn!(meili_url = %url, error = %e, "meili loader: refresh failed")
                    }
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
    let mut rulebook_roots: Vec<PathBuf> = Vec::new();
    let task_loaders: Vec<cortex_api::TaskLoader> = if let Ok(roots) =
        std::env::var("CORTEX_RULEBOOK_ROOTS")
    {
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
            rulebook_roots.push(p.clone());
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
        rulebook_roots.push(rulebook_root.clone());
        let mut loader = cortex_api::TaskLoader::new(rulebook_root);
        if let Some(s) = slug {
            loader = loader.with_repo(s);
        }
        vec![loader]
    } else {
        task_loaders
    };
    let tasks = StdArc::new(cortex_api::MultiTaskLoader::new(task_loaders));

    // Phase11m §2.4 — dashboard event bus + filesystem watcher per root.
    // Watcher publishes `DashboardEvent`s (TaskChanged / HandoffAppended /
    // DecisionChanged / KnowledgeAdded) into the bus; SSE subscribers fan
    // them out to GUI clients. Watchers are gated by `CORTEX_DASHBOARD_WATCH`
    // (default `1`) so cold-stack tests can opt out without unsetting the
    // root env vars.
    let dashboard_bus = cortex_api::dashboard_watcher::DashboardEventBus::new();
    let watch_enabled = std::env::var("CORTEX_DASHBOARD_WATCH")
        .map(|v| v != "0")
        .unwrap_or(true);
    let mut watcher_handles: Vec<cortex_api::dashboard_watcher::WatcherHandle> = Vec::new();
    if watch_enabled {
        for root in &rulebook_roots {
            match cortex_api::dashboard_watcher::spawn_watcher(root.clone(), dashboard_bus.clone())
            {
                Ok(h) => {
                    tracing::info!(
                        rulebook_root = %root.display(),
                        "dashboard watcher: started"
                    );
                    watcher_handles.push(h);
                }
                Err(err) => {
                    tracing::warn!(
                        rulebook_root = %root.display(),
                        error = %err,
                        "dashboard watcher: failed to start (push updates fall back to MCP path only)"
                    );
                }
            }
        }
    } else {
        tracing::info!("dashboard watcher: disabled by CORTEX_DASHBOARD_WATCH=0");
    }

    // Phase11n §2.5 — SQLite tail loop for `.rulebook/memory/memory.db`.
    // The FS watcher cannot tell the GUI which row appeared on a `.db`
    // mtime jitter; this loop polls the database read-only at the same
    // 250 ms cadence and emits one `MemoryAppended` per genuinely-new
    // row. Gated by `CORTEX_DASHBOARD_MEMORY_TAIL` (default `1`).
    let memory_tail_enabled = std::env::var("CORTEX_DASHBOARD_MEMORY_TAIL")
        .map(|v| v != "0")
        .unwrap_or(true);
    let mut memory_tail_handles: Vec<tokio::task::JoinHandle<()>> = Vec::new();
    if watch_enabled && memory_tail_enabled {
        let run_flag = std::sync::Arc::new(|| true) as std::sync::Arc<dyn Fn() -> bool + Send + Sync>;
        for root in &rulebook_roots {
            let db_path = root.join("memory").join("memory.db");
            let handle = cortex_api::memory_tail::spawn_tail_loop(
                db_path.clone(),
                dashboard_bus.clone(),
                cortex_api::dashboard_watcher::DEFAULT_DEBOUNCE,
                run_flag.clone(),
            );
            tracing::info!(
                memory_db = %db_path.display(),
                "dashboard memory tail: started"
            );
            memory_tail_handles.push(handle);
        }
    } else if !memory_tail_enabled {
        tracing::info!("dashboard memory tail: disabled by CORTEX_DASHBOARD_MEMORY_TAIL=0");
    }
    // Keep watchers alive for the daemon lifetime. Leaking is the right
    // shape here — the process owns them until it exits, and any other
    // hold pattern would force every downstream caller to thread the
    // handles through just to satisfy the borrow checker.
    Box::leak(Box::new(watcher_handles));

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
        events_bus: dashboard_bus,
    };

    // Phase11i §3.6 — load relevance.toml first so the env-derived
    // RRF knobs layer on top of the file-supplied defaults. A
    // missing / unreadable file logs INFO + falls back to compile-
    // time defaults; a malformed file logs WARN and falls back the
    // same way (so a typo never bricks the daemon).
    let relevance_path = cortex_api::default_relevance_config_path();
    let relevance = load_relevance_config(&relevance_path);
    let fusion = resolve_fusion_config_from_env_with_base(relevance.base_fusion());
    tracing::info!(
        alpha = fusion.alpha,
        k = fusion.k,
        cross_repo_boost = fusion.cross_repo_boost,
        same_session_boost = fusion.same_session_boost,
        cohort_session_boost = fusion.cohort_session_boost,
        config_path = %relevance_path.display(),
        "fusion config resolved (relevance.toml + CORTEX_RRF_ALPHA / CORTEX_RRF_K)"
    );
    let rewriter = resolve_query_rewriter_from_env();
    let orchestrator = Orchestrator::new(vector, keyword.clone(), graph)
        .with_fusion(fusion)
        .with_rewriter(rewriter);
    spawn_relevance_reload_task(orchestrator.clone(), relevance_path.clone());
    // Phase11e §6 — coverage snapshot handle, shared between the
    // boot-time audit (which writes the result here) and `/v1/status`
    // (which reads it). Empty until the audit completes; subsequent
    // refreshes overwrite atomically.
    let coverage_snapshot: Arc<
        tokio::sync::RwLock<Option<cortex_api::coverage::CoverageResponse>>,
    > = Arc::new(tokio::sync::RwLock::new(None));
    // Wire the `keyword_memory` snapshot into the service so
    // `/v1/status.indexed_repos` and `notice.repo_not_indexed` (issue
    // hivellm/cortex#1) read from the same source the dashboard does.
    let service = Arc::new(
        QueryService::with_memory_defaults(orchestrator)
            .with_indexed_repos(keyword_memory.clone())
            .with_coverage_snapshot(coverage_snapshot.clone()),
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
        let mut watcher =
            cortex_api::silent_drop::SilentDropWatcher::new(silent_drop_cfg, watcher_aggregator);
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

    // Phase11e — boot-time collection / index inventory diff. Logs
    // a per-backend INFO summary plus a WARN per missing collection
    // / index so a misconfigured indexer (e.g. embedder shipping
    // `cortex-cortex-{code,docs,misc,governance}` only, missing
    // every `*-decisions` / `*-turns` / `*-analyses` collection) is
    // visible at startup instead of only at query time. Slugs come
    // from `CORTEX_COVERAGE_SLUGS` plus whatever the keyword lane
    // already learned from the archive / Meili loader.
    //
    // Phase11e §6 — the snapshot also writes through to
    // `coverage_snapshot` so `/v1/status` and `cortex_status` MCP
    // can report honest per-backend coverage without re-running the
    // diff on every call.
    if let Some(snapshot) = audit_coverage_at_boot(&keyword_memory).await {
        let mut guard = coverage_snapshot.write().await;
        *guard = Some(snapshot);
    }

    // Phase3 §7.4 — open the api_keys store and thread it into the
    // router builder. When CORTEX_DASHBOARD_AUTH is unset/false the
    // store is still opened (so admin subcommands can mint keys
    // ahead of enabling the gate) but the middleware short-circuits
    // and adds zero per-request overhead.
    let api_keys_path = api_keys_db_path();
    let api_keys_store: Option<Arc<cortex_api::storage::api_keys::ApiKeyStore>> =
        match cortex_api::storage::api_keys::ApiKeyStore::open(&api_keys_path) {
            Ok(s) => {
                tracing::info!(
                    path = %api_keys_path.display(),
                    auth_enabled = cortex_api::auth::dashboard_auth_enabled(),
                    "api_keys store opened",
                );
                Some(Arc::new(s))
            }
            Err(e) => {
                tracing::warn!(
                    path = %api_keys_path.display(),
                    error = %e,
                    "api_keys store unavailable; dashboard auth gate disabled even if env says enable"
                );
                None
            }
        };

    tracing::info!(bind = %cli.bind, "cortex-api starting");
    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    let app = cortex_api::build_router_with_auth(service, Some(dashboard_state), api_keys_store);
    axum::serve(listener, app).await?;
    Ok(())
}

/// Phase11e — run the boot-time collection / index inventory diff
/// against the live Vectorizer + Meili and log per-backend coverage.
///
/// `keyword_memory_lane` is the same `Arc<MemoryKeywordLane>` the
/// `/v1/status.indexed_repos` field reads from; we reuse it as the
/// canonical source of slugs so the coverage report does not
/// disagree with the rest of the daemon. The `CORTEX_COVERAGE_SLUGS`
/// env adds slugs the keyword lane has not yet seen (useful for
/// pinning new repos before the first envelope flows through).
///
/// Returns the [`cortex_api::coverage::CoverageResponse`] the audit
/// produced so the caller can store it on the status snapshot
/// (phase11e §6). `None` when no backends are configured or the
/// expected set is empty (e.g. unit-test environment).
async fn audit_coverage_at_boot(
    keyword_memory: &Arc<cortex_api::MemoryKeywordLane>,
) -> Option<cortex_api::coverage::CoverageResponse> {
    use std::collections::BTreeSet;
    use std::time::Duration;

    let mut slugs: BTreeSet<String> = keyword_memory
        .indexed_repos()
        .into_iter()
        .map(|s| s.to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let env_slugs = cortex_api::coverage::slugs_from_env(
        std::env::var("CORTEX_COVERAGE_SLUGS").ok().as_deref(),
    );
    slugs.extend(env_slugs);
    if slugs.is_empty() {
        // Default to the cortex slug so the diff has something to
        // report on a fresh boot before any envelope has flowed
        // through. Operators can override via CORTEX_COVERAGE_SLUGS.
        slugs.insert("cortex".to_string());
    }
    let expected = cortex_api::coverage::expected_collections(&slugs);
    if expected.is_empty() {
        tracing::debug!("coverage audit: expected set empty; skipping");
        return None;
    }

    let http = match reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(error = %e, "coverage audit: reqwest builder failed; skipping");
            return None;
        }
    };

    let vectorizer_url = std::env::var("CORTEX_VECTORIZER_URL")
        .or_else(|_| std::env::var("VECTORIZER_URL"))
        .ok();
    let bearer = match vectorizer_url.as_deref() {
        Some(url) => resolve_vectorizer_bearer(&http, url).await,
        None => None,
    };
    let meili_url = std::env::var("CORTEX_FULLTEXT_MEILI_URL").ok();
    let meili_key = std::env::var("CORTEX_FULLTEXT_MEILI_API_KEY")
        .ok()
        .or_else(|| std::env::var("MEILI_MASTER_KEY").ok());

    if vectorizer_url.is_none() && meili_url.is_none() {
        tracing::debug!("coverage audit: no backends configured; skipping");
        return None;
    }

    let archive_watchers = cortex_api::coverage::resolve_archive_watcher_urls();
    let response = cortex_api::coverage::run_coverage_audit_with_archive_watchers(
        &http,
        slugs,
        vectorizer_url.as_deref(),
        bearer.as_deref(),
        meili_url.as_deref(),
        meili_key.as_deref(),
        &archive_watchers,
        Duration::from_secs(5),
    )
    .await;

    // Log per-backend INFO summary + WARN per missing collection.
    // The structured INFO + WARN records remain the operator's
    // primary observability surface; the cached response below is
    // the consumer-facing JSON for `/v1/status` and the MCP tool.
    for backend in &response.backends {
        if let Some(url) = backend.base_url.as_deref() {
            // Reconstruct a CoverageDiff for log_diff's existing
            // signature without round-tripping the BTreeSets.
            let diff = cortex_api::coverage::CoverageDiff {
                expected: backend
                    .present
                    .iter()
                    .chain(backend.missing.iter())
                    .cloned()
                    .collect(),
                present: backend.present.iter().cloned().collect(),
                missing: backend.missing.iter().cloned().collect(),
                unexpected: backend.unexpected.iter().cloned().collect(),
            };
            cortex_api::coverage::log_diff(backend.backend, url, &diff);
            if let Some(reason) = backend.error.as_deref() {
                tracing::warn!(
                    backend = backend.backend,
                    base_url = url,
                    reason = reason,
                    "coverage audit: backend probe failed"
                );
            }
        }
    }

    Some(response)
}

/// Phase11e — resolve a Vectorizer bearer for the inventory probe.
/// Mirrors the boot-time auth precedence in the vector-lane wiring
/// (api_key wins, otherwise user+password → /auth/login). Returns
/// `None` when no credentials are configured (anonymous dev stack).
async fn resolve_vectorizer_bearer(http: &reqwest::Client, base_url: &str) -> Option<String> {
    if let Ok(key) =
        std::env::var("CORTEX_VECTORIZER_API_KEY").or_else(|_| std::env::var("VECTORIZER_API_KEY"))
    {
        if !key.is_empty() {
            return Some(key);
        }
    }
    let user = std::env::var("CORTEX_VECTORIZER_USER")
        .or_else(|_| std::env::var("CORTEX_EMBEDDER_VECTORIZER_USER"))
        .ok()?;
    let pass = std::env::var("CORTEX_VECTORIZER_PASSWORD")
        .or_else(|_| std::env::var("CORTEX_EMBEDDER_VECTORIZER_PASSWORD"))
        .ok()?;
    let url = format!("{}/auth/login", base_url.trim_end_matches('/'));
    let body = serde_json::json!({ "username": user, "password": pass });
    match http
        .post(&url)
        .timeout(std::time::Duration::from_secs(5))
        .json(&body)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => match resp.json::<serde_json::Value>().await {
            Ok(j) => j
                .get("access_token")
                .and_then(|v| v.as_str())
                .map(String::from),
            Err(e) => {
                tracing::debug!(error = %e, "coverage audit: /auth/login parse failed");
                None
            }
        },
        Ok(resp) => {
            tracing::debug!(
                status = resp.status().as_u16(),
                "coverage audit: /auth/login non-2xx"
            );
            None
        }
        Err(e) => {
            tracing::debug!(error = %e, "coverage audit: /auth/login transport failed");
            None
        }
    }
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
/// Phase11i §3.6 — env-derived RRF overrides layered on top of a
/// `base` config (typically the `relevance.toml` snapshot). The
/// boot path passes the file-derived defaults; SIGHUP reload
/// re-reads the same file and re-applies the env overrides so an
/// operator running with `CORTEX_RRF_ALPHA` set keeps that bias
/// across reloads.
fn resolve_fusion_config_from_env_with_base(
    base: cortex_api::fusion::FusionConfig,
) -> cortex_api::fusion::FusionConfig {
    let default = base.clone();
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
    cortex_api::fusion::FusionConfig {
        alpha: alpha.clamp(0.0, 1.0),
        k: k.max(1),
        ..default
    }
}

/// Phase11i §3.6 — load `relevance.toml` honouring the
/// `CORTEX_RELEVANCE_CONFIG` env var (or the in-tree default).
/// Logs INFO when the file is absent (boot continues at compile-
/// time defaults) and WARN when the file is malformed, so the
/// operator sees the typo but the daemon still starts.
fn load_relevance_config(path: &std::path::Path) -> cortex_api::RelevanceConfig {
    match cortex_api::RelevanceConfig::load_from_path(path) {
        Ok(cfg) => {
            tracing::info!(
                path = %path.display(),
                "relevance config loaded"
            );
            cfg
        }
        Err(cortex_api::RelevanceConfigError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound =>
        {
            tracing::info!(
                path = %path.display(),
                "relevance.toml not found — using compile-time defaults"
            );
            cortex_api::RelevanceConfig::default()
        }
        Err(err) => {
            tracing::warn!(
                error = %err,
                "relevance config unreadable — using compile-time defaults"
            );
            cortex_api::RelevanceConfig::default()
        }
    }
}

/// Phase11i §3.6 — SIGHUP reload task. On Unix, every SIGHUP
/// re-reads the configured `relevance.toml` and stamps the new
/// snapshot into the orchestrator's `Arc<RwLock<FusionConfig>>`
/// via [`Orchestrator::replace_fusion`]. On non-Unix platforms
/// the task is a no-op + a one-shot WARN so the operator knows
/// hot-reload is unavailable there. The env-derived overrides
/// (`CORTEX_RRF_ALPHA` / `CORTEX_RRF_K`) are also re-read so
/// operators can flip them via systemd `Environment=` overrides
/// without a daemon restart.
#[cfg(unix)]
fn spawn_relevance_reload_task(orchestrator: Orchestrator, path: PathBuf) {
    use tokio::signal::unix::{signal, SignalKind};
    tokio::spawn(async move {
        let mut hup = match signal(SignalKind::hangup()) {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!(error = %err, "SIGHUP listener unavailable; relevance hot-reload disabled");
                return;
            }
        };
        tracing::info!(
            path = %path.display(),
            "SIGHUP relevance reload listener installed"
        );
        while hup.recv().await.is_some() {
            let relevance = load_relevance_config(&path);
            let fusion = resolve_fusion_config_from_env_with_base(relevance.base_fusion());
            orchestrator.replace_fusion(fusion.clone());
            tracing::info!(
                alpha = fusion.alpha,
                k = fusion.k,
                cross_repo_boost = fusion.cross_repo_boost,
                same_session_boost = fusion.same_session_boost,
                cohort_session_boost = fusion.cohort_session_boost,
                "relevance config reloaded (SIGHUP)"
            );
        }
    });
}

#[cfg(not(unix))]
fn spawn_relevance_reload_task(_orchestrator: Orchestrator, _path: PathBuf) {
    tracing::warn!(
        "SIGHUP relevance hot-reload only supported on Unix; current build will only honour boot-time relevance.toml"
    );
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
    use cortex_api::query_rewrite::{NounPhraseRewriter, PassthroughRewriter, SonnetRewriter};
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
