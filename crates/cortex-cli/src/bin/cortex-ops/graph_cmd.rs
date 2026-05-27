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

/// Phase15b §3.3 — `cortex-ops graph backfill --since <RFC3339>`
/// dispatcher. Walks the archive, projects every envelope newer
/// than `--since` through `cortex_workers::graph::projection::project_envelope`,
/// and reports the per-edge-kind count. `--dry-run` (default)
/// prints the summary without touching Nexus; the live-Nexus
/// writeback lands behind a follow-up commit that threads the
/// `GraphWriter` chain through (today the archive walk lacks
/// the classifier output that the §2.1-§2.4/§2.8/§2.11/§2.12
/// extractors need, so the §3.3 commit ships the count-only
/// surface to unblock the §4.1 doctor wiring; payload-driven
/// extractors — SUPERSEDES / CONTRADICTS / EMITTED_BY /
/// ANSWERED_BY / CITES-body-regex — produce useful counts even
/// without a classifier replay).
pub(super) fn graph_backfill(
    since: Option<String>,
    archive_root: Option<String>,
    json: bool,
) -> ExitCode {
    use cortex_core::events::Envelope;
    use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
    use cortex_workers::embedder::EnrichedEvent;
    use cortex_workers::graph::projection::{project_envelope, registered_edge_kinds};
    use std::path::PathBuf;

    let archive_root: PathBuf = archive_root
        .or_else(|| {
            cortex_config::Config::load()
                .ok()
                .and_then(|c| c.ingestion.archive_root)
        })
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // Fallback: <HOME>/.cortex/archive via $HOME / $USERPROFILE.
            let home = std::env::var("HOME")
                .or_else(|_| std::env::var("USERPROFILE"))
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".cortex").join("archive")
        });

    if !archive_root.exists() {
        eprintln!(
            "ERROR: archive root does not exist: {}",
            archive_root.display()
        );
        return ExitCode::from(1);
    }

    let since_filter = since.clone();
    let envelopes =
        match cortex_storage::archive::walk_envelopes(&archive_root, |env| match &since_filter {
            Some(cursor) => env.occurred_at.as_str() >= cursor.as_str(),
            None => true,
        }) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("ERROR: walk archive {}: {e}", archive_root.display());
                return ExitCode::from(1);
            }
        };

    let ctx = cortex_workers::graph::extractors::ExtractCtx::new("phase15b-backfill-v1");
    let mut per_kind: std::collections::BTreeMap<String, u64> = std::collections::BTreeMap::new();
    for k in registered_edge_kinds() {
        per_kind.insert(k.to_string(), 0);
    }
    let mut envelopes_walked: u64 = 0;
    let mut edges_total: u64 = 0;

    for env in &envelopes {
        envelopes_walked += 1;
        // Build a classifier-less EnrichedEvent. Payload-driven
        // extractors still produce edges; classifier-driven ones
        // emit nothing (documented scope cut).
        let static_classifier = ClassifierOutput {
            event_id: env.event_id.clone(),
            kind_refinement: None,
            topics: Vec::new(),
            severity: Severity::Info,
            pii_risk: PiiRisk::Low,
            redaction_suggestions: Vec::new(),
            summary: None,
            entities: Vec::new(),
            relations: Vec::new(),
            source: ClassifierSource::StaticFallback,
            prompt_version: "v1".into(),
            model: "static-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        };
        let enriched = EnrichedEvent {
            event_id: env.event_id.clone(),
            kind: env.kind,
            content_hash: env.content_hash.clone(),
            redacted_payload: env.payload.clone(),
            classifier: static_classifier,
            context_repo: env.context.repo.clone(),
            // Envelope context has no `path` field — use cwd as a
            // best-effort fallback; most extractors don't read it.
            context_path: env.context.cwd.clone(),
            parent_event_id: env.parent_event_id.clone(),
            session_id: Some(env.session_id.clone()),
        };
        let patch = project_envelope(&enriched, &ctx);
        for edge in &patch.edges {
            *per_kind.entry(edge.edge_type.clone()).or_insert(0) += 1;
            edges_total += 1;
        }
    }

    if json {
        let payload = serde_json::json!({
            "mode": "dry-run",
            "archive_root": archive_root.display().to_string(),
            "since": since,
            "envelopes_walked": envelopes_walked,
            "edges_total": edges_total,
            "by_kind": per_kind,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else {
        println!("cortex-ops graph backfill (dry-run)");
        println!("  archive_root      = {}", archive_root.display());
        println!(
            "  since             = {}",
            since.as_deref().unwrap_or("<unset>")
        );
        println!("  envelopes_walked  = {envelopes_walked}");
        println!("  edges_total       = {edges_total}");
        println!("  per_kind:");
        for (k, v) in &per_kind {
            println!("    {k:<14} = {v}");
        }
    }
    // Unused for now — Envelope import retained for downstream
    // wiring (suppress dead-code warning via a no-op cast).
    let _: Option<Envelope> = None;
    ExitCode::SUCCESS
}
