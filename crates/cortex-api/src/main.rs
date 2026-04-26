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
use std::sync::Arc;

use anyhow::Result;
use clap::Parser;
use cortex_api::{
    build_router, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane, Orchestrator,
    QueryService,
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
    let orchestrator = Orchestrator::new(vector, keyword, graph);
    let service = Arc::new(QueryService::with_memory_defaults(orchestrator));

    tracing::info!(bind = %cli.bind, "cortex-api starting");
    let listener = tokio::net::TcpListener::bind(cli.bind).await?;
    let app = build_router(service);
    axum::serve(listener, app).await?;
    Ok(())
}

fn init_tracing(verbose: bool) {
    let level = if verbose { "debug" } else { "info" };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("{level},cortex_api={level}")));
    fmt().with_env_filter(filter).with_target(true).init();
}
