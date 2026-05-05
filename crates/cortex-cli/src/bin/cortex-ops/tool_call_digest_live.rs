//! phase11w live backend for `cortex-ops tool-call-digest`.
//!
//! Wires the in-process orchestrator from
//! [`cortex_workers::retention::tool_call_digest`] to three live
//! services:
//!
//! 1. **Meilisearch** — paginates the `cortex_tool_calls` index,
//!    materialises every doc whose `occurred_at < cutoff_ts` (and
//!    that has not been digested already) into a [`ToolCall`].
//! 2. **`cortex-ingestion /v1/events`** — receives one canonical
//!    `Memory{memory_type=tool_call_digest}` envelope per bucket.
//!    Embedder + Meili + Nexus pick it up downstream so the digest
//!    appears in every backend without per-bucket fan-out from this
//!    binary.
//! 3. **`cortex-api /v1/admin/forget`** — phase11t already owns the
//!    hard-purge cascade (Vectorizer `delete_vectors` + Meili
//!    `delete-batch` + Nexus `DETACH DELETE` + Parquet partition
//!    rewrite) behind the
//!    `I-UNDERSTAND-FORGET-IS-IRREVERSIBLE` confirmation token. The
//!    live backend's `delete_source_tool_calls` POSTs one request
//!    per source event id — the endpoint has no batch shape yet, so
//!    failures on individual ids are surfaced via stderr but do not
//!    abort the bucket; the bookkeeping row carries the partial
//!    count so the operator can re-run with the same plan to retry
//!    the gaps.

use std::time::Duration;

use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use cortex_workers::retention::tool_call_digest::{
    Bucket, DigestResult, ToolCall, ToolCallDigestBackend,
};

const FORGET_CONFIRMATION_TOKEN: &str = "I-UNDERSTAND-FORGET-IS-IRREVERSIBLE";
const MEILI_INDEX_MEMORIES: &str = "cortex_memories";

/// Live backend wiring Meili + cortex-api + cortex-ingestion.
pub struct LiveToolCallDigestBackend {
    /// Base URL for cortex-api (`/v1/admin/forget`,
    /// `/v1/admin/forget` again — every cascade-safe operation goes
    /// through here so we never bypass the audit trail).
    pub api_base: String,
    /// Optional bearer token for cortex-api when auth is enabled.
    pub api_token: Option<String>,
    /// Base URL for cortex-ingestion (`/v1/events`). The digest
    /// envelope lands here so the existing classifier → embedder →
    /// Meili / Vectorizer / Nexus fan-out delivers it to every
    /// backend without per-bucket plumbing in this binary.
    pub ingestion_base: String,
    /// Base URL for Meili — used by `lookup_existing` to short-
    /// circuit re-summarisation on previously-completed buckets.
    pub meili_base: String,
    /// Optional Meili API key.
    pub meili_key: Option<String>,
    /// Shared HTTP client.
    pub http: reqwest::Client,
}

impl LiveToolCallDigestBackend {
    /// Verify a freshly-persisted digest envelope is queryable in
    /// Meili before authorising the purge of source rows. The
    /// ingestion pipeline is async; polling the bucket's
    /// `tool_call_digest_key` confirms the summary is indexed.
    /// Returns `Ok(true)` once found, `Ok(false)` after timeout,
    /// `Err` on transport failure.
    ///
    /// **Safety contract** (per user instruction 2026-05-05):
    /// data may only be removed AFTER the summary lands. The
    /// `/v1/admin/forget` cascade MUST be gated on this returning
    /// `true`.
    pub async fn verify_digest_indexed(
        &self,
        repo: &str,
        year_week: &str,
        tool: &str,
        timeout: Duration,
    ) -> Result<bool, String> {
        let deadline = std::time::Instant::now() + timeout;
        let mut backoff_ms: u64 = 500;
        loop {
            match self.lookup_existing(repo, year_week, tool).await {
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

    /// Hard-purge source tool_call event ids via cortex-api
    /// `/v1/admin/forget`. Per-id failures surface via stderr but
    /// do not abort. Called from the binary handler AFTER
    /// `verify_digest_indexed` returns true.
    pub async fn delete_source_tool_calls_external(
        &self,
        event_ids: &[String],
    ) -> Result<u64, String> {
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
                        eprintln!("tool-call-digest: forget {event_id}: {status} {snippet}");
                    }
                }
                Err(e) => {
                    eprintln!("tool-call-digest: forget {event_id}: transport: {e}");
                }
            }
        }
        Ok(purged)
    }

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
}

#[async_trait]
impl ToolCallDigestBackend for LiveToolCallDigestBackend {
    async fn lookup_existing(
        &self,
        repo: &str,
        year_week: &str,
        tool: &str,
    ) -> Result<Option<String>, String> {
        // Query Meili `cortex_memories` for an entry whose
        // `extras.tool_call_digest_key` matches the bucket's
        // `(repo, year_week, tool)` triple. The persist path stamps
        // that exact key so a re-run of the same plan finds the
        // prior digest and short-circuits without burning a second
        // classifier round-trip.
        let bucket_key = format!("{repo}|{year_week}|{tool}");
        let url = format!("{}/indexes/{MEILI_INDEX_MEMORIES}/search", self.meili_base);
        let body = serde_json::json!({
            "q": "",
            "filter": format!("memory_type = \"tool_call_digest\" AND tool_call_digest_key = \"{bucket_key}\""),
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
        // Deterministic aggregation over the bucket's source rows.
        // The body MUST capture every retrieval-relevant signal a
        // future caller could query — `(repo, year_week, tool)` for
        // facet filters, the call count for volume sense, and a
        // sorted, deduplicated event-id list so an operator can
        // walk back to the original Parquet rows during forensic
        // inspection. No LLM round-trip here: the aggregate shape
        // alone is the retrieval payload, and skipping classifier
        // calls keeps the cron's per-run cost at zero.
        let mut ids: Vec<String> = bucket.event_ids.iter().cloned().collect();
        ids.sort();
        ids.dedup();
        let body = format!(
            "# Tool-call digest — `{repo}` · {week} · `{tool}`\n\n\
             Aggregates {count} `{tool}` calls observed in repo \
             `{repo}` during ISO week {week}. Original envelopes are \
             listed below for forensic round-trip; the summarisation \
             pass collapses them into this single Memory entry.\n\n\
             ## Counters\n\n\
             - calls: {count}\n\
             - repo: {repo}\n\
             - tool: {tool}\n\
             - week: {week}\n\n\
             ## Source event ids\n\n\
             {id_list}\n",
            repo = bucket.repo,
            week = bucket.year_week,
            tool = bucket.tool,
            count = ids.len(),
            id_list = ids
                .iter()
                .map(|s| format!("- `{s}`"))
                .collect::<Vec<_>>()
                .join("\n"),
        );
        // Tokens are accounted for telemetry parity; no external
        // call made so the cost surfaces as zero.
        let tokens_in = body.len() as u64 / 4;
        Ok(DigestResult {
            body,
            tokens_in,
            tokens_out: 0,
            usd_cents: 0,
        })
    }

    /// Build a minimum-viable Cortex envelope. The required fields
    /// (`schema_version`, `event_id`, `occurred_at`, `session_id`,
    /// `stream`, `tool`, `kind`, `context`, `payload`,
    /// `content_hash`) are populated from deterministic bucket
    /// data; the consolidation payload (phase11j §1) carries the
    /// digest body keyed by `(repo, year_week, tool)` via the
    /// `topic` grain so the existing pre-thinking renderer surfaces
    /// it alongside the other consolidation tiers.
    async fn persist_digest(
        &self,
        bucket: &Bucket,
        digest: &DigestResult,
    ) -> Result<String, String> {
        // POST one canonical envelope to cortex-ingestion; the
        // existing pipeline (classifier → embedder → Meili
        // `cortex_memories` + Vectorizer `cortex.memory.fp32` +
        // Nexus `:Memory` node + `(:Memory)-[:SUMMARIZES]->`
        // edges) carries it to every backend.
        let event_id = ulid::Ulid::new().to_string();
        let consolidation_id = ulid::Ulid::new().to_string();
        let session_id = ulid::Ulid::new().to_string();
        let now = Utc::now();
        let occurred_iso = now.to_rfc3339();
        // Build the payload first so its content_hash can be
        // derived deterministically from the rendered bytes.
        let bucket_key = format!("{}|{}|{}", bucket.repo, bucket.year_week, bucket.tool);
        let title_short = {
            let raw = format!(
                "{} {} calls — {} · {}",
                bucket.event_ids.len(),
                bucket.tool,
                bucket.repo,
                bucket.year_week,
            );
            let mut t: String = raw.chars().take(80).collect();
            if t.len() < 80 && raw.len() > t.len() {
                // ensure we did not over-trim; nothing else.
            }
            if t.is_empty() {
                t = format!("digest:{bucket_key}");
            }
            t
        };
        let mut summary = digest.body.clone();
        if summary.len() < 200 {
            // Pad with a deterministic footer so the schema's
            // 200-char minimum holds without losing information.
            let pad =
                "\n\n---\nThis digest aggregates the listed source events; \
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
            "model": "cortex-ops:tool-call-digest",
            "depth": "shallow",
            "temporal_span": {
                "start_ms": 0,
                "end_ms": now.timestamp_millis(),
                "duration_ms": now.timestamp_millis(),
            },
            "repos": [bucket.repo.clone()],
            "tags": ["tool_call_digest", bucket.tool.clone()],
        });
        let payload_bytes = serde_json::to_vec(&payload)
            .map_err(|e| format!("encode payload: {e}"))?;
        use sha2::Digest;
        let mut hasher = sha2::Sha256::new();
        hasher.update(&payload_bytes);
        let content_hash = format!("sha256:{:x}", hasher.finalize());

        let envelope = serde_json::json!({
            "event_id": event_id,
            "schema_version": "1.0.0",
            "occurred_at": occurred_iso,
            "session_id": session_id,
            "stream": "cortex.events.enriched",
            "tool": "cortex-ops:tool-call-digest",
            "kind": "consolidation",
            "context": { "repo": bucket.repo },
            "payload": payload,
            "content_hash": content_hash,
        });
        let url = format!("{}/v1/events", self.ingestion_base);
        let body = serde_json::json!({ "events": [envelope] });
        let resp = self
            .http
            .post(&url)
            .json(&body)
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

    async fn tag_source_tool_calls(
        &self,
        digest_event_id: &str,
        event_ids: &[String],
    ) -> Result<(), String> {
        // The orchestrator only invokes this branch when
        // `purge_originals = false` — i.e. the operator wants to
        // keep the source rows AND have re-runs short-circuit them.
        // The right place for that side-effect is the Parquet
        // partition rewriter that already lives behind
        // `cortex-api /v1/admin/forget` (phase11t §1) — but
        // `forget` deletes; there is no companion endpoint that
        // stamps `payload.summarized_by` without removing the row.
        //
        // Until that endpoint ships, refuse the request explicitly
        // so the operator picks one of the two viable production
        // modes: pair `--purge-originals` with `--apply` (clean
        // shrinkage), or run preview without `--apply` (no
        // mutation). Returning Err here means the bucket lands in
        // the report's `outcomes[*].error` slot rather than
        // silently doing the wrong thing.
        Err(format!(
            "tag_source_tool_calls: keeping originals + tagging requires the Parquet \
             rewriter; pair --apply with --purge-originals or use --dry-run \
             (digest_event_id={digest_event_id}, event_count={count})",
            count = event_ids.len()
        ))
    }

    async fn delete_source_tool_calls(
        &self,
        _event_ids: &[String],
    ) -> Result<u64, String> {
        // Neutralised at the trait level. The orchestrator must NOT
        // delete originals immediately after `persist_digest` because
        // the ingestion → classifier → embedder → fulltext-worker
        // chain is async; calling `/v1/admin/forget` before the
        // summary envelope is indexed would orphan originals if any
        // downstream stage drops the envelope. The binary handler
        // (digest.rs) implements the verify-then-purge cascade
        // outside the orchestrator using
        // [`Self::verify_digest_indexed`] +
        // [`Self::delete_source_tool_calls_external`]. This impl
        // returns `Ok(0)` so the orchestrator records the bucket
        // as "digested" and the report includes it; the actual
        // purge count surfaces from the external loop.
        //
        // Safety contract (user instruction 2026-05-05): "somente
        // pode remover os dados da memoria apos validar que foram
        // sumarizados ou consolidados".
        Ok(0)
    }
}

/// Make a value safe to embed in an event id without breaking the
/// `01PREFIX-…-…` convention: ASCII alphanumerics + `_` survive,
/// every other byte collapses to `_`.
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

/// Walk every per-repo `cortex-<repo>-code` index page-by-page,
/// filtering by `kind = "tool_call"`, projecting every doc whose
/// `occurred_at < cutoff_ts` into a [`ToolCall`]. Already-summarised
/// rows (`summarized_by` present) are excluded so re-runs converge.
///
/// The unified `cortex_tool_calls` index from `cortex_storage::names`
/// is declared but never actually populated by the live indexer
/// (`cortex_workers::fulltext::routing::family_for(Kind::ToolCall)`
/// returns `"code"`, so tool-call events land in
/// `cortex-<repo>-code` alongside source-code chunks). This helper
/// adapts to the deployed reality by enumerating every `*-code`
/// index and filtering server-side.
pub async fn fetch_old_tool_calls(
    meili_url: &str,
    meili_key: Option<&str>,
    cutoff_ts: DateTime<Utc>,
    page_size: u32,
    max_records: usize,
) -> anyhow::Result<Vec<ToolCall>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("reqwest client")?;
    let base = meili_url.trim_end_matches('/');
    let indexes = list_code_indexes(&client, base, meili_key).await?;
    let mut out: Vec<ToolCall> = Vec::new();
    for index_uid in indexes {
        if out.len() >= max_records {
            break;
        }
        let remaining = max_records - out.len();
        let mut hits = fetch_tool_calls_from_index(
            &client, base, meili_key, &index_uid, cutoff_ts, page_size, remaining,
        )
        .await
        .with_context(|| format!("fetch tool_calls from {index_uid}"))?;
        out.append(&mut hits);
    }
    Ok(out)
}

async fn list_code_indexes(
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
            if r.uid.starts_with("cortex-") && r.uid.ends_with("-code") {
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

async fn fetch_tool_calls_from_index(
    client: &reqwest::Client,
    base: &str,
    meili_key: Option<&str>,
    index_uid: &str,
    cutoff_ts: DateTime<Utc>,
    page_size: u32,
    max_records: usize,
) -> anyhow::Result<Vec<ToolCall>> {
    let limit = page_size.max(1) as usize;
    let mut out: Vec<ToolCall> = Vec::new();
    let mut offset: usize = 0;
    loop {
        if out.len() >= max_records {
            break;
        }
        let url = format!("{base}/indexes/{index_uid}/search");
        let mut req = client.post(&url).json(&serde_json::json!({
            "q": "",
            "filter": "kind = \"tool_call\"",
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
            let occurred_at = match occurred_str
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
            {
                Some(t) => t.with_timezone(&Utc),
                None => continue,
            };
            if occurred_at >= cutoff_ts {
                continue;
            }
            let tool = v
                .get("payload")
                .and_then(|p| p.get("tool"))
                .and_then(|x| x.as_str())
                .or_else(|| {
                    v.get("extras")
                        .and_then(|e| e.get("tool"))
                        .and_then(|x| x.as_str())
                })
                .or_else(|| v.get("tool").and_then(|x| x.as_str()))
                .unwrap_or("other")
                .to_string();
            let repo = v
                .get("context")
                .and_then(|c| c.get("repo"))
                .and_then(|x| x.as_str())
                .or_else(|| v.get("repo").and_then(|x| x.as_str()))
                .unwrap_or_else(|| index_uid_to_repo(index_uid))
                .to_string();
            out.push(ToolCall {
                event_id,
                repo,
                occurred_at,
                tool,
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
/// `cortex-<repo>-code`. Returns `"other"` if the pattern does not
/// match (defensive — every caller already filters on the prefix).
fn index_uid_to_repo(uid: &str) -> &str {
    uid.strip_prefix("cortex-")
        .and_then(|s| s.strip_suffix("-code"))
        .unwrap_or("other")
}

/// Walk the cortex-api admin lane projection
/// (`GET /v1/admin/list-events?kind=tool_call&before=<cutoff>&limit=N`)
/// and materialise every row into a [`ToolCall`].
///
/// The keyword lane the endpoint reads from is built at boot from
/// the parquet archive (real `occurred_at` timestamps) plus the
/// Meili indexes (where `ts = 0` is stamped on tool_call docs by
/// the loader). Falling back to the lane projection sidesteps the
/// Meili `ts = 0` shape — every tool_call row the lane carries was
/// stamped with a real epoch by the archive loader.
pub async fn fetch_old_tool_calls_via_admin(
    api_base: &str,
    api_token: Option<&str>,
    cutoff_ts: DateTime<Utc>,
    max_records: usize,
) -> anyhow::Result<Vec<ToolCall>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
        .context("reqwest client")?;
    // Manual percent-encoding for the RFC3339 timestamp's `:` and
    // `+` characters; everything else in the ISO-8601 form is
    // already query-safe ASCII.
    let cutoff_iso = cutoff_ts.to_rfc3339();
    let cutoff_enc = cutoff_iso.replace(':', "%3A").replace('+', "%2B");
    let url = format!(
        "{}/v1/admin/list-events?kind=tool_call&before={}&limit={}",
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
        tool: Option<String>,
        #[serde(default)]
        summarized_by: Option<String>,
    }
    let rows: Vec<Row> = resp
        .json()
        .await
        .context("decode /v1/admin/list-events response")?;
    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        if r.kind != "tool_call" {
            continue;
        }
        if r.summarized_by
            .as_deref()
            .is_some_and(|s| !s.is_empty())
        {
            continue;
        }
        // Recover real timestamp from the ULID when the lane stamped
        // `1970-01-01` on the row (boot-time `ts = 0` shape).
        let occurred_at = chrono::DateTime::parse_from_rfc3339(&r.occurred_at)
            .ok()
            .map(|t| t.with_timezone(&Utc))
            .filter(|_| !r.occurred_at.starts_with("1970"))
            .or_else(|| {
                super::turn_digest_live::ulid_timestamp_ms(&r.event_id)
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
        let tool = r.tool.unwrap_or_else(|| "other".to_string());
        out.push(ToolCall {
            event_id: r.event_id,
            repo,
            occurred_at,
            tool,
            summarized_by: None,
        });
        if out.len() >= max_records {
            break;
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_id_token_keeps_ascii_alphanumerics_and_underscore() {
        assert_eq!(sanitize_id_token("Bash"), "Bash");
        assert_eq!(sanitize_id_token("e--HiveLLM-Cortex"), "e__HiveLLM_Cortex");
        assert_eq!(sanitize_id_token("alpha bravo"), "alpha_bravo");
        assert_eq!(sanitize_id_token("repo/with:special"), "repo_with_special");
    }
}
