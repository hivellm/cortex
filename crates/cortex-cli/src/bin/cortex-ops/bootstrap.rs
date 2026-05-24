use super::helpers::resolve_metadata_db;
use std::process::ExitCode;

/// Phase10c — `cortex-ops bootstrap-dedup` dispatcher. Walks the
/// `bootstrap_seen` ledger looking for `(repo, content_hash)`
/// groups whose distinct paths > 1 (the same redacted body
/// emitted under different file paths) and prints a summary so
/// operators can decide whether to clean up the live lane. The
/// `--apply` flag is reserved for the future live-backend cleanup
/// path; today it returns an actionable error pointing at the
/// dry-run output.
pub(super) fn bootstrap_dedup(
    repo: Option<String>,
    dry_run: bool,
    apply: bool,
    metadata_db: Option<String>,
    json: bool,
) -> ExitCode {
    if !dry_run && !apply {
        eprintln!("bootstrap-dedup: pass either --dry-run (read-only) or --apply (reserved)");
        return ExitCode::from(2);
    }
    if apply && !dry_run {
        // Honest about the current scope. The dry-run output is
        // the actionable surface today.
        eprintln!(
            "bootstrap-dedup --apply: not yet wired to the live Vectorizer/Meili/Nexus \
             backends. Re-run with --dry-run to inspect the duplicate groups."
        );
        return ExitCode::from(3);
    }
    let db_path = match resolve_metadata_db(metadata_db) {
        Some(p) => p,
        None => {
            eprintln!(
                "bootstrap-dedup: cannot resolve metadata DB path \
                 (set --metadata-db, $CORTEX_METADATA_DB, or $HOME)"
            );
            return ExitCode::from(2);
        }
    };
    let store = match cortex_storage::MetadataStore::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("bootstrap-dedup: open {}: {e}", db_path.display());
            return ExitCode::from(1);
        }
    };
    let total_rows = match store.bootstrap_seen_count(repo.as_deref()) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("bootstrap-dedup: count: {e}");
            return ExitCode::from(1);
        }
    };
    let target_repos: Vec<String> = match repo.clone() {
        Some(r) => vec![r],
        None => match list_distinct_dedup_repos(&store) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("bootstrap-dedup: list repos: {e}");
                return ExitCode::from(1);
            }
        },
    };
    let mut groups: Vec<cortex_storage::BootstrapSeenDuplicate> = Vec::new();
    for r in &target_repos {
        match store.bootstrap_seen_duplicates_by_hash(r) {
            Ok(g) => groups.extend(g),
            Err(e) => {
                eprintln!("bootstrap-dedup: scan {r}: {e}");
                return ExitCode::from(1);
            }
        }
    }
    if json {
        let payload = serde_json::json!({
            "ledger_rows": total_rows,
            "scanned_repos": target_repos.len(),
            "duplicate_groups": groups.len(),
            "groups": groups
                .iter()
                .map(|g| serde_json::json!({
                    "content_hash": g.content_hash,
                    "paths": g.paths,
                    "count": g.count,
                }))
                .collect::<Vec<_>>(),
        });
        println!("{}", payload);
    } else {
        println!(
            "bootstrap-dedup ({} mode):",
            if apply { "apply" } else { "dry-run" }
        );
        println!("  ledger rows scanned: {total_rows}");
        println!("  repos: {}", target_repos.len());
        println!("  duplicate-by-content groups: {}", groups.len());
        for g in &groups {
            println!(
                "    {hash}  ({n} paths)",
                hash = g.content_hash,
                n = g.count,
            );
            for p in &g.paths {
                println!("      - {p}");
            }
        }
    }
    ExitCode::SUCCESS
}

pub(super) fn list_distinct_dedup_repos(
    store: &cortex_storage::MetadataStore,
) -> Result<Vec<String>, cortex_storage::MetadataError> {
    let mut stmt = store
        .conn()
        .prepare("SELECT DISTINCT repo FROM bootstrap_seen ORDER BY repo")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r?);
    }
    Ok(out)
}

/// Phase10d — `cortex-ops repo-canonicalize` dispatcher.
/// Lowercases mixed-case `repo` columns in the metadata SQLite so
/// the orchestrator's scope filter and the dashboard's wire
/// shape agree on a single canonical form. Live-backend
/// rewrites (Vectorizer, Meili, Nexus) are listed in the report
/// but skipped — those backends carry the repo as a payload
/// field rather than a filterable column, so a separate
/// migration tool will handle them.
pub(super) fn repo_canonicalize(
    repo: Option<String>,
    dry_run: bool,
    apply: bool,
    metadata_db: Option<String>,
    json: bool,
) -> ExitCode {
    if !dry_run && !apply {
        eprintln!("repo-canonicalize: pass either --dry-run or --apply");
        return ExitCode::from(2);
    }
    let db_path = match resolve_metadata_db(metadata_db) {
        Some(p) => p,
        None => {
            eprintln!("repo-canonicalize: cannot resolve metadata DB path");
            return ExitCode::from(2);
        }
    };
    let store = match cortex_storage::MetadataStore::open(&db_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("repo-canonicalize: open {}: {e}", db_path.display());
            return ExitCode::from(1);
        }
    };
    let conn = store.conn();
    let target_filter = repo.clone();

    // Count rewrite candidates (rows whose `repo` differs from its
    // lowercase form) per table.
    let sessions_candidates =
        match count_canonicalize_candidates(conn, "sessions", "repo", target_filter.as_deref()) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("repo-canonicalize: count sessions: {e}");
                return ExitCode::from(1);
            }
        };
    let bootstrap_candidates = match count_canonicalize_candidates(
        conn,
        "bootstrap_jobs",
        "repo_path",
        target_filter.as_deref(),
    ) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("repo-canonicalize: count bootstrap_jobs: {e}");
            return ExitCode::from(1);
        }
    };
    let bootstrap_seen_candidates = match count_canonicalize_candidates(
        conn,
        "bootstrap_seen",
        "repo",
        target_filter.as_deref(),
    ) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("repo-canonicalize: count bootstrap_seen: {e}");
            return ExitCode::from(1);
        }
    };

    let mut sessions_rewrites = 0u64;
    let mut bootstrap_rewrites = 0u64;
    let mut bootstrap_seen_rewrites = 0u64;
    if apply {
        sessions_rewrites =
            match apply_canonicalize(conn, "sessions", "repo", target_filter.as_deref()) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("repo-canonicalize: rewrite sessions: {e}");
                    return ExitCode::from(1);
                }
            };
        bootstrap_rewrites = match apply_canonicalize(
            conn,
            "bootstrap_jobs",
            "repo_path",
            target_filter.as_deref(),
        ) {
            Ok(n) => n,
            Err(e) => {
                eprintln!("repo-canonicalize: rewrite bootstrap_jobs: {e}");
                return ExitCode::from(1);
            }
        };
        bootstrap_seen_rewrites =
            match apply_canonicalize(conn, "bootstrap_seen", "repo", target_filter.as_deref()) {
                Ok(n) => n,
                Err(e) => {
                    eprintln!("repo-canonicalize: rewrite bootstrap_seen: {e}");
                    return ExitCode::from(1);
                }
            };
    }
    if json {
        let payload = serde_json::json!({
            "mode": if apply { "apply" } else { "dry-run" },
            "sessions": { "candidates": sessions_candidates, "rewritten": sessions_rewrites },
            "bootstrap_jobs": { "candidates": bootstrap_candidates, "rewritten": bootstrap_rewrites },
            "bootstrap_seen": { "candidates": bootstrap_seen_candidates, "rewritten": bootstrap_seen_rewrites },
            "live_backends_pending": ["vectorizer", "meili", "nexus"],
        });
        println!("{}", payload);
    } else {
        println!(
            "repo-canonicalize ({} mode):",
            if apply { "apply" } else { "dry-run" }
        );
        println!(
            "  sessions.repo:           candidates {sessions_candidates}, rewritten {sessions_rewrites}"
        );
        println!(
            "  bootstrap_jobs.repo_path: candidates {bootstrap_candidates}, rewritten {bootstrap_rewrites}"
        );
        println!(
            "  bootstrap_seen.repo:     candidates {bootstrap_seen_candidates}, rewritten {bootstrap_seen_rewrites}"
        );
        println!(
            "  live backends (vectorizer / meili / nexus): \
             carry `repo` in payload bodies; not handled by this tool"
        );
    }
    ExitCode::SUCCESS
}

/// Count rows where `<column>` differs from `lower(<column>)`.
/// Optional `target` restricts to the original-case value (so a
/// caller can pre-flight a single repo migration).
fn count_canonicalize_candidates(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
    target: Option<&str>,
) -> rusqlite::Result<u64> {
    let sql = match target {
        Some(_) => format!(
            "SELECT COUNT(*) FROM {table} \
             WHERE {column} = ?1 AND {column} != lower({column})"
        ),
        None => format!(
            "SELECT COUNT(*) FROM {table} \
             WHERE {column} != lower({column})"
        ),
    };
    let count: i64 = match target {
        Some(t) => conn.query_row(&sql, rusqlite::params![t], |r| r.get(0))?,
        None => conn.query_row(&sql, [], |r| r.get(0))?,
    };
    Ok(count.max(0) as u64)
}

/// Apply the lowercase rewrite. Returns the row count touched.
fn apply_canonicalize(
    conn: &rusqlite::Connection,
    table: &str,
    column: &str,
    target: Option<&str>,
) -> rusqlite::Result<u64> {
    let sql = match target {
        Some(_) => format!(
            "UPDATE {table} SET {column} = lower({column}) \
             WHERE {column} = ?1 AND {column} != lower({column})"
        ),
        None => format!(
            "UPDATE {table} SET {column} = lower({column}) \
             WHERE {column} != lower({column})"
        ),
    };
    let touched = match target {
        Some(t) => conn.execute(&sql, rusqlite::params![t])?,
        None => conn.execute(&sql, [])?,
    };
    Ok(touched as u64)
}

/// Phase11p §4.3 — dedupe `law.imported` documents on each
/// `cortex-{slug}-governance` Meili index plus the global
/// `cortex_laws`. Groups by `(law_id, content_hash)` and keeps the
/// oldest by `ts`. Default is dry-run; `--apply` issues
/// `DELETE /indexes/{uid}/documents/delete-batch` per group.
pub(super) fn dedupe_laws(
    meili: Option<String>,
    meili_key: Option<String>,
    target_index: Option<String>,
    apply: bool,
    json: bool,
) -> ExitCode {
    let cfg = cortex_config::Config::load().unwrap_or_default();
    let meili_url = meili
        .or_else(|| cfg.meili.meili_url.clone())
        .unwrap_or_else(|| "http://127.0.0.1:17004".to_string());
    let api_key = meili_key
        .or_else(|| cfg.meili.meili_api_key.clone())
        .or_else(|| std::env::var("MEILI_MASTER_KEY").ok());

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("dedupe-laws: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };

    runtime.block_on(async move {
        match dedupe_laws_async(
            &meili_url,
            api_key.as_deref(),
            target_index.as_deref(),
            apply,
            json,
        )
        .await
        {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("dedupe-laws: {e}");
                ExitCode::from(2)
            }
        }
    })
}

async fn dedupe_laws_async(
    meili_url: &str,
    api_key: Option<&str>,
    target_index: Option<&str>,
    apply: bool,
    json: bool,
) -> anyhow::Result<()> {
    let http = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(60))
        .build()
        .map_err(|e| anyhow::anyhow!("reqwest builder: {e}"))?;

    let auth = |req: reqwest::RequestBuilder| match api_key {
        Some(k) => req.bearer_auth(k),
        None => req,
    };

    // List candidate indexes — every per-repo `cortex-{slug}-governance`
    // plus the global `cortex_laws`.
    let candidates: Vec<String> = if let Some(t) = target_index {
        vec![t.to_string()]
    } else {
        let stats: serde_json::Value =
            auth(http.get(format!("{}/stats", meili_url.trim_end_matches('/'))))
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
        let map = stats
            .get("indexes")
            .and_then(|v| v.as_object())
            .ok_or_else(|| anyhow::anyhow!("/stats payload missing `indexes`"))?;
        map.keys()
            .filter(|k| k.ends_with("-governance") || k.as_str() == "cortex_laws")
            .cloned()
            .collect()
    };

    #[derive(Default)]
    struct IndexReport {
        total_docs: usize,
        groups: usize,
        keep: Vec<String>,
        drop: Vec<String>,
    }
    let mut per_index: std::collections::BTreeMap<String, IndexReport> =
        std::collections::BTreeMap::new();

    for index in &candidates {
        let mut all_docs: Vec<serde_json::Value> = Vec::new();
        let mut offset = 0usize;
        let limit = 1000;
        loop {
            let url = format!(
                "{}/indexes/{}/documents?limit={}&offset={}",
                meili_url.trim_end_matches('/'),
                index,
                limit,
                offset
            );
            let resp = auth(http.get(&url)).send().await?;
            if !resp.status().is_success() {
                anyhow::bail!(
                    "list documents {index} offset={offset}: status {}",
                    resp.status()
                );
            }
            let body: serde_json::Value = resp.json().await?;
            let results = body
                .get("results")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if results.is_empty() {
                break;
            }
            offset += results.len();
            all_docs.extend(results);
            // Stop when offset exceeds total reported to avoid loops on
            // edge servers.
            let total = body.get("total").and_then(|v| v.as_u64()).unwrap_or(0) as usize;
            if total > 0 && offset >= total {
                break;
            }
        }

        // Group by (law_id, content_hash). Keep the document with the
        // oldest `ts` (numeric epoch ms).
        let mut groups: std::collections::HashMap<(String, String), Vec<(String, i64)>> =
            std::collections::HashMap::new();
        for d in &all_docs {
            let id = d
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let law_id = d
                .get("law_id")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    d.get("ext")
                        .and_then(|e| e.get("law_violation"))
                        .and_then(|lv| lv.get("law_id"))
                        .and_then(|v| v.as_str())
                })
                .unwrap_or("")
                .to_string();
            let content_hash = d
                .get("content_hash")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if id.is_empty() || (law_id.is_empty() && content_hash.is_empty()) {
                continue;
            }
            let ts = d.get("ts").and_then(|v| v.as_i64()).unwrap_or(0);
            groups
                .entry((law_id, content_hash))
                .or_default()
                .push((id, ts));
        }

        let mut report = IndexReport {
            total_docs: all_docs.len(),
            ..Default::default()
        };
        for (_key, mut docs) in groups {
            if docs.len() < 2 {
                continue;
            }
            report.groups += 1;
            docs.sort_by_key(|d| d.1);
            let keeper = docs.first().unwrap().0.clone();
            report.keep.push(keeper);
            for (drop_id, _) in docs.into_iter().skip(1) {
                report.drop.push(drop_id);
            }
        }

        per_index.insert(index.clone(), report);
    }

    // Render plan + optionally apply.
    if json {
        let body: serde_json::Value = serde_json::json!({
            "mode": if apply { "apply" } else { "dry_run" },
            "meili_url": meili_url,
            "indexes": per_index
                .iter()
                .map(|(k, v)| serde_json::json!({
                    "uid": k,
                    "total_docs": v.total_docs,
                    "duplicate_groups": v.groups,
                    "kept": v.keep.len(),
                    "to_drop": v.drop.len(),
                    "drop_ids": v.drop,
                }))
                .collect::<Vec<_>>(),
        });
        println!("{}", serde_json::to_string_pretty(&body)?);
    } else {
        println!(
            "cortex-ops dedupe-laws ({})",
            if apply { "apply" } else { "dry-run" }
        );
        println!("meili: {meili_url}");
        for (uid, r) in &per_index {
            println!(
                "  {uid}: total={}, groups={}, keep={}, drop={}",
                r.total_docs,
                r.groups,
                r.keep.len(),
                r.drop.len()
            );
        }
        let total_drop: usize = per_index.values().map(|r| r.drop.len()).sum();
        println!("Total docs to drop: {total_drop}");
    }

    if !apply {
        return Ok(());
    }

    // --apply: Meili's `delete-batch` endpoint accepts a JSON array
    // of document ids.
    for (uid, report) in &per_index {
        if report.drop.is_empty() {
            continue;
        }
        let url = format!(
            "{}/indexes/{}/documents/delete-batch",
            meili_url.trim_end_matches('/'),
            uid
        );
        // Chunk into 500 ids per request to stay within Meili's
        // body-size guard.
        for chunk in report.drop.chunks(500) {
            let resp = auth(http.post(&url).json(&chunk)).send().await?;
            if !resp.status().is_success() {
                let detail = resp.text().await.unwrap_or_default();
                anyhow::bail!("delete-batch {uid}: {detail}");
            }
        }
        eprintln!(
            "dedupe-laws: applied {} deletes on {}",
            report.drop.len(),
            uid
        );
    }
    Ok(())
}
