//! `cortex-api` binary — Axum daemon binding `POST /v1/query`.
//!
//! Spec 11 §HTTP endpoint: the daemon listens on
//! `127.0.0.1:15011` by default. Live lane wiring (Vectorizer,
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
use cortex_api::{
    MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane, Orchestrator, QueryService,
};
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Debug, Clone, Parser)]
#[command(name = "cortex-api", version, about)]
struct Cli {
    /// Bind address.
    #[arg(long, default_value = "127.0.0.1:15011")]
    bind: SocketAddr,
    /// Verbose tracing output.
    #[arg(long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    init_tracing(cli.verbose);

    let vector = Arc::new(MemoryVectorLane::new());
    let keyword = Arc::new(MemoryKeywordLane::new());
    let graph = Arc::new(MemoryGraphLane::new());

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
        let initial = cortex_api::load_into_keyword_lane(&archive_root, &keyword);
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
        let lane = keyword.clone();
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

    let nexus_client = build_nexus_client().await;
    let dashboard_state = cortex_api::DashboardState {
        lane: keyword.clone(),
        nexus: nexus_client,
    };

    let orchestrator = Orchestrator::new(vector, keyword.clone(), graph);
    let service = Arc::new(QueryService::with_memory_defaults(orchestrator));

    tracing::info!(bind = %cli.bind, "cortex-api starting");
    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    let app = cortex_api::build_router_with(service, Some(dashboard_state));
    axum::serve(listener, app).await?;
    Ok(())
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
