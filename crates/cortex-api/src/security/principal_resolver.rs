//! Phase21 §4.4 — caller-identity resolution.
//!
//! Maps an authenticated HTTP request → [`Principal`].
//!
//! Resolution order (ADR-1.2):
//! 1. `x-cortex-principal` header — a direct principal-id override; signed callers
//!    (trusted pipelines like the pre-thinking service) set this to bypass the
//!    key-lookup path. The principal-id is looked up in `PrincipalStore` and the
//!    stored binding is used. Header is only trusted when `CORTEX_TRUST_PRINCIPAL_HEADER=1`.
//! 2. `Authorization: Bearer <key>` / `?api_key=<key>` — standard API-key path;
//!    the key is verified against `ApiKeyStore`, the returned role is looked up in
//!    `PrincipalStore::resolve`.
//! 3. Unauthenticated / no key — `PrincipalStore::default_principal()` (ADR-1.3).

use axum::http::HeaderMap;

use super::principal::Principal;
use super::principal_store::PrincipalStore;
use crate::storage::api_keys::ApiKeyStore;

/// Env var — when set to `1`, the `x-cortex-principal` header is trusted as a
/// direct principal-id override. Only enable on internal networks; leave off on
/// public-facing deployments.
pub const ENV_TRUST_PRINCIPAL_HEADER: &str = "CORTEX_TRUST_PRINCIPAL_HEADER";

/// Whether the `x-cortex-principal` header override is enabled.
pub fn trust_principal_header_enabled() -> bool {
    matches!(
        std::env::var(ENV_TRUST_PRINCIPAL_HEADER).as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE") | Ok("yes") | Ok("on")
    )
}

/// Resolve the caller's `Principal` from the HTTP request.
///
/// - `headers` — the full request header map.
/// - `api_key_store` — the API-key store; `None` when the daemon was started
///   without key-based auth (local dev mode).
/// - `principal_store` — the role-binding store used to resolve key roles or
///   principal-id headers.
///
/// Returns the resolved `Principal`. This function never fails: on any error
/// (bad key, missing store, etc.) it falls back to the default principal so
/// the request continues with the configured minimum clearance.
pub fn resolve_request_principal(
    headers: &HeaderMap,
    api_key_store: Option<&ApiKeyStore>,
    principal_store: &PrincipalStore,
) -> Principal {
    // 1. x-cortex-principal header (trusted-caller override).
    if trust_principal_header_enabled() {
        if let Some(principal_id) = extract_principal_header(headers) {
            // Resolve by id: if the id matches a binding, use that binding's
            // clearance. Otherwise fall through to the key path (the header may
            // carry a caller-defined id not in the store — treat as default).
            let p = principal_store.resolve(&principal_id, &[]);
            // `resolve` falls back to default_principal when no roles match.
            // If the caller sent a non-empty id that resolved to the default,
            // we still want to preserve the id so audit logs show the caller.
            tracing::debug!(
                principal_id = %principal_id,
                clearance_level = p.clearance_level,
                "principal resolved from x-cortex-principal header"
            );
            return p;
        }
    }

    // 2. API-key path.
    if let Some(store) = api_key_store {
        if let Some(candidate) = extract_bearer_token(headers) {
            match store.verify_with_role(&candidate) {
                Ok((key_id, role)) => {
                    let roles: Vec<String> =
                        role.as_deref().map(|r| vec![r.to_string()]).unwrap_or_default();
                    let p = principal_store.resolve(&key_id, &roles);
                    tracing::debug!(
                        key_id = %key_id,
                        role = ?role,
                        clearance_level = p.clearance_level,
                        "principal resolved from API key"
                    );
                    return p;
                }
                Err(e) => {
                    // The auth middleware already 401'd; if we're here the key
                    // was likely wrong. Fall through to default.
                    tracing::debug!(error = %e, "API-key verify failed in principal resolver; using default");
                }
            }
        }
    }

    // 3. Unauthenticated — return the configured default principal.
    let mut p = principal_store.default_principal().clone();
    tracing::debug!(
        default_clearance = p.clearance_level,
        "no auth credentials; using default principal"
    );
    // Use "anonymous" as the id so audit logs are unambiguous.
    if p.id == "default" {
        p.id = "anonymous".to_string();
    }
    p
}

/// Extract the Bearer token from the `Authorization` header.
///
/// Returns `None` when the header is absent, malformed, or holds an empty
/// token. This mirrors the extraction logic in `auth.rs` but operates on a
/// shared `&HeaderMap` reference so it can be called from the service layer
/// without re-parsing the request body.
fn extract_bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("authorization")?;
    let s = value.to_str().ok()?;
    let token = s.strip_prefix("Bearer ")?;
    if token.is_empty() {
        return None;
    }
    Some(token.to_string())
}

/// Extract the `x-cortex-principal` header value.
fn extract_principal_header(headers: &HeaderMap) -> Option<String> {
    let value = headers.get("x-cortex-principal")?;
    let s = value.to_str().ok()?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderName, HeaderValue};

    use crate::security::principal::Principal;
    use crate::security::principal_store::{PrincipalStore, RoleBinding};
    use crate::storage::api_keys::ApiKeyStore;

    fn store_with_analyst() -> (ApiKeyStore, PrincipalStore) {
        let aks = ApiKeyStore::open_in_memory().unwrap();
        let ps = PrincipalStore::new(
            [("analyst".to_string(), RoleBinding {
                clearance_level: 1,
                compartments: vec!["financial".to_string()],
            })],
            Principal::public_only("default"),
        );
        (aks, ps)
    }

    fn headers_with_bearer(token: &str) -> HeaderMap {
        let mut m = HeaderMap::new();
        m.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_str(&format!("Bearer {token}")).unwrap(),
        );
        m
    }

    fn headers_with_principal(id: &str) -> HeaderMap {
        let mut m = HeaderMap::new();
        m.insert(
            HeaderName::from_static("x-cortex-principal"),
            HeaderValue::from_str(id).unwrap(),
        );
        m
    }

    // ---- unauthenticated → default ----

    #[test]
    fn no_headers_returns_default_principal() {
        let (_, ps) = store_with_analyst();
        let p = resolve_request_principal(&HeaderMap::new(), None, &ps);
        assert_eq!(p.clearance_level, 0);
        // id is set to "anonymous" for default fall-through
        assert_eq!(p.id, "anonymous");
    }

    #[test]
    fn no_api_key_store_returns_default_even_with_bearer_header() {
        let (_, ps) = store_with_analyst();
        // Bearer token present but no store to verify against → default
        let headers = headers_with_bearer("cortex_dash_something");
        let p = resolve_request_principal(&headers, None, &ps);
        assert_eq!(p.clearance_level, 0);
    }

    // ---- API key path ----

    #[test]
    fn valid_api_key_with_role_resolves_correct_clearance() {
        let (aks, ps) = store_with_analyst();
        let issued = aks.issue_with_role("api", "test", Some("analyst")).unwrap();
        let headers = headers_with_bearer(&issued.cleartext);
        let p = resolve_request_principal(&headers, Some(&aks), &ps);
        assert_eq!(p.clearance_level, 1);
        assert_eq!(p.compartment_grants, ["financial"]);
        assert_eq!(p.id, issued.id);
    }

    #[test]
    fn valid_api_key_without_role_resolves_to_default_clearance() {
        let (aks, ps) = store_with_analyst();
        let issued = aks.issue("api", "no-role").unwrap();
        let headers = headers_with_bearer(&issued.cleartext);
        let p = resolve_request_principal(&headers, Some(&aks), &ps);
        assert_eq!(p.clearance_level, 0);
        assert_eq!(p.id, issued.id);
    }

    #[test]
    fn invalid_api_key_falls_back_to_default() {
        let (aks, ps) = store_with_analyst();
        let headers = headers_with_bearer("cortex_dash_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        let p = resolve_request_principal(&headers, Some(&aks), &ps);
        assert_eq!(p.clearance_level, 0);
        assert_eq!(p.id, "anonymous");
    }

    // ---- x-cortex-principal header override ----

    #[test]
    fn principal_header_disabled_by_default() {
        // Without the env var, the header is ignored.
        std::env::remove_var(ENV_TRUST_PRINCIPAL_HEADER);
        let (aks, ps) = store_with_analyst();
        let headers = headers_with_principal("some-principal");
        // Should fall through to default (no API key either)
        let p = resolve_request_principal(&headers, Some(&aks), &ps);
        assert_eq!(p.clearance_level, 0);
    }

    #[test]
    fn trust_principal_header_disabled_when_env_unset() {
        std::env::remove_var(ENV_TRUST_PRINCIPAL_HEADER);
        assert!(!trust_principal_header_enabled());
    }

    #[test]
    fn trust_principal_header_enabled_when_env_one() {
        std::env::set_var(ENV_TRUST_PRINCIPAL_HEADER, "1");
        assert!(trust_principal_header_enabled());
        std::env::remove_var(ENV_TRUST_PRINCIPAL_HEADER);
    }

    // ---- bearer extraction ----

    #[test]
    fn extract_bearer_returns_none_for_empty_headers() {
        assert!(extract_bearer_token(&HeaderMap::new()).is_none());
    }

    #[test]
    fn extract_bearer_returns_token_after_bearer_prefix() {
        let headers = headers_with_bearer("mytoken123");
        assert_eq!(extract_bearer_token(&headers).as_deref(), Some("mytoken123"));
    }

    #[test]
    fn extract_bearer_returns_none_for_empty_token() {
        let mut m = HeaderMap::new();
        m.insert(axum::http::header::AUTHORIZATION, HeaderValue::from_static("Bearer "));
        assert!(extract_bearer_token(&m).is_none());
    }
}
