//! `cortex-ops laws-repair` — repair malformed `cortex_laws` docs IN
//! PLACE from their own embedded payload (no source re-walk).
//!
//! ## Why
//!
//! The historical `cortex_laws` docs are malformed: `title == id` (a
//! random ULID) and `body` = the STRINGIFIED original law payload
//! (`{"law_id":…,"title":…,"body":"<markdown>","severity":…}`). The
//! real content is therefore recoverable from each doc itself — no need
//! to re-walk `.claude/rules` + `docs/specs` + AGENTS (the three sources
//! that populate `cortex_laws`), which a source-driven reindex would have
//! to cover all of to avoid deleting laws it does not re-emit.
//!
//! For every legacy (non-`bootstrap-`-keyed) doc whose `body` parses as a
//! law payload, this command:
//!
//! 1. Reconstructs an `EnrichedEvent` (`Kind::Law`, `redacted_payload` =
//!    the unwrapped payload, `repo`/`path`/`content_hash` from the doc).
//! 2. Runs it through the production `build_doc` `Kind::Law` arm → a clean
//!    `Document` (derived title, prose body, stable `bootstrap-` id).
//! 3. Upserts the clean doc, then deletes the old ULID-keyed doc.
//!
//! Docs already `bootstrap-`-keyed, or whose body is not a parseable law
//! payload, or missing `repo`/`path`/`content_hash`, are left untouched.
//! Idempotent: a second run finds nothing to repair.
//!
//! ## Usage
//!
//! ```text
//! CORTEX_FULLTEXT_MEILI_URL=http://127.0.0.1:7700 \
//! CORTEX_FULLTEXT_MEILI_API_KEY=<key> \
//!   cortex-ops laws-repair --dry-run
//! ```

use std::process::ExitCode;

use cortex_workers::fulltext::builders::{build_doc, BuildOutcome};
use cortex_workers::fulltext::config::FulltextConfig;

/// Global laws index (matches `cortex_storage::names::INDEX_LAWS`).
const INDEX_LAWS: &str = "cortex_laws";

#[derive(Debug)]
struct RepairReport {
    index: String,
    meili_url: String,
    dry_run: bool,
    total: usize,
    already_keyed: usize,
    repaired: usize,
    old_deleted: usize,
    skipped_unparseable: usize,
    skipped_no_identity: usize,
    error: Option<String>,
}

/// Entry point for `cortex-ops laws-repair`.
pub(super) fn laws_repair(
    meili_url: Option<String>,
    meili_key: Option<String>,
    index: Option<String>,
    dry_run: bool,
    json: bool,
) -> ExitCode {
    let cfg = FulltextConfig::from_env();
    let url = meili_url
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cfg.meili_url.clone());
    let key = meili_key
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.meili_api_key.clone());
    let target = index
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| INDEX_LAWS.to_string());

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("laws-repair: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };

    let report = rt.block_on(run_repair(&target, &url, key.as_deref(), dry_run));

    if json {
        print_json(&report);
    } else {
        print_text(&report);
    }

    if report.error.is_some() {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

async fn run_repair(
    index: &str,
    meili_url: &str,
    meili_key: Option<&str>,
    dry_run: bool,
) -> RepairReport {
    let mut report = RepairReport {
        index: index.to_string(),
        meili_url: meili_url.to_string(),
        dry_run,
        total: 0,
        already_keyed: 0,
        repaired: 0,
        old_deleted: 0,
        skipped_unparseable: 0,
        skipped_no_identity: 0,
        error: None,
    };

    let docs = match fetch_all_docs(meili_url, meili_key, index).await {
        Ok(d) => d,
        Err(e) => {
            report.error = Some(format!("fetch docs: {e}"));
            return report;
        }
    };
    report.total = docs.len();

    let mut rebuilt: std::collections::BTreeMap<String, serde_json::Value> =
        std::collections::BTreeMap::new();
    let mut old_ids: Vec<String> = Vec::new();

    for doc in &docs {
        let Some(id) = doc.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        if id.starts_with("bootstrap-") {
            report.already_keyed += 1;
            continue;
        }
        let (Some(repo), Some(path), Some(hash)) = (
            doc.get("repo").and_then(|v| v.as_str()),
            doc.get("path").and_then(|v| v.as_str()),
            doc.get("content_hash").and_then(|v| v.as_str()),
        ) else {
            report.skipped_no_identity += 1;
            continue;
        };
        // Unwrap the stringified law payload from `body`.
        let Some(payload) = doc
            .get("body")
            .and_then(|v| v.as_str())
            .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
            .filter(|p| p.get("law_id").is_some() && p.get("body").is_some())
        else {
            report.skipped_unparseable += 1;
            continue;
        };
        match rebuild_law_doc(id, repo, path, hash, payload) {
            Some(clean) => {
                if let Some(new_id) = clean.get("id").and_then(|v| v.as_str()) {
                    rebuilt.insert(new_id.to_string(), clean.clone());
                    old_ids.push(id.to_string());
                } else {
                    report.skipped_unparseable += 1;
                }
            }
            None => report.skipped_unparseable += 1,
        }
    }

    report.repaired = rebuilt.len();
    report.old_deleted = old_ids.len();

    if dry_run {
        return report;
    }

    if !rebuilt.is_empty() {
        let batch: Vec<serde_json::Value> = rebuilt.into_values().collect();
        if let Err(e) = upsert_documents(meili_url, meili_key, index, &batch).await {
            report.error = Some(format!("upsert failed: {e}"));
            return report;
        }
    }
    if !old_ids.is_empty() {
        if let Err(e) = delete_documents(meili_url, meili_key, index, &old_ids).await {
            report.error = Some(format!("delete old ids failed: {e}"));
            return report;
        }
    }

    report
}

/// Rebuild a clean law `Document` (as JSON) from the unwrapped payload by
/// running it through the production `build_doc` `Kind::Law` arm. Returns
/// `None` when the builder skips it (empty body).
fn rebuild_law_doc(
    old_event_id: &str,
    repo: &str,
    path: &str,
    content_hash: &str,
    payload: serde_json::Value,
) -> Option<serde_json::Value> {
    use cortex_core::events::Kind;
    use cortex_workers::classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
    use cortex_workers::embedder::EnrichedEvent;

    let classifier = ClassifierOutput {
        event_id: old_event_id.to_string(),
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
        event_id: old_event_id.to_string(),
        kind: Kind::Law,
        content_hash: content_hash.to_string(),
        redacted_payload: payload,
        classifier,
        context_repo: Some(repo.to_string()),
        context_path: Some(path.to_string()),
        parent_event_id: None,
        session_id: None,
        occurred_at_ms: 0,
    };
    match build_doc(&enriched, true, 1024 * 1024) {
        BuildOutcome::Ready(doc) => serde_json::to_value(&*doc).ok(),
        BuildOutcome::Skipped => None,
    }
}

async fn fetch_all_docs(
    meili_url: &str,
    meili_key: Option<&str>,
    index: &str,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let client = build_http_client(meili_key)?;
    let url = format!("{}/indexes/{}/search", meili_url.trim_end_matches('/'), index);
    let mut out: Vec<serde_json::Value> = Vec::new();
    let page = 1000usize;
    let mut offset = 0usize;
    loop {
        let body = serde_json::json!({ "q": "", "limit": page, "offset": offset });
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
            .cloned()
            .unwrap_or_default();
        let got = hits.len();
        out.extend(hits);
        if got < page {
            break;
        }
        offset += page;
    }
    Ok(out)
}

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
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| anyhow::anyhow!("reqwest client: {e}"))
}

fn print_text(r: &RepairReport) {
    println!("cortex-ops laws-repair");
    println!("index:                {}", r.index);
    println!("meili_url:            {}", r.meili_url);
    println!("dry_run:              {}", r.dry_run);
    println!();
    println!("total:                {}", r.total);
    println!("already bootstrap-:   {}", r.already_keyed);
    println!(
        "repaired (clean ids): {} {}",
        r.repaired,
        if r.dry_run { "(dry-run)" } else { "" }
    );
    println!(
        "old ids deleted:      {} {}",
        r.old_deleted,
        if r.dry_run { "(dry-run)" } else { "" }
    );
    println!("skipped unparseable:  {}", r.skipped_unparseable);
    println!("skipped no-identity:  {}", r.skipped_no_identity);
    if let Some(err) = &r.error {
        println!();
        println!("ERROR: {err}");
    }
}

fn print_json(r: &RepairReport) {
    let payload = serde_json::json!({
        "index": r.index,
        "meili_url": r.meili_url,
        "dry_run": r.dry_run,
        "total": r.total,
        "already_keyed": r.already_keyed,
        "repaired": r.repaired,
        "old_deleted": r.old_deleted,
        "skipped_unparseable": r.skipped_unparseable,
        "skipped_no_identity": r.skipped_no_identity,
        "error": r.error,
    });
    match serde_json::to_string_pretty(&payload) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("laws-repair: serialize report: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rebuild_law_doc_unwraps_payload_to_clean_doc() {
        // A malformed doc's body is the stringified payload; rebuild must
        // produce a clean doc: bootstrap- id, title != old id, prose body.
        let payload = serde_json::json!({
            "law_id": "CONSULT-ANALYSIS-BEFORE-IMPLEMENTING",
            "title": "Consult analysis before implementing",
            "severity": "info",
            "detector": null,
            "body": "When you start working on any task ... read the analysis first."
        });
        let clean = rebuild_law_doc(
            "01OLDULIDLEGACY0000000000000",
            "cortex",
            ".claude/rules/consult-analysis-before-implementing.md",
            "sha256:deadbeef",
            payload,
        )
        .expect("rebuild");
        let id = clean.get("id").and_then(|v| v.as_str()).unwrap();
        let title = clean.get("title").and_then(|v| v.as_str()).unwrap();
        let body = clean.get("body").and_then(|v| v.as_str()).unwrap();
        assert!(id.starts_with("bootstrap-"), "re-keyed: {id}");
        assert_ne!(title, "01OLDULIDLEGACY0000000000000");
        assert!(!body.trim_start().starts_with('{'), "body is prose, not JSON: {body}");
        assert!(body.contains("Consult") || body.contains("analysis"));
    }

    #[test]
    fn rebuild_is_deterministic_for_same_identity() {
        let p = serde_json::json!({"law_id":"L","title":"T","body":"some rule body"});
        let a = rebuild_law_doc("e1", "cortex", "a.md", "sha256:1", p.clone());
        let b = rebuild_law_doc("e2", "cortex", "a.md", "sha256:1", p);
        let ida = a.unwrap().get("id").and_then(|v| v.as_str()).map(String::from);
        let idb = b.unwrap().get("id").and_then(|v| v.as_str()).map(String::from);
        assert_eq!(ida, idb, "same (repo,path,hash) -> same bootstrap- id");
    }
}
