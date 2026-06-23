//! Phase11v — fine-grained per-backend search proxies.
//!
//! Three handlers expose direct access to the underlying retrieval
//! backends without the spec-11 fusion layer:
//!
//! - `POST /v1/search/keyword` — raw Meili search on a named index.
//! - `POST /v1/search/vector`  — raw Vectorizer cosine search on a
//!   named collection (query string embedded server-side, or raw
//!   embedding accepted).
//! - `POST /v1/search/graph`   — Nexus neighbors / Cypher.
//!
//! These exist so MCP callers can ask "show me ONLY relationships" /
//! "show me the closest vectors in collection X" / "show me the raw
//! Meili hits" — questions the fused `cortex_query` cannot answer
//! without losing fidelity.
//!
//! Each handler reads its backend URL + auth out of the same env vars
//! the rest of the workers use (`CORTEX_FULLTEXT_MEILI_URL` /
//! `CORTEX_FULLTEXT_MEILI_API_KEY` / `CORTEX_EMBEDDER_VECTORIZER_URL` /
//! `CORTEX_GRAPH_NEXUS_URL`), so the daemon does not need extra
//! configuration plumbing for this surface.

use axum::{extract::State, http::{HeaderMap, StatusCode}, response::Response, Json};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::http::ApiState;

/// Default Meili URL when `CORTEX_FULLTEXT_MEILI_URL` is unset.
const MEILI_URL_DEFAULT: &str = "http://127.0.0.1:17004";
/// Default Vectorizer URL when `CORTEX_EMBEDDER_VECTORIZER_URL` is unset.
const VECTORIZER_URL_DEFAULT: &str = "http://127.0.0.1:15002";
/// Hard cap on neighbours-mode hops.
const GRAPH_DEPTH_MAX: u32 = 5;
/// Default vector hits cap.
const VECTOR_K_DEFAULT: u32 = 10;
/// Hard cap on vector hits per call.
const VECTOR_K_MAX: u32 = 200;

/// Maximum raw-document hits the keyword surface returns in a single
/// call. Mirrors `cortex_query` defaults so the MCP transport cap is
/// not the first thing operators trip over.
const KEYWORD_LIMIT_DEFAULT: u32 = 10;
const KEYWORD_LIMIT_MAX: u32 = 100;

/// Per-string-field byte cap applied to raw keyword hits when the
/// caller did NOT pass `attributes_to_retrieve` (issue #4 secondary
/// finding). A single oversized document field — the report saw a
/// 91 KB line — otherwise blows the MCP per-result transport cap and
/// makes `cortex_keyword_search` unusable without manually restricting
/// fields. Callers who need a field in full opt out by passing
/// `attributes_to_retrieve`, which bypasses capping entirely.
const KEYWORD_HIT_FIELD_CAP_BYTES: usize = 2_048;

/// Recursively truncate every string leaf in `value` longer than `cap`
/// bytes, replacing the dropped tail with a `…[+N bytes]` marker.
/// UTF-8 safe — never splits a multi-byte codepoint.
fn cap_string_leaves(value: &mut Value, cap: usize) {
    match value {
        Value::String(s) if s.len() > cap => {
            let mut idx = cap;
            while idx > 0 && !s.is_char_boundary(idx) {
                idx -= 1;
            }
            let dropped = s.len() - idx;
            let mut truncated = s[..idx].to_string();
            truncated.push_str(&format!("…[+{dropped} bytes]"));
            *s = truncated;
        }
        Value::Array(arr) => {
            for v in arr.iter_mut() {
                cap_string_leaves(v, cap);
            }
        }
        Value::Object(map) => {
            for (_, v) in map.iter_mut() {
                cap_string_leaves(v, cap);
            }
        }
        _ => {}
    }
}

/// Request body for `POST /v1/search/keyword`.
#[derive(Debug, Clone, Deserialize)]
pub struct KeywordSearchRequest {
    /// Meili index uid (`cortex_consolidations`, `cortex-cortex-turns`, ...).
    pub index: String,
    /// Free-text query. Empty string returns the full document set
    /// up to `limit` (Meili's default behaviour).
    #[serde(default)]
    pub q: String,
    /// Hits cap. Defaults to [`KEYWORD_LIMIT_DEFAULT`]; clamped to
    /// [`KEYWORD_LIMIT_MAX`].
    #[serde(default)]
    pub limit: Option<u32>,
    /// Raw Meili filter expression (`repo = "cortex" AND kind = "consolidation"`).
    /// Forwarded verbatim — operators must reference filterable
    /// attributes that the index actually has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<String>,
    /// Raw Meili sort expression list (`["occurred_at:desc"]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<Vec<String>>,
    /// Optional attributes-to-retrieve list — when present Meili
    /// only returns those fields per hit (saves transport budget).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attributes_to_retrieve: Option<Vec<String>>,
}

/// Response body for `POST /v1/search/keyword`.
#[derive(Debug, Clone, Serialize)]
pub struct KeywordSearchResponse {
    /// Echo of the requested index.
    pub index: String,
    /// Raw Meili documents (untouched).
    pub hits: Vec<Value>,
    /// `processingTimeMs` echoed from Meili.
    pub processing_time_ms: u64,
    /// `estimatedTotalHits` echoed from Meili.
    pub estimated_total_hits: u64,
}

fn meili_base_url() -> String {
    cortex_config::Config::load()
        .ok()
        .and_then(|c| c.meili.meili_url)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| MEILI_URL_DEFAULT.to_string())
}

fn vectorizer_base_url() -> String {
    cortex_config::Config::load()
        .ok()
        .and_then(|c| c.embedder.vectorizer_url)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| VECTORIZER_URL_DEFAULT.to_string())
}

fn vectorizer_jwt() -> Option<String> {
    cortex_config::Config::load()
        .ok()
        .and_then(|c| c.embedder.vectorizer_jwt)
        .filter(|s| !s.trim().is_empty())
}

fn vectorizer_credentials() -> Option<(String, String)> {
    let cfg = cortex_config::Config::load().ok()?;
    let user = cfg.embedder.vectorizer_user;
    let pwd = cfg.embedder.vectorizer_password?;
    if user.trim().is_empty() || pwd.trim().is_empty() {
        return None;
    }
    Some((user, pwd))
}

/// Mint a Vectorizer JWT via `POST /auth/login`. Used as a fallback
/// when `vectorizer_jwt` is unset in the config — the embedder worker
/// owns the persistent JWT cache, so this surface mints a short-lived
/// token per call (debug path; not hot).
async fn vectorizer_login(base: &str, user: &str, pwd: &str) -> Option<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .ok()?;
    let url = format!("{}/auth/login", base.trim_end_matches('/'));
    let resp = client
        .post(&url)
        .json(&json!({ "username": user, "password": pwd }))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: Value = resp.json().await.ok()?;
    body.get("access_token")
        .and_then(Value::as_str)
        .map(str::to_string)
}

async fn resolve_vectorizer_bearer(base: &str) -> Option<String> {
    if let Some(jwt) = vectorizer_jwt() {
        tracing::debug!("vector_search: using preset jwt");
        return Some(jwt);
    }
    let Some((user, pwd)) = vectorizer_credentials() else {
        tracing::warn!("vector_search: no jwt + no credentials in config");
        return None;
    };
    match vectorizer_login(base, &user, &pwd).await {
        Some(t) => {
            tracing::debug!("vector_search: minted jwt via /auth/login");
            Some(t)
        }
        None => {
            tracing::warn!(base, user, "vector_search: /auth/login failed");
            None
        }
    }
}

fn meili_api_key() -> Option<String> {
    cortex_config::Config::load()
        .ok()
        .and_then(|c| c.meili.meili_api_key)
        .filter(|s| !s.trim().is_empty())
}

fn json_err(status: StatusCode, reason: &str, detail: impl Into<String>) -> Response {
    use axum::response::IntoResponse;
    (
        status,
        Json(json!({ "reason": reason, "detail": detail.into() })),
    )
        .into_response()
}

/// Phase21 §5.7 — merge a caller-supplied Meili filter with the ACL
/// clause. Both are wrapped in parens so the AND precedence is
/// unambiguous when the filter grammar has OR at the top level.
fn merged_meili_filter(caller: Option<&str>, acl: &str) -> String {
    match caller {
        Some(c) if !c.trim().is_empty() => format!("({c}) AND ({acl})"),
        _ => acl.to_string(),
    }
}

/// Phase21 §5.7 — Bell-LaPadula ACL check for a raw Vectorizer hit.
///
/// Reads `class_level` / `class_compartments` from `hit["payload"]`
/// (canonical ≥3.0.0), falling back to `hit["payload"]["payload"]`
/// for legacy embedder builds — mirrors the vectorizer-lane reading
/// pattern in `lanes/vectorizer_lane.rs`.
fn vector_hit_passes_acl(hit: &Value, clearance_level: u8, compartment_grants: &[String]) -> bool {
    let payload = hit.get("payload").unwrap_or(hit);
    let nested = payload.get("payload");
    let fact_level: Option<u8> = payload
        .get("class_level")
        .or_else(|| nested.and_then(|n| n.get("class_level")))
        .and_then(Value::as_u64)
        .map(|n| n as u8);
    let fact_compartments: Vec<String> = payload
        .get("class_compartments")
        .or_else(|| nested.and_then(|n| n.get("class_compartments")))
        .and_then(Value::as_array)
        .map(|arr| arr.iter().filter_map(Value::as_str).map(str::to_owned).collect())
        .unwrap_or_default();
    cortex_workers::acl::filter::vectorizer_acl_matches(
        clearance_level,
        compartment_grants,
        fact_level,
        &fact_compartments,
    )
}

#[allow(clippy::manual_clamp)]
fn clamp_limit(req: &KeywordSearchRequest) -> u32 {
    req.limit
        .unwrap_or(KEYWORD_LIMIT_DEFAULT)
        .min(KEYWORD_LIMIT_MAX)
        .max(1)
}

/// Handler — `POST /v1/search/keyword`. Forwards the request to the
/// backing Meili index and returns the raw hits.
pub async fn handle_keyword_search(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<KeywordSearchRequest>,
) -> Response {
    use axum::response::IntoResponse;

    let index = req.index.trim();
    if index.is_empty() {
        return json_err(StatusCode::BAD_REQUEST, "bad_input", "`index` is required");
    }
    let limit = clamp_limit(&req);
    let url = format!(
        "{}/indexes/{}/search",
        meili_base_url().trim_end_matches('/'),
        index
    );

    let mut body = json!({
        "q": req.q,
        "limit": limit,
    });
    // Phase21 §5.7 — inject the ACL filter; non-admin principals get
    // their caller-supplied filter merged with the lattice clause so
    // classified rows cannot escape via the raw Meili proxy.
    let principal = state.service.resolve_principal(&headers);
    if !principal.is_acl_admin() {
        let acl = cortex_workers::acl::filter::meili_acl_filter(
            principal.clearance_level,
            &principal.compartment_grants,
        );
        body["filter"] = Value::String(merged_meili_filter(req.filter.as_deref(), &acl));
    } else if let Some(filter) = &req.filter {
        body["filter"] = Value::String(filter.clone());
    }
    if let Some(sort) = &req.sort {
        body["sort"] = json!(sort);
    }
    if let Some(attrs) = &req.attributes_to_retrieve {
        body["attributesToRetrieve"] = json!(attrs);
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return json_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "client_build_failed",
                format!("{e}"),
            );
        }
    };
    let mut http = client.post(&url).json(&body);
    if let Some(key) = meili_api_key() {
        http = http.bearer_auth(key);
    }
    let resp = match http.send().await {
        Ok(r) => r,
        Err(e) => {
            return json_err(
                StatusCode::BAD_GATEWAY,
                "meili_unreachable",
                format!("POST {url}: {e}"),
            );
        }
    };
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return json_err(
            StatusCode::NOT_FOUND,
            "index_not_found",
            format!("index `{index}` does not exist"),
        );
    }
    if !status.is_success() {
        let detail = resp.text().await.unwrap_or_default();
        return json_err(
            StatusCode::BAD_GATEWAY,
            "meili_error",
            format!("{status}: {detail}"),
        );
    }
    let parsed: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return json_err(
                StatusCode::BAD_GATEWAY,
                "meili_decode_failed",
                format!("{e}"),
            );
        }
    };
    let mut hits: Vec<Value> = parsed
        .get("hits")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    // When the caller did not restrict fields, cap oversized string
    // leaves so one 91 KB document field can't blow the MCP
    // per-result transport budget (issue #4). Explicit
    // `attributes_to_retrieve` opts out — the caller is steering
    // field selection themselves.
    if req.attributes_to_retrieve.is_none() {
        for hit in hits.iter_mut() {
            cap_string_leaves(hit, KEYWORD_HIT_FIELD_CAP_BYTES);
        }
    }
    let processing_time_ms = parsed
        .get("processingTimeMs")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    let estimated_total_hits = parsed
        .get("estimatedTotalHits")
        .and_then(|v| v.as_u64())
        .unwrap_or(hits.len() as u64);
    let body = KeywordSearchResponse {
        index: index.to_string(),
        hits,
        processing_time_ms,
        estimated_total_hits,
    };
    (StatusCode::OK, Json(body)).into_response()
}

// ----------------------------------------------------------------------
// Vector search
// ----------------------------------------------------------------------

/// Request body for `POST /v1/search/vector`.
#[derive(Debug, Clone, Deserialize)]
pub struct VectorSearchRequest {
    /// Vectorizer collection name (`cortex.consolidation.fp32`,
    /// `cortex-cortex-turns`, etc.).
    pub collection: String,
    /// Raw embedding vector. Exactly one of `query_vector` /
    /// `query_text` MUST be present. v1 only accepts `query_vector`;
    /// `query_text` is reserved for the embedding-side wiring.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_vector: Option<Vec<f32>>,
    /// Reserved — server-side embedding lands in a follow-up.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub query_text: Option<String>,
    /// Hits cap. Default [`VECTOR_K_DEFAULT`]; clamped to [`VECTOR_K_MAX`].
    #[serde(default)]
    pub k: Option<u32>,
    /// Optional cosine-score floor — server drops hits below this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub score_threshold: Option<f32>,
}

/// Response body for `POST /v1/search/vector`.
#[derive(Debug, Clone, Serialize)]
pub struct VectorSearchResponse {
    /// Echo of the requested collection.
    pub collection: String,
    /// Vector hits as Vectorizer returned them — id + score + payload.
    pub hits: Vec<Value>,
    /// Wall-clock latency for the upstream call.
    pub upstream_latency_ms: u64,
}

#[allow(clippy::manual_clamp)]
fn clamp_k(req: &VectorSearchRequest) -> u32 {
    req.k.unwrap_or(VECTOR_K_DEFAULT).min(VECTOR_K_MAX).max(1)
}

/// Handler — `POST /v1/search/vector`. Forwards the raw vector to
/// the named collection's HNSW search endpoint.
pub async fn handle_vector_search(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<VectorSearchRequest>,
) -> Response {
    use axum::response::IntoResponse;

    let collection = req.collection.trim();
    if collection.is_empty() {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            "`collection` is required",
        );
    }
    if req.query_vector.is_some() == req.query_text.is_some() {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            "exactly one of `query_vector` or `query_text` is required",
        );
    }
    if req.query_text.is_some() {
        return json_err(
            StatusCode::BAD_REQUEST,
            "not_implemented",
            "server-side embedding (`query_text`) lands in a follow-up; pass `query_vector` for now",
        );
    }
    let vector = req.query_vector.as_ref().expect("validated above");
    if vector.is_empty() {
        return json_err(
            StatusCode::BAD_REQUEST,
            "bad_input",
            "`query_vector` is empty",
        );
    }
    let k = clamp_k(&req);

    let url = format!(
        "{}/collections/{}/search",
        vectorizer_base_url().trim_end_matches('/'),
        collection
    );
    let mut body = json!({
        "vector": vector,
        "limit": k,
    });
    if let Some(t) = req.score_threshold {
        body["score_threshold"] = json!(t);
    }

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return json_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "client_build_failed",
                format!("{e}"),
            );
        }
    };
    let mut http = client.post(&url).json(&body);
    if let Some(jwt) = resolve_vectorizer_bearer(&vectorizer_base_url()).await {
        http = http.bearer_auth(jwt);
    }
    let started = std::time::Instant::now();
    let resp = match http.send().await {
        Ok(r) => r,
        Err(e) => {
            return json_err(
                StatusCode::BAD_GATEWAY,
                "vectorizer_unreachable",
                format!("POST {url}: {e}"),
            );
        }
    };
    let latency_ms = started.elapsed().as_millis() as u64;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return json_err(
            StatusCode::NOT_FOUND,
            "collection_not_found",
            format!("collection `{collection}` does not exist"),
        );
    }
    if !status.is_success() {
        let detail = resp.text().await.unwrap_or_default();
        return json_err(
            StatusCode::BAD_GATEWAY,
            "vectorizer_error",
            format!("{status}: {detail}"),
        );
    }
    let parsed: Value = match resp.json().await {
        Ok(v) => v,
        Err(e) => {
            return json_err(
                StatusCode::BAD_GATEWAY,
                "vectorizer_decode_failed",
                format!("{e}"),
            );
        }
    };
    // Phase21 §5.7 — client-side ACL post-filter (Vectorizer has no
    // server-side filter parameter). Apply the exact Bell-LaPadula
    // predicate before returning hits to the caller.
    let principal = state.service.resolve_principal(&headers);
    let raw_hits: Vec<Value> = parsed
        .get("results")
        .or_else(|| parsed.get("hits"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let hits: Vec<Value> = raw_hits
        .into_iter()
        .filter(|h| {
            principal.is_acl_admin()
                || vector_hit_passes_acl(
                    h,
                    principal.clearance_level,
                    &principal.compartment_grants,
                )
        })
        .collect();
    (
        StatusCode::OK,
        Json(VectorSearchResponse {
            collection: collection.to_string(),
            hits,
            upstream_latency_ms: latency_ms,
        }),
    )
        .into_response()
}

// ----------------------------------------------------------------------
// Graph query
// ----------------------------------------------------------------------

/// Request body for `POST /v1/search/graph`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum GraphQueryRequest {
    /// BFS / hop-limited walk from a seed node.
    Neighbors {
        /// Seed node id (`event:01HSESS01`, `consolidation:cons-ses-...`, ...).
        node_id: String,
        /// Hop limit. Default 1, clamped to [`GRAPH_DEPTH_MAX`].
        #[serde(default)]
        depth: Option<u32>,
        /// Optional edge-kind filter (e.g. `["EMITTED", "DERIVED_FROM"]`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        edge_kinds: Option<Vec<String>>,
    },
    /// Raw Cypher — gated on `CORTEX_GRAPH_CYPHER_ENABLED=1`.
    Cypher {
        /// Cypher statement.
        statement: String,
        /// Optional bind parameters.
        #[serde(default)]
        parameters: Value,
    },
}

/// Response body for `POST /v1/search/graph`.
#[derive(Debug, Clone, Serialize)]
pub struct GraphQueryResponse {
    /// Echo of the request mode.
    pub mode: &'static str,
    /// Nodes touched by the walk.
    pub nodes: Vec<Value>,
    /// Edges touched by the walk.
    pub edges: Vec<Value>,
    /// Wall-clock for the upstream call.
    pub upstream_latency_ms: u64,
}

fn cypher_enabled() -> bool {
    cortex_config::Config::load()
        .map(|c| c.nexus.cypher_enabled)
        .unwrap_or(false)
}

/// Handler — `POST /v1/search/graph`.
pub async fn handle_graph_query(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<GraphQueryRequest>,
) -> Response {
    use axum::response::IntoResponse;

    // Phase21 §5.7 — resolve once; both Neighbors and Cypher arms need it.
    let principal = state.service.resolve_principal(&headers);
    let acl_admin = principal.is_acl_admin();

    // Early security gates for Cypher mode — checked before the nexus
    // lookup so unauthorized callers get a clean 403 even when Nexus is
    // absent, and so the existence of a Nexus backend is not revealed.
    if let GraphQueryRequest::Cypher { .. } = &req {
        if !cypher_enabled() {
            return json_err(
                StatusCode::FORBIDDEN,
                "cypher_disabled",
                "set `CORTEX_GRAPH_CYPHER_ENABLED=1` to enable cypher mode",
            );
        }
        if !acl_admin {
            return json_err(
                StatusCode::FORBIDDEN,
                "acl_required",
                "raw Cypher requires acl_admin clearance; use mode=neighbors for ACL-filtered graph traversal",
            );
        }
    }

    let nexus = match state.nexus.clone() {
        Some(n) => n,
        None => {
            return json_err(
                StatusCode::SERVICE_UNAVAILABLE,
                "nexus_unconfigured",
                "cortex-api was started without a Nexus client",
            );
        }
    };

    match req {
        GraphQueryRequest::Neighbors {
            node_id,
            depth,
            edge_kinds,
        } => {
            let depth = depth.unwrap_or(1);
            if depth == 0 || depth > GRAPH_DEPTH_MAX {
                return json_err(
                    StatusCode::BAD_REQUEST,
                    "depth_exceeds_cap",
                    format!("`depth` must be in 1..={GRAPH_DEPTH_MAX}"),
                );
            }
            let id_trim = node_id.trim();
            if id_trim.is_empty() {
                return json_err(
                    StatusCode::BAD_REQUEST,
                    "bad_input",
                    "`node_id` is required",
                );
            }
            // Validate node_id shape before splicing into cypher.
            // Nexus 2.2.0 does NOT substitute `$id` parameters at
            // query-execution time (verified empirically — same
            // query with `$id` returns 0 rows while the inlined
            // form returns the expected neighbors). Inline the
            // sanitized id directly. ULID + slug shape, ≤128.
            let id_safe = id_trim
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
                .collect::<String>();
            if id_safe.is_empty() || id_safe.len() > 128 || id_safe != id_trim {
                return json_err(
                    StatusCode::BAD_REQUEST,
                    "bad_input",
                    "`node_id` must be alphanumeric (ULID or slug, ≤128 chars)",
                );
            }
            // Nexus 2.2.0 cypher syntax for multi-edge alternation
            // is `:A|B|C` (single leading colon, pipe-separated).
            // Backtick-quoted identifiers are rejected. Sanitize
            // each kind to ASCII identifier chars before splicing.

            // Phase21 §5.7 — build the ACL WHERE fragment for alias `n`.
            // Inserted between the MATCH pattern and RETURN so classified
            // nodes are filtered at the Nexus level before rows are sent.
            let acl_where = if !acl_admin {
                let w = cortex_workers::acl::filter::nexus_acl_where_for_alias(
                    principal.clearance_level,
                    &principal.compartment_grants,
                    "n",
                );
                format!("WHERE {w} ")
            } else {
                String::new()
            };

            let cypher = match &edge_kinds {
                Some(kinds) if !kinds.is_empty() => {
                    let alt = kinds
                        .iter()
                        .filter_map(|k| {
                            let stripped: String = k
                                .chars()
                                .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
                                .collect();
                            if stripped.is_empty() {
                                None
                            } else {
                                Some(stripped)
                            }
                        })
                        .collect::<Vec<_>>()
                        .join("|");
                    if alt.is_empty() {
                        format!(
                            "MATCH (s {{ id: '{id_safe}' }})-[r*1..{depth}]-(n) {acl_where}RETURN s, r, n LIMIT 200"
                        )
                    } else {
                        format!(
                            "MATCH (s {{ id: '{id_safe}' }})-[r:{alt}*1..{depth}]-(n) \
                             {acl_where}RETURN s, r, n LIMIT 200"
                        )
                    }
                }
                _ => format!(
                    "MATCH (s {{ id: '{id_safe}' }})-[r*1..{depth}]-(n) {acl_where}RETURN s, r, n LIMIT 200"
                ),
            };
            let started = std::time::Instant::now();
            let res = nexus.execute_cypher(&cypher, None).await;
            let latency_ms = started.elapsed().as_millis() as u64;
            match res {
                Ok(qres) => (
                    StatusCode::OK,
                    Json(GraphQueryResponse {
                        mode: "neighbors",
                        nodes: nexus_rows_column(&qres, "n"),
                        edges: nexus_rows_column(&qres, "r"),
                        upstream_latency_ms: latency_ms,
                    }),
                )
                    .into_response(),
                Err(e) => json_err(StatusCode::BAD_GATEWAY, "nexus_error", format!("{e}")),
            }
        }
        GraphQueryRequest::Cypher {
            statement,
            parameters,
        } => {
            // cypher_enabled + acl_admin already checked before the nexus lookup above.
            let stmt = statement.trim();
            if stmt.is_empty() {
                return json_err(
                    StatusCode::BAD_REQUEST,
                    "bad_input",
                    "`statement` is required",
                );
            }
            let params = match parameters {
                Value::Object(map) => map
                    .into_iter()
                    .map(|(k, v)| (k, value_to_nexus(v)))
                    .collect(),
                _ => std::collections::HashMap::new(),
            };
            let started = std::time::Instant::now();
            let res = nexus.execute_cypher(stmt, Some(params)).await;
            let latency_ms = started.elapsed().as_millis() as u64;
            match res {
                Ok(qres) => (
                    StatusCode::OK,
                    Json(GraphQueryResponse {
                        mode: "cypher",
                        nodes: nexus_rows_all(&qres),
                        edges: vec![],
                        upstream_latency_ms: latency_ms,
                    }),
                )
                    .into_response(),
                Err(e) => json_err(StatusCode::BAD_GATEWAY, "nexus_error", format!("{e}")),
            }
        }
    }
}

fn value_to_nexus(v: Value) -> nexus_sdk::Value {
    match v {
        Value::Null => nexus_sdk::Value::Null,
        Value::Bool(b) => nexus_sdk::Value::Bool(b),
        Value::Number(n) => n
            .as_i64()
            .map(nexus_sdk::Value::Int)
            .or_else(|| n.as_f64().map(nexus_sdk::Value::Float))
            .unwrap_or(nexus_sdk::Value::Null),
        Value::String(s) => nexus_sdk::Value::String(s),
        other => nexus_sdk::Value::String(other.to_string()),
    }
}

/// Extract a column across every row in a `QueryResult` into a JSON array.
/// Each row in the SDK is `serde_json::Value::Array(_)` aligned with `columns`.
fn nexus_rows_column(qres: &nexus_sdk::QueryResult, column: &str) -> Vec<Value> {
    let Some(idx) = qres.columns.iter().position(|c| c == column) else {
        return Vec::new();
    };
    qres.rows
        .iter()
        .filter_map(|row| row.as_array().and_then(|arr| arr.get(idx)).cloned())
        .collect()
}

fn nexus_rows_all(qres: &nexus_sdk::QueryResult) -> Vec<Value> {
    qres.rows
        .iter()
        .map(|row| {
            let arr = row.as_array().cloned().unwrap_or_default();
            let mut obj = serde_json::Map::new();
            for (i, col) in qres.columns.iter().enumerate() {
                obj.insert(col.clone(), arr.get(i).cloned().unwrap_or(Value::Null));
            }
            Value::Object(obj)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyword_request_serde_round_trips() {
        let raw = json!({
            "index": "cortex-cortex-consolidations",
            "q": "auth",
            "limit": 5,
            "filter": "repo = \"cortex\"",
            "sort": ["occurred_at:desc"],
            "attributes_to_retrieve": ["id", "title"],
        });
        let req: KeywordSearchRequest = serde_json::from_value(raw).expect("decode");
        assert_eq!(req.index, "cortex-cortex-consolidations");
        assert_eq!(req.q, "auth");
        assert_eq!(req.limit, Some(5));
        assert_eq!(req.filter.as_deref(), Some("repo = \"cortex\""));
        assert_eq!(
            req.sort.as_deref().map(<[String]>::to_vec).unwrap().len(),
            1
        );
        assert_eq!(req.attributes_to_retrieve.unwrap(), vec!["id", "title"]);
    }

    #[test]
    fn cap_string_leaves_truncates_oversized_fields_and_marks_them() {
        let big = "x".repeat(5_000);
        let mut hit = json!({
            "id": "abc",
            "body": big,
            "nested": { "deep": "y".repeat(3_000) },
            "tags": ["short", "z".repeat(4_000)],
            "count": 42,
        });
        cap_string_leaves(&mut hit, 2_048);
        // Small fields and non-strings are untouched.
        assert_eq!(hit["id"], "abc");
        assert_eq!(hit["count"], 42);
        // Oversized leaves are truncated + marked, recursively.
        let body = hit["body"].as_str().unwrap();
        assert!(body.len() < 5_000 && body.contains("…[+"));
        assert!(body.starts_with("xxx"));
        let deep = hit["nested"]["deep"].as_str().unwrap();
        assert!(deep.len() < 3_000 && deep.contains("…[+"));
        let arr = hit["tags"].as_array().unwrap();
        assert_eq!(arr[0], "short");
        assert!(arr[1].as_str().unwrap().contains("…[+"));
    }

    #[test]
    fn cap_string_leaves_respects_utf8_boundaries() {
        // A field of multi-byte codepoints right at the cap must not
        // split a codepoint.
        let mut hit = json!({ "body": "é".repeat(2_000) });
        cap_string_leaves(&mut hit, 2_048);
        // Still valid UTF-8 (as_str succeeds) and was truncated.
        let body = hit["body"].as_str().unwrap();
        assert!(body.contains("…[+"));
    }

    #[test]
    fn keyword_request_defaults_q_and_limit() {
        let raw = json!({ "index": "cortex_decisions" });
        let req: KeywordSearchRequest = serde_json::from_value(raw).expect("decode");
        assert_eq!(req.q, "");
        assert!(req.limit.is_none());
        assert_eq!(clamp_limit(&req), KEYWORD_LIMIT_DEFAULT);
    }

    #[test]
    fn keyword_clamp_limit_caps_at_max() {
        let raw = json!({ "index": "x", "limit": 9999 });
        let req: KeywordSearchRequest = serde_json::from_value(raw).expect("decode");
        assert_eq!(clamp_limit(&req), KEYWORD_LIMIT_MAX);
    }

    #[test]
    fn keyword_clamp_limit_floors_at_one() {
        let raw = json!({ "index": "x", "limit": 0 });
        let req: KeywordSearchRequest = serde_json::from_value(raw).expect("decode");
        assert_eq!(clamp_limit(&req), 1);
    }

    #[test]
    fn vector_request_serde_round_trips() {
        let raw = json!({
            "collection": "cortex.consolidation.fp32",
            "query_vector": [0.1f32, 0.2, 0.3],
            "k": 5,
            "score_threshold": 0.7
        });
        let req: VectorSearchRequest = serde_json::from_value(raw).expect("decode");
        assert_eq!(req.collection, "cortex.consolidation.fp32");
        assert_eq!(req.query_vector.unwrap().len(), 3);
        assert_eq!(req.k, Some(5));
        assert_eq!(req.score_threshold, Some(0.7));
    }

    #[test]
    fn vector_clamp_k_caps_at_max() {
        let raw = json!({ "collection": "x", "query_vector": [0.0f32], "k": 9_999 });
        let req: VectorSearchRequest = serde_json::from_value(raw).expect("decode");
        assert_eq!(clamp_k(&req), VECTOR_K_MAX);
    }

    #[test]
    fn graph_neighbors_request_decodes() {
        let raw = json!({
            "mode": "neighbors",
            "node_id": "event:01HSESS01",
            "depth": 2,
            "edge_kinds": ["EMITTED"],
        });
        let req: GraphQueryRequest = serde_json::from_value(raw).expect("decode");
        match req {
            GraphQueryRequest::Neighbors {
                node_id,
                depth,
                edge_kinds,
            } => {
                assert_eq!(node_id, "event:01HSESS01");
                assert_eq!(depth, Some(2));
                assert_eq!(edge_kinds.unwrap(), vec!["EMITTED"]);
            }
            other => panic!("expected Neighbors, got {other:?}"),
        }
    }

    #[test]
    fn graph_cypher_request_decodes() {
        let raw = json!({
            "mode": "cypher",
            "statement": "MATCH (n) RETURN n LIMIT 1",
            "parameters": {},
        });
        let req: GraphQueryRequest = serde_json::from_value(raw).expect("decode");
        match req {
            GraphQueryRequest::Cypher { statement, .. } => {
                assert!(statement.starts_with("MATCH"));
            }
            other => panic!("expected Cypher, got {other:?}"),
        }
    }

    // ADR-016 §3.6 — env-precedence test for cypher_enabled removed.
    // cortex_config's typed bool now requires literal `true` / `false`
    // (the `1` / `no` aliases this test exercised are no longer
    // accepted — captured as an operator-visible break in CHANGELOG).
    // The bool resolution is centrally tested in
    // crates/cortex-config/src/load.rs::tests; per-helper duplicates
    // race each other on shared CORTEX_GRAPH_CYPHER_ENABLED state.

    // ── Phase21 §5.7 helpers ─────────────────────────────────────────

    #[test]
    fn merged_meili_filter_wraps_both_in_parens() {
        let out = merged_meili_filter(Some("repo = 'cortex'"), "(class_level IS EMPTY OR class_level <= 1)");
        assert_eq!(
            out,
            "(repo = 'cortex') AND ((class_level IS EMPTY OR class_level <= 1))"
        );
    }

    #[test]
    fn merged_meili_filter_acl_only_when_no_caller() {
        let acl = "(class_level IS EMPTY OR class_level <= 2)";
        assert_eq!(merged_meili_filter(None, acl), acl);
        assert_eq!(merged_meili_filter(Some(""), acl), acl);
        assert_eq!(merged_meili_filter(Some("   "), acl), acl);
    }

    #[test]
    fn vector_hit_passes_acl_level_gate() {
        let hit = json!({
            "id": "evt-001",
            "payload": { "class_level": 3, "class_compartments": [] }
        });
        assert!(!vector_hit_passes_acl(&hit, 1, &[]), "clearance 1 must not see level 3");
        assert!(vector_hit_passes_acl(&hit, 3, &[]), "clearance 3 may see level 3");
    }

    #[test]
    fn vector_hit_passes_acl_compartment_gate() {
        let hit = json!({
            "id": "evt-002",
            "payload": {
                "class_level": 1,
                "class_compartments": ["financial"]
            }
        });
        assert!(!vector_hit_passes_acl(&hit, 2, &[]), "no grant must be denied");
        assert!(
            vector_hit_passes_acl(&hit, 2, &["financial".to_string()]),
            "grant held must pass"
        );
    }

    #[test]
    fn vector_hit_passes_acl_unclassified_passes() {
        let hit = json!({ "id": "evt-003", "payload": {} });
        assert!(vector_hit_passes_acl(&hit, 0, &[]));
    }

    #[test]
    fn vector_hit_passes_acl_legacy_nested_payload() {
        // Legacy embedder builds nest class fields under payload.payload.
        let hit = json!({
            "id": "evt-004",
            "payload": {
                "payload": {
                    "class_level": 2,
                    "class_compartments": ["hr"]
                }
            }
        });
        assert!(!vector_hit_passes_acl(&hit, 1, &["hr".to_string()]), "level gate must fire");
        assert!(!vector_hit_passes_acl(&hit, 2, &[]), "missing compartment grant");
        assert!(vector_hit_passes_acl(&hit, 2, &["hr".to_string()]), "both pass");
    }
}
