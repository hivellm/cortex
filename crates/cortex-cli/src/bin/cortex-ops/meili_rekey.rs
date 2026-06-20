//! `cortex-ops meili-rekey` — migrate legacy random-ULID-keyed
//! content-addressable docs to the stable, Meili-safe
//! `bootstrap-<sha256hex(repo|path|content_hash)>` primary key, IN PLACE.
//!
//! ## Why
//!
//! `bootstrap_doc_id` used to emit `bootstrap:<repo>:<path>:<hash>`, an
//! invalid Meilisearch primary key (`:` `/` `.` are rejected). Every
//! content-addressable doc therefore failed to index under its stable
//! key, leaving only legacy copies keyed on the random per-event ULID
//! (`live_doc_id`). The key bug is fixed
//! (`crates/cortex-workers/src/fulltext/document.rs`), but existing
//! indexes still hold the legacy-keyed docs.
//!
//! Unlike `decisions-reindex` (which re-emits from `.rulebook/decisions`
//! files — the COMPLETE source of truth for decisions), the `knowledge`
//! and `learning` families are NOT fully file-backed: most entries were
//! added live via `rulebook_knowledge_add` / `rulebook_learn_capture`
//! and exist only in the index. A source-driven prune would destroy
//! them. This command instead RE-KEYS each existing doc in place:
//!
//! 1. Read every doc in the index.
//! 2. For each doc NOT already `bootstrap-`-keyed that carries
//!    `repo` + `path` + `content_hash`, recompute the canonical
//!    `bootstrap-` id from those fields, clone the doc under the new id,
//!    and queue the old id for deletion.
//! 3. Upsert the re-keyed docs, then delete the old ids.
//!
//! Content is preserved verbatim; duplicates (same `repo`+`path`+
//! `content_hash`) collapse onto one canonical doc. Idempotent: a second
//! run finds zero legacy docs. Docs missing the identity triple are left
//! untouched and reported.
//!
//! ## Usage
//!
//! ```text
//! CORTEX_FULLTEXT_MEILI_URL=http://127.0.0.1:7700 \
//! CORTEX_FULLTEXT_MEILI_API_KEY=<key> \
//!   cortex-ops meili-rekey --index cortex-cortex-knowledge --dry-run
//! ```

use std::process::ExitCode;

use cortex_workers::fulltext::bootstrap_doc_id;
use cortex_workers::fulltext::config::FulltextConfig;

/// Summary of the re-key migration (or what it would do under dry-run).
#[derive(Debug)]
struct RekeyReport {
    index: String,
    meili_url: String,
    dry_run: bool,
    /// Total docs scanned.
    total: usize,
    /// Docs already `bootstrap-`-keyed (left as-is).
    already_keyed: usize,
    /// Legacy docs re-keyed (distinct new ids after dedup).
    rekeyed: usize,
    /// Distinct old ids deleted.
    old_deleted: usize,
    /// Legacy docs skipped for lacking the `repo`+`path`+`content_hash`
    /// identity triple (cannot recompute the stable key).
    skipped_no_identity: usize,
    error: Option<String>,
}

/// Entry point for `cortex-ops meili-rekey`.
pub(super) fn meili_rekey(
    index: String,
    meili_url: Option<String>,
    meili_key: Option<String>,
    dry_run: bool,
    json: bool,
) -> ExitCode {
    if index.trim().is_empty() {
        eprintln!("meili-rekey: --index is required");
        return ExitCode::from(2);
    }
    let cfg = FulltextConfig::from_env();
    let url = meili_url
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| cfg.meili_url.clone());
    let key = meili_key
        .filter(|s| !s.is_empty())
        .or_else(|| cfg.meili_api_key.clone());

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("meili-rekey: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };

    let report = rt.block_on(run_rekey(&index, &url, key.as_deref(), dry_run));

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

async fn run_rekey(
    index: &str,
    meili_url: &str,
    meili_key: Option<&str>,
    dry_run: bool,
) -> RekeyReport {
    let mut report = RekeyReport {
        index: index.to_string(),
        meili_url: meili_url.to_string(),
        dry_run,
        total: 0,
        already_keyed: 0,
        rekeyed: 0,
        old_deleted: 0,
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

    // Map new canonical id -> re-keyed doc (dedups duplicates), plus the
    // set of distinct old ids to delete once the upsert lands.
    let mut rekeyed: std::collections::BTreeMap<String, serde_json::Value> =
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
        let new_id = bootstrap_doc_id(repo, path, hash);
        let mut cloned = doc.clone();
        if let Some(obj) = cloned.as_object_mut() {
            obj.insert("id".to_string(), serde_json::Value::String(new_id.clone()));
        }
        rekeyed.insert(new_id, cloned);
        old_ids.push(id.to_string());
    }

    report.rekeyed = rekeyed.len();
    report.old_deleted = old_ids.len();

    if dry_run {
        return report;
    }

    if !rekeyed.is_empty() {
        let batch: Vec<serde_json::Value> = rekeyed.into_values().collect();
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

/// Fetch every document in the index (paginated search with `q:""`).
async fn fetch_all_docs(
    meili_url: &str,
    meili_key: Option<&str>,
    index: &str,
) -> anyhow::Result<Vec<serde_json::Value>> {
    let client = build_http_client(meili_key)?;
    let url = format!(
        "{}/indexes/{}/search",
        meili_url.trim_end_matches('/'),
        index,
    );
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

/// POST documents to Meili `addOrReplaceDocuments` (upsert by `id`).
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

/// DELETE documents by id via Meili `delete-batch`.
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

fn print_text(r: &RekeyReport) {
    println!("cortex-ops meili-rekey");
    println!("index:               {}", r.index);
    println!("meili_url:           {}", r.meili_url);
    println!("dry_run:             {}", r.dry_run);
    println!();
    println!("total:               {}", r.total);
    println!("already bootstrap-:  {}", r.already_keyed);
    println!(
        "rekeyed (new ids):   {} {}",
        r.rekeyed,
        if r.dry_run { "(dry-run)" } else { "" }
    );
    println!(
        "old ids deleted:     {} {}",
        r.old_deleted,
        if r.dry_run { "(dry-run)" } else { "" }
    );
    println!("skipped (no triple): {}", r.skipped_no_identity);
    if let Some(err) = &r.error {
        println!();
        println!("ERROR: {err}");
    }
}

fn print_json(r: &RekeyReport) {
    let payload = serde_json::json!({
        "index": r.index,
        "meili_url": r.meili_url,
        "dry_run": r.dry_run,
        "total": r.total,
        "already_keyed": r.already_keyed,
        "rekeyed": r.rekeyed,
        "old_deleted": r.old_deleted,
        "skipped_no_identity": r.skipped_no_identity,
        "error": r.error,
    });
    match serde_json::to_string_pretty(&payload) {
        Ok(s) => println!("{s}"),
        Err(e) => eprintln!("meili-rekey: serialize report: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rekey_id_is_meili_safe_and_stable() {
        // The recomputed id must be the Meili-safe bootstrap- hash and
        // be deterministic for the same identity triple.
        let a = bootstrap_doc_id("cortex", ".rulebook/knowledge/x.md", "sha256:abc");
        let b = bootstrap_doc_id("cortex", ".rulebook/knowledge/x.md", "sha256:abc");
        assert_eq!(a, b, "same triple -> same id");
        assert!(a.starts_with("bootstrap-"));
        assert!(
            a.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
            "id must be a valid Meili primary key: {a}"
        );
    }

    #[test]
    fn distinct_triples_yield_distinct_ids() {
        let a = bootstrap_doc_id("cortex", "a.md", "sha256:1");
        let b = bootstrap_doc_id("cortex", "b.md", "sha256:1");
        let c = bootstrap_doc_id("cortex", "a.md", "sha256:2");
        assert_ne!(a, b);
        assert_ne!(a, c);
    }
}
