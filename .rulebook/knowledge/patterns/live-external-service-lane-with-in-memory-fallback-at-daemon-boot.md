# Live external-service lane with in-memory fallback at daemon boot

**Category**: architecture
**Tags**: rust, trait-object, fallback, fail-open, lanes, cortex-api, spec-08, spec-11

## Description

When a Rust daemon depends on an external service (Meilisearch, Nexus, Vectorizer, etc.) but must stay usable on a cold dev stack, build a trait-object lane that probes the live service at startup and falls back to the existing in-memory test double on any failure. Both implementations stamp a single source-attribution invariant in `LaneHit.extras["source"]` so downstream code (RRF fusion, dashboards) can't tell the difference. A `debug_assert!` on the invariant catches future regressions before they reach prod.

## Example

// crates/cortex-api/src/main.rs — boot-time selection
let keyword: Arc<dyn KeywordLane> = if let Ok(url) = std::env::var("CORTEX_FULLTEXT_MEILI_URL") {
    match MeiliKeywordLane::new(&url, api_key) {
        Ok(live) => match live.probe().await {
            Ok(()) => Arc::new(live),
            Err(reason) => { tracing::warn!(reason = %reason, "fallback"); keyword_memory.clone() }
        },
        Err(reason) => { tracing::warn!(reason = %reason, "fallback"); keyword_memory.clone() }
    }
} else { keyword_memory.clone() };

// orchestrator.rs — invariant
debug_assert!(
    keyword_result.hits.iter().all(|h| h.extras.get("source").and_then(|v| v.as_str()) == Some("keyword")),
    "keyword lane returned a hit without extras[\"source\"] = \"keyword\""
);

## When to Use

Any orchestrator/service crate that fans out to lanes / clients / providers and currently uses an in-memory double in production (the `MemoryKeywordLane` shape — search method ignores the request).

## When NOT to Use

When the in-memory double is genuinely the production implementation (pure CPU work, no I/O), or when probe latency is unacceptable on the hot path.
