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

/// Cached credentials so the lane can transparently re-mint a JWT
/// when the upstream returns 401. Recorded on `with_login` only; the
/// `new(api_key=...)` path leaves this `None` because the caller
/// supplied the token directly and there is no flow we can re-run.
#[derive(Clone)]
struct LoginCreds {
    username: String,
    password: String,
}

/// Concrete `VectorLane` backed by a live Vectorizer instance.
///
/// Issue hivellm/cortex#2 — the JWT minted at boot expires after
/// ~1 hour. The lane stores the credentials it logged in with so a
/// 401 Unauthorized on `search_vectors` triggers one transparent
/// re-mint + retry; subsequent calls then carry the fresh JWT until
/// it expires again.
#[derive(Clone)]
pub struct VectorizerLane {
    client: Arc<tokio::sync::RwLock<Arc<VectorizerClient>>>,
    base_url: String,
    creds: Option<LoginCreds>,
}

impl std::fmt::Debug for VectorizerLane {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VectorizerLane")
            .field("base_url", &self.base_url)
            .field("auto_refresh", &self.creds.is_some())
            .finish_non_exhaustive()
    }
}

impl VectorizerLane {
    /// Build a new lane against `base_url` (e.g. `http://127.0.0.1:17001`).
    /// `api_key` is the Vectorizer JWT / X-API-Key (optional in
    /// no-auth dev). Wraps the SDK's `ClientConfig` rather than
    /// re-implementing transport — the same path
    /// `cortex-embedder-worker` uses for write traffic.
    pub fn new(base_url: impl Into<String>, api_key: Option<String>) -> Result<Self, String> {
        let base_url = base_url.into();
        let client = build_client(&base_url, api_key)?;
        Ok(Self {
            client: Arc::new(tokio::sync::RwLock::new(Arc::new(client))),
            base_url,
            creds: None,
        })
    }

    /// Build a lane after exchanging `(username, password)` for a JWT
    /// via the SDK's `/auth/login` endpoint. The same flow
    /// `cortex-embedder-worker` runs at boot when its
    /// `vectorizer_password` is not already a JWT. The credentials
    /// are kept in-memory so the lane can re-mint the JWT whenever
    /// the Vectorizer returns 401 — see `Self::search` for the
    /// refresh-and-retry path.
    pub async fn with_login(
        base_url: impl Into<String>,
        username: &str,
        password: &str,
    ) -> Result<Self, String> {
        let base_url = base_url.into();
        let jwt = mint_jwt(&base_url, username, password).await?;
        let client = build_client(&base_url, Some(jwt))?;
        Ok(Self {
            client: Arc::new(tokio::sync::RwLock::new(Arc::new(client))),
            base_url,
            creds: Some(LoginCreds {
                username: username.to_string(),
                password: password.to_string(),
            }),
        })
    }

    /// Probe `/health` so the caller can decide whether to swap in
    /// the lane or fall back to `MemoryVectorLane`. Returns `Ok(())`
    /// only when the SDK's `health_check` succeeds.
    pub async fn probe(&self) -> Result<(), String> {
        let client = self.client.read().await.clone();
        client
            .health_check()
            .await
            .map(|_| ())
            .map_err(|e| format!("probe {}: {e}", self.base_url))
    }

    /// Force a JWT refresh — exposed for tests and for the daemon's
    /// optional periodic warmup. Returns `Err` when no credentials
    /// were captured (the `new(api_key=...)` path) or the upstream
    /// `/auth/login` rejects them.
    pub async fn refresh_token(&self) -> Result<(), String> {
        let creds = self
            .creds
            .as_ref()
            .ok_or_else(|| "refresh_token: lane has no cached credentials".to_string())?;
        let jwt = mint_jwt(&self.base_url, &creds.username, &creds.password).await?;
        let new_client = build_client(&self.base_url, Some(jwt))?;
        let mut w = self.client.write().await;
        *w = Arc::new(new_client);
        Ok(())
    }
}

fn build_client(base_url: &str, api_key: Option<String>) -> Result<VectorizerClient, String> {
    let cfg = ClientConfig {
        base_url: Some(base_url.to_string()),
        api_key,
        timeout_secs: Some(10),
        ..Default::default()
    };
    VectorizerClient::new(cfg).map_err(|e| format!("vectorizer-sdk client: {e}"))
}

async fn mint_jwt(base_url: &str, username: &str, password: &str) -> Result<String, String> {
    // Build a transient client purely to run `/auth/login`. The SDK
    // requires an instance to call the method; we discard it after
    // pulling out the JWT.
    let login_cfg = ClientConfig {
        base_url: Some(base_url.to_string()),
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
    Ok(jwt.access_token)
}

/// Match the SDK error message against the HTTP status codes that
/// indicate the cached JWT is no longer accepted (`401`, `403`, or
/// the literal "unauthorized" / "expired" tokens). Strings come from
/// `VectorizerError::Server { message }` which today renders as
/// `"Server error: HTTP 401 Unauthorized: {...}"`.
fn looks_like_auth_failure(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("401")
        || lower.contains("403")
        || lower.contains("unauthorized")
        || lower.contains("expired")
        || lower.contains("invalid token")
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
        let client = self.client.read().await.clone();
        let attempt = client
            .search_vectors(&req.collection, &req.query, Some(req.k), None)
            .await;

        let resp = match attempt {
            Ok(r) => r,
            Err(e) => {
                let msg =
                    format!("{}: search_vectors({}): {e}", self.base_url, req.collection);
                let lower = msg.to_ascii_lowercase();
                // 404 on the per-project collection is the legitimate
                // empty-index case (the spec-06 worker materialises
                // collections lazily on first upsert). The SDK
                // surfaces these as `VectorizerError::server` with
                // a "not found" message — fall through to empty
                // hits rather than failing the whole orchestrator
                // turn.
                if lower.contains("not found") || lower.contains("404") {
                    return Ok(Vec::new());
                }
                // Issue hivellm/cortex#2 — the boot-time JWT expires
                // after ~1 h and every subsequent search returns
                // 401. Re-mint the token using the cached creds and
                // retry once; if the refresh path is unavailable
                // (no creds, or `/auth/login` rejected the retry) we
                // still surface the 401 so the orchestrator's
                // `debug.errors.vector` lane carries an actionable
                // signal.
                if looks_like_auth_failure(&msg) {
                    if self.creds.is_some() {
                        match self.refresh_token().await {
                            Ok(()) => {
                                tracing::info!(
                                    base_url = %self.base_url,
                                    collection = %req.collection,
                                    "vector lane: refreshed JWT after upstream 401, retrying"
                                );
                                let client2 = self.client.read().await.clone();
                                match client2
                                    .search_vectors(
                                        &req.collection,
                                        &req.query,
                                        Some(req.k),
                                        None,
                                    )
                                    .await
                                {
                                    Ok(r) => r,
                                    Err(e2) => {
                                        return Err(LaneError::Transport(format!(
                                            "{msg}; refresh-retry failed: {e2}"
                                        )));
                                    }
                                }
                            }
                            Err(reason) => {
                                return Err(LaneError::Transport(format!(
                                    "{msg}; auth refresh failed: {reason}"
                                )));
                            }
                        }
                    } else {
                        return Err(LaneError::Transport(format!(
                            "{msg}; no cached credentials to mint a fresh JWT — set \
                             CORTEX_VECTORIZER_USER + _PASSWORD (or _EMBEDDER_VECTORIZER_*)"
                        )));
                    }
                } else {
                    return Err(LaneError::Transport(msg));
                }
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

    #[test]
    fn auth_failure_detector_matches_known_upstream_strings() {
        // Issue hivellm/cortex#2 — the SDK renders 401 errors as the
        // string `"Server error: HTTP 401 Unauthorized: ..."`. The
        // detector must catch every variant the upstream emits so
        // the auto-refresh path fires whenever the cached JWT is
        // rejected.
        assert!(super::looks_like_auth_failure(
            "Server error: HTTP 401 Unauthorized: {\"error\":\"unauthorized\"}"
        ));
        assert!(super::looks_like_auth_failure("HTTP 403 Forbidden"));
        assert!(super::looks_like_auth_failure("token expired"));
        assert!(super::looks_like_auth_failure("Invalid token: signature mismatch"));
        // Negative — anything else (404, 500, transport noise) keeps
        // the existing not-found / generic-transport paths.
        assert!(!super::looks_like_auth_failure("HTTP 404 Not Found"));
        assert!(!super::looks_like_auth_failure(
            "tcp connect timeout after 10s"
        ));
    }
}
