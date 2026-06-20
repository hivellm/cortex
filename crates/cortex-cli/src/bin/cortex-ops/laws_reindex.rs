//! `cortex-ops laws-reindex` — re-emit all `.claude/rules/*.md` law
//! definitions into the global `cortex_laws` Meilisearch index using
//! stable content-addressable doc ids (`bootstrap-<hash>`), then prune
//! any legacy (non-`bootstrap-`-keyed) docs that are stale from the old
//! random-ULID emit path.
//!
//! ## Why
//!
//! The `cortex_laws` index accumulated 240 malformed docs (title == id,
//! body == stringified JSON object) from an old emit path. The new path
//! (`emit_spec_laws_imported` → `build_doc`) produces clean title + body
//! fields and a stable `bootstrap-<hash>` primary key. Re-running is
//! idempotent — Meilisearch upserts on the stable key.
//!
//! Pruning is gated on `files_built > 0`: an empty/misread rules dir
//! never wipes the index.
//!
//! ## Contract
//!
//! `cortex_laws` = law DEFINITIONS only (per
//! `phase0_laws-index-routing-and-malformed-docs`). Law violations stay
//! in the per-repo `cortex-{slug}-governance` index. This subcommand
//! writes only to `cortex_laws`.
//!
//! ## Usage
//!
//! ```text
//! # Dry-run: report what would change
//! CORTEX_FULLTEXT_MEILI_URL=http://127.0.0.1:7700 \
//! CORTEX_FULLTEXT_MEILI_API_KEY=<key> \
//!   cortex-ops laws-reindex --rules-dir .claude/rules --dry-run
//!
//! # Apply: upsert + prune
//! CORTEX_FULLTEXT_MEILI_URL=http://127.0.0.1:7700 \
//! CORTEX_FULLTEXT_MEILI_API_KEY=<key> \
//!   cortex-ops laws-reindex --rules-dir .claude/rules
//!
//! # JSON output (for scripting / CI)
//! CORTEX_FULLTEXT_MEILI_URL=http://127.0.0.1:7700 \
//! CORTEX_FULLTEXT_MEILI_API_KEY=<key> \
//!   cortex-ops laws-reindex --rules-dir .claude/rules --json
//! ```
//!
//! Both `CORTEX_FULLTEXT_MEILI_URL` and `CORTEX_FULLTEXT_MEILI_API_KEY`
//! are read via `cortex_workers::fulltext::config::FulltextConfig::from_env()`.
//! CLI flags `--meili-url` / `--meili-key` take precedence when provided.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cortex_cli::bootstrap::emitter::emit_spec_laws_imported;
use cortex_workers::fulltext::builders::{build_doc, BuildOutcome};
use cortex_workers::fulltext::config::FulltextConfig;

/// Global laws index (matches `cortex_storage::names::INDEX_LAWS`).
const INDEX_LAWS: &str = "cortex_laws";

/// Repo identifier stamped on bootstrap events for local rules files.
const LOCAL_REPO: &str = "cortex";

/// Synthetic session id used when emitting bootstrap events from this CLI.
const BOOTSTRAP_SESSION: &str = "01LAWS-REINDEX-CLI000000000000";

/// Summary of what the subcommand did (or would do under `--dry-run`).
#[derive(Debug)]
struct ReindexReport {
    rules_dir: PathBuf,
    meili_url: String,
    index: String,
    dry_run: bool,
    /// Rule files successfully read.
    files_read: usize,
    /// Law documents successfully built across all files.
    files_built: usize,
    /// Docs that would be / were upserted.
    docs_upserted: usize,
    /// Law events skipped because the builder returned `Skipped`.
    events_skipped: usize,
    /// Files that could not be read or built.
    files_error: usize,
    /// Malformed orphan docs found (title == id) in the live index.
    orphans_found: usize,
    /// Legacy (non-`bootstrap-`-keyed) docs found in the live index.
    legacy_found: usize,
    /// Legacy docs that would be / were deleted.
    legacy_pruned: usize,
    /// Error message when the backend was unreachable or a write failed.
    error: Option<String>,
}

/// Entry point for `cortex-ops laws-reindex`.
pub(super) fn laws_reindex(
    rules_dir: Option<String>,
    meili_url: Option<String>,
    meili_key: Option<String>,
    index: Option<String>,
    dry_run: bool,
    json: bool,
) -> ExitCode {
    let target_index = index
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| INDEX_LAWS.to_string());
    let cfg = FulltextConfig::from_env();
    let resolved_url = meili_url
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cfg.meili_url.clone());
    let resolved_key = meili_key
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.meili_api_key.clone());

    // Resolve the rules directory: CLI flag → cwd-relative default.
    let dir_path: PathBuf = match rules_dir {
        Some(d) => PathBuf::from(d),
        None => {
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            cwd.join(".claude").join("rules")
        }
    };

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("laws-reindex: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };

    let report = rt.block_on(run_reindex(
        &dir_path,
        &resolved_url,
        resolved_key.as_deref(),
        &target_index,
        dry_run,
    ));

    if json {
        print_json(&report);
    } else {
        print_text(&report);
    }

    if report.error.is_some() || report.files_error > 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

/// Core async implementation.
async fn run_reindex(
    rules_dir: &Path,
    meili_url: &str,
    meili_key: Option<&str>,
    index: &str,
    dry_run: bool,
) -> ReindexReport {
    let mut report = ReindexReport {
        rules_dir: rules_dir.to_path_buf(),
        meili_url: meili_url.to_string(),
        index: index.to_string(),
        dry_run,
        files_read: 0,
        files_built: 0,
        docs_upserted: 0,
        events_skipped: 0,
        files_error: 0,
        orphans_found: 0,
        legacy_found: 0,
        legacy_pruned: 0,
        error: None,
    };

    // Step 1 — collect rule files.
    let files = match collect_rules_files(rules_dir) {
        Ok(f) => f,
        Err(e) => {
            report.error = Some(format!(
                "cannot read rules dir {}: {e}",
                rules_dir.display()
            ));
            return report;
        }
    };

    // Step 2 — build Meili documents from each file. Each file may
    // produce multiple law events (one per `## ` section in the file).
    let mut docs: Vec<serde_json::Value> = Vec::new();
    for file in &files {
        let body = match std::fs::read_to_string(file) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("laws-reindex: skip {}: {e}", file.display());
                report.files_error += 1;
                continue;
            }
        };
        report.files_read += 1;
        let rel_path = rules_rel_path(file);
        match build_law_docs(file, &rel_path, &body) {
            Ok((built, skipped)) => {
                report.files_built += built.len();
                report.events_skipped += skipped;
                docs.extend(built);
            }
            Err(e) => {
                eprintln!("laws-reindex: build failed {}: {e}", file.display());
                report.files_error += 1;
            }
        }
    }

    // Step 3 — scan live index for stale legacy docs (before the upsert
    // so fresh bootstrap docs are never in the prune set).
    let scan = match scan_legacy_docs(meili_url, meili_key, index).await {
        Ok(s) => {
            report.orphans_found = s.title_id_count;
            report.legacy_found = s.legacy_ids.len();
            s.legacy_ids
        }
        Err(e) => {
            eprintln!("laws-reindex: legacy scan failed: {e}");
            Vec::new()
        }
    };

    report.docs_upserted = docs.len();
    // Only prune when at least one doc was re-emitted — an empty/misread
    // rules dir must never wipe the index.
    let prune_ids: Vec<String> = if docs.is_empty() { Vec::new() } else { scan };
    report.legacy_pruned = prune_ids.len();

    if dry_run {
        return report;
    }

    // Step 4 — upsert documents (stable bootstrap keys).
    if !docs.is_empty() {
        if let Err(e) = upsert_documents(meili_url, meili_key, index, &docs).await {
            report.error = Some(format!("upsert failed: {e}"));
            return report;
        }
    }

    // Step 5 — delete the legacy docs captured before the upsert.
    if !prune_ids.is_empty() {
        if let Err(e) = delete_documents(meili_url, meili_key, index, &prune_ids).await {
            report.error = Some(format!("legacy prune failed: {e}"));
            return report;
        }
    }

    report
}

/// Walk `dir` and return paths of every `*.md` file, sorted.
fn collect_rules_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
    if !dir.exists() {
        return Err(anyhow::anyhow!(
            "directory does not exist: {}",
            dir.display()
        ));
    }
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
        .collect();
    files.sort();
    Ok(files)
}

/// Emit law events from a single rules file, then build a Meili document
/// for each one.  Returns `(built_docs, skipped_count)`.
///
/// Uses `emit_spec_laws_imported` which fans out one event per `## `
/// section (or one event for the whole file when no sections exist).
fn build_law_docs(
    file: &Path,
    rel_path: &str,
    body: &str,
) -> anyhow::Result<(Vec<serde_json::Value>, usize)> {
    let events = emit_spec_laws_imported(
        LOCAL_REPO,
        BOOTSTRAP_SESSION,
        None,
        rel_path,
        body,
        "cortex.events.bootstrap",
    );

    let mut built: Vec<serde_json::Value> = Vec::new();
    let mut skipped = 0usize;

    for event in events {
        let enriched = bootstrap_event_to_enriched_law(&event);
        // Use 1 MiB body limit — same default as the production worker.
        match build_doc(&enriched, true, 1024 * 1024) {
            BuildOutcome::Ready(doc) => {
                let json = serde_json::to_value(&*doc)
                    .map_err(|e| anyhow::anyhow!("serialize doc from {}: {e}", file.display()))?;
                built.push(json);
            }
            BuildOutcome::Skipped => {
                skipped += 1;
            }
        }
    }

    Ok((built, skipped))
}

/// Derive a repo-relative path of the form `.claude/rules/<filename>`
/// regardless of whether the absolute path contains Windows separators.
fn rules_rel_path(file: &Path) -> String {
    let normalized = file.to_string_lossy().replace('\\', "/");
    // Walk up to find the `.claude/rules` anchor.
    if let Some(idx) = normalized.rfind("/.claude/rules/") {
        return normalized[idx + 1..].to_string();
    }
    if let Some(idx) = normalized.find(".claude/rules/") {
        return normalized[idx..].to_string();
    }
    // Fallback: just the filename.
    file.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown.md")
        .to_string()
}

/// Convert a [`cortex_cli::bootstrap::emitter::BootstrapEvent`] from
/// `law.imported` into the [`cortex_workers::embedder::EnrichedEvent`]
/// shape the fulltext builder expects. Hard-codes `Kind::Law` because
/// every event produced by `emit_spec_laws_imported` /
/// `emit_law_imported` carries the `"law.imported"` discriminator.
fn bootstrap_event_to_enriched_law(
    event: &cortex_cli::bootstrap::emitter::BootstrapEvent,
) -> cortex_workers::embedder::EnrichedEvent {
    use cortex_core::events::Kind;
    use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
    use cortex_workers::embedder::EnrichedEvent;

    let context_repo = event
        .source
        .get("repo")
        .and_then(|v| v.as_str())
        .map(String::from);
    let context_path = event
        .source
        .get("path")
        .and_then(|v| v.as_str())
        .map(String::from);

    let classifier = ClassifierOutput {
        event_id: event.event_id.clone(),
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

    EnrichedEvent {
        event_id: event.event_id.clone(),
        kind: Kind::Law,
        content_hash: event.content_hash.clone(),
        redacted_payload: event.redacted_payload.clone(),
        classifier,
        context_repo,
        context_path,
        parent_event_id: None,
        session_id: Some(event.session_id.clone()),
        occurred_at_ms: event.ts,
    }
}

/// Result of scanning the live `cortex_laws` index.
struct LegacyScan {
    legacy_ids: Vec<String>,
    title_id_count: usize,
}

/// Scan the `cortex_laws` index and return ids of every doc NOT keyed by
/// the content-addressable `bootstrap-` scheme plus a count of the
/// malformed `title == id` subset.
async fn scan_legacy_docs(
    meili_url: &str,
    meili_key: Option<&str>,
    index: &str,
) -> anyhow::Result<LegacyScan> {
    let client = build_http_client(meili_key)?;
    let url = format!(
        "{}/indexes/{}/search",
        meili_url.trim_end_matches('/'),
        index,
    );
    let body = serde_json::json!({
        "q": "",
        "limit": 1000,
        "attributesToRetrieve": ["id", "title"],
    });
    let resp = client.post(&url).json(&body).send().await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "meili search HTTP {status}: {}",
            text.chars().take(200).collect::<String>()
        ));
    }
    let payload: serde_json::Value = resp.json().await?;
    let hits = payload
        .get("hits")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);

    let mut legacy_ids: Vec<String> = Vec::new();
    let mut title_id_count = 0usize;
    for hit in hits {
        let Some(id) = hit.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        if id.starts_with("bootstrap-") {
            continue;
        }
        if hit.get("title").and_then(|v| v.as_str()) == Some(id) {
            title_id_count += 1;
        }
        legacy_ids.push(id.to_string());
    }

    Ok(LegacyScan {
        legacy_ids,
        title_id_count,
    })
}

/// POST documents to Meili `addOrReplaceDocuments` (upsert by primary key).
async fn upsert_documents(
    meili_url: &str,
    meili_key: Option<&str>,
    index: &str,
    docs: &[serde_json::Value],
) -> anyhow::Result<()> {
    let client = build_http_client(meili_key)?;
    let url = format!(
        "{}/indexes/{}/documents?primaryKey=id",
        meili_url.trim_end_matches('/'),
        index,
    );
    let resp = client.post(&url).json(docs).send().await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "meili upsert HTTP {status}: {}",
            text.chars().take(400).collect::<String>()
        ));
    }
    Ok(())
}

/// DELETE documents by id via Meili `deleteDocuments` (POST with id array).
async fn delete_documents(
    meili_url: &str,
    meili_key: Option<&str>,
    index: &str,
    ids: &[String],
) -> anyhow::Result<()> {
    let client = build_http_client(meili_key)?;
    let url = format!(
        "{}/indexes/{}/documents/delete-batch",
        meili_url.trim_end_matches('/'),
        index,
    );
    let resp = client.post(&url).json(ids).send().await?;
    if !resp.status().is_success() {
        let status = resp.status().as_u16();
        let text = resp.text().await.unwrap_or_default();
        return Err(anyhow::anyhow!(
            "meili delete-batch HTTP {status}: {}",
            text.chars().take(400).collect::<String>()
        ));
    }
    Ok(())
}

/// Build a `reqwest::Client` with the optional bearer token.
fn build_http_client(api_key: Option<&str>) -> anyhow::Result<reqwest::Client> {
    use reqwest::header::{HeaderMap, HeaderValue, AUTHORIZATION, CONTENT_TYPE};
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    if let Some(key) = api_key {
        let bearer = format!("Bearer {key}");
        let val = HeaderValue::from_str(&bearer)
            .map_err(|e| anyhow::anyhow!("invalid api key header: {e}"))?;
        headers.insert(AUTHORIZATION, val);
    }
    reqwest::Client::builder()
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| anyhow::anyhow!("reqwest client: {e}"))
}

fn print_text(r: &ReindexReport) {
    println!("cortex-ops laws-reindex");
    println!("rules_dir:     {}", r.rules_dir.display());
    println!("meili_url:     {}", r.meili_url);
    println!("index:         {}", r.index);
    println!("dry_run:       {}", r.dry_run);
    println!();
    println!("files_read:    {}", r.files_read);
    println!("files_built:   {}", r.files_built);
    println!(
        "docs_upserted: {} {}",
        r.docs_upserted,
        if r.dry_run { "(dry-run)" } else { "" }
    );
    println!("events_skipped:{}", r.events_skipped);
    println!("files_error:   {}", r.files_error);
    println!();
    println!(
        "legacy_found:   {} (non-bootstrap-keyed; incl. {} malformed title==id)",
        r.legacy_found, r.orphans_found
    );
    println!(
        "legacy_pruned:  {} {}",
        r.legacy_pruned,
        if r.dry_run { "(dry-run)" } else { "" }
    );
    if let Some(err) = &r.error {
        println!();
        println!("ERROR: {err}");
    }
}

fn print_json(r: &ReindexReport) {
    let payload = serde_json::json!({
        "rules_dir": r.rules_dir.display().to_string(),
        "meili_url": r.meili_url,
        "index": r.index,
        "dry_run": r.dry_run,
        "files_read": r.files_read,
        "files_built": r.files_built,
        "docs_upserted": r.docs_upserted,
        "events_skipped": r.events_skipped,
        "files_error": r.files_error,
        "orphans_found": r.orphans_found,
        "legacy_found": r.legacy_found,
        "legacy_pruned": r.legacy_pruned,
        "error": r.error,
    });
    match serde_json::to_string_pretty(&payload) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("laws-reindex: serialize report: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_rel_path_extracts_claude_rules_prefix() {
        let p = PathBuf::from("/home/user/project/.claude/rules/no-shortcuts.md");
        assert_eq!(rules_rel_path(&p), ".claude/rules/no-shortcuts.md");
    }

    #[test]
    fn rules_rel_path_handles_windows_separator() {
        let p = PathBuf::from(r"C:\project\.claude\rules\git-safety.md");
        let result = rules_rel_path(&p);
        assert!(
            result.ends_with(".claude/rules/git-safety.md"),
            "got: {result}"
        );
    }

    #[test]
    fn rules_rel_path_falls_back_to_filename() {
        let p = PathBuf::from("/tmp/some-rule.md");
        assert_eq!(rules_rel_path(&p), "some-rule.md");
    }

    #[test]
    fn build_law_docs_produces_stable_bootstrap_id() {
        // Same file content must yield the same doc ids across calls
        // (content-addressable contract — mirrors decisions_reindex).
        let dir = tempfile::tempdir().expect("tmpdir");
        let rules_dir = dir.path().join(".claude").join("rules");
        std::fs::create_dir_all(&rules_dir).expect("mkdir");
        let md = rules_dir.join("no-shortcuts.md");
        std::fs::write(
            &md,
            "# No Shortcuts\n\n## Rule 1\n\nNever use stubs or TODOs.\n\n## Rule 2\n\nImplement completely.\n",
        )
        .expect("write");

        let rel = rules_rel_path(&md);
        let body = std::fs::read_to_string(&md).expect("read");
        let (docs_a, _) = build_law_docs(&md, &rel, &body).expect("build_a");
        let (docs_b, _) = build_law_docs(&md, &rel, &body).expect("build_b");

        assert!(!docs_a.is_empty(), "must produce at least one law doc");
        assert_eq!(docs_a.len(), docs_b.len(), "same file must yield same doc count");
        for (a, b) in docs_a.iter().zip(docs_b.iter()) {
            let id_a = a["id"].as_str().expect("id_a");
            let id_b = b["id"].as_str().expect("id_b");
            assert_eq!(id_a, id_b, "same content must produce same doc id");
            assert!(
                id_a.starts_with("bootstrap-"),
                "id must be content-addressable; got {id_a}"
            );
        }
    }

    #[test]
    fn build_law_docs_title_differs_from_id_and_body_is_not_json_object() {
        // phase0_laws-index-routing-and-malformed-docs contract:
        // 1. title must differ from the doc primary key (not the ULID / id).
        // 2. body must be plain text — NOT a stringified JSON object.
        let dir = tempfile::tempdir().expect("tmpdir");
        let rules_dir = dir.path().join(".claude").join("rules");
        std::fs::create_dir_all(&rules_dir).expect("mkdir");
        let md = rules_dir.join("research-first.md");
        std::fs::write(
            &md,
            "# Research First\n\n## Always research before implementing\n\nNever guess at bug causes.\n",
        )
        .expect("write");

        let rel = rules_rel_path(&md);
        let body = std::fs::read_to_string(&md).expect("read");
        let (docs, _) = build_law_docs(&md, &rel, &body).expect("build");

        assert!(!docs.is_empty(), "must produce at least one law doc");
        for doc in &docs {
            let id = doc["id"].as_str().expect("id str");
            let title = doc["title"].as_str().expect("title str");
            let body_val = doc["body"].as_str().expect("body str");

            assert_ne!(title, id, "title must differ from doc primary key");
            // Body must not be a stringified JSON object (the old malformed path
            // set body to the entire payload serialised as a JSON string).
            assert!(
                !body_val.trim_start().starts_with('{'),
                "body must not be a stringified JSON object; got: {body_val:.80}"
            );
        }
    }
}
