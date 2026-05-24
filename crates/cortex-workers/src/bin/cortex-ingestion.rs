//! `cortex-ingestion` — the ingestion HTTP server.

#![forbid(unsafe_code)]

use cortex_workers::ingestion::{
    build_router, AppState, IngestionConfig, Metrics, NdJsonZstdArchive, Publisher, SynapPublisher,
};
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cfg = IngestionConfig::from_env();
    info!(?cfg.bind, archive_root = ?cfg.archive_root, "starting cortex-ingestion");

    let archive = Arc::new(NdJsonZstdArchive::new(
        cfg.archive_root.clone(),
        cfg.archive_zstd_level,
    ));

    let publisher: Arc<dyn Publisher> = match cfg.synap_url.as_deref() {
        Some(url) => {
            info!(url, "synap publisher enabled");
            Arc::new(SynapPublisher::new(url)?)
        }
        None => {
            info!("synap publisher disabled (SYNAP_URL unset); using memory publisher");
            Arc::new(cortex_workers::ingestion::MemoryPublisher::default())
        }
    };

    let metrics = Arc::new(Metrics::default());
    let state = AppState::new(archive.clone(), publisher, metrics)
        .with_archive_root(cfg.archive_root.clone());
    let app = build_router(state);

    let listener = tokio::net::TcpListener::bind(cfg.bind).await?;
    info!("listening on {}", cfg.bind);
    axum::serve(listener, app).await?;
    Ok(())
}
