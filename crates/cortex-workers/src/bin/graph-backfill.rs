//! `cortex-graph-backfill` — replay archived envelopes through the live
//! graph writer.
//!
//! The streaming worker consumes from Synap, but a fresh dev stack only
//! has a Synap that's been fed by recent activity — historical events
//! captured into `~/.cortex/archive/events/**/raw-*.parquet` are zstd
//! NDJSON files of canonical [`cortex_core::events::Envelope`]s that
//! never reach the consumer. This binary walks that archive, maps each
//! envelope to a [`GraphPatch`] via the production mapper, and writes
//! the patches to Nexus through the same [`NexusGraphWriter`] the live
//! worker uses. End result: the graph explorer populates with the
//! HAS_TURN / HAS_TOOL_CALL / TOUCHED / OBSERVED_IN / OF / SUPERSEDES
//! edges that exist in the captured stream but were never persisted.
//!
//! Idempotent — every node uses its natural key and every edge MERGEs.
//! Re-running against the same archive on a populated Nexus is a no-op.

use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result};
use clap::Parser;
use cortex_classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_core::events::Envelope;
use cortex_workers::embedder::EnrichedEvent;
use cortex_workers::graph::{
    cypher::{load_from_dir, REQUIRED_TEMPLATES},
    schema, GraphClient, GraphConfig, GraphWriter, LiveNexusClient, Metrics, NexusGraphWriter,
};
use tracing_subscriber::{fmt, EnvFilter};

#[derive(Parser, Debug)]
#[command(
    name = "cortex-graph-backfill",
    about = "Replay archived envelopes through the graph writer (idempotent)."
)]
struct Cli {
    /// Archive root. Defaults to `$CORTEX_ARCHIVE_ROOT` or
    /// `~/.cortex/archive` when neither flag nor env is set.
    #[arg(long)]
    archive_root: Option<PathBuf>,
    /// Stop after replaying this many envelopes. Mostly a dev knob.
    #[arg(long)]
    limit: Option<usize>,
    /// Patch batch size — one Cypher transaction per batch. Matches
    /// the live worker's default of 256.
    #[arg(long, default_value_t = 256)]
    batch_size: usize,
    /// Verbose tracing.
    #[arg(long)]
    verbose: bool,
    /// Comma-separated list of `Kind`s to skip when replaying. Useful
    /// when one kind's payload trips a Cypher escape bug and the rest
    /// of the archive is still worth landing.
    #[arg(long, default_value = "")]
    skip_kinds: String,
    /// Render the first event's patch and print every Cypher statement
    /// the writer would send, then send them one by one and report
    /// status. Useful for isolating which clause Nexus rejects.
    #[arg(long)]
    dump_first_patch: bool,
    /// Apply the schema-bootstrap statements (constraints + indexes
    /// from `cortex_workers::graph::schema::SCHEMA_STATEMENTS`) and exit. The
    /// phase4e backfill runbook calls this before replay so a missing
    /// `symbol_natural_key` constraint surfaces in one line instead of
    /// after the full scan.
    #[arg(long)]
    ensure_schema_only: bool,
    /// Run a single Cypher statement and print the [`QueryResult`]
    /// (columns + rows) as one JSON object on stdout, then exit. Used
    /// by the runbook's verification probes — keeps the probe surface
    /// in lockstep with the writer's transport so a working backfill
    /// implies a working probe.
    #[arg(long, value_name = "CYPHER")]
    probe: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let level = if cli.verbose { "debug" } else { "info" };
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(format!("{level},cortex_workers={level}")));
    fmt().with_env_filter(filter).with_target(false).init();

    let archive_root = resolve_archive_root(cli.archive_root.clone())?;
    if !archive_root.is_dir() {
        anyhow::bail!("archive root not found: {}", archive_root.display());
    }
    tracing::info!(archive_root = %archive_root.display(), "scanning archive");

    let config = GraphConfig::from_env();
    tracing::info!(
        nexus_url = %config.nexus_url,
        batch_size = cli.batch_size,
        "wiring graph writer"
    );

    // The Cypher template registry is loaded for parity with the live
    // worker's startup contract (per spec 07 §Compat note the 1.15
    // writer renders inline-literal Cypher; the registry remains the
    // future-state shape and `ensure_required` enforces the surface
    // hasn't drifted).
    let cypher_dir = std::env::var("CORTEX_GRAPH_CYPHER_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("crates/cortex-graph/cypher"));
    let templates = load_from_dir(&cypher_dir)
        .with_context(|| format!("load cypher templates from {:?}", cypher_dir))?;
    templates
        .ensure_required(REQUIRED_TEMPLATES)
        .with_context(|| format!("required cypher templates missing under {:?}", cypher_dir))?;
    let templates = Arc::new(templates);

    let client = Arc::new(
        LiveNexusClient::new(config.clone())
            .map_err(|e| anyhow::anyhow!("nexus client build: {e}"))?,
    );
    // Smoke-probe before consuming the archive so a bad Nexus state
    // (auth, transport mismatch, dropped connection) surfaces in one
    // line instead of after a 30-minute scan.
    match client.execute_with_retry("RETURN 1 AS smoke", None).await {
        Ok(r) => tracing::info!(rows = r.rows.len(), "nexus smoke probe ok"),
        Err(e) => {
            return Err(anyhow::anyhow!(
                "nexus smoke probe failed (writer cannot proceed): {e}"
            ));
        }
    }

    // Phase4e runbook hooks. Both modes short-circuit before writer
    // setup so a wrong archive root or missing template directory
    // doesn't matter when the operator only wants to bootstrap or
    // probe.
    if cli.ensure_schema_only {
        let stmts = schema::statements();
        client
            .ensure_schema(&stmts)
            .await
            .map_err(|e| anyhow::anyhow!("ensure_schema failed: {e}"))?;
        println!(
            "schema bootstrap ok ({} statements applied or already present)",
            stmts.len()
        );
        return Ok(());
    }
    if let Some(cypher) = cli.probe.as_deref() {
        let result = client
            .execute_with_retry(cypher, None)
            .await
            .map_err(|e| anyhow::anyhow!("probe failed: {e}"))?;
        let payload = serde_json::json!({
            "columns": result.columns,
            "rows": result.rows,
            "execution_time_ms": result.execution_time_ms,
        });
        println!("{}", serde_json::to_string(&payload)?);
        return Ok(());
    }

    let metrics = Arc::new(Metrics::new());
    let writer = Arc::new(NexusGraphWriter::new(
        config.clone(),
        client.clone(),
        templates,
        metrics,
    ));

    let skip_kinds: std::collections::HashSet<String> = cli
        .skip_kinds
        .split(',')
        .filter_map(|s| {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_ascii_lowercase())
            }
        })
        .collect();
    if !skip_kinds.is_empty() {
        tracing::info!(?skip_kinds, "kind filter active");
    }

    let mut scan = ScanState {
        files: 0,
        envelopes: 0,
        dropped: 0,
        events: Vec::new(),
        limit: cli.limit,
        skip_kinds,
    };
    walk_archive(&archive_root, &mut scan);

    // Trim oversized payloads so the rendered Cypher stays under
    // Nexus 1.15's per-statement size limit (`reqwest` returns
    // "http error sending request" when the body is too large; the
    // graph mapper only reads a handful of payload fields, so 8 KB
    // is well above what any single MERGE statement actually needs).
    const MAX_PAYLOAD_BYTES: usize = 8 * 1024;
    let mut trimmed = 0usize;
    for ev in &mut scan.events {
        if let Ok(serialized) = serde_json::to_string(&ev.redacted_payload) {
            if serialized.len() > MAX_PAYLOAD_BYTES {
                ev.redacted_payload = trim_payload(&ev.redacted_payload, MAX_PAYLOAD_BYTES);
                trimmed += 1;
            }
        }
    }
    if trimmed > 0 {
        tracing::info!(
            trimmed,
            "redacted_payload trimmed to fit Cypher size budget"
        );
    }

    tracing::info!(
        files = scan.files,
        envelopes = scan.envelopes,
        dropped = scan.dropped,
        events_to_write = scan.events.len(),
        "scan complete; starting writes"
    );

    if cli.dump_first_patch {
        if let Some(ev) = scan.events.first() {
            let patch = cortex_workers::graph::map_event_to_patch(ev);
            println!("=== first event ===");
            println!("event_id: {}", ev.event_id);
            println!("kind: {:?}", ev.kind);
            println!("session_id: {:?}", ev.session_id);
            println!(
                "payload bytes: {}",
                serde_json::to_string(&ev.redacted_payload)
                    .map(|s| s.len())
                    .unwrap_or(0)
            );
            println!(
                "=== patch: {} nodes, {} edges ===",
                patch.nodes.len(),
                patch.edges.len()
            );
            for (i, n) in patch.nodes.iter().enumerate() {
                println!(
                    "  node[{i}] label={} key_len={} props={}",
                    n.label,
                    n.natural_key.len(),
                    n.props.len()
                );
                let cy = render_node_merge_for_print(n);
                println!("    cypher_len={} bytes", cy.len());
                println!("    cypher: {cy}");
            }
            for (i, e) in patch.edges.iter().enumerate() {
                println!(
                    "  edge[{i}] {} {}->{}",
                    e.edge_type, e.from_label, e.to_label
                );
            }
            println!("=== sending each node statement individually ===");
            for (i, n) in patch.nodes.iter().enumerate() {
                let cy = render_node_merge_for_print(n);
                match client.execute_with_retry(&cy, None).await {
                    Ok(r) => println!("  node[{i}] OK rows={}", r.rows.len()),
                    Err(e) => {
                        println!("  node[{i}] ERR {e}");
                        println!(
                            "    cypher head: {}",
                            &cy.chars().take(400).collect::<String>()
                        );
                    }
                }
            }
        }
        return Ok(());
    }

    let mut total_nodes: u32 = 0;
    let mut total_edges: u32 = 0;
    let mut chunk_idx = 0usize;
    let mut total_dropped_chunks: usize = 0;
    let mut last_err: Option<String> = None;
    for chunk in scan.events.chunks(cli.batch_size) {
        let report = match writer.write_batch(chunk).await {
            Ok(r) => r,
            Err(e) => {
                let example_kind = chunk
                    .first()
                    .map(|ev| format!("{:?}", ev.kind))
                    .unwrap_or_default();
                let example_id = chunk
                    .first()
                    .map(|ev| ev.event_id.clone())
                    .unwrap_or_default();
                tracing::warn!(
                    chunk = chunk_idx + 1,
                    events = chunk.len(),
                    example_kind = %example_kind,
                    example_id = %example_id,
                    error = %e,
                    "batch failed; continuing with the next batch"
                );
                last_err = Some(e.to_string());
                total_dropped_chunks += 1;
                chunk_idx += 1;
                continue;
            }
        };
        total_nodes = total_nodes.saturating_add(report.nodes_upserted);
        total_edges = total_edges.saturating_add(report.edges_upserted);
        chunk_idx += 1;
        tracing::info!(
            chunk = chunk_idx,
            events = chunk.len(),
            nodes = report.nodes_upserted,
            edges = report.edges_upserted,
            ms = report.latency_ms,
            "batch flushed"
        );
    }

    tracing::info!(
        chunks = chunk_idx,
        chunks_dropped = total_dropped_chunks,
        total_events = scan.events.len(),
        nodes_upserted = total_nodes,
        edges_upserted = total_edges,
        last_error = %last_err.as_deref().unwrap_or(""),
        "backfill done"
    );
    Ok(())
}

struct ScanState {
    files: usize,
    envelopes: usize,
    dropped: usize,
    events: Vec<EnrichedEvent>,
    limit: Option<usize>,
    skip_kinds: std::collections::HashSet<String>,
}

fn kind_label(k: cortex_core::events::Kind) -> &'static str {
    use cortex_core::events::Kind;
    match k {
        Kind::Turn => "turn",
        Kind::ToolCall => "tool_call",
        Kind::AgentCall => "agent_call",
        Kind::Memory => "memory",
        Kind::Decision => "decision",
        Kind::Analysis => "analysis",
        Kind::LawViolation => "law_violation",
        Kind::Artifact => "artifact",
        Kind::Knowledge => "knowledge",
        Kind::Learning => "learning",
        Kind::Consolidation => "consolidation",
    }
}

fn walk_archive(dir: &Path, scan: &mut ScanState) {
    if let Some(l) = scan.limit {
        if scan.events.len() >= l {
            return;
        }
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) => {
            tracing::warn!(dir = %dir.display(), error = %e, "read_dir failed; skipping");
            return;
        }
    };
    for entry in entries.flatten() {
        if let Some(l) = scan.limit {
            if scan.events.len() >= l {
                return;
            }
        }
        let path = entry.path();
        if path.is_dir() {
            walk_archive(&path, scan);
            continue;
        }
        // Archive files use the `.parquet` extension by historical
        // accident — the bytes are zstd-compressed line-delimited JSON.
        if path.extension().and_then(|s| s.to_str()) != Some("parquet") {
            continue;
        }
        scan.files += 1;
        if let Err(e) = read_one_file(&path, scan) {
            tracing::debug!(
                path = %path.display(),
                error = %e,
                "partial frame (live file or trailing corruption)"
            );
        }
    }
}

fn read_one_file(path: &Path, scan: &mut ScanState) -> std::io::Result<()> {
    let file = File::open(path)?;
    let decoder = zstd::stream::read::Decoder::new(file)?;
    let reader = BufReader::new(decoder);
    for line in reader.lines() {
        if let Some(l) = scan.limit {
            if scan.events.len() >= l {
                return Ok(());
            }
        }
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<Envelope>(trimmed) {
            Ok(env) => {
                scan.envelopes += 1;
                if scan.skip_kinds.contains(kind_label(env.kind)) {
                    continue;
                }
                scan.events.push(envelope_to_enriched(env));
            }
            Err(_) => scan.dropped += 1,
        }
    }
    Ok(())
}

/// Lift a canonical [`Envelope`] to an [`EnrichedEvent`] using the same
/// extraction rules as
/// `cortex_workers::classifier::worker::ClassifiedEvent::from_canonical_envelope`.
/// The graph mapper does not consume any [`ClassifierOutput`] field, so
/// the classifier slot is filled with the static-fallback descriptor —
/// the same record `cortex-classifier::statics::StaticClassifier` emits
/// when the budget halts. Marking it `backfill-v1` lets downstream
/// audits trace any node back to its replay run.
fn envelope_to_enriched(env: Envelope) -> EnrichedEvent {
    let event_id = env.event_id.clone();
    let context_path = env
        .context
        .extras
        .get("path")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let session_id = if env.session_id.is_empty() {
        None
    } else {
        Some(env.session_id.clone())
    };
    EnrichedEvent {
        event_id: event_id.clone(),
        kind: env.kind,
        content_hash: env.content_hash,
        redacted_payload: env.payload,
        classifier: ClassifierOutput {
            event_id,
            kind_refinement: None,
            topics: Vec::new(),
            severity: Severity::Info,
            pii_risk: PiiRisk::Low,
            redaction_suggestions: Vec::new(),
            summary: None,
            // Backfill replays archived envelopes through the
            // structural mapper only; Sonnet-driven entities +
            // relations come from the live classifier path, never
            // from the backfill (we don't want to burn tokens
            // re-classifying historical data here).
            entities: Vec::new(),
            relations: Vec::new(),
            source: ClassifierSource::StaticFallback,
            prompt_version: "backfill-v1".into(),
            model: "backfill-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        },
        context_repo: env.context.repo,
        context_path,
        parent_event_id: env.parent_event_id,
        session_id,
    }
}

/// Reduce a `redacted_payload` value so its serialized form fits the
/// per-statement budget. Strategy: keep the structure (so the mapper's
/// `serde_json::from_value::<TurnPayload>()` style lookups still
/// resolve their named fields), but truncate every string leaf to
/// 256 chars and drop array elements past index 16. The resulting
/// JSON is still parseable as the per-kind payload structs and the
/// mapper only consumes labelable fields (`tool_name`, `outcome`,
/// `touched[].path`, `user_message`) — all of which live in the
/// shallow head of the payload.
fn trim_payload(v: &serde_json::Value, _budget: usize) -> serde_json::Value {
    fn shrink(v: &serde_json::Value) -> serde_json::Value {
        match v {
            serde_json::Value::String(s) => {
                if s.len() > 256 {
                    let mut out = String::with_capacity(260);
                    for (i, ch) in s.chars().enumerate() {
                        if i >= 256 {
                            break;
                        }
                        out.push(ch);
                    }
                    serde_json::Value::String(out)
                } else {
                    serde_json::Value::String(s.clone())
                }
            }
            serde_json::Value::Array(items) => {
                let cap = items.len().min(16);
                serde_json::Value::Array(items.iter().take(cap).map(shrink).collect())
            }
            serde_json::Value::Object(map) => {
                let mut out = serde_json::Map::with_capacity(map.len());
                for (k, child) in map {
                    out.insert(k.clone(), shrink(child));
                }
                serde_json::Value::Object(out)
            }
            other => other.clone(),
        }
    }
    shrink(v)
}

/// Mirror of `render_node_merge` from `nexus_client.rs` — duplicated
/// only so the backfill's `--dump-first-patch` mode can show the exact
/// Cypher the writer sends without exposing the private helper. Stays
/// in lockstep with `cypher_string_literal` (Cypher-canonical escapes,
/// UTF-8 passes through verbatim) so the dump matches the live wire.
fn render_node_merge_for_print(node: &cortex_workers::graph::NodeOp) -> String {
    let key_field = match node.label.as_str() {
        "Artifact" => "natural_key",
        "Repo" => "name",
        _ => "id",
    };
    let mut cy = format!(
        "MERGE (n:{label} {{ {kf}: {key} }})",
        label = node.label,
        kf = key_field,
        key = cypher_str(&node.natural_key),
    );
    if !node.props.is_empty() {
        cy.push_str(" SET ");
        let mut first = true;
        for (k, v) in &node.props {
            if !first {
                cy.push_str(", ");
            }
            first = false;
            let lit = match v {
                serde_json::Value::Null => "null".to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::String(s) => cypher_str(s),
                _ => cypher_str(&serde_json::to_string(v).unwrap_or_default()),
            };
            cy.push_str(&format!("n.{k} = {lit}"));
        }
    }
    cy.push_str(" RETURN count(n) AS written");
    cy
}

/// Cypher-canonical string literal — same rules as
/// `nexus_client::cypher_string_literal`. Duplicated locally so the
/// backfill's dump renderer stays accurate without exposing the
/// writer-internal helper. ASCII-only output (Nexus 1.15's JSON body
/// parser rejects codepoints `>= 0x80`, so non-ASCII is transliterated
/// to `?` and `…` becomes `...`).
fn cypher_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push(' '),
            '\u{2026}' => out.push_str("..."),
            c if (c as u32) < 0x80 => out.push(c),
            _ => out.push('?'),
        }
    }
    out.push('"');
    out
}

fn resolve_archive_root(cli_arg: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(p) = cli_arg {
        return Ok(p);
    }
    if let Ok(env) = std::env::var("CORTEX_ARCHIVE_ROOT") {
        return Ok(PathBuf::from(env));
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .context("no --archive-root and no USERPROFILE/HOME")?;
    Ok(PathBuf::from(home).join(".cortex").join("archive"))
}
