# vectorizer-sdk SearchResult silently drops the live server's `payload` and `vector` fields

**Category**: integration
**Tags**: none

## Description

Drift between `vectorizer-sdk` 3.0.x and the `hivehub/vectorizer:3.0.x` live image, specifically on the **read path** of `POST /collections/{c}/search/text`. Same family as the documented write-path drifts (`insert_texts`, `get_vector`).

**Wire shape mismatch:**

```text
Live server returns:  { id, score, vector: [...], payload: { path, body, kind, repo, ... } }
SDK's SearchResult:   { id, score, content: Option<String>, metadata: Option<HashMap> }
```

`serde` tolerantly skips unknown fields → the SDK deserialises every hit as `SearchResult { content: None, metadata: None }`, dropping every projection-relevant field on the floor. cortex-api's lane projection then reads `metadata.get("path")` etc., produces `LaneHit { text: "", path: None, ... }`, and the bundle renderer drops the empty hits silently. Symptom: vector_ms > 0 (lane runs), but every snippet that surfaces is from the keyword lane.

**Fix (phase11d):** bypass the SDK's `search_vectors` for the read path. Use a direct `reqwest::Client` POST to `/collections/{c}/search/text`, deserialising into a local `WireSearchHit { id, score, vector, payload }` that matches the actual wire shape. Auth and probes (`probe_authenticated`, `refresh_token`, `health_check`, `/auth/login`) stay on the SDK because their wire shapes already match. Same approach the embedder already adopted for the write path drift.

**Detection:** the giveaway is `source-mix: { keyword: N, vector: 0 }` even after the auth fix (phase11a) makes vector_ms > 0. The vector lane is calling the server, getting hits, but every projected `LaneHit` has empty text — so RRF fusion ranks them last and the renderer drops them. A direct curl against `/collections/{c}/search/text` with the cached JWT shows the rich `payload` the SDK is dropping.

**Forward-compatible:** when SDK 3.1+ aligns its `SearchResult` shape with the live server (or the server changes back to `metadata` / `content`), revisit the bypass. Until then, the direct reqwest path is the canonical surface — it's the same pattern `cortex-embedder` follows for `insert_texts`.

## Example

// Anti-pattern (pre-phase11d):
let resp = client.search_vectors(&req.collection, &req.query, Some(req.k), None).await?;
for r in resp.results {
    let hit = LaneHit {
        path: r.metadata.and_then(|m| m.get("path"))...,  // always None
        text: r.content.unwrap_or_default(),              // always ""
        ...
    };
}

// Fix (phase11d):
#[derive(serde::Deserialize)]
struct WireSearchHit {
    #[serde(default)] id: String,
    #[serde(default)] score: f32,
    #[serde(default)] payload: serde_json::Map<String, serde_json::Value>,
    #[serde(default)] vector: Option<Vec<f32>>,
}
let body = serde_json::json!({ "query": req.query, "limit": req.k });
let resp = http.post(&url).bearer_auth(jwt).json(&body).send().await?;
let parsed: WireSearchResponse = resp.json().await?;
for r in parsed.results {
    let path = r.payload.get("path").and_then(|v| v.as_str())...;
    let text = r.payload.get("body").and_then(|v| v.as_str())...;
}

## When to Use

Any cortex-api / cortex-* read path that consumes `vectorizer-sdk` 3.0.x's `SearchResult` against the live `hivehub/vectorizer:3.0.x` image. The drift applies uniformly across `search_vectors`, `intelligent_search`, `semantic_search`, `contextual_search`, `hybrid_search` — they all return `SearchResponse { results: Vec<SearchResult> }`. Bypass each one that you actually call.</whenToUse>
<parameter name="whenNotToUse">When the SDK ships a version (3.1+) whose `SearchResult` matches the live server's `payload`/`vector` shape (`serde(alias)` or rename). Drop the bypass and use the SDK directly. Also skip the bypass on test doubles (`MemoryVectorLane`) — they don't go through HTTP at all.</parameter>
<parameter name="tags">["phase11d", "vectorizer", "vectorizer-sdk", "wire-shape", "drift", "search_vectors", "payload", "cortex-api", "reqwest", "anti-pattern"]
