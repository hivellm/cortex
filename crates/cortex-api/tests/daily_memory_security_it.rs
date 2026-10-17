//! Authenticated daily-memory boundaries through real HTTP handlers and SQLite.

use std::path::Path;
use std::sync::{Arc, RwLock};

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::Router;
use cortex_api::storage::api_keys::{ApiKeyStore, IssuedKey};
use cortex_api::{
    AclStore, AuditStore, InMemoryCache, MemoryAuditPublisher, MemoryGraphLane, MemoryKeywordLane,
    MemoryVectorLane, Orchestrator, PrincipalStore, QueryService, RateConfig, RateLimiter,
    RoleBinding, CALLER_HEADER,
};
use cortex_storage::MetadataStore;
use serde_json::{json, Value};
use tower::ServiceExt;

fn app(path: &Path) -> (Router, IssuedKey, IssuedKey) {
    let keys = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let first = keys
        .issue_with_role("memory", "first", Some("reader"))
        .unwrap();
    let second = keys
        .issue_with_role("memory", "second", Some("reader"))
        .unwrap();
    let mut principals = PrincipalStore::deny_by_default();
    principals.upsert_binding(
        "reader",
        RoleBinding {
            clearance_level: 1,
            compartments: Vec::new(),
        },
    );
    let acl = Arc::new(AclStore::new());
    acl.allow(&first.id, "repo");
    acl.allow(&second.id, "repo");
    // A permissive adapter label must not override the authenticated principal.
    acl.allow("cortex-mcp-server", "*");
    let service = Arc::new(QueryService {
        orchestrator: Orchestrator::new(
            Arc::new(MemoryVectorLane::new()),
            Arc::new(MemoryKeywordLane::new()),
            Arc::new(MemoryGraphLane::new()),
        ),
        cache: Arc::new(InMemoryCache::new()),
        acl,
        rate_limiter: Arc::new(RateLimiter::new(RateConfig::default_for_spec_11())),
        audit: Arc::new(MemoryAuditPublisher::new()),
        audit_store: Arc::new(AuditStore::new()),
        indexed_repos: None,
        coverage_snapshot: None,
        principal_store: Some(Arc::new(RwLock::new(principals))),
        api_key_store: Some(keys.clone()),
    });
    let mut cfg = cortex_config::Config::default();
    cfg.ingestion.metadata_db = Some(path.to_string_lossy().into_owned());
    cfg.ingestion.ingestion_url = Some("http://127.0.0.1:1".into());
    cfg.access_control.enabled = true;
    cfg.access_control.deny_on_missing_principal = true;
    (
        cortex_api::http::build_router_with_auth_and_cfg(service, None, Some(keys), Arc::new(cfg)),
        first,
        second,
    )
}

async fn post(app: &Router, uri: &str, token: Option<&str>, body: Value) -> (StatusCode, Value) {
    let mut request = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json")
        .header(CALLER_HEADER, "cortex-mcp-server");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = app
        .clone()
        .oneshot(
            request
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
async fn authenticated_scope_cannot_be_broadened_by_agent_or_adapter_claims() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("metadata.sqlite");
    let (app, first, _) = app(&path);
    for body in [
        json!({"kind":"decision","repo":"other","body":"scopeguard","agent":"admin","caller":"admin"}),
        json!({"kind":"decision","repo":"repo","body":"scopeguard","global":true,"agent":"admin"}),
    ] {
        let (status, response) = post(&app, "/v1/ingest", Some(&first.cleartext), body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
    }
    for body in [
        json!({"repo":"other","query":"scopeguard"}),
        json!({"repo":"repo","query":"scopeguard","include_global":true}),
    ] {
        let (status, response) = post(&app, "/v1/memory/query", Some(&first.cleartext), body).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
    }
    for token in [None, Some("invalid-test-token")] {
        let (status, response) = post(
            &app,
            "/v1/ingest",
            token,
            json!({"kind":"learning","repo":"repo","body":"scopeguard","agent":"admin"}),
        )
        .await;
        assert!(
            matches!(status, StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN),
            "{status}: {response}"
        );
    }
    let store = MetadataStore::open(&path).unwrap();
    assert_eq!(
        store
            .daily_memory_revision(&cortex_storage::daily_memory::MemoryScope::Repository(
                "repo".into()
            ))
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn receipt_feedback_belongs_to_authenticated_caller_even_with_shared_adapter() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("metadata.sqlite");
    let (app, first, second) = app(&path);
    let (status, response) = post(
        &app,
        "/v1/ingest",
        Some(&first.cleartext),
        json!({"kind":"decision","repo":"repo","body":"feedbackguard uses durable storage","memory_id":"feedback-memory"}),
    )
    .await;
    assert!(status.is_success(), "{status}: {response}");
    let (status, delivery) = post(
        &app,
        "/v1/memory/query",
        Some(&first.cleartext),
        json!({"repo":"repo","query":"feedbackguard"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{delivery}");
    let query_id = delivery["query_id"].as_str().unwrap();
    assert_eq!(delivery["memories"][0]["id"], "feedback-memory");
    let body = json!({"query_id":query_id,"helpful":false,"caller":first.id,"agent":"admin"});
    let (status, response) = post(
        &app,
        "/v1/pre-thinking/feedback",
        Some(&second.cleartext),
        body,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
    let store = MetadataStore::open(&path).unwrap();
    assert!(store.pre_thinking_feedback(query_id).unwrap().is_none());
    let (status, response) = post(
        &app,
        "/v1/pre-thinking/feedback",
        Some(&first.cleartext),
        json!({"query_id":query_id,"helpful":true,"rating":5}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{response}");
    assert!(store.pre_thinking_feedback(query_id).unwrap().is_some());
    let receipt = store
        .daily_memory_delivery("repo", query_id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.request.memories[0].id, "feedback-memory");
    assert_eq!(receipt.request.memories[0].version, 1);
    let (status, response) = post(
        &app,
        "/v1/feedback/list",
        Some(&second.cleartext),
        json!({"limit":5}),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{response}");
}

#[tokio::test]
async fn retry_namespace_uses_authenticated_identity_not_shared_agent_label() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("metadata.sqlite");
    let (app, first, second) = app(&path);
    let body = json!({"kind":"learning","repo":"repo","body":"retryguard persists independently","agent":"shared-agent","retry_key":"same-client-retry"});
    for key in [&first, &first, &second] {
        let (status, response) = post(&app, "/v1/ingest", Some(&key.cleartext), body.clone()).await;
        assert!(status.is_success(), "{status}: {response}");
    }
    let store = MetadataStore::open(&path).unwrap();
    let results = store
        .query_daily_memories(&cortex_storage::daily_memory::MemoryQuery::new(
            "repo",
            "retryguard",
        ))
        .unwrap();
    assert_eq!(results.records.len(), 2);
    assert_eq!(results.revision, 2);
    assert_ne!(results.records[0].id, results.records[1].id);
}

#[tokio::test]
async fn production_builder_wires_separate_key_store_and_respects_local_auth_mode() {
    let temp = tempfile::tempdir().unwrap();
    let keys = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let key = keys.issue("memory", "builder-only").unwrap();
    for enabled in [false, true] {
        let service = Arc::new(QueryService::with_memory_defaults(Orchestrator::new(
            Arc::new(MemoryVectorLane::new()),
            Arc::new(MemoryKeywordLane::new()),
            Arc::new(MemoryGraphLane::new()),
        )));
        assert!(service.api_key_store.is_none());
        let mut cfg = cortex_config::Config::default();
        cfg.ingestion.metadata_db = Some(
            temp.path()
                .join(format!("builder-{enabled}.sqlite"))
                .to_string_lossy()
                .into_owned(),
        );
        cfg.access_control.enabled = enabled;
        cfg.access_control.default_level = 0;
        let router = cortex_api::http::build_router_with_auth_and_cfg(
            service,
            None,
            Some(keys.clone()),
            Arc::new(cfg),
        );
        let body = json!({"repo":"repo","query":"builderguard"});
        let (status, response) = post(
            &router,
            "/v1/memory/query",
            Some(&key.cleartext),
            body.clone(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "enabled={enabled}: {response}");
        let (status, response) = post(&router, "/v1/memory/query", None, body.clone()).await;
        assert_eq!(
            status,
            if enabled {
                StatusCode::UNAUTHORIZED
            } else {
                StatusCode::OK
            },
            "enabled={enabled}: {response}"
        );
        let (status, response) = post(
            &router,
            "/v1/memory/query",
            Some("invalid-test-token"),
            body,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "enabled={enabled}: {response}"
        );
    }
}
