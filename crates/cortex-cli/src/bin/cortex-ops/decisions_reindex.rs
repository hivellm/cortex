//! `cortex-ops decisions-reindex` — re-emit all `.rulebook/decisions/*.md`
//! into the `cortex_decisions` Meilisearch index using the stable
//! content-addressable doc_id, then prune stale orphan docs whose
//! `title == id` (the malformed-batch signature from early buggy emits).
//!
//! ## Why
//!
//! The `cortex_decisions` index accumulated 51 docs for only 27 actual
//! decision files.  Two root causes:
//!
//! 1. **Duplication** — bootstrap emitted each decision under a random
//!    `event_id`-derived key; re-bootstrapping added NEW docs instead of
//!    overwriting.  Phase22 P1 fixed the live path (stable `bootstrap:`
//!    key), but the stale duplicates remain.
//!
//! 2. **Malformed orphans** — the `01KQNYF4J*` batch produced docs where
//!    `title == id` (the ULID) and `decision_title` was absent.  These
//!    are unrecoverable via normal upsert because the key itself is wrong.
//!
//! This subcommand closes both gaps:
//!
//! - Re-emits every decision file through the production builder pipeline,
//!   producing a stable `bootstrap:<repo>:<path>:<hash>` doc_id.
//!   Meilisearch upserts on that key, so re-running is idempotent.
//!
//! - Scans the live `cortex_decisions` index for every legacy
//!   (non-`bootstrap:`-keyed) doc — stale pre-Phase22 duplicates AND the
//!   malformed `title == id` orphans — and deletes them after the upsert
//!   (or reports them under `--dry-run`). Because every current decision
//!   file is re-emitted first, each pruned decision survives under its
//!   stable content-addressable key (ADR-022 never-delete honoured). The
//!   prune is skipped entirely when zero files were re-emitted, so a
//!   misread decisions dir can never wipe the index.
//!
//! ## Usage
//!
//! ```text
//! # Dry-run: report what would change
//! CORTEX_FULLTEXT_MEILI_URL=http://127.0.0.1:7700 \
//! CORTEX_FULLTEXT_MEILI_API_KEY=<key> \
//!   cortex-ops decisions-reindex --decisions-dir .rulebook/decisions --dry-run
//!
//! # Apply: upsert + prune
//! CORTEX_FULLTEXT_MEILI_URL=http://127.0.0.1:7700 \
//! CORTEX_FULLTEXT_MEILI_API_KEY=<key> \
//!   cortex-ops decisions-reindex --decisions-dir .rulebook/decisions
//! ```
//!
//! Both `CORTEX_FULLTEXT_MEILI_URL` and `CORTEX_FULLTEXT_MEILI_API_KEY`
//! are read from the environment (via `cortex_config::Config::load()`).
//! CLI overrides `--meili-url` / `--meili-key` take precedence.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use cortex_cli::bootstrap::emitter::emit_decision_imported;
use cortex_workers::fulltext::builders::{build_doc, BuildOutcome};
use cortex_workers::fulltext::config::FulltextConfig;

/// Identifier of the global decisions index (matches `cortex_storage::names::INDEX_DECISIONS`).
const INDEX_DECISIONS: &str = "cortex_decisions";

/// Repo identifier stamped on bootstrap events for local decision files.
const LOCAL_REPO: &str = "cortex";

/// Synthetic session id used when emitting bootstrap events from this CLI.
const BOOTSTRAP_SESSION: &str = "01DECISIONS-REINDEX-CLI0000000";

/// Summary of what the subcommand did (or would do under `--dry-run`).
#[derive(Debug)]
struct ReindexReport {
    decisions_dir: PathBuf,
    meili_url: String,
    /// Target Meili index (global `cortex_decisions` or a per-repo one).
    index: String,
    dry_run: bool,
    /// Decision files successfully read and built.
    files_built: usize,
    /// Docs that would be / were upserted.
    docs_upserted: usize,
    /// Files skipped because the builder returned `Skipped` (empty body).
    files_skipped: usize,
    /// Files that could not be read.
    files_error: usize,
    /// Malformed orphan docs found (title == id) — a subset of the
    /// legacy docs below, reported separately for the doctor signal.
    orphans_found: usize,
    /// Legacy (non-`bootstrap:`-keyed) docs found — stale duplicates +
    /// malformed orphans from the pre-Phase22 random-ULID key path.
    legacy_found: usize,
    /// Legacy docs that would be / were deleted.
    legacy_pruned: usize,
    /// Error message when the live backend was unreachable.
    error: Option<String>,
}

/// Entry point for `cortex-ops decisions-reindex`.
pub(super) fn decisions_reindex(
    decisions_dir: Option<String>,
    meili_url: Option<String>,
    meili_key: Option<String>,
    index: Option<String>,
    dry_run: bool,
    json: bool,
) -> ExitCode {
    let target_index = index
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| INDEX_DECISIONS.to_string());
    let cfg = FulltextConfig::from_env();
    let resolved_url = meili_url
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cfg.meili_url.clone());
    let resolved_key = meili_key
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.meili_api_key.clone());

    // Resolve the decisions directory: CLI flag → `--decisions-dir` → cwd-relative default.
    let dir_path: PathBuf = match decisions_dir {
        Some(d) => PathBuf::from(d),
        None => {
            // Default: `.rulebook/decisions` relative to cwd.
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            cwd.join(".rulebook").join("decisions")
        }
    };

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("decisions-reindex: tokio runtime: {e}");
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
    decisions_dir: &Path,
    meili_url: &str,
    meili_key: Option<&str>,
    index: &str,
    dry_run: bool,
) -> ReindexReport {
    let mut report = ReindexReport {
        decisions_dir: decisions_dir.to_path_buf(),
        meili_url: meili_url.to_string(),
        index: index.to_string(),
        dry_run,
        files_built: 0,
        docs_upserted: 0,
        files_skipped: 0,
        files_error: 0,
        orphans_found: 0,
        legacy_found: 0,
        legacy_pruned: 0,
        error: None,
    };

    // Step 1 — collect decision files.
    let files = match collect_decision_files(decisions_dir) {
        Ok(f) => f,
        Err(e) => {
            report.error = Some(format!(
                "cannot read decisions dir {}: {e}",
                decisions_dir.display()
            ));
            return report;
        }
    };

    // Step 2 — build Meili documents from each decision file.
    let mut docs: Vec<serde_json::Value> = Vec::new();
    for file in &files {
        match build_decision_doc(file) {
            Ok(Some(doc_json)) => {
                report.files_built += 1;
                docs.push(doc_json);
            }
            Ok(None) => {
                report.files_skipped += 1;
            }
            Err(e) => {
                eprintln!("decisions-reindex: skip {}: {e}", file.display());
                report.files_error += 1;
            }
        }
    }

    // Step 3 — scan live index for stale legacy docs. Every decision is
    // content-addressable since Phase22 P1 (`bootstrap:<repo>:<path>:<hash>`
    // key), so any doc NOT keyed that way is a pre-fix duplicate or a
    // malformed `title == id` orphan. We capture these BEFORE the upsert
    // (step 4) so the freshly-written bootstrap docs are never in the
    // prune set; re-emitting every current file first guarantees each
    // pruned decision has a canonical replacement (ADR-022 never-delete is
    // honoured — the decision survives under its stable key).
    let scan = match scan_legacy_docs(meili_url, meili_key, index).await {
        Ok(s) => {
            report.orphans_found = s.title_id_count;
            report.legacy_found = s.legacy_ids.len();
            s.legacy_ids
        }
        Err(e) => {
            // Non-fatal when dry-run: we still report what we built.
            eprintln!("decisions-reindex: legacy scan failed: {e}");
            Vec::new()
        }
    };

    report.docs_upserted = docs.len();
    // Only prune legacy docs when we actually re-emitted replacements;
    // an empty/misread decisions dir must never wipe the index.
    let prune_ids: Vec<String> = if docs.is_empty() { Vec::new() } else { scan };
    report.legacy_pruned = prune_ids.len();

    if dry_run {
        // Dry-run: print what would happen, touch nothing.
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

/// Walk `dir` and return paths of every `*.md` file.
fn collect_decision_files(dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
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

/// Build a Meilisearch document JSON value from a single decision file.
/// Returns `None` when the builder skips the file (empty body).
fn build_decision_doc(file: &Path) -> anyhow::Result<Option<serde_json::Value>> {
    let body = std::fs::read_to_string(file)?;
    // rel_path: use the normalized path relative to the repo root so it
    // matches the same shape the bootstrap walker produces.  Since we
    // don't know the repo root from here we derive a portable relative
    // form: just the last two components (`.rulebook/decisions/<stem>.md`).
    let rel_path = repo_rel_path(file);

    let event = emit_decision_imported(
        LOCAL_REPO,
        BOOTSTRAP_SESSION,
        None,
        &rel_path,
        &body,
        "cortex.events.bootstrap",
    );

    // Convert BootstrapEvent → EnrichedEvent (same shape the builder expects).
    let enriched = bootstrap_event_to_enriched(&event);

    // Use 1 MiB body limit — same default as the production worker.
    match build_doc(&enriched, true, 1024 * 1024) {
        BuildOutcome::Ready(doc) => {
            let json = serde_json::to_value(&*doc)?;
            Ok(Some(json))
        }
        BuildOutcome::Skipped => Ok(None),
    }
}

/// Derive a repo-relative path of the form `.rulebook/decisions/<filename>`
/// regardless of whether the absolute path contains Windows separators.
fn repo_rel_path(file: &Path) -> String {
    // Walk up to find the `.rulebook` anchor if present; otherwise fall back
    // to just the filename so the path is still usable as a key.
    let normalized = file.to_string_lossy().replace('\\', "/");
    if let Some(idx) = normalized.rfind("/.rulebook/") {
        return normalized[idx + 1..].to_string(); // trim leading /
    }
    if normalized.contains(".rulebook/") {
        // Already relative-ish.
        if let Some(idx) = normalized.find(".rulebook/") {
            return normalized[idx..].to_string();
        }
    }
    // Fallback: just the filename.
    file.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown.md")
        .to_string()
}

/// Convert a [`cortex_cli::bootstrap::emitter::BootstrapEvent`] into the
/// [`cortex_workers::embedder::EnrichedEvent`] shape the builder expects.
///
/// The conversion is lossless for the fields the builder reads:
/// `event_id`, `kind`, `content_hash`, `redacted_payload`,
/// `context_repo`, `context_path`, `occurred_at_ms`.  The classifier
/// fields are set to the static-fallback defaults that the production
/// worker also uses when the classifier returns no output.
fn bootstrap_event_to_enriched(
    event: &cortex_cli::bootstrap::emitter::BootstrapEvent,
) -> cortex_workers::embedder::EnrichedEvent {
    use cortex_core::events::Kind;
    use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
    use cortex_workers::embedder::EnrichedEvent;

    // Parse the source object for repo / path hints.
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
        sensitivity: Default::default(),
        source: ClassifierSource::StaticFallback,
        prompt_version: "v1".into(),
        model: "static-v1".into(),
        latency_ms: 0,
        tokens_in: 0,
        tokens_out: 0,
    };

    EnrichedEvent {
        event_id: event.event_id.clone(),
        kind: Kind::Decision,
        content_hash: event.content_hash.clone(),
        redacted_payload: event.redacted_payload.clone(),
        classifier,
        context_repo,
        context_path,
        parent_event_id: None,
        session_id: Some(event.session_id.clone()),
        occurred_at_ms: event.ts,
        class_level: None,
        class_compartments: None,
    }
}

/// Result of scanning the live `cortex_decisions` index.
struct LegacyScan {
    /// Ids of every legacy (non-`bootstrap:`-keyed) doc — the prune set.
    legacy_ids: Vec<String>,
    /// How many of those are malformed orphans (`title == id`) — the
    /// doctor signature, reported separately.
    title_id_count: usize,
}

/// Scan the `cortex_decisions` index and classify each doc. Returns the
/// ids of every doc NOT keyed by the content-addressable `bootstrap:`
/// scheme (stale pre-Phase22 duplicates + malformed `title == id`
/// orphans), plus a count of the malformed subset.
///
/// Uses a Meili search with a very large limit to enumerate all docs
/// in the index. The index is small (tens of docs) so this is safe.
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
    // Fetch up to 1000 docs — the index is small.
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
        // Content-addressable docs (the canonical, freshly-emitted form)
        // are kept; everything else is a stale legacy doc.
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
    println!("cortex-ops decisions-reindex");
    println!("decisions_dir: {}", r.decisions_dir.display());
    println!("meili_url:     {}", r.meili_url);
    println!("index:         {}", r.index);
    println!("dry_run:       {}", r.dry_run);
    println!();
    println!("files_built:   {}", r.files_built);
    println!(
        "docs_upserted: {} {}",
        r.docs_upserted,
        if r.dry_run { "(dry-run)" } else { "" }
    );
    println!("files_skipped: {}", r.files_skipped);
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
        "decisions_dir": r.decisions_dir.display().to_string(),
        "meili_url": r.meili_url,
        "index": r.index,
        "dry_run": r.dry_run,
        "files_built": r.files_built,
        "docs_upserted": r.docs_upserted,
        "files_skipped": r.files_skipped,
        "files_error": r.files_error,
        "orphans_found": r.orphans_found,
        "legacy_found": r.legacy_found,
        "legacy_pruned": r.legacy_pruned,
        "error": r.error,
    });
    match serde_json::to_string_pretty(&payload) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("decisions-reindex: serialize report: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repo_rel_path_extracts_rulebook_prefix() {
        let p = PathBuf::from("/home/user/project/.rulebook/decisions/0042-adopt-meili.md");
        assert_eq!(repo_rel_path(&p), ".rulebook/decisions/0042-adopt-meili.md");
    }

    #[test]
    fn repo_rel_path_handles_windows_separator() {
        let p = PathBuf::from(r"C:\project\.rulebook\decisions\0001-init.md");
        let result = repo_rel_path(&p);
        assert!(
            result.ends_with(".rulebook/decisions/0001-init.md"),
            "got: {result}"
        );
    }

    #[test]
    fn repo_rel_path_falls_back_to_filename_when_no_rulebook_anchor() {
        let p = PathBuf::from("/tmp/some-decision.md");
        let result = repo_rel_path(&p);
        assert_eq!(result, "some-decision.md");
    }

    #[test]
    fn build_decision_doc_produces_stable_id_for_same_file_content() {
        // Two calls with the same file content must yield the same doc id
        // (content-addressable, Phase22 P1 contract).
        let dir = tempfile::tempdir().expect("tmpdir");
        let file = dir.path().join(".rulebook").join("decisions");
        std::fs::create_dir_all(&file).expect("mkdir");
        let md = file.join("0042-adopt-meili.md");
        std::fs::write(
            &md,
            "# Adopt Meilisearch\n\nStatus: accepted\n\nBody text.\n",
        )
        .expect("write");

        let doc_a = build_decision_doc(&md)
            .expect("build_a ok")
            .expect("build_a not skipped");
        let doc_b = build_decision_doc(&md)
            .expect("build_b ok")
            .expect("build_b not skipped");

        let id_a = doc_a["id"].as_str().expect("id_a str");
        let id_b = doc_b["id"].as_str().expect("id_b str");
        assert_eq!(id_a, id_b, "same file must produce same doc id");
        assert!(
            id_a.starts_with("bootstrap-"),
            "id must be content-addressable; got {id_a}"
        );
    }

    #[test]
    fn build_decision_doc_title_differs_from_id() {
        // The reindex path must not produce docs where title == id.
        let dir = tempfile::tempdir().expect("tmpdir");
        let base = dir.path().join(".rulebook").join("decisions");
        std::fs::create_dir_all(&base).expect("mkdir");
        let md = base.join("0007-pick-nexus.md");
        std::fs::write(&md, "# Pick Nexus\n\nStatus: accepted\n\nDetails.\n").expect("write");

        let doc = build_decision_doc(&md)
            .expect("build ok")
            .expect("not skipped");

        let id = doc["id"].as_str().expect("id str");
        let title = doc["title"].as_str().expect("title str");
        assert_ne!(title, id, "title must differ from doc primary key");
        assert_eq!(title, "Pick Nexus");
    }
}
