//! Phase21 §6.2 — ACL admin HTTP routes.
//!
//! Routes:
//! - `GET  /v1/acl/roles`          — list all RBAC role bindings
//! - `POST /v1/acl/roles`          — create or overwrite a binding   [acl_admin]
//! - `POST /v1/acl/grants`         — assign a role to a principal     [acl_admin]
//! - `GET  /v1/acl/classify-rules` — list active path-classification rules
//! - `GET  /v1/acl/whoami`         — resolve the caller's effective principal
//!
//! Non-`acl_admin` callers that attempt a mutation receive:
//!
//! ```json
//! { "reason": "forbidden", "detail": "acl_admin role required for this operation" }
//! ```
//!
//! The `classify-rules` endpoint returns an empty list in this phase;
//! live-reload of `[cortex.classification]` rules is wired in §7.1 when
//! `AccessControlConfig` is added to `cortex-config`.

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::json;

use crate::http::ApiState;
use crate::principal_store::RoleBinding;

// ── Helpers ──────────────────────────────────────────────────────────────────

fn json_err(status: StatusCode, reason: &str, detail: impl Into<String>) -> Response {
    (
        status,
        Json(json!({ "reason": reason, "detail": detail.into() })),
    )
        .into_response()
}

/// Return `Some(403 response)` when the resolved principal is not `acl_admin`.
fn acl_admin_guard(headers: &HeaderMap, state: &ApiState) -> Option<Response> {
    let p = state.service.resolve_principal(headers);
    if p.is_acl_admin() {
        None
    } else {
        Some(json_err(
            StatusCode::FORBIDDEN,
            "forbidden",
            "acl_admin role required for this operation",
        ))
    }
}

// ── GET /v1/acl/roles ────────────────────────────────────────────────────────

/// List all registered RBAC role bindings. Any caller may read; only
/// `acl_admin` may write.
async fn handle_acl_role_list(State(state): State<ApiState>) -> Response {
    let roles: Vec<serde_json::Value> = match &state.service.principal_store {
        None => vec![],
        Some(lock) => {
            let store = lock.read().expect("principal_store RwLock poisoned");
            let mut entries: Vec<serde_json::Value> = store
                .bindings()
                .iter()
                .map(|(name, b)| {
                    json!({
                        "name": name,
                        "clearance_level": b.clearance_level,
                        "compartments": b.compartments,
                    })
                })
                .collect();
            // Stable output order — sort by name so callers can diff easily.
            entries.sort_by(|a, b| {
                a["name"]
                    .as_str()
                    .unwrap_or("")
                    .cmp(b["name"].as_str().unwrap_or(""))
            });
            entries
        }
    };
    Json(json!({ "roles": roles })).into_response()
}

// ── POST /v1/acl/roles ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct CreateRoleBody {
    name: String,
    clearance_level: u8,
    #[serde(default)]
    compartments: Vec<String>,
}

/// Create or overwrite a RBAC role binding. Requires `acl_admin`.
async fn handle_acl_role_create(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<CreateRoleBody>,
) -> Response {
    if let Some(err) = acl_admin_guard(&headers, &state) {
        return err;
    }
    let name = body.name.trim().to_string();
    if name.is_empty() {
        return json_err(
            StatusCode::BAD_REQUEST,
            "invalid_name",
            "role name must not be empty",
        );
    }
    if body.clearance_level > 3 {
        return json_err(
            StatusCode::BAD_REQUEST,
            "invalid_clearance",
            format!(
                "clearance_level must be 0–3; got {}",
                body.clearance_level
            ),
        );
    }
    let binding = RoleBinding {
        clearance_level: body.clearance_level,
        compartments: body.compartments.clone(),
    };
    match &state.service.principal_store {
        None => json_err(
            StatusCode::SERVICE_UNAVAILABLE,
            "no_principal_store",
            "access control is not configured on this daemon",
        ),
        Some(lock) => {
            lock.write()
                .expect("principal_store RwLock poisoned")
                .upsert_binding(name.clone(), binding);
            Json(json!({
                "ok": true,
                "name": name,
                "clearance_level": body.clearance_level,
                "compartments": body.compartments,
            }))
            .into_response()
        }
    }
}

// ── POST /v1/acl/grants ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct GrantBody {
    principal_id: String,
    role: Option<String>,
    clearance_level: Option<u8>,
    #[serde(default)]
    compartments: Vec<String>,
}

/// Grant a principal an explicit role, clearance level, or compartments.
///
/// If `role` is provided the named binding must exist; its
/// clearance + compartments are copied into a per-principal synthetic
/// binding `"principal:<id>"`. If `clearance_level`/`compartments` are
/// given directly a synthetic binding is upserted from those values.
///
/// When the daemon's `ApiKeyStore` is wired the synthetic role is also
/// assigned on the matching key row so the resolver picks it up.
///
/// Requires `acl_admin`.
async fn handle_acl_grant(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<GrantBody>,
) -> Response {
    if let Some(err) = acl_admin_guard(&headers, &state) {
        return err;
    }
    let principal_id = body.principal_id.trim().to_string();
    if principal_id.is_empty() {
        return json_err(
            StatusCode::BAD_REQUEST,
            "invalid_principal",
            "principal_id must not be empty",
        );
    }
    if body.role.is_none() && body.clearance_level.is_none() && body.compartments.is_empty() {
        return json_err(
            StatusCode::BAD_REQUEST,
            "missing_grant",
            "at least one of role / clearance_level / compartments is required",
        );
    }
    if let Some(c) = body.clearance_level {
        if c > 3 {
            return json_err(
                StatusCode::BAD_REQUEST,
                "invalid_clearance",
                format!("clearance_level must be 0–3; got {c}"),
            );
        }
    }

    let ps = match &state.service.principal_store {
        Some(ps) => Arc::clone(ps),
        None => {
            return json_err(
                StatusCode::SERVICE_UNAVAILABLE,
                "no_principal_store",
                "access control is not configured on this daemon",
            )
        }
    };

    // Resolve or build the effective binding.
    let binding = {
        let store = ps.read().expect("principal_store RwLock poisoned");
        if let Some(role_name) = &body.role {
            match store.bindings().get(role_name.as_str()).cloned() {
                Some(b) => b,
                None => {
                    return json_err(
                        StatusCode::NOT_FOUND,
                        "role_not_found",
                        format!("role '{role_name}' is not registered in this daemon"),
                    )
                }
            }
        } else {
            RoleBinding {
                clearance_level: body.clearance_level.unwrap_or(0),
                compartments: body.compartments.clone(),
            }
        }
    };

    let synthetic_role = format!("principal:{principal_id}");
    let granted_level = binding.clearance_level;
    let granted_comps = binding.compartments.clone();

    ps.write()
        .expect("principal_store RwLock poisoned")
        .upsert_binding(synthetic_role.clone(), binding);

    // Best-effort ApiKeyStore assignment — silently skipped when not wired
    // or when the key ID does not exist.
    if let Some(aks) = &state.service.api_key_store {
        let _ = aks.assign_role(&principal_id, Some(&synthetic_role));
    }

    Json(json!({
        "ok": true,
        "principal_id": principal_id,
        "synthetic_role": synthetic_role,
        "clearance_level": granted_level,
        "compartments": granted_comps,
    }))
    .into_response()
}

// ── GET /v1/acl/classify-rules ───────────────────────────────────────────────

/// List active classification path rules from cortex.toml.
///
/// Phase21 §6.2 initial cut: returns an empty list. Full live-reload
/// is wired in §7.1 when `AccessControlConfig` is added to `cortex-config`
/// and threaded into `ApiState`.
async fn handle_acl_classify_rule_list(_state: State<ApiState>) -> Response {
    Json(json!({ "rules": [] })).into_response()
}

// ── GET /v1/acl/whoami ───────────────────────────────────────────────────────

/// Resolve and return the caller's effective principal.
///
/// Returns `{ id, clearance_level, compartment_grants, roles }`.
/// When no principal store is configured, the backward-compat super-admin
/// pass-through is returned.
async fn handle_acl_whoami(
    State(state): State<ApiState>,
    headers: HeaderMap,
) -> Response {
    let principal = state.service.resolve_principal(&headers);
    let body = serde_json::to_value(&principal).unwrap_or_else(|_| json!({}));
    Json(body).into_response()
}

// ── Router ───────────────────────────────────────────────────────────────────

/// Build the `/v1/acl/*` sub-router. Call `Router::merge` with the result
/// inside `build_router_with_auth_and_cfg`.
pub fn build_acl_router(state: ApiState) -> Router {
    Router::new()
        .route(
            "/v1/acl/roles",
            get(handle_acl_role_list).post(handle_acl_role_create),
        )
        .route("/v1/acl/grants", post(handle_acl_grant))
        .route("/v1/acl/classify-rules", get(handle_acl_classify_rule_list))
        .route("/v1/acl/whoami", get(handle_acl_whoami))
        .with_state(state)
}
