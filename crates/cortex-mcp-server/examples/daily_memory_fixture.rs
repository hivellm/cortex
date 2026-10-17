//! Disposable loopback API for installed-client daily-memory acceptance.
//! Never load operator configuration, existing databases, or model providers.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{DefaultBodyLimit, Request};
use axum::http::StatusCode;
use axum::middleware::{from_fn, Next};
use axum::response::{IntoResponse, Response};
use axum::Json;
use clap::Parser;
use cortex_api::{
    build_router_with_auth_and_cfg, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane,
    Orchestrator, QueryService,
};
use cortex_storage::MetadataStore;
use serde_json::json;

const REPO: &str = "phase34-harness";

#[derive(Parser)]
#[command(about = "Start a disposable daily-memory API on an ephemeral loopback port")]
struct Args {
    /// Private test-artifact directory; existing databases require verified resume.
    #[arg(long)]
    directory: PathBuf,
    /// Resume only a synthetic fixture previously created in this exact directory.
    #[arg(long)]
    resume: bool,
}

fn ownership_marker(directory: &Path, database: &Path) -> String {
    format!(
        "cortex-daily-memory-synthetic-fixture-v1\n{}\n{}\n",
        directory.display(),
        database.display()
    )
}

fn verify_resume(directory: &Path, database: &Path) -> std::io::Result<()> {
    let marker = directory.join("synthetic-fixture.owner");
    if fs::metadata(&marker)?.len() > 8192
        || fs::read_to_string(&marker)? != ownership_marker(directory, database)
        || !database.is_file()
        || fs::canonicalize(database)? != database
    {
        return Err(std::io::Error::other(
            "synthetic fixture ownership mismatch",
        ));
    }
    Ok(())
}

async fn bounded_request(request: Request, next: Next) -> Response {
    match tokio::time::timeout(Duration::from_secs(5), next.run(request)).await {
        Ok(response) => response,
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error":"fixture_request_timeout"})),
        )
            .into_response(),
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if !args.resume {
        fs::create_dir_all(&args.directory)?;
    }
    let directory = fs::canonicalize(&args.directory)?;
    let db_path = directory.join("daily-memory-fixture.sqlite");
    if args.resume {
        verify_resume(&directory, &db_path)?;
    } else {
        // Atomic reservation never overwrites an existing database.
        drop(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&db_path)?,
        );
    }
    drop(MetadataStore::open(&db_path)?);
    let cwd = directory.join("repository");
    if !args.resume {
        fs::create_dir(&cwd)?;
        fs::create_dir(cwd.join(".git"))?;
        fs::write(cwd.join(".git/HEAD"), "ref: refs/heads/main\n")?;
        fs::write(
            cwd.join("cortex.toml"),
            format!("[cortex]\nid = '{REPO}'\n"),
        )?;
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("synthetic-fixture.owner"))?
            .write_all(ownership_marker(&directory, &db_path).as_bytes())?;
    }

    let service = QueryService::with_memory_defaults(Orchestrator::new(
        Arc::new(MemoryVectorLane::new().with_fail()),
        Arc::new(MemoryKeywordLane::new().with_fail()),
        Arc::new(MemoryGraphLane::new().with_fail()),
    ));
    let mut config = cortex_config::Config::default();
    config.runtime.profile = cortex_config::RuntimeProfile::Daily;
    config.ingestion.metadata_db = Some(db_path.to_string_lossy().into_owned());
    config.ingestion.ingestion_url = Some("http://127.0.0.1:1".into());
    // This builder explicitly leaves post-capture model advice disabled.
    let app = build_router_with_auth_and_cfg(Arc::new(service), None, None, Arc::new(config))
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(from_fn(bounded_request));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    println!(
        "{}",
        json!({
            "api_url":format!("http://{}", listener.local_addr()?),
            "repo":REPO,
            "db_path":db_path,
            "cwd":cwd,
            "authentication":"disabled_synthetic_fixture_only",
            "optional_lanes":"unavailable",
            "model_advice":"disabled"
        })
    );
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_requires_exact_fixture_ownership_without_rewriting_files() {
        let temp = tempfile::tempdir().unwrap();
        let directory = fs::canonicalize(temp.path()).unwrap();
        let db = directory.join("daily-memory-fixture.sqlite");
        fs::write(&db, b"existing fixture bytes").unwrap();
        assert!(verify_resume(&directory, &db).is_err());
        let marker = directory.join("synthetic-fixture.owner");
        fs::write(&marker, ownership_marker(&directory, &db)).unwrap();
        verify_resume(&directory, &db).unwrap();
        fs::write(&marker, ownership_marker(&directory.join("another"), &db)).unwrap();
        assert!(verify_resume(&directory, &db).is_err());
        assert_eq!(fs::read(&db).unwrap(), b"existing fixture bytes");
    }
}
