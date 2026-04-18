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
    }
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
