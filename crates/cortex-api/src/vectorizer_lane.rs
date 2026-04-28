//! Live Vectorizer-backed `VectorLane`.
//!
//! `MemoryVectorLane` ships as a test double — its hits are whatever
//! the orchestrator's strategies layer happened to seed under a
//! collection alias, with no embedding ever computed. The 2026-04-27
//! audit logged `debug.lanes.vector_ms = 0` on every probe and the
//! "vector" snippets surfacing under that label were keyword-lane
//! hits the orchestrator's `lane_label()` fallback mislabelled.
//!
//! This module ships the production read-path: per-query semantic
//! search against the same per-project Vectorizer collections the
//! spec-06 embedder-worker upserts to. Translates the orchestrator's
//! `VectorRequest { collection, query, k, scope }` into the Vectorizer
//! SDK's `search_vectors(collection, query, limit, threshold)` and
//! maps each `SearchResult` back into a `LaneHit`.
//!
//! The lane uses the official `vectorizer-sdk` crate end-to-end —
//! same dependency `cortex-embedder-worker` ships with — per the
//! "use Hive SDKs, don't reimplement" memory rule.

use std::sync::Arc;

use async_trait::async_trait;
use vectorizer_sdk::{ClientConfig, VectorizerClient};

use crate::lanes::{LaneError, LaneHit, VectorLane, VectorRequest};

/// Concrete `VectorLane` backed by a live Vectorizer instance.
#[derive(Clone)]
pub struct VectorizerLane {
    client: Arc<VectorizerClient>,
    base_url: String,
}

impl std::fmt::Debug for VectorizerLane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VectorizerLane")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl VectorizerLane {
    /// Build a new lane against `base_url` (e.g. `http://127.0.0.1:15001`).
    /// `api_key` is the Vectorizer JWT / X-API-Key (optional in
    /// no-auth dev). Wraps the SDK's `ClientConfig` rather than
    /// re-implementing transport — the same path
    /// `cortex-embedder-worker` uses for write traffic.
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Result<Self, String> {
        let base_url = base_url.into();
        let cfg = ClientConfig {
            base_url: Some(base_url.clone()),
            api_key,
            timeout_secs: Some(10),
            ..Default::default()
        };
        let client = VectorizerClient::new(cfg)
            .map_err(|e| format!("vectorizer-sdk client: {e}"))?;
        Ok(Self {
            client: Arc::new(client),
            base_url,
        })
    }

    /// Build a lane after exchanging `(username, password)` for a JWT
    /// via the SDK's `/auth/login` endpoint. The same flow
    /// `cortex-embedder-worker` runs at boot when its
    /// `vectorizer_password` is not already a JWT. Returns the same
    /// lane shape as [`Self::new`], with the minted JWT carried as
    /// the SDK's `api_key`.
    pub async fn with_login(
        base_url: impl Into<String>,
        username: &str,
        password: &str,
    ) -> Result<Self, String> {
        let base_url = base_url.into();
        // Build a transient client purely to run `/auth/login`. The
        // SDK requires an instance to call the method; we discard
        // it after pulling out the JWT.
        let login_cfg = ClientConfig {
            base_url: Some(base_url.clone()),
            api_key: None,
            timeout_secs: Some(10),
            ..Default::default()
        };
        let login_client = VectorizerClient::new(login_cfg)
            .map_err(|e| format!("vectorizer-sdk login client: {e}"))?;
        let jwt = login_client
            .login(username, password)
            .await
            .map_err(|e| format!("/auth/login {base_url}: {e}"))?;
        Self::new(base_url, Some(jwt.access_token))
    }

    /// Probe `/health` so the caller can decide whether to swap in
    /// the lane or fall back to `MemoryVectorLane`. Returns `Ok(())`
    /// only when the SDK's `health_check` succeeds.
    pub async fn probe(&self) -> Result<(), String> {
        self.client
            .health_check()
            .await
            .map(|_| ())
            .map_err(|e| format!("probe {}: {e}", self.base_url))
    }
}

#[async_trait]
impl VectorLane for VectorizerLane {
    async fn search(&self, req: &VectorRequest) -> Result<Vec<LaneHit>, LaneError> {
        // The SDK's `search_vectors` is the right surface for our
        // shape: collection-scoped text query, server-side embedding
        // (so we don't have to keep an embedder client open in
        // cortex-api), top-k cap. The richer surfaces (semantic /
        // intelligent / hybrid) layer reranking + multi-query
        // expansion that the orchestrator's RRF fusion already does
        // — running them here would be redundant work plus a second
        // ranking signal the fusion stage doesn't know about.
        let resp = match self
            .client
            .search_vectors(&req.collection, &req.query, Some(req.k), None)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                let msg = format!("{}: search_vectors({}): {e}", self.base_url, req.collection);
                // 404 on the per-project collection is the legitimate
                // empty-index case (the spec-06 worker materialises
                // collections lazily on first upsert). The SDK
                // surfaces these as `VectorizerError::server` with
                // a "not found" message — fall through to empty
                // hits rather than failing the whole orchestrator
                // turn.
                let lower = msg.to_ascii_lowercase();
                if lower.contains("not found") || lower.contains("404") {
                    return Ok(Vec::new());
                }
                return Err(LaneError::Transport(msg));
            }
        };

        let hits = resp
            .results
            .into_iter()
            .map(|r| project(r, req))
            .collect();
        Ok(hits)
    }
}

/// Project one Vectorizer search result into a `LaneHit`. Stamps
/// `extras["source"] = "vector"` so the orchestrator's
/// source-attribution invariant is met (the keyword-lane fix
/// flipped the default; both lanes now stamp explicitly).
fn project(r: vectorizer_sdk::models::SearchResult, req: &VectorRequest) -> LaneHit {
    let metadata = r.metadata.unwrap_or_default();
    let get_str =
        |key: &str| -> Option<String> { metadata.get(key).and_then(|v| v.as_str()).map(String::from) };
    let get_i64 = |key: &str| -> Option<i64> {
        metadata
            .get(key)
            .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
    };

    let mut extras = std::collections::BTreeMap::new();
    extras.insert(
        "source".to_string(),
        serde_json::Value::String("vector".to_string()),
    );
    extras.insert(
        "collection".to_string(),
        serde_json::Value::String(req.collection.clone()),
    );

    let text = r
        .content
        .filter(|s| !s.is_empty())
        .or_else(|| get_str("summary"))
        .or_else(|| get_str("title"))
        .or_else(|| get_str("body"))
        .unwrap_or_default();

    LaneHit {
        doc_id: format!("vec|{}|{}", req.collection, r.id),
        text,
        repo: get_str("repo"),
        path: get_str("path"),
        symbol: get_str("kind"),
        content_hash: get_str("content_hash"),
        score: r.score as f64,
        ts: get_i64("ts").unwrap_or(0),
        severity: get_str("severity"),
        extras,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Scope;
    use std::collections::HashMap;
    use vectorizer_sdk::models::SearchResult;

    fn req(query: &str) -> VectorRequest {
        VectorRequest {
            collection: "cortex-cortex-code".into(),
            query: query.into(),
            k: 5,
            scope: Scope::default(),
        }
    }

    #[test]
    fn projects_a_search_result_with_metadata_into_a_lane_hit() {
        let mut metadata = HashMap::new();
        metadata.insert("repo".to_string(), serde_json::json!("Cortex"));
        metadata.insert("path".to_string(), serde_json::json!("src/lib.rs"));
        metadata.insert("kind".to_string(), serde_json::json!("turn"));
        metadata.insert("content_hash".to_string(), serde_json::json!("sha256:abc"));
        metadata.insert("ts".to_string(), serde_json::json!(1714200000000_i64));

        let r = SearchResult {
            id: "vec-1".into(),
            score: 0.91,
            content: Some("semantic body".into()),
            metadata: Some(metadata),
        };

        let hit = project(r, &req("embedder"));
        assert_eq!(hit.doc_id, "vec|cortex-cortex-code|vec-1");
        assert_eq!(hit.text, "semantic body");
        assert_eq!(hit.repo.as_deref(), Some("Cortex"));
        assert_eq!(hit.symbol.as_deref(), Some("turn"));
        assert!((hit.score - 0.91).abs() < 1e-6);
        assert_eq!(hit.ts, 1714200000000);
        assert_eq!(
            hit.extras.get("source").and_then(|v| v.as_str()),
            Some("vector")
        );
        assert_eq!(
            hit.extras.get("collection").and_then(|v| v.as_str()),
            Some("cortex-cortex-code")
        );
    }

    #[test]
    fn projects_falls_back_to_metadata_text_when_content_missing() {
        let mut metadata = HashMap::new();
        metadata.insert("summary".to_string(), serde_json::json!("curated summary"));
        metadata.insert("body".to_string(), serde_json::json!("raw body"));

        let r = SearchResult {
            id: "vec-2".into(),
            score: 0.5,
            content: None,
            metadata: Some(metadata),
        };
        let hit = project(r, &req("x"));
        assert_eq!(hit.text, "curated summary");
    }

    #[test]
    fn projects_emits_empty_text_only_when_no_text_anywhere() {
        let r = SearchResult {
            id: "vec-3".into(),
            score: 0.0,
            content: None,
            metadata: None,
        };
        let hit = project(r, &req("x"));
        assert_eq!(hit.text, "");
        assert_eq!(hit.score, 0.0);
        assert_eq!(hit.ts, 0);
    }
}
