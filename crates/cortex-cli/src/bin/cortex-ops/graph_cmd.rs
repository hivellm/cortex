use super::helpers::resolve_metadata_db_path;
use std::process::ExitCode;

/// Phase11l §7.1 — `cortex-ops graph drop` dispatcher. Calls Nexus
/// `MATCH (n:Label) DETACH DELETE n` for every label Cortex owns,
/// returning the per-label delete count. Refuses to run without
/// `--confirm`; `--dry-run` reports the planned counts via
/// `MATCH (n:Label) RETURN count(n)` without mutation.
///
/// Cortex-owned labels list mirrors `SCHEMA_STATEMENTS` plus the
/// phase11k §1.4 graph-correlation labels. The list lives here
/// rather than in the schema module because it is admin-only —
/// the runtime worker never enumerates labels for deletion.
pub(super) fn graph_drop(
    confirm: bool,
    dry_run: bool,
    nexus: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_workers::graph::config::GraphConfig;
    use cortex_workers::graph::nexus_client::{GraphClient, LiveNexusClient};

    const CORTEX_LABELS: &[&str] = &[
        "Session",
        "Turn",
        "ToolCall",
        "AgentCall",
        "Artifact",
        "Symbol",
        "Repo",
        "Decision",
        "Memory",
        "Analysis",
        "Law",
        "LawViolation",
        "Knowledge",
        "Learning",
        "Consolidation",
        "Spec",
        "ExternalPackage",
        "UnresolvedImport",
        "DocSection",
        "Concept",
        "Topic",
        "Tool",
    ];

    if !confirm && !dry_run {
        eprintln!(
            "ERROR: cortex-ops graph drop refuses to run without --confirm. \
             Pass --dry-run to see the plan without mutation."
        );
        return ExitCode::from(2);
    }

    let nexus_url = nexus
        .or_else(|| {
            cortex_config::Config::load()
                .ok()
                .and_then(|c| c.nexus.nexus_url)
        })
        .unwrap_or_else(|| "http://127.0.0.1:17002".to_string());
    let cfg = GraphConfig {
        nexus_url: nexus_url.clone(),
        ..GraphConfig::default()
    };
    let client = match LiveNexusClient::new(cfg) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("ERROR: connect to Nexus at {nexus_url}: {e}");
            return ExitCode::from(1);
        }
    };
    let _ = &client as &dyn GraphClient;

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("ERROR: build tokio runtime: {e}");
            return ExitCode::from(1);
        }
    };

    let report = runtime.block_on(async {
        let mut results: Vec<(String, u64)> = Vec::with_capacity(CORTEX_LABELS.len());
        for label in CORTEX_LABELS {
            let cypher = if dry_run {
                format!("MATCH (n:{label}) RETURN count(n) AS c")
            } else {
                format!("MATCH (n:{label}) DETACH DELETE n RETURN count(n) AS c")
            };
            match client.sdk().execute_cypher(&cypher, None).await {
                Ok(out) => {
                    let count = out
                        .rows
                        .first()
                        .and_then(|row| row.get("c"))
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0);
                    results.push((label.to_string(), count));
                }
                Err(e) => {
                    eprintln!("warn: {label}: {e}");
                    results.push((label.to_string(), 0));
                }
            }
        }
        results
    });

    let total: u64 = report.iter().map(|(_, c)| *c).sum();
    if json {
        let payload = serde_json::json!({
            "mode": if dry_run { "dry-run" } else { "applied" },
            "nexus_url": nexus_url,
            "total_nodes": total,
            "by_label": report
                .iter()
                .map(|(l, c)| (l.clone(), *c))
                .collect::<std::collections::BTreeMap<_, _>>(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else {
        println!(
            "cortex-ops graph drop @ {nexus_url}  ({}{})",
            if dry_run { "dry-run" } else { "applied" },
            if dry_run { "" } else { " — DESTRUCTIVE" }
        );
        for (label, count) in &report {
            println!("  {label:<24}  {count:>10}");
        }
        println!("  {:<24}  {total:>10}", "TOTAL");
    }
    ExitCode::SUCCESS
}

/// Phase11s §2.4 — rewind the durable graph-worker offset to a
/// known starting point. `since=N` makes the next worker boot
/// resume from `N + 1` (replaying every envelope after `N`).
/// `--dry-run` reports the planned write without mutating the row.
///
/// Operator runbook (§5): when a known event window was lost
/// (e.g. a worker restart during an indexer rebuild),
/// `cortex-ops graph replay --since=<known_good_offset>` rewinds
/// the consumer cursor; the next `docker restart cortex-graph-worker`
/// (or natural boot cycle) replays the missing window.
pub(super) fn graph_replay(
    since: u64,
    consumer_id: String,
    stream: String,
    metadata_db: Option<String>,
    dry_run: bool,
) -> ExitCode {
    use cortex_storage::MetadataStore;

    let db_path = match metadata_db {
        Some(p) => std::path::PathBuf::from(p),
        None => resolve_metadata_db_path(),
    };
    if !db_path.exists() {
        eprintln!(
            "ERROR: metadata DB not found at {db_path:?}. \
             Set --metadata-db / CORTEX_METADATA_DB to point at the worker's SQLite file."
        );
        return ExitCode::from(1);
    }

    let store = match MetadataStore::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ERROR: open metadata DB at {db_path:?}: {e}");
            return ExitCode::from(1);
        }
    };

    let prior = match store.consumer_offset_lookup(&consumer_id, &stream) {
        Ok(row) => row,
        Err(e) => {
            eprintln!("ERROR: consumer_offset_lookup({consumer_id}, {stream}): {e}");
            return ExitCode::from(1);
        }
    };

    println!(
        "graph replay plan: consumer_id={consumer_id} stream={stream}\n  current_offset={current}\n  rewind_to={since} (next boot resumes from {resume})\n  metadata_db={db_path:?}",
        current = prior
            .as_ref()
            .map(|r| r.last_offset.to_string())
            .unwrap_or_else(|| "<unset>".to_string()),
        resume = since.saturating_add(1),
    );

    if dry_run {
        println!("dry-run: no rows written");
        return ExitCode::SUCCESS;
    }

    if let Err(e) = store.consumer_offset_set(&consumer_id, &stream, since) {
        eprintln!("ERROR: consumer_offset_set: {e}");
        return ExitCode::from(1);
    }
    println!("OK: offset rewound. Restart cortex-graph-worker to apply.");
    ExitCode::SUCCESS
}
