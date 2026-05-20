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
//! `VectorRequest { collection, query, k, scope }` into a Vectorizer
//! `POST /collections/{c}/search/text` call and maps each result back
//! into a `LaneHit`.
//!
//! ## phase11d — direct HTTP for the read path
//!
//! The lane previously delegated `search_vectors` to `vectorizer-sdk`.
//! The SDK's `SearchResult { content: Option<String>, metadata:
//! Option<HashMap> }` does NOT match the live server's wire shape —
//! the server responds with `{id, score, vector, payload}`, where
//! `payload` carries every projection-relevant field (`path`, `kind`,
//! `repo`, `body`, ...). `serde` tolerantly skipped `payload` and
//! `vector`, leaving `SearchResult { content: None, metadata: None }`
//! on every hit. The projection then read empty path/text and the
//! orchestrator's bundle renderer dropped the hits silently.
//!
//! Fix: bypass the SDK's `search_vectors` and POST direct via
//! `reqwest`, deserialising the real wire shape into [`WireSearchHit`].
//! Auth (`probe_authenticated`, `refresh_token`, `health_check`,
//! `/auth/login`) stays on the SDK — those endpoints' wire shapes
//! already match.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Deserialize;
use vectorizer_sdk::{ClientConfig, VectorizerClient};

use crate::lanes::{
    collection_missing_marker, collection_missing_report_enabled, LaneError, LaneHit, VectorLane,
    VectorRequest,
};

/// phase11d — wire shape of one entry in `POST /collections/{c}/search/text`'s
/// `results` array on the live Vectorizer image. `serde(default)` on
/// every field so a missing key never fails deserialisation; absent
/// fields collapse to `None` / empty maps.
#[derive(Debug, Clone, Deserialize)]
pub(crate) struct WireSearchHit {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub score: f32,
    /// Server-side payload. Carries `path`, `kind`, `repo`, `body`,
    /// `summary`, `title`, `severity`, `ts`, `topics`, every spec-11
    /// projection-contract key, etc. Empty map when the upstream
    /// response omitted the field.
    #[serde(default)]
    pub payload: serde_json::Map<String, serde_json::Value>,
    /// Raw embedding. Deserialised but immediately dropped — the
    /// projection doesn't use it. Kept here so the type round-trips
    /// the full wire shape and the `vector` field doesn't get lost
    /// to an unknown-field warning if we ever turn `deny_unknown_fields`
    /// on.
    #[serde(default)]
    #[allow(dead_code)]
    pub vector: Option<Vec<f32>>,
}

/// phase11d — wire envelope for `POST /collections/{c}/search/text`.
#[derive(Debug, Clone, Deserialize)]
struct WireSearchResponse {
    #[serde(default)]
    results: Vec<WireSearchHit>,
}

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
///
/// phase11d — the search read-path is now a direct `reqwest` POST
/// to `/collections/{c}/search/text` (the SDK's `search_vectors`
/// silently dropped the server's `payload` field). Auth and probes
/// stay on the SDK because their wire shapes already match.
#[derive(Clone)]
pub struct VectorizerLane {
    client: Arc<tokio::sync::RwLock<Arc<VectorizerClient>>>,
    /// phase11d — direct HTTP transport for the read path. Distinct
    /// from the SDK's internal transport so the search code-path
    /// never sees the SDK's `SearchResult` deserializer.
    http: reqwest::Client,
    /// phase11d — current bearer token (JWT or static API key) for
    /// the direct HTTP call. Mirrors whatever the SDK client was
    /// last built with. `refresh_token()` updates both this and the
    /// SDK client so the next search and the next SDK call carry
    /// the fresh credential.
    bearer: Arc<tokio::sync::RwLock<Option<String>>>,
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
        let client = build_client(&base_url, api_key.clone())?;
        let http = build_http_client()?;
        Ok(Self {
            client: Arc::new(tokio::sync::RwLock::new(Arc::new(client))),
            http,
            bearer: Arc::new(tokio::sync::RwLock::new(api_key)),
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
        let client = build_client(&base_url, Some(jwt.clone()))?;
        let http = build_http_client()?;
        Ok(Self {
            client: Arc::new(tokio::sync::RwLock::new(Arc::new(client))),
            http,
            bearer: Arc::new(tokio::sync::RwLock::new(Some(jwt))),
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
    ///
    /// `/health` is unauthenticated on the live Vectorizer image, so a
    /// successful probe says **only** "the server is reachable" — it
    /// does NOT say "the credentials we cached are accepted". Use
    /// [`probe_authenticated`] at boot to catch a misconfigured-creds
    /// stack before the first real `search_vectors` call.
    ///
    /// [`probe_authenticated`]: Self::probe_authenticated
    pub async fn probe(&self) -> Result<(), String> {
        let client = self.client.read().await.clone();
        client
            .health_check()
            .await
            .map(|_| ())
            .map_err(|e| format!("probe {}: {e}", self.base_url))
    }

    /// Test-only constructor combining a hand-crafted initial JWT
    /// with cached login credentials. Lets the integration tests
    /// drive the refresh-and-retry path against a `wiremock`
    /// Vectorizer double — the public `with_login` would consume
    /// one `/auth/login` call up front and leave the test unable to
    /// distinguish the boot-time login from the post-401 refresh.
    #[doc(hidden)]
    pub fn with_initial_jwt_for_test(
        base_url: impl Into<String>,
        initial_jwt: &str,
        username: &str,
        password: &str,
    ) -> Self {
        let base_url = base_url.into();
        let client = build_client(&base_url, Some(initial_jwt.to_string()))
            .expect("test client build must succeed");
        let http = build_http_client().expect("test reqwest builder must succeed");
        Self {
            client: Arc::new(tokio::sync::RwLock::new(Arc::new(client))),
            http,
            bearer: Arc::new(tokio::sync::RwLock::new(Some(initial_jwt.to_string()))),
            base_url,
            creds: Some(LoginCreds {
                username: username.to_string(),
                password: password.to_string(),
            }),
        }
    }

    /// Phase11a — run one cheap **authenticated** round-trip
    /// (`list_collections`) so the caller can prove the cached JWT /
    /// API key is actually accepted before declaring the lane live.
    ///
    /// `/health` (used by [`probe`]) does not require auth on the
    /// live Vectorizer image, so `probe()` returning `Ok` only proves
    /// the server is reachable. Without this stronger probe, a
    /// daemon booted with no credentials (or wrong credentials) wires
    /// the live lane successfully and only surfaces the failure on
    /// the first real `/v1/query` call as `errors.vector = "...HTTP 401..."`.
    ///
    /// On the first 401 with cached credentials, this method runs one
    /// transparent [`refresh_token`] + retry — same recovery shape as
    /// the per-call refresh in [`Self::search`]. A persistent 401
    /// (or any non-401 transport failure) propagates so the boot path
    /// can fall back to `MemoryVectorLane` and log loudly.
    ///
    /// [`probe`]: Self::probe
    /// [`refresh_token`]: Self::refresh_token
    pub async fn probe_authenticated(&self) -> Result<(), String> {
        let client = self.client.read().await.clone();
        let attempt = client.list_collections().await;
        match attempt {
            Ok(_) => Ok(()),
            Err(e) => {
                let msg = format!("probe_authenticated {}: {e}", self.base_url);
                if !looks_like_auth_failure(&msg) {
                    return Err(msg);
                }
                // 401 — try one refresh + retry, mirroring the
                // per-call recovery in `search`.
                if self.creds.is_none() {
                    return Err(format!(
                        "{msg}; no cached credentials to mint a fresh JWT — set \
                         CORTEX_VECTORIZER_USER + _PASSWORD (or _EMBEDDER_VECTORIZER_*)"
                    ));
                }
                if let Err(reason) = self.refresh_token().await {
                    return Err(format!("{msg}; auth refresh failed: {reason}"));
                }
                tracing::info!(
                    base_url = %self.base_url,
                    "vector lane: refreshed JWT after probe_authenticated 401, retrying"
                );
                let client2 = self.client.read().await.clone();
                client2
                    .list_collections()
                    .await
                    .map(|_| ())
                    .map_err(|e2| format!("{msg}; refresh-retry failed: {e2}"))
            }
        }
    }

    /// Force a JWT refresh — exposed for tests and for the daemon's
    /// optional periodic warmup. Returns `Err` when no credentials
    /// were captured (the `new(api_key=...)` path) or the upstream
    /// `/auth/login` rejects them.
    ///
    /// Updates both the SDK client (used by `probe_authenticated`,
    /// `health_check`, `list_collections`) AND the cached bearer
    /// (used by the direct-HTTP read path in `VectorLane::search`)
    /// so the next call on either path carries the fresh JWT.
    pub async fn refresh_token(&self) -> Result<(), String> {
        let creds = self
            .creds
            .as_ref()
            .ok_or_else(|| "refresh_token: lane has no cached credentials".to_string())?;
        let jwt = mint_jwt(&self.base_url, &creds.username, &creds.password).await?;
        let new_client = build_client(&self.base_url, Some(jwt.clone()))?;
        {
            let mut w = self.client.write().await;
            *w = Arc::new(new_client);
        }
        {
            let mut b = self.bearer.write().await;
            *b = Some(jwt);
        }
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

/// phase11d — build the `reqwest::Client` used by the direct read
/// path. Same 10 s timeout the SDK wires its transport with so the
/// fail-open behaviour matches.
fn build_http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| format!("vectorizer reqwest builder: {e}"))
}

/// phase11d — outcome of one `direct_search` round-trip. The
/// `Auth401` arm is the only branch that triggers the lane's
/// refresh-and-retry; every other failure surfaces as `Transport`
/// so the orchestrator can record it under `debug.errors.vector`.
enum DirectSearchOutcome {
    Ok(Vec<WireSearchHit>),
    NotFound,
    Auth401(String),
    Transport(String),
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

impl VectorizerLane {
    /// phase11d — direct `POST /collections/{c}/search/text` against
    /// the live Vectorizer image. Replaces `vectorizer-sdk`'s
    /// `search_vectors` because the SDK's `SearchResult` deserializer
    /// silently drops the server's `payload` and `vector` fields
    /// (anti-pattern documented in the rulebook knowledge base —
    /// same family as the embedder's write-path drift).
    ///
    /// Classifies the outcome into one of four [`DirectSearchOutcome`]
    /// variants so the caller can drive the existing 401 → refresh →
    /// retry flow without re-parsing the upstream error.
    async fn direct_search(&self, req: &VectorRequest) -> DirectSearchOutcome {
        let url = format!(
            "{}/collections/{}/search/text",
            self.base_url.trim_end_matches('/'),
            req.collection,
        );
        let body = serde_json::json!({
            "query": req.query,
            "limit": req.k,
        });
        let mut http_req = self.http.post(&url).json(&body);
        if let Some(token) = self.bearer.read().await.clone() {
            http_req = http_req.bearer_auth(token);
        }
        let resp = match http_req.send().await {
            Ok(r) => r,
            Err(e) => {
                return DirectSearchOutcome::Transport(format!(
                    "{}: search_vectors({}): {e}",
                    self.base_url, req.collection
                ));
            }
        };
        let status = resp.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            // 404 on the per-project collection is the legitimate
            // empty-index case (the spec-06 worker materialises
            // collections lazily on first upsert). Fall through to
            // empty hits rather than failing the whole orchestrator
            // turn.
            return DirectSearchOutcome::NotFound;
        }
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            let body_text = resp.text().await.unwrap_or_default();
            return DirectSearchOutcome::Auth401(format!(
                "{}: search_vectors({}): HTTP {} {}",
                self.base_url,
                req.collection,
                status.as_u16(),
                body_text.chars().take(200).collect::<String>(),
            ));
        }
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return DirectSearchOutcome::Transport(format!(
                "{}: search_vectors({}): HTTP {} {}",
                self.base_url,
                req.collection,
                status.as_u16(),
                body_text.chars().take(200).collect::<String>(),
            ));
        }
        match resp.json::<WireSearchResponse>().await {
            Ok(parsed) => DirectSearchOutcome::Ok(parsed.results),
            Err(e) => DirectSearchOutcome::Transport(format!(
                "{}: search_vectors({}): parse: {e}",
                self.base_url, req.collection
            )),
        }
    }
}

#[async_trait]
impl VectorLane for VectorizerLane {
    async fn search(&self, req: &VectorRequest) -> Result<Vec<LaneHit>, LaneError> {
        // phase11d — the SDK's `search_vectors` is bypassed because
        // its `SearchResult` deserializer silently drops the server's
        // `payload` field (see module docs). Auth, probes, and
        // refresh stay on the SDK; only the read path runs through
        // the direct `reqwest` lane.
        let hits = match self.direct_search(req).await {
            DirectSearchOutcome::Ok(hits) => hits,
            DirectSearchOutcome::NotFound => {
                // Phase11e §3 — when the env switch is on, emit a
                // synthetic `LaneHit` so the orchestrator can
                // surface the missing collection in `debug.notes`.
                // Default (env unset) keeps the existing fail-open
                // empty-vec behaviour so the response shape is
                // backwards-compatible.
                if collection_missing_report_enabled() {
                    return Ok(vec![collection_missing_marker(req.collection.clone())]);
                }
                return Ok(Vec::new());
            }
            DirectSearchOutcome::Transport(msg) => {
                return Err(LaneError::Transport(msg));
            }
            DirectSearchOutcome::Auth401(msg) => {
                // Issue hivellm/cortex#2 — the boot-time JWT expires
                // after ~1 h and every subsequent search returns
                // 401. Re-mint the token using the cached creds and
                // retry once; if the refresh path is unavailable
                // (no creds, or `/auth/login` rejected the retry) we
                // still surface the 401 so the orchestrator's
                // `debug.errors.vector` lane carries an actionable
                // signal.
                if self.creds.is_none() {
                    return Err(LaneError::Transport(format!(
                        "{msg}; no cached credentials to mint a fresh JWT — set \
                         CORTEX_VECTORIZER_USER + _PASSWORD (or _EMBEDDER_VECTORIZER_*)"
                    )));
                }
                if let Err(reason) = self.refresh_token().await {
                    return Err(LaneError::Transport(format!(
                        "{msg}; auth refresh failed: {reason}"
                    )));
                }
                tracing::info!(
                    base_url = %self.base_url,
                    collection = %req.collection,
                    "vector lane: refreshed JWT after upstream 401, retrying"
                );
                match self.direct_search(req).await {
                    DirectSearchOutcome::Ok(hits) => hits,
                    DirectSearchOutcome::NotFound => {
                        if collection_missing_report_enabled() {
                            return Ok(vec![collection_missing_marker(req.collection.clone())]);
                        }
                        return Ok(Vec::new());
                    }
                    DirectSearchOutcome::Auth401(msg2) | DirectSearchOutcome::Transport(msg2) => {
                        return Err(LaneError::Transport(format!(
                            "{msg}; refresh-retry failed: {msg2}"
                        )));
                    }
                }
            }
        };

        let projected: Vec<LaneHit> = hits
            .into_iter()
            .map(|h| project(h, req))
            .filter(|h| scope_matches(&req.scope, h))
            .collect();
        Ok(projected)
    }
}

/// phase10h — post-projection scope filter for the Vectorizer
/// lane. The Vectorizer SDK's `search_vectors` does not expose a
/// server-side filter parameter, so the lane filters
/// client-side after deserialising metadata into `LaneHit`. Each
/// scope dimension applies independently (AND); empty
/// dimensions are no-ops.
///
/// The filter is intentionally permissive: a hit whose metadata
/// is missing the field the scope dimension references stays in
/// the result set rather than being dropped silently. Operators
/// hitting that case see "scope quietly didn't apply" rather
/// than "scope dropped every hit", which is the right default
/// for fail-open retrieval semantics.
fn scope_matches(scope: &crate::types::Scope, hit: &crate::lanes::LaneHit) -> bool {
    if let Some(since) = scope.since.as_deref().filter(|s| !s.is_empty()) {
        if let Some(since_ms) = rfc3339_to_ms(since) {
            // Drop hits with a known-too-old timestamp. Hits with
            // `ts == 0` (timestamp absent in metadata) round-trip
            // through — see the permissive note above.
            if hit.ts > 0 && hit.ts < since_ms {
                return false;
            }
        }
    }
    if !scope.files.is_empty() {
        let path = hit.path.as_deref().unwrap_or("");
        if !path.is_empty() {
            let any_match = scope
                .files
                .iter()
                .filter(|p| !p.is_empty())
                .any(|prefix| path.starts_with(prefix));
            if !any_match {
                return false;
            }
        }
    }
    if !scope.topics.is_empty() {
        // Topics may live under `topics` (array) or `topic`
        // (single value) on the upstream metadata. Round-trip
        // both through `LaneHit.extras` per the spec-11 lane
        // projection contract.
        let allow: std::collections::HashSet<&str> = scope
            .topics
            .iter()
            .map(String::as_str)
            .filter(|s| !s.is_empty())
            .collect();
        let mut matched = false;
        if let Some(arr) = hit.extras.get("topics").and_then(|v| v.as_array()) {
            for t in arr {
                if let Some(s) = t.as_str() {
                    if allow.contains(s) {
                        matched = true;
                        break;
                    }
                }
            }
        }
        if !matched {
            if let Some(t) = hit.extras.get("topic").and_then(|v| v.as_str()) {
                if allow.contains(t) {
                    matched = true;
                }
            }
        }
        // Drop only when the metadata exposes a topic field AND
        // none of the values match. Hits without topic metadata
        // round-trip (fail-open).
        let topics_present = hit.extras.contains_key("topics") || hit.extras.contains_key("topic");
        if topics_present && !matched {
            return false;
        }
    }
    true
}

/// phase10h — RFC-3339 timestamp → epoch ms. The orchestrator's
/// scope.since is documented as ISO-8601 / RFC-3339; chrono
/// parses both. Returns `None` on a malformed input so the
/// caller treats the filter as a no-op rather than dropping
/// every hit.
fn rfc3339_to_ms(since: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(since)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

/// Project one Vectorizer search result into a `LaneHit`. Stamps
/// `extras["source"] = "vector"` so the orchestrator's
/// source-attribution invariant is met (the keyword-lane fix
/// flipped the default; both lanes now stamp explicitly).
/// Crate-internal test seam — drive [`project`] against a
/// hand-rolled [`WireSearchHit`]. The regression guard in
/// `crate::lane_contract` uses this to exercise the Vectorizer
/// projection without the HTTP transport.
#[cfg(test)]
pub(crate) fn project_search_result(r: WireSearchHit, req: &VectorRequest) -> LaneHit {
    project(r, req)
}

/// phase11d — project one [`WireSearchHit`] from `POST
/// /collections/{c}/search/text` into a `LaneHit`.
///
/// The server places every projection-relevant field under `payload`
/// (verified against `hivehub/vectorizer:3.0.0`). Older embedder
/// builds nested those keys under `payload.payload.<key>`; that
/// fallback stays in place so a mixed corpus during an indexer
/// rollout still surfaces decisions / turns / law violations.
fn project(r: WireSearchHit, req: &VectorRequest) -> LaneHit {
    let payload = &r.payload;
    let get_str = |key: &str| -> Option<String> {
        payload.get(key).and_then(|v| v.as_str()).map(String::from)
    };
    let get_i64 = |key: &str| -> Option<i64> {
        payload
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
    // Phase6b — spec-11 lane projection contract. Canonical bootstrap
    // pipelines (≥3.0.3) place the contract keys directly under
    // `payload`, but earlier embedder builds nested them under
    // `payload.payload.<key>`. Prefer the canonical top-level
    // location, fall back to the legacy nesting, so a mixed corpus
    // during a worker rollout still surfaces decisions / turns / law
    // violations correctly.
    let nested_payload = payload.get("payload").and_then(|v| v.as_object());
    for key in crate::lanes::LANE_EXTRAS_KEYS {
        let from_top = payload.get(*key).cloned();
        let from_nested = nested_payload.and_then(|p| p.get(*key).cloned());
        let val = from_top.or(from_nested);
        if let Some(v) = val {
            if !v.is_null() {
                extras.insert((*key).to_string(), v);
            }
        }
    }

    let mut text = get_str("body")
        .filter(|s| !s.is_empty())
        .or_else(|| get_str("summary"))
        .or_else(|| get_str("title"))
        .or_else(|| {
            // phase11d — when older embedder builds nested the
            // text-bearing keys under `payload.payload`, walk the
            // same fallback set there before giving up.
            nested_payload.and_then(|p| {
                p.get("body")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(String::from)
                    .or_else(|| p.get("summary").and_then(|v| v.as_str()).map(String::from))
                    .or_else(|| p.get("title").and_then(|v| v.as_str()).map(String::from))
            })
        })
        .unwrap_or_default();
    // phase10b §1 — stop projecting `path` as `text` when the
    // upstream payload only carries the path. The audit logged
    // `text='crates/cortex-api/src/types.rs'` snippets that the
    // bundle renderer formatted as `path:artifact — \n   path` —
    // an `ls`-grade result. When `text` matches the path verbatim
    // we drop it and stamp `body_truncated = true` so the renderer
    // collapses to a header-only line.
    let path_str = get_str("path").unwrap_or_default();
    let body_truncated = !path_str.is_empty() && text == path_str;
    if body_truncated {
        text.clear();
        extras.insert("body_truncated".to_string(), serde_json::Value::Bool(true));
    }

    // phase10d — canonical lowercase `repo`. See the matching
    // comment in `crate::meili_lane::project` for the full
    // rationale; the read-path keeps both lanes aligned.
    let raw_repo = get_str("repo");
    let canonical_repo = raw_repo.as_deref().map(str::to_ascii_lowercase);
    if let Some(label) = raw_repo.as_deref() {
        if canonical_repo.as_deref() != Some(label) && !label.is_empty() {
            extras.insert(
                "repo_label".to_string(),
                serde_json::Value::String(label.to_string()),
            );
        }
    }
    // ADR-011 — typed overlay alongside extras. Vector lane owns
    // turn_id / model / summary; the rest are decision / governance
    // signals filled by other lanes.
    let overlay = crate::lanes::Overlay {
        turn_id: extras
            .get("turn_id")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        model: extras
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        summary: extras
            .get("summary")
            .and_then(|v| v.as_str())
            .map(str::to_string),
        severity: get_str("severity"),
        source: crate::lanes::LaneSource::Vector,
        ..crate::lanes::Overlay::default()
    };
    LaneHit {
        doc_id: format!("vec|{}|{}", req.collection, r.id),
        text,
        repo: canonical_repo,
        path: get_str("path"),
        symbol: get_str("kind"),
        content_hash: get_str("content_hash"),
        score: r.score as f64,
        ts: get_i64("ts").unwrap_or(0),
        severity: get_str("severity"),
        extras,
        overlay,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Scope;
    use serde_json::Map as JsonMap;

    fn req(query: &str) -> VectorRequest {
        VectorRequest {
            collection: "cortex-cortex-code".into(),
            query: query.into(),
            k: 5,
            scope: Scope::default(),
        }
    }

    #[test]
    fn projects_a_wire_hit_with_payload_into_a_lane_hit() {
        let mut payload: JsonMap<String, serde_json::Value> = JsonMap::new();
        payload.insert("repo".into(), serde_json::json!("Cortex"));
        payload.insert("path".into(), serde_json::json!("src/lib.rs"));
        payload.insert("kind".into(), serde_json::json!("turn"));
        payload.insert("content_hash".into(), serde_json::json!("sha256:abc"));
        payload.insert("ts".into(), serde_json::json!(1714200000000_i64));
        payload.insert("body".into(), serde_json::json!("semantic body"));

        let r = WireSearchHit {
            id: "vec-1".into(),
            score: 0.91,
            payload,
            vector: None,
        };

        let hit = project(r, &req("embedder"));
        assert_eq!(hit.doc_id, "vec|cortex-cortex-code|vec-1");
        assert_eq!(hit.text, "semantic body");
        // phase10d — canonical lowercase on the lane hit, original
        // case preserved in `extras.repo_label`.
        assert_eq!(hit.repo.as_deref(), Some("cortex"));
        assert_eq!(
            hit.extras.get("repo_label").and_then(|v| v.as_str()),
            Some("Cortex")
        );
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
    fn projects_falls_back_through_summary_title_chain() {
        let mut payload: JsonMap<String, serde_json::Value> = JsonMap::new();
        payload.insert("summary".into(), serde_json::json!("curated summary"));
        payload.insert("title".into(), serde_json::json!("the title"));

        let r = WireSearchHit {
            id: "vec-2".into(),
            score: 0.5,
            payload,
            vector: None,
        };
        let hit = project(r, &req("x"));
        assert_eq!(hit.text, "curated summary");
    }

    #[test]
    fn projects_emits_empty_text_only_when_no_text_anywhere() {
        let r = WireSearchHit {
            id: "vec-3".into(),
            score: 0.0,
            payload: JsonMap::new(),
            vector: None,
        };
        let hit = project(r, &req("x"));
        assert_eq!(hit.text, "");
        assert_eq!(hit.score, 0.0);
        assert_eq!(hit.ts, 0);
    }

    #[test]
    fn projects_legacy_nested_payload_for_text_and_contract_keys() {
        // phase11d — older embedder builds nested every projection
        // key under `payload.payload.<key>`. The fallback walks one
        // level deeper for both the text-bearing keys (`body`,
        // `summary`, `title`) and every spec-11 contract key.
        let mut nested: JsonMap<String, serde_json::Value> = JsonMap::new();
        nested.insert("body".into(), serde_json::json!("nested body text"));
        nested.insert(
            "turn_id".into(),
            serde_json::json!("01HTURNNESTED0000000000000"),
        );
        nested.insert("model".into(), serde_json::json!("claude-sonnet-4-6"));
        let mut payload: JsonMap<String, serde_json::Value> = JsonMap::new();
        payload.insert("repo".into(), serde_json::json!("Cortex"));
        payload.insert("payload".into(), serde_json::Value::Object(nested));

        let r = WireSearchHit {
            id: "vec-legacy".into(),
            score: 0.7,
            payload,
            vector: None,
        };
        let hit = project(r, &req("x"));
        assert_eq!(hit.text, "nested body text");
        assert_eq!(
            hit.extras.get("turn_id").and_then(|v| v.as_str()),
            Some("01HTURNNESTED0000000000000")
        );
        assert_eq!(
            hit.extras.get("model").and_then(|v| v.as_str()),
            Some("claude-sonnet-4-6")
        );
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
        assert!(super::looks_like_auth_failure(
            "Invalid token: signature mismatch"
        ));
        // Negative — anything else (404, 500, transport noise) keeps
        // the existing not-found / generic-transport paths.
        assert!(!super::looks_like_auth_failure("HTTP 404 Not Found"));
        assert!(!super::looks_like_auth_failure(
            "tcp connect timeout after 10s"
        ));
    }

    // ---- phase10h — Vectorizer post-projection scope filter ----

    fn scope() -> crate::types::Scope {
        crate::types::Scope::default()
    }

    fn hit_with(ts: i64, path: Option<&str>, topics: Option<Vec<&str>>) -> crate::lanes::LaneHit {
        let mut extras = std::collections::BTreeMap::new();
        if let Some(t) = topics {
            extras.insert(
                "topics".to_string(),
                serde_json::Value::Array(
                    t.into_iter()
                        .map(|s| serde_json::Value::String(s.to_string()))
                        .collect(),
                ),
            );
        }
        crate::lanes::LaneHit {
            doc_id: "vec|test|1".into(),
            text: "body".into(),
            repo: Some("cortex".into()),
            path: path.map(String::from),
            symbol: None,
            content_hash: None,
            score: 0.5,
            ts,
            severity: None,
            extras,
            overlay: crate::lanes::Overlay::default(),
        }
    }

    #[test]
    fn scope_filter_drops_hits_older_than_since() {
        let mut s = scope();
        s.since = Some("2026-04-01T00:00:00Z".to_string());
        // April 1 2026 = 1 775 692 800 000 ms.
        let recent = hit_with(1_780_000_000_000, None, None);
        let old = hit_with(1_700_000_000_000, None, None);
        assert!(super::scope_matches(&s, &recent));
        assert!(!super::scope_matches(&s, &old));
    }

    #[test]
    fn scope_filter_keeps_ts_zero_hits_when_since_set() {
        // Fail-open: a hit whose metadata didn't carry a timestamp
        // (`ts == 0`) round-trips. Better to surface a possibly
        // out-of-window row than to drop everything silently.
        let mut s = scope();
        s.since = Some("2026-04-01T00:00:00Z".to_string());
        let no_ts = hit_with(0, None, None);
        assert!(super::scope_matches(&s, &no_ts));
    }

    #[test]
    fn scope_filter_prefix_matches_files() {
        let mut s = scope();
        s.files = vec!["crates/cortex-api/src/".to_string()];
        let inside = hit_with(0, Some("crates/cortex-api/src/strategies.rs"), None);
        let outside = hit_with(0, Some("crates/cortex-graph/src/lib.rs"), None);
        assert!(super::scope_matches(&s, &inside));
        assert!(!super::scope_matches(&s, &outside));
    }

    #[test]
    fn scope_filter_topics_or_match_required_when_metadata_present() {
        let mut s = scope();
        s.topics = vec!["law".to_string(), "governance".to_string()];
        let in_topic = hit_with(0, None, Some(vec!["law"]));
        let off_topic = hit_with(0, None, Some(vec!["code"]));
        let no_topics_meta = hit_with(0, None, None);
        assert!(super::scope_matches(&s, &in_topic));
        assert!(!super::scope_matches(&s, &off_topic));
        // No topic metadata → fail-open round-trip.
        assert!(super::scope_matches(&s, &no_topics_meta));
    }
}
