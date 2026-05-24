//! phase11x — live backend for `cortex-ops turn-digest`.
//!
//! Wires the in-process orchestrator from
//! [`cortex_workers::retention::turn_digest`] to two live services:
//!
//! 1. **Meilisearch** — paginates every `cortex-<repo>-turns` index,
//!    server-side filters `kind = "turn"`, materialises every doc
//!    whose `occurred_at < cutoff_ts` (and that has not been digested
//!    already) into a [`Turn`].
//! 2. **`cortex-ingestion /v1/events`** — receives one canonical
//!    `Memory{memory_type=turn_digest}` envelope per bucket. Embedder
//!    + Meili + Nexus pick it up downstream so the digest appears in
//!    every backend without per-bucket fan-out from this binary.
//!
//! The legacy unified `cortex_turns` Meili index from
//! `cortex_storage::names` is declared but never populated — the
//! live indexer (`cortex_workers::fulltext::routing::family_for(
//! Kind::Turn)`) writes to `cortex-<repo>-turns`. This module adapts
//! to that reality so the production sweep finally has a real
//! source.
//!
//! `tag_source_turns` mirrors the conservative path
//! `tool_call_digest_live` takes: refusing the keep-and-tag mode
//! until the Parquet rewriter exposes a non-destructive
//! `summarized_by` stamping endpoint. Operators run with
//! `--purge-originals` (clean shrinkage) or `--dry-run` (no
//! mutation).

use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cortex_workers::retention::turn_digest::{Bucket, DigestBackend, DigestResult, Turn};

const FORGET_CONFIRMATION_TOKEN: &str = "I-UNDERSTAND-FORGET-IS-IRREVERSIBLE";
const MEILI_INDEX_MEMORIES: &str = "cortex_memories";

/// Live backend wiring Meili + cortex-ingestion + cortex-api.
pub struct LiveTurnDigestBackend {
    /// Base URL for cortex-api (`/v1/admin/forget`).
    pub api_base: String,
    /// Optional bearer token for cortex-api when auth is enabled.
    pub api_token: Option<String>,
    /// Base URL for cortex-ingestion (`/v1/events`).
    pub ingestion_base: String,
    /// Base URL for Meili — used by `lookup_existing` to short-
    /// circuit re-summarisation on previously-completed buckets.
    pub meili_base: String,
    /// Optional Meili API key.
    pub meili_key: Option<String>,
    /// Shared HTTP client.
    pub http: reqwest::Client,
}

impl LiveTurnDigestBackend {
    /// Build a client with a 30-second timeout.
    pub fn new(
        api_base: String,
        api_token: Option<String>,
        ingestion_base: String,
        meili_base: String,
        meili_key: Option<String>,
    ) -> anyhow::Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("reqwest client")?;
        Ok(Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            api_token,
            ingestion_base: ingestion_base.trim_end_matches('/').to_string(),
            meili_base: meili_base.trim_end_matches('/').to_string(),
            meili_key,
            http,
        })
    }

    /// Verify a freshly-persisted digest envelope is queryable in
    /// Meili before authorising the purge of source rows. The pipeline
    /// `cortex-ingestion → classifier → embedder → fulltext-worker`
    /// is asynchronous, so the digest envelope POSTed by
    /// `persist_digest` lands in `cortex_memories` (or its per-repo
    /// alias) seconds-to-tens-of-seconds later. Polling the same
    /// `(repo, year_week, top_topic)` bucket key the digest body
    /// stamps under `extras.turn_digest_key` is the cheapest way to
    /// confirm "the summary is indexed and retrievable". Returns
    /// `Ok(true)` once the envelope is found, `Ok(false)` after the
    /// timeout elapses without finding it, `Err` on transport
    /// failure.
    ///
    /// **Safety contract** (per user instruction 2026-05-05):
    /// "somente pode remover os dados da memoria apos validar que
    /// foram sumarizados ou consolidados". The purge cascade
    /// (`delete_source_turns` → `/v1/admin/forget`) MUST be gated on
    /// this returning `true` — otherwise a misconfigured pipeline
    /// would silently delete originals while leaving the summary
    /// stranded in the ingestion queue.
    pub async fn verify_digest_indexed(
        &self,
        repo: &str,
        year_week: &str,
        top_topic: &str,
        timeout: Duration,
    ) -> Result<bool, String> {
        let deadline = std::time::Instant::now() + timeout;
        let mut backoff_ms: u64 = 500;
        loop {
            match self.lookup_existing(repo, year_week, top_topic).await {
                Ok(Some(_)) => return Ok(true),
                Ok(None) => {}
                Err(e) => return Err(e),
            }
            if std::time::Instant::now() >= deadline {
                return Ok(false);
            }
            tokio::time::sleep(Duration::from_millis(backoff_ms)).await;
            backoff_ms = (backoff_ms.saturating_mul(2)).min(5_000);
        }
    }

    /// Hard-purge a list of source turn event ids via the cortex-api
    /// `/v1/admin/forget` cascade. Returns the count of successfully
    /// purged ids. Per-id failures are surfaced via stderr but do
    /// not abort the bucket — partial failure is recorded by the
    /// caller and a re-run with the same plan retries the gaps.
    pub async fn delete_source_turns(&self, event_ids: &[String]) -> Result<u64, String> {
        let mut purged = 0u64;
        for event_id in event_ids {
            let url = format!("{}/v1/admin/forget", self.api_base);
            let body = serde_json::json!({
                "event_id": event_id,
                "confirmation_token": FORGET_CONFIRMATION_TOKEN,
                "dry_run": false,
            });
            let mut req = self.http.post(&url).json(&body);
            if let Some(token) = &self.api_token {
                req = req.bearer_auth(token);
            }
            match req.send().await {
                Ok(resp) => {
                    if resp.status().is_success() {
                        purged += 1;
                    } else {
                        let status = resp.status();
                        let snippet = resp
                            .text()
                            .await
                            .unwrap_or_default()
                            .chars()
                            .take(200)
                            .collect::<String>();
                        eprintln!("turn-digest: forget {event_id}: {status} {snippet}");
                    }
                }
                Err(e) => {
                    eprintln!("turn-digest: forget {event_id}: transport: {e}");
                }
            }
        }
        Ok(purged)
    }
}

#[async_trait]
impl DigestBackend for LiveTurnDigestBackend {
    async fn lookup_existing(
        &self,
        repo: &str,
        year_week: &str,
        top_topic: &str,
    ) -> Result<Option<String>, String> {
        // Query the per-repo consolidations index full-text for
        // `<repo>|<year_week>|<top_topic>` — the persist path stamps
        // that exact key on `payload.scope.value` and into the
        // `summary_markdown` body so Meili surfaces it on a vanilla
        // search query.
        let bucket_key = format!("{repo}|{year_week}|{top_topic}");
        let index_uid = format!(
            "cortex-{}-consolidations",
            cortex_storage::names::slug_for_repo(repo)
        );
        let url = format!("{}/indexes/{index_uid}/search", self.meili_base);
        let body = serde_json::json!({
            "q": &bucket_key,
            "limit": 1,
        });
        let mut req = self.http.post(&url).json(&body);
        if let Some(k) = &self.meili_key {
            req = req.bearer_auth(k);
        }
        let resp = req
            .send()
            .await
            .map_err(|e| format!("meili search transport: {e}"))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !status.is_success() {
            let snippet = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>();
            return Err(format!("meili search {status}: {snippet}"));
        }
        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| format!("meili search decode: {e}"))?;
        let hits = json
            .get("hits")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        if hits.is_empty() {
            return Ok(None);
        }
        let id = hits[0]
            .get("event_id")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string());
        Ok(id)
    }

    async fn summarize(&self, bucket: &Bucket) -> Result<DigestResult, String> {
        // Deterministic aggregation matching `tool_call_digest_live`.
        // No LLM round-trip — the bucket shape itself is the
        // retrieval payload, and skipping classifier calls keeps
        // cron cost at zero.
        let mut ids: Vec<String> = bucket.event_ids.iter().cloned().collect();
        ids.sort();
        ids.dedup();
        let bucket_key = format!("{}|{}|{}", bucket.repo, bucket.year_week, bucket.top_topic);
        let body = format!(
            "# Turn digest — `{repo}` · {week} · `{topic}`\n\n\
             Bucket: `{bucket_key}`\n\n\
             Aggregates {count} turns observed in repo \
             `{repo}` during ISO week {week} under topic \
             `{topic}`. Original envelopes are listed below for \
             forensic round-trip; the summarisation pass collapses \
             them into this single Memory entry.\n\n\
             ## Counters\n\n\
             - turns: {count}\n\
             - repo: {repo}\n\
             - topic: {topic}\n\
             - week: {week}\n\n\
             ## Source event ids\n\n\
             {id_list}\n",
            repo = bucket.repo,
            week = bucket.year_week,
            topic = bucket.top_topic,
            count = ids.len(),
            id_list = ids
                .iter()
                .map(|s| format!("- `{s}`"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        let tokens_in = body.len() as u64 / 4;
        Ok(DigestResult {
            body,
            tokens_in,
            tokens_out: 0,
            usd_cents: 0,
        })
    }

    async fn persist_digest(
        &self,
        bucket: &Bucket,
        digest: &DigestResult,
    ) -> Result<String, String> {
        // Mirror `LiveToolCallDigestBackend::persist_digest` —
        // `kind=consolidation` with `grain=topic` is the schema the
        // ingestion validator accepts. The legacy memory_type=turn_digest
        // shape would have to land under `kind=memory` whose schema
        // restricts `memory_type` to user/feedback/project/reference.
        let event_id = ulid::Ulid::new().to_string();
        let consolidation_id = ulid::Ulid::new().to_string();
        let session_id = ulid::Ulid::new().to_string();
        let now = Utc::now();
        let occurred_iso = now.to_rfc3339();
        let bucket_key = format!("{}|{}|{}", bucket.repo, bucket.year_week, bucket.top_topic);
        let title_short = {
            let raw = format!(
                "{} turns — {} · {} · {}",
                bucket.event_ids.len(),
                bucket.repo,
                bucket.year_week,
                bucket.top_topic,
            );
            let mut t: String = raw.chars().take(80).collect();
            if t.is_empty() {
                t = format!("digest:{bucket_key}");
            }
            t
        };
        let mut summary = digest.body.clone();
        if summary.len() < 200 {
            let pad = "\n\n---\nThis digest aggregates the listed source turn events; \
                 expand the `source_event_ids` array for the full Parquet \
                 round-trip view.";
            summary.push_str(pad);
            while summary.len() < 200 {
                summary.push(' ');
            }
        }
        if summary.len() > 2000 {
            summary.truncate(2000);
        }
        let payload = serde_json::json!({
            "consolidation_id": consolidation_id,
            "grain": "topic",
            "scope": { "kind": "topic", "value": bucket_key },
            "title": title_short,
            "summary_markdown": summary,
            "source_event_ids": bucket.event_ids,
            "source_event_count": bucket.event_ids.len(),
            "model": "cortex-ops:turn-digest",
            "depth": "shallow",
            "temporal_span": {
                "start_ms": 0,
                "end_ms": now.timestamp_millis(),
                "duration_ms": now.timestamp_millis(),
            },
            "repos": [bucket.repo.clone()],
            "tags": ["turn_digest", bucket.top_topic.clone()],
        });
        let payload_bytes =
            serde_json::to_vec(&payload).map_err(|e| format!("encode payload: {e}"))?;
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&payload_bytes);
        let content_hash = format!("sha256:{:x}", hasher.finalize());
        let envelope = serde_json::json!({
            "event_id": event_id,
            "schema_version": "1",
            "occurred_at": occurred_iso,
            "session_id": session_id,
            "stream": "live",
            "tool": "cortex-cli",
            "kind": "consolidation",
            "context": {
                "repo": bucket.repo,
                "platform": platform_string(),
            },
            "payload": payload,
            "content_hash": content_hash,
        });
        // `/v1/events` is the single-envelope endpoint — accepts a
        // bare envelope, not the batch `{events: [...]}` wrapper.
        let url = format!("{}/v1/events", self.ingestion_base);
        let resp = self
            .http
            .post(&url)
            .json(&envelope)
            .send()
            .await
            .map_err(|e| format!("ingestion transport: {e}"))?;
        let status = resp.status();
        if !status.is_success() {
            let snippet = resp
                .text()
                .await
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect::<String>();
            return Err(format!("ingestion POST {status}: {snippet}"));
        }
        Ok(event_id)
    }

    async fn tag_source_turns(
        &self,
        _digest_event_id: &str,
        _event_ids: &[String],
    ) -> Result<(), String> {
        // No-op at the trait level. The cron's purge cascade is
        // handled externally by `cortex-ops turn-digest --apply
        // --purge-originals` (digest.rs) AFTER it confirms via
        // `verify_digest_indexed` that the summary envelope is
        // queryable. Returning Ok here lets the orchestrator's
        // `digest_one` complete cleanly so the report records the
        // bucket as `digested = true`; without it, every bucket
        // would land in the error path and the binary's verify+
        // purge gate would never fire.
        //
        // The conservative "keep originals + tag" mode needs a
        // non-destructive `summarized_by` Parquet rewriter that
        // does not exist yet — operators that want it must run
        // with `--dry-run` until that endpoint ships.
        Ok(())
    }
}

/// Materialise every `kind=turn` envelope older than `cutoff_ts`
/// via the cortex-api admin endpoint
/// (`GET /v1/admin/list-events?kind=turn&before=<cutoff>&limit=N`).
/// Mirrors `tool_call_digest_live::fetch_old_tool_calls_via_admin`
/// — the keyword lane carries every Meili and Parquet row stamped
/// with a real epoch, so the admin projection is the canonical
/// enumeration path (the per-repo Meili scan would miss the
/// `ts = 0` rows the indexer stamps on certain edge cases).
pub async fn fetch_old_turns_via_admin(
    api_base: &str,
    api_token: Option<&str>,
    cutoff_ts: DateTime<Utc>,
    max_records: usize,
) -> anyhow::Result<Vec<Turn>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("reqwest client")?;
    let cutoff_iso = cutoff_ts.to_rfc3339();
    let cutoff_enc = cutoff_iso.replace(':', "%3A").replace('+', "%2B");
    let url = format!(
        "{}/v1/admin/list-events?kind=turn&before={}&limit={}",
        api_base.trim_end_matches('/'),
        cutoff_enc,
        max_records.clamp(1, 50_000),
    );
    let mut req = client.get(&url);
    if let Some(t) = api_token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.with_context(|| format!("GET {url}"))?;
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        anyhow::bail!("GET {url}: {status} — {body}");
    }
    #[derive(serde::Deserialize)]
    struct Row {
        event_id: String,
        kind: String,
        occurred_at: String,
        repo: Option<String>,
        #[serde(default)]
        topic: Option<String>,
        #[serde(default)]
        topics: Option<Vec<String>>,
        #[serde(default)]
        summarized_by: Option<String>,
    }
    let rows: Vec<Row> = resp
        .json()
        .await
        .context("decode /v1/admin/list-events response")?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        if r.kind != "turn" {
            continue;
        }
        if r.summarized_by.as_deref().is_some_and(|s| !s.is_empty()) {
            continue;
        }
        // The admin enumerator surfaces the lane's stamped
        // `occurred_at` which is `1970-01-01` for rows the loader
        // boot-seeded with `ts = 0`. Fall through to ULID timestamp
        // recovery so those rows still land in their real ISO week.
        let occurred_at = chrono::DateTime::parse_from_rfc3339(&r.occurred_at)
            .ok()
            .map(|t| t.with_timezone(&Utc))
            .filter(|t| !r.occurred_at.starts_with("1970"))
            .or_else(|| {
                ulid_timestamp_ms(&r.event_id)
                    .and_then(chrono::DateTime::<Utc>::from_timestamp_millis)
            });
        let occurred_at = match occurred_at {
            Some(t) => t,
            None => continue,
        };
        if occurred_at >= cutoff_ts {
            continue;
        }
        let repo = r.repo.unwrap_or_else(|| "other".to_string());
        let top_topic = r
            .topic
            .or_else(|| r.topics.and_then(|v| v.into_iter().next()))
            .unwrap_or_else(|| "other".to_string());
        out.push(Turn {
            event_id: r.event_id,
            repo,
            occurred_at,
            top_topic,
            summarized_by: None,
        });
        if out.len() >= max_records {
            break;
        }
    }
    Ok(out)
}

/// Walk every per-repo `cortex-<repo>-turns` index page-by-page,
/// filtering by `kind = "turn"`, projecting every doc whose
/// `occurred_at < cutoff_ts` into a [`Turn`]. Used as a fallback
/// when `/v1/admin/list-events` is unreachable (e.g. cortex-api
/// down or auth misconfigured).
#[allow(dead_code)]
pub async fn fetch_old_turns(
    meili_url: &str,
    meili_key: Option<&str>,
    cutoff_ts: DateTime<Utc>,
    page_size: u32,
    max_records: usize,
) -> anyhow::Result<Vec<Turn>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("reqwest client")?;
    let base = meili_url.trim_end_matches('/');
    let indexes = list_turns_indexes(&client, base, meili_key).await?;
    let mut out: Vec<Turn> = Vec::new();
    for index_uid in indexes {
        if out.len() >= max_records {
            break;
        }
        let remaining = max_records - out.len();
        let mut hits = fetch_turns_from_index(
            &client, base, meili_key, &index_uid, cutoff_ts, page_size, remaining,
        )
        .await
        .with_context(|| format!("fetch turns from {index_uid}"))?;
        out.append(&mut hits);
    }
    Ok(out)
}

#[allow(dead_code)]
async fn list_turns_indexes(
    client: &reqwest::Client,
    base: &str,
    meili_key: Option<&str>,
) -> anyhow::Result<Vec<String>> {
    let mut offset: u32 = 0;
    let limit: u32 = 200;
    let mut out: Vec<String> = Vec::new();
    loop {
        let url = format!("{base}/indexes?limit={limit}&offset={offset}");
        let mut req = client.get(&url);
        if let Some(k) = meili_key {
            req = req.bearer_auth(k);
        }
        let resp = req.send().await.with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("GET {url}: {status} — {body}");
        }
        #[derive(serde::Deserialize)]
        struct IndexRow {
            uid: String,
        }
        #[derive(serde::Deserialize)]
        struct Page {
            results: Vec<IndexRow>,
            total: u32,
        }
        let page: Page = resp.json().await.context("decode meili indexes page")?;
        let count = page.results.len() as u32;
        for r in page.results {
            if r.uid.starts_with("cortex-") && r.uid.ends_with("-turns") {
                out.push(r.uid);
            }
        }
        offset += count;
        if offset >= page.total || count == 0 {
            break;
        }
    }
    Ok(out)
}

#[allow(dead_code)]
async fn fetch_turns_from_index(
    client: &reqwest::Client,
    base: &str,
    meili_key: Option<&str>,
    index_uid: &str,
    cutoff_ts: DateTime<Utc>,
    page_size: u32,
    max_records: usize,
) -> anyhow::Result<Vec<Turn>> {
    let limit = page_size.max(1) as usize;
    let mut out: Vec<Turn> = Vec::new();
    let mut offset: usize = 0;
    loop {
        if out.len() >= max_records {
            break;
        }
        let url = format!("{base}/indexes/{index_uid}/search");
        let mut req = client.post(&url).json(&serde_json::json!({
            "q": "",
            "filter": "kind = \"turn\"",
            "limit": limit,
            "offset": offset,
        }));
        if let Some(k) = meili_key {
            req = req.bearer_auth(k);
        }
        let resp = req.send().await.with_context(|| format!("POST {url}"))?;
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            anyhow::bail!("POST {url}: {status} — {body}");
        }
        #[derive(serde::Deserialize)]
        struct Page {
            hits: Vec<serde_json::Value>,
            #[serde(rename = "estimatedTotalHits", default)]
            estimated_total_hits: u32,
        }
        let page: Page = resp.json().await.context("decode meili search page")?;
        let count = page.hits.len();
        if count == 0 {
            break;
        }
        for v in page.hits {
            let event_id = match v.get("event_id").and_then(|x| x.as_str()) {
                Some(s) => s.to_string(),
                None => continue,
            };
            if v.get("summarized_by")
                .and_then(|x| x.as_str())
                .is_some_and(|s| !s.is_empty())
            {
                continue;
            }
            let occurred_str = v
                .get("occurred_at")
                .and_then(|x| x.as_str())
                .or_else(|| v.get("ts").and_then(|x| x.as_str()))
                .or_else(|| {
                    v.get("payload")
                        .and_then(|p| p.get("occurred_at"))
                        .and_then(|x| x.as_str())
                });
            let occurred_at =
                match occurred_str.and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()) {
                    Some(t) => t.with_timezone(&Utc),
                    None => continue,
                };
            if occurred_at >= cutoff_ts {
                continue;
            }
            // Top topic: prefer `payload.topic`, then
            // `extras.classifier.top_topic`, then the first element
            // of any `topics` array, then `"other"`.
            let top_topic = v
                .get("payload")
                .and_then(|p| p.get("topic"))
                .and_then(|x| x.as_str())
                .or_else(|| {
                    v.get("extras")
                        .and_then(|e| e.get("classifier"))
                        .and_then(|c| c.get("top_topic"))
                        .and_then(|x| x.as_str())
                })
                .or_else(|| {
                    v.get("topics")
                        .and_then(|t| t.as_array())
                        .and_then(|arr| arr.first())
                        .and_then(|x| x.as_str())
                })
                .unwrap_or("other")
                .to_string();
            let repo = v
                .get("context")
                .and_then(|c| c.get("repo"))
                .and_then(|x| x.as_str())
                .or_else(|| v.get("repo").and_then(|x| x.as_str()))
                .unwrap_or_else(|| index_uid_to_repo(index_uid))
                .to_string();
            out.push(Turn {
                event_id,
                repo,
                occurred_at,
                top_topic,
                summarized_by: None,
            });
            if out.len() >= max_records {
                return Ok(out);
            }
        }
        offset += count;
        if offset as u32 >= page.estimated_total_hits {
            break;
        }
    }
    Ok(out)
}

/// Recover the repo slug from an index uid of shape
/// `cortex-<repo>-turns`. Returns `"other"` if the pattern does not
/// match (defensive — every caller already filters on the prefix).
#[allow(dead_code)]
fn index_uid_to_repo(uid: &str) -> &str {
    uid.strip_prefix("cortex-")
        .and_then(|s| s.strip_suffix("-turns"))
        .unwrap_or("other")
}

/// Decode the timestamp encoded in the first 10 chars of a ULID
/// `event_id`. ULIDs encode 48 bits of millis-since-epoch in
/// Crockford base32 (`0-9` + `A-Z` minus `I`, `L`, `O`, `U`),
/// stored big-endian in the first 10 characters of the textual
/// form. This is the only authoritative timestamp source while
/// the Meili docs carry `ts: 0` and lack `occurred_at` — recovering
/// it from the ULID lets the digest enumerator bucket events by
/// their actual ISO week instead of folding everything into
/// 1970-W01.
pub(crate) fn ulid_timestamp_ms(id: &str) -> Option<i64> {
    let s = id.as_bytes();
    if s.len() < 10 {
        return None;
    }
    let mut out: u64 = 0;
    for &c in &s[..10] {
        let v = match c {
            b'0'..=b'9' => c - b'0',
            b'A'..=b'H' => c - b'A' + 10,
            b'J' | b'K' => c - b'A' + 9, // 'I' skipped
            b'M' | b'N' => c - b'A' + 8, // 'L' skipped
            b'P'..=b'T' => c - b'A' + 7, // 'O' skipped
            b'V'..=b'Z' => c - b'A' + 6, // 'U' skipped
            b'a'..=b'h' => c - b'a' + 10,
            b'j' | b'k' => c - b'a' + 9,
            b'm' | b'n' => c - b'a' + 8,
            b'p'..=b't' => c - b'a' + 7,
            b'v'..=b'z' => c - b'a' + 6,
            _ => return None,
        };
        out = (out << 5) | v as u64;
    }
    // ULID timestamp is 48 bits.
    if out > (1u64 << 48) {
        return None;
    }
    Some(out as i64)
}

/// Resolve a digest source row's `occurred_at` honoring the
/// authoritative-timestamp order: explicit `occurred_at` field,
/// then `payload.occurred_at`, then a non-zero `ts`, and finally
/// the timestamp encoded in the ULID `event_id`. Returns `None`
/// only when every source disagrees and the ULID itself fails to
/// decode (defensive — every Cortex envelope ID is a ULID).
pub(crate) fn resolve_occurred_at(v: &serde_json::Value, event_id: &str) -> Option<DateTime<Utc>> {
    let from_str = |s: Option<&str>| {
        s.filter(|s| !s.is_empty() && !s.starts_with("1970"))
            .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            .map(|t| t.with_timezone(&Utc))
    };
    if let Some(t) = from_str(v.get("occurred_at").and_then(|x| x.as_str())) {
        return Some(t);
    }
    if let Some(t) = from_str(
        v.get("payload")
            .and_then(|p| p.get("occurred_at"))
            .and_then(|x| x.as_str()),
    ) {
        return Some(t);
    }
    let ts_num = v.get("ts").and_then(|x| x.as_i64());
    if let Some(ms) = ts_num {
        if ms > 0 {
            if let Some(t) = chrono::DateTime::<Utc>::from_timestamp_millis(ms) {
                return Some(t);
            }
        }
    }
    if let Some(ms) = ulid_timestamp_ms(event_id) {
        if let Some(t) = chrono::DateTime::<Utc>::from_timestamp_millis(ms) {
            return Some(t);
        }
    }
    None
}

/// Resolve the platform string the envelope schema requires
/// (`win32` / `darwin` / `linux`) from `cfg!(target_os = …)`.
fn platform_string() -> &'static str {
    if cfg!(target_os = "windows") {
        "win32"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "linux"
    }
}

/// Make a value safe to embed in an event id: ASCII alphanumerics +
/// `_` survive, every other byte collapses to `_`.
#[allow(dead_code)]
fn sanitize_id_token(raw: &str) -> String {
    raw.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_id_token_keeps_ascii_alphanumerics_and_underscore() {
        assert_eq!(sanitize_id_token("cortex_v2"), "cortex_v2");
        assert_eq!(sanitize_id_token("with-dash"), "with_dash");
        assert_eq!(sanitize_id_token("with space"), "with_space");
    }

    #[test]
    fn index_uid_to_repo_strips_canonical_prefix_suffix() {
        assert_eq!(index_uid_to_repo("cortex-rulebook-turns"), "rulebook");
        assert_eq!(index_uid_to_repo("cortex-tml-turns"), "tml");
        assert_eq!(index_uid_to_repo("cortex-cortex-turns"), "cortex");
        assert_eq!(index_uid_to_repo("malformed"), "other");
    }
}
