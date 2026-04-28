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
        /// Emit JSON instead of the markdown table.
        #[arg(long)]
        json: bool,
    },
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
        Command::DoctorConsistency {
            archive_root,
            meili,
            meili_key,
            vectorizer,
            vectorizer_user,
            vectorizer_password,
            nexus,
            json,
        } => doctor_consistency(
            archive_root,
            meili,
            meili_key,
            vectorizer,
            vectorizer_user,
            vectorizer_password,
            nexus,
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

    let archive = match cortex_ops::ArchiveProbe::new(&archive_root).scan() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("archive scan failed: {e}");
            return ExitCode::FAILURE;
        }
    };

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

    let report = cortex_ops::coverage_report_full(
        archive,
        meili_partitions,
        non_canonical,
        vec_partitions,
        non_canonical_vec,
        nexus_repo_counts,
        cortex_ops::CoverageOptions::default(),
    );
    if json {
        match serde_json::to_string_pretty(&report) {
            Ok(s) => println!("{s}"),
            Err(e) => {
                eprintln!("serialize report: {e}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        print!("{}", cortex_ops::render_coverage_markdown(&report));
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
    std::collections::BTreeMap<cortex_ops::PartitionKey, u64>,
    Vec<String>,
)> {
    use cortex_fulltext::{FulltextConfig, LiveMeiliClient};
    let config = FulltextConfig {
        meili_url: url.to_string(),
        meili_api_key: api_key.map(String::from),
        ..FulltextConfig::default()
    };
    let client = LiveMeiliClient::new(&config)
        .map_err(|e| anyhow::anyhow!("meili client: {e}"))?;
    cortex_ops::doctor::meili_partition_counts(&client).await
}

async fn probe_vectorizer(
    url: &str,
    user: &str,
    password: &str,
) -> anyhow::Result<(
    std::collections::BTreeMap<cortex_ops::PartitionKey, u64>,
    Vec<String>,
)> {
    use cortex_ops::{LiveVectorizerCoverageProbe, VectorizerCoverageScan};
    let probe = LiveVectorizerCoverageProbe::new(url, user, password).await?;
    probe.scan().await
}

async fn probe_nexus(url: &str) -> anyhow::Result<cortex_ops::NexusCounts> {
    use cortex_graph::GraphConfig;
    use cortex_ops::{LiveNexusCoverageProbe, NexusCoverageScan};
    // GraphConfig::from_env reads the rest of the auth / transport
    // knobs (CORTEX_NEXUS_USER / _PASSWORD, transport selection, …)
    // so we let the operator set them through the same env vars the
    // streaming worker already honours.
    let mut config = GraphConfig::from_env();
    config.nexus_url = url.to_string();
    let probe = LiveNexusCoverageProbe::new(config)?;
    probe.scan().await
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
