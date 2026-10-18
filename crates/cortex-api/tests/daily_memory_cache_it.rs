//! Revision-aware context caching still authorizes and records every preparation.
use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    Router,
};
use cortex_api::{
    storage::api_keys::{ApiKeyStore, IssuedKey},
    AclStore, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane, Orchestrator, PrincipalStore,
    QueryService, RoleBinding,
};
use cortex_storage::{
    daily_memory::{MemoryCapture, MemoryCategory, MemoryOrigin, MemoryScope},
    MetadataStore,
};
use serde_json::{json, Value};
use std::{
    path::Path,
    sync::{Arc, RwLock},
};
use tower::ServiceExt;

fn service() -> QueryService {
    QueryService::with_memory_defaults(Orchestrator::new(
        Arc::new(MemoryVectorLane::new().with_fail()),
        Arc::new(MemoryKeywordLane::new().with_fail()),
        Arc::new(MemoryGraphLane::new().with_fail()),
    ))
}
fn config(database: &Path) -> cortex_config::Config {
    let mut cfg = cortex_config::Config::default();
    cfg.ingestion.metadata_db = Some(database.to_string_lossy().into_owned());
    cfg.ingestion.home = None;
    cfg
}
fn app(database: &Path) -> Router {
    // Isolate cache assertions from first-open schema bootstrap.
    drop(MetadataStore::open(database).unwrap());
    cortex_api::http::build_router_with_auth_and_cfg(
        Arc::new(service()),
        None,
        None,
        Arc::new(config(database)),
    )
}
async fn post(app: &Router, token: Option<&str>, uri: &str, body: Value) -> (StatusCode, Value) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    let operation = async {
        let response = app
            .clone()
            .oneshot(
                builder
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 65536).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    };
    // Cache semantics use real SQLite/auth paths, but not host scheduling deadlines.
    // A ready polling branch prevents paused Tokio time from auto-advancing.
    let limit = std::time::Instant::now() + std::time::Duration::from_secs(10);
    tokio::pin!(operation);
    loop {
        assert!(
            std::time::Instant::now() < limit,
            "cache fixture stalled: POST {uri}, body={body}"
        );
        tokio::select! {
            biased;
            response = &mut operation => return response,
            _ = tokio::task::yield_now() => {}
        }
    }
}
async fn read(app: &Router, body: Value) -> Value {
    let (status, response) = post(app, None, "/v1/memory/query", body.clone()).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "POST /v1/memory/query body={body}; response={response}"
    );
    response
}
fn query() -> Value {
    json!({"repo":"repo","query":"cachetoken"})
}
fn seed(database: &Path, id: &str, repo: &str, body: &str) {
    let mut capture = MemoryCapture::new(repo, "Cache guidance", body, MemoryCategory::Lesson);
    capture.id = Some(id.to_owned());
    MetadataStore::open(database)
        .unwrap()
        .capture_daily_memory(&capture)
        .unwrap();
}

struct AuthApp {
    app: Router,
    keys: Arc<ApiKeyStore>,
    acl: Arc<AclStore>,
    admin: IssuedKey,
    reader: IssuedKey,
}
fn authenticated(database: &Path) -> AuthApp {
    let keys = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let admin = keys
        .issue_with_role("memory", "admin", Some("acl_admin"))
        .unwrap();
    let reader = keys
        .issue_with_role("memory", "reader", Some("reader"))
        .unwrap();
    let mut principals = PrincipalStore::deny_by_default();
    for (role, clearance) in [("acl_admin", 3), ("reader", 1)] {
        principals.upsert_binding(
            role,
            RoleBinding {
                clearance_level: clearance,
                compartments: Vec::new(),
            },
        );
    }
    let acl = Arc::new(AclStore::new());
    acl.allow(&admin.id, "*");
    acl.allow(&reader.id, "repo");
    let mut service = service();
    service.api_key_store = Some(keys.clone());
    service.principal_store = Some(Arc::new(RwLock::new(principals)));
    service.acl = acl.clone();
    let mut cfg = config(database);
    cfg.access_control.enabled = true;
    cfg.access_control.deny_on_missing_principal = true;
    AuthApp {
        app: cortex_api::http::build_router_with_auth_and_cfg(
            Arc::new(service),
            None,
            Some(keys.clone()),
            Arc::new(cfg),
        ),
        keys,
        acl,
        admin,
        reader,
    }
}

#[tokio::test(start_paused = true)]
async fn cache_hit_reuses_only_context_and_persists_a_fresh_receipt() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("metadata.sqlite");
    seed(
        &database,
        "original",
        "repo",
        "cachetoken exact original advice",
    );
    let app = app(&database);
    let first = read(&app, query()).await;
    let second = read(&app, query()).await;
    assert_eq!(first["cache_hit"], false);
    assert_eq!(second["cache_hit"], true);
    assert_eq!(first["context"], second["context"]);
    assert_eq!(first["memories"], second["memories"]);
    assert_ne!(first["delivery_id"], second["delivery_id"]);
    let store = MetadataStore::open(&database).unwrap();
    for response in [first, second] {
        let delivery = store
            .daily_memory_delivery("repo", response["delivery_id"].as_str().unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(
            serde_json::to_value(delivery.request.memories).unwrap(),
            response["memories"]
        );
        assert_eq!(
            delivery.request.context_bytes,
            response["context"].as_str().unwrap().len()
        );
    }
}

#[tokio::test(start_paused = true)]
async fn correction_restart_and_stale_evidence_invalidate_cached_context_immediately() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("metadata.sqlite");
    seed(&database, "old", "repo", "cachetoken old instruction");
    let running = app(&database);
    let first = read(&running, query()).await;
    assert_eq!(read(&running, query()).await["cache_hit"], true);
    let (status, correction) = post(&running, None, "/v1/ingest", json!({"kind":"decision","repo":"repo","body":"cachetoken corrected instruction","memory_id":"corrected","supersedes":"old","expected_revision":first["revision"]})).await;
    assert_eq!(status, StatusCode::ACCEPTED, "{correction}");
    let corrected = read(&running, query()).await;
    assert_eq!(corrected["cache_hit"], false);
    assert!(!corrected["context"]
        .as_str()
        .unwrap()
        .contains("old instruction"));
    assert_eq!(corrected["memories"][0]["id"], "corrected");
    assert_eq!(read(&running, query()).await["cache_hit"], true);
    let restarted = app(&database);
    let fresh = read(&restarted, query()).await;
    assert_eq!(fresh["cache_hit"], false);
    assert_eq!(fresh["context"], corrected["context"]);
    MetadataStore::open(&database)
        .unwrap()
        .mark_daily_memory_stale(
            &MemoryScope::Repository("repo".to_owned()),
            "corrected",
            true,
            fresh["revision"].as_u64().unwrap(),
            &MemoryOrigin::default(),
        )
        .unwrap();
    let stale = read(&running, query()).await;
    assert_eq!(stale["cache_hit"], false);
    assert_eq!(stale["context"], "");
    assert_eq!(read(&running, query()).await["cache_hit"], true);
}

#[tokio::test(start_paused = true)]
async fn cache_keys_keep_branch_path_repo_prompt_limit_and_budget_isolated() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("metadata.sqlite");
    let mut restricted = MemoryCapture::new(
        "repo",
        "Scoped advice",
        "cachetoken branch path guidance",
        MemoryCategory::Lesson,
    );
    restricted.id = Some("restricted".to_owned());
    restricted.branch = Some("main".to_owned());
    restricted.path = Some("src/memory".to_owned());
    MetadataStore::open(&database)
        .unwrap()
        .capture_daily_memory(&restricted)
        .unwrap();
    seed(
        &database,
        "foreign",
        "other",
        "cachetoken foreign repository guidance",
    );
    let app = app(&database);
    let selected =
        json!({"repo":"repo","query":"cachetoken","branch":"main","paths":["src/memory/store.rs"]});
    let first = read(&app, selected.clone()).await;
    assert_eq!(first["memories"][0]["id"], "restricted");
    assert_eq!(read(&app, selected.clone()).await["cache_hit"], true);
    for changed in [
        json!({"repo":"repo","query":"cachetoken","branch":"other","paths":["src/memory/store.rs"]}),
        json!({"repo":"repo","query":"cachetoken","branch":"main","paths":["src/memory-other/store.rs"]}),
        json!({"repo":"repo","query":"unrelated","branch":"main","paths":["src/memory/store.rs"]}),
        json!({"repo":"repo","query":"cachetoken","branch":"main","paths":["src/memory/store.rs"],"limit":0}),
        json!({"repo":"repo","query":"cachetoken","branch":"main","paths":["src/memory/store.rs"],"budget_bytes":1}),
    ] {
        let response = read(&app, changed.clone()).await;
        assert_eq!(response["cache_hit"], false);
        assert_eq!(response["context"], "");
        assert_eq!(read(&app, changed).await["cache_hit"], true);
    }
    let foreign = read(&app, json!({"repo":"other","query":"cachetoken"})).await;
    assert_eq!(foreign["cache_hit"], false);
    assert_eq!(foreign["memories"][0]["id"], "foreign");
    assert_eq!(
        read(&app, selected).await["memories"][0]["id"],
        "restricted"
    );
}

#[tokio::test(start_paused = true)]
async fn authenticated_caller_cache_isolation_and_revocation_precede_every_hit() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("metadata.sqlite");
    seed(&database, "shared", "repo", "cachetoken authorized advice");
    let auth = authenticated(&database);
    for token in [&auth.admin.cleartext, &auth.reader.cleartext] {
        let (status, first) = post(&auth.app, Some(token), "/v1/memory/query", query()).await;
        assert_eq!(status, StatusCode::OK, "{first}");
        assert_eq!(first["cache_hit"], false);
        let (_, repeated) = post(&auth.app, Some(token), "/v1/memory/query", query()).await;
        assert_eq!(repeated["cache_hit"], true);
    }
    auth.acl.set_allowed(&auth.reader.id, Vec::new());
    assert_eq!(
        post(
            &auth.app,
            Some(&auth.reader.cleartext),
            "/v1/memory/query",
            query()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    auth.keys.revoke(&auth.admin.id).unwrap();
    assert_eq!(
        post(
            &auth.app,
            Some(&auth.admin.cleartext),
            "/v1/memory/query",
            query()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test(start_paused = true)]
async fn global_revision_and_explicit_opt_in_invalidate_only_eligible_cache_entries() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("metadata.sqlite");
    seed(&database, "local", "repo", "cachetoken local advice");
    let mut global = MemoryCapture::new(
        "repo",
        "Global advice",
        "cachetoken global original advice",
        MemoryCategory::Lesson,
    );
    global.id = Some("global-old".to_owned());
    global.scope = MemoryScope::Global;
    MetadataStore::open(&database)
        .unwrap()
        .capture_daily_memory(&global)
        .unwrap();
    let auth = authenticated(&database);
    let with_global = json!({"repo":"repo","query":"cachetoken","include_global":true});
    let (_, local_only) = post(
        &auth.app,
        Some(&auth.admin.cleartext),
        "/v1/memory/query",
        query(),
    )
    .await;
    assert_eq!(local_only["memories"].as_array().unwrap().len(), 1);
    let (status, first) = post(
        &auth.app,
        Some(&auth.admin.cleartext),
        "/v1/memory/query",
        with_global.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{first}");
    assert_eq!(first["cache_hit"], false);
    assert_eq!(first["memories"].as_array().unwrap().len(), 2);
    assert_eq!(
        post(
            &auth.app,
            Some(&auth.admin.cleartext),
            "/v1/memory/query",
            with_global.clone()
        )
        .await
        .1["cache_hit"],
        true
    );
    let mut successor = global;
    successor.id = Some("global-new".to_owned());
    successor.body = "cachetoken global corrected advice".to_owned();
    successor.supersedes = Some("global-old".to_owned());
    successor.expected_revision = first["global_revision"].as_u64();
    MetadataStore::open(&database)
        .unwrap()
        .capture_daily_memory(&successor)
        .unwrap();
    let (_, corrected) = post(
        &auth.app,
        Some(&auth.admin.cleartext),
        "/v1/memory/query",
        with_global,
    )
    .await;
    assert_eq!(corrected["cache_hit"], false);
    assert!(corrected["context"]
        .as_str()
        .unwrap()
        .contains("global corrected advice"));
    assert!(!corrected["context"]
        .as_str()
        .unwrap()
        .contains("global original advice"));
    let (_, local_only_again) = post(
        &auth.app,
        Some(&auth.admin.cleartext),
        "/v1/memory/query",
        query(),
    )
    .await;
    assert_eq!(local_only_again["cache_hit"], true);
    assert_eq!(local_only_again["global_revision"], Value::Null);
    assert_eq!(
        post(
            &auth.app,
            Some(&auth.reader.cleartext),
            "/v1/memory/query",
            json!({"repo":"repo","query":"cachetoken","include_global":true})
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test(start_paused = true)]
async fn cache_evicts_after_256_successful_distinct_contexts() {
    let temp = tempfile::tempdir().unwrap();
    let database = temp.path().join("metadata.sqlite");
    let app = app(&database);
    assert_eq!(read(&app, query()).await["cache_hit"], false);
    for i in 0..256 {
        let response = read(&app, json!({"repo":"repo","query":format!("distinct{i}")})).await;
        assert_eq!(response["cache_hit"], false);
    }
    assert_eq!(read(&app, query()).await["cache_hit"], false);
    assert_eq!(read(&app, query()).await["cache_hit"], true);
}
