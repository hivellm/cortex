#![allow(missing_docs)]
//! `require_api_key` Axum middleware — phase3 §7.3.
//!
//! When the operator sets `CORTEX_DASHBOARD_AUTH=1`, the dashboard
//! sub-router (every `/v1/dashboard/*` endpoint) is wrapped with
//! this layer. Each request must carry either:
//!
//! - `Authorization: Bearer <key>` — the canonical header path; or
//! - `?api_key=<key>` — the SSE escape hatch, since `EventSource`
//!   has no header API.
//!
//! On success the request continues; on failure the daemon returns
//! 401 with `{"reason":"missing_or_invalid_api_key"}`. Liveness
//! probes (`/v1/status`) are mounted on a separate router and do
//! NOT run through this layer.
//!
//! Localhost dev keeps `CORTEX_DASHBOARD_AUTH=0` (default) and
//! short-circuits the layer at boot — `wrap_dashboard_router`
//! returns the input router unchanged in that mode, so there is no
//! per-request cost for the local-only flow.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::State,
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    Json, Router,
};
use serde::Serialize;

use crate::storage::api_keys::{ApiKeyError, ApiKeyStore};

/// Environment variable that flips the middleware on. Anything
/// other than `"1"` / `"true"` (case-insensitive) is treated as
/// off so localhost dev with the var unset keeps working.
pub const ENV_DASHBOARD_AUTH: &str = "CORTEX_DASHBOARD_AUTH";

/// 401 body returned when the middleware rejects a request.
/// Mirrors the existing `/v1/query` error envelope shape so the
/// renderer's ApiError handler can branch on `reason` without a
/// special case for this endpoint family.
#[derive(Debug, Serialize)]
struct AuthErrorBody {
    reason: &'static str,
}

const REASON_MISSING_OR_INVALID: &str = "missing_or_invalid_api_key";

/// Read `CORTEX_DASHBOARD_AUTH` and decide whether the middleware
/// should apply. `1`, `true`, `yes`, `on` (any case) → on; anything
/// else → off. Off is the default.
pub fn dashboard_auth_enabled() -> bool {
    matches!(
        std::env::var(ENV_DASHBOARD_AUTH).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("True") | Ok("yes") | Ok("on")
    )
}

/// Layer factory: wrap `router` with the auth middleware when
/// `CORTEX_DASHBOARD_AUTH` is set, else return the router
/// unchanged. The store handle is shared (Arc-cloned internally)
/// across every request the middleware admits.
pub fn wrap_dashboard_router(router: Router, store: Arc<ApiKeyStore>) -> Router {
    if !dashboard_auth_enabled() {
        return router;
    }
    let layer_state = AuthState { store };
    router.layer(middleware::from_fn_with_state(
        layer_state,
        require_api_key,
    ))
}

/// State threaded into the middleware. Only the store handle for
/// now; future fields (rate limit window per id, audit publisher)
/// land here.
#[derive(Clone)]
pub struct AuthState {
    pub store: Arc<ApiKeyStore>,
}

/// Extract a candidate cleartext key from either the
/// `Authorization: Bearer …` header or the `?api_key=…` query
/// param. Header wins when both are present.
fn extract_candidate(req: &Request<Body>) -> Option<String> {
    if let Some(value) = req.headers().get("authorization") {
        if let Ok(s) = value.to_str() {
            if let Some(rest) = s.strip_prefix("Bearer ") {
                if !rest.is_empty() {
                    return Some(rest.to_string());
                }
            }
        }
    }
    let query = req.uri().query()?;
    for pair in query.split('&') {
        let mut it = pair.splitn(2, '=');
        let key = it.next().unwrap_or("");
        if key == "api_key" {
            let value = it.next().unwrap_or("");
            if value.is_empty() {
                return None;
            }
            // The query value is always urlencoded; decode percent
            // sequences manually to avoid pulling in a urlencoding
            // crate. cortex-api keys are Crockford base32 + a fixed
            // prefix, so the only encoded char in practice is the
            // `+` ↔ ` ` substitution which lower-case keys never
            // emit. Decode `%XX` triplets defensively anyway.
            return Some(decode_percent(value));
        }
    }
    None
}

fn decode_percent(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push(((h as u8) << 4) | (l as u8));
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| input.to_string())
}

fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(AuthErrorBody {
            reason: REASON_MISSING_OR_INVALID,
        }),
    )
        .into_response()
}

/// Per-request middleware function. Calls into [`ApiKeyStore::verify`]
/// to do the actual Argon2id constant-time compare.
pub async fn require_api_key(
    State(state): State<AuthState>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let Some(candidate) = extract_candidate(&req) else {
        return unauthorized();
    };
    let store = state.store.clone();
    // Argon2id verify is CPU-bound and runs inside `verify`; spawn
    // a blocking task so the runtime's worker threads stay free
    // for I/O. The store's mutex is held briefly inside this task.
    let verify_result = tokio::task::spawn_blocking(move || store.verify(&candidate))
        .await
        .ok();
    match verify_result {
        Some(Ok(_id)) => next.run(req).await,
        Some(Err(ApiKeyError::Revoked)) | Some(Err(ApiKeyError::NotFound)) | Some(Err(_)) => {
            unauthorized()
        }
        None => {
            // join error — middleware should fail closed, not open.
            unauthorized()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::Request;

    fn req_with(headers: &[(&str, &str)], query: &str) -> Request<Body> {
        let path = if query.is_empty() {
            "/v1/dashboard/overview".to_string()
        } else {
            format!("/v1/dashboard/overview?{query}")
        };
        let mut builder = Request::builder().uri(path).method("GET");
        for (k, v) in headers {
            builder = builder.header(*k, *v);
        }
        builder.body(Body::empty()).unwrap()
    }

    #[test]
    fn extract_prefers_authorization_header() {
        let req = req_with(
            &[("authorization", "Bearer header-key")],
            "api_key=query-key",
        );
        assert_eq!(extract_candidate(&req).as_deref(), Some("header-key"));
    }

    #[test]
    fn extract_falls_back_to_query_param() {
        let req = req_with(&[], "api_key=query-key");
        assert_eq!(extract_candidate(&req).as_deref(), Some("query-key"));
    }

    #[test]
    fn extract_returns_none_when_neither_present() {
        let req = req_with(&[], "");
        assert!(extract_candidate(&req).is_none());
    }

    #[test]
    fn extract_returns_none_for_bare_authorization_keyword() {
        let req = req_with(&[("authorization", "Bearer ")], "");
        assert!(extract_candidate(&req).is_none());
    }

    #[test]
    fn extract_handles_other_query_params_alongside_api_key() {
        let req = req_with(&[], "session_id=s1&api_key=k1&kind=turn");
        assert_eq!(extract_candidate(&req).as_deref(), Some("k1"));
    }

    #[test]
    fn dashboard_auth_disabled_when_env_unset() {
        std::env::remove_var(ENV_DASHBOARD_AUTH);
        assert!(!dashboard_auth_enabled());
    }

    #[test]
    fn dashboard_auth_enabled_when_env_one() {
        std::env::set_var(ENV_DASHBOARD_AUTH, "1");
        assert!(dashboard_auth_enabled());
        std::env::remove_var(ENV_DASHBOARD_AUTH);
    }

    #[test]
    fn dashboard_auth_disabled_for_garbage_value() {
        std::env::set_var(ENV_DASHBOARD_AUTH, "maybe");
        assert!(!dashboard_auth_enabled());
        std::env::remove_var(ENV_DASHBOARD_AUTH);
    }

    #[test]
    fn decode_percent_round_trips_an_unencoded_key() {
        assert_eq!(decode_percent("cortex_dash_aaa"), "cortex_dash_aaa");
    }

    #[test]
    fn decode_percent_decodes_a_percent_triplet() {
        assert_eq!(decode_percent("a%2Bb"), "a+b");
    }

    #[test]
    fn decode_percent_passes_invalid_triplet_through() {
        assert_eq!(decode_percent("a%zzb"), "a%zzb");
    }
}
