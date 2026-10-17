//! Service cache hits must preserve the same authority as fresh retrieval.
use std::{
    collections::BTreeMap,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use async_trait::async_trait;
use axum::http::{HeaderMap, HeaderValue};
use cortex_api::{
    lanes::{KeywordLane, KeywordRequest, LaneError, LaneHit, LaneSource, Overlay},
    storage::api_keys::{ApiKeyStore, IssuedKey},
    IncludeField, Intent, MemoryGraphLane, MemoryVectorLane, Orchestrator, Principal,
    PrincipalStore, QueryRequest, QueryResponse, QueryService, RateConfig, RateLimiter,
    RoleBinding, Scope, ServiceOutcome,
};
use cortex_config::AccessControlConfig;
use serde_json::json;

static AUTH_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct ClassifiedLane;
#[async_trait]
impl KeywordLane for ClassifiedLane {
    async fn search(&self, _request: &KeywordRequest) -> Result<Vec<LaneHit>, LaneError> {
        Ok([
            ("public", 0, Vec::<&str>::new()),
            ("secret", 3, vec!["security"]),
        ]
        .into_iter()
        .map(|(id, level, compartments)| LaneHit {
            doc_id: id.into(),
            text: format!("query {id}"),
            repo: Some("repo".into()),
            path: Some(format!("{id}.rs")),
            symbol: None,
            content_hash: None,
            score: 1.0,
            ts: 0,
            severity: None,
            extras: BTreeMap::from([
                ("class_level".into(), json!(level)),
                ("class_compartments".into(), json!(compartments)),
            ]),
            overlay: Overlay {
                source: LaneSource::Keyword,
                ..Default::default()
            },
        })
        .collect())
    }
}

fn service() -> QueryService {
    QueryService::with_memory_defaults(
        Orchestrator::new(
            Arc::new(MemoryVectorLane::new()),
            Arc::new(ClassifiedLane),
            Arc::new(MemoryGraphLane::new()),
        )
        .with_access_control(AccessControlConfig {
            enabled: true,
            ..Default::default()
        }),
    )
}
fn request() -> QueryRequest {
    QueryRequest {
        intent: Intent::PreChangeContext,
        scope: Scope {
            repo: Some("repo".into()),
            ..Default::default()
        },
        query: "query".into(),
        limit: 20,
        k: 50,
        include: vec![IncludeField::Snippets],
        budget_ms: 2000,
        budget_bytes: None,
        as_of: None,
        branch: None,
        projects: None,
        include_history: None,
        include_future: None,
        include_branches: None,
        principal: None,
    }
}
fn headers(key: &IssuedKey) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        "authorization",
        HeaderValue::from_str(&format!("Bearer {}", key.cleartext)).unwrap(),
    );
    headers
}
fn response(outcome: ServiceOutcome) -> Box<QueryResponse> {
    match outcome {
        ServiceOutcome::Ok(response) => response,
        outcome => panic!("expected query success, got {outcome:?}"),
    }
}
fn has_secret(response: &QueryResponse) -> bool {
    response
        .results
        .snippets
        .iter()
        .any(|s| s.path.as_deref() == Some("secret.rs"))
}
fn binding(level: u8, compartments: &[&str]) -> RoleBinding {
    RoleBinding {
        clearance_level: level,
        compartments: compartments.iter().map(|s| s.to_string()).collect(),
    }
}

#[tokio::test]
async fn authenticated_cache_isolates_principals_grants_and_revoked_credentials() {
    let _serial = AUTH_TESTS.lock().await;
    let keys = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let admin = keys
        .issue_with_role("api", "admin", Some("acl_admin"))
        .unwrap();
    let reader = keys
        .issue_with_role("api", "reader", Some("reader"))
        .unwrap();
    let principals = Arc::new(RwLock::new(PrincipalStore::new(
        [
            ("acl_admin".into(), binding(3, &[])),
            ("reader".into(), binding(0, &[])),
        ],
        Principal::public_only("default"),
    )));
    let svc = service()
        .with_principal_store(principals.clone())
        .with_api_key_store(keys.clone());
    let admin_headers = headers(&admin);
    let reader_headers = headers(&reader);
    let high = response(
        svc.handle_with_headers("adapter", request(), &admin_headers)
            .await,
    );
    assert!(has_secret(&high));
    let high_hit = response(
        svc.handle_with_headers("adapter", request(), &admin_headers)
            .await,
    );
    assert_eq!(high_hit.budget.cache, "hit");
    let low = response(
        svc.handle_with_headers("adapter", request(), &reader_headers)
            .await,
    );
    assert!(
        !has_secret(&low),
        "reader must not receive administrator cache entry"
    );
    assert_eq!(low.budget.cache, "miss");
    principals
        .write()
        .unwrap()
        .upsert_binding("reader", binding(3, &["security"]));
    assert!(has_secret(&response(
        svc.handle_with_headers("adapter", request(), &reader_headers)
            .await
    )));
    principals
        .write()
        .unwrap()
        .upsert_binding("reader", binding(3, &[]));
    assert!(!has_secret(&response(
        svc.handle_with_headers("adapter", request(), &reader_headers)
            .await
    )));
    keys.revoke(&reader.id).unwrap();
    assert!(matches!(
        svc.handle_with_headers("adapter", request(), &reader_headers)
            .await,
        ServiceOutcome::Denied
    ));
    let mut invalid = HeaderMap::new();
    invalid.insert("authorization", HeaderValue::from_static("Bearer invalid"));
    assert!(matches!(
        svc.handle_with_headers("adapter", request(), &invalid)
            .await,
        ServiceOutcome::Denied
    ));
}

#[tokio::test]
async fn credential_identity_controls_acl_and_rate_limit_while_audit_keeps_observed_caller() {
    let _serial = AUTH_TESTS.lock().await;
    let keys = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    let key = keys.issue("api", "reader").unwrap();
    let mut svc = service().with_api_key_store(keys);
    svc.acl.allow("spoofed-admin", "*");
    svc.acl.set_allowed(&key.id, vec![]);
    let headers = headers(&key);
    assert!(matches!(
        svc.handle_with_headers("spoofed-admin", request(), &headers)
            .await,
        ServiceOutcome::Denied
    ));
    svc.acl.allow(&key.id, "repo");
    svc.acl.set_allowed("denied-adapter", vec![]);
    svc.rate_limiter = Arc::new(RateLimiter::new(RateConfig {
        rps_sustained: 0,
        rps_burst: 1,
    }));
    let result = response(
        svc.handle_with_headers("denied-adapter", request(), &headers)
            .await,
    );
    let envelope = svc.audit_store.get(&result.query_id).unwrap();
    assert_eq!(envelope["caller"], "denied-adapter");
    assert!(matches!(
        svc.handle_with_headers("different-adapter", request(), &headers)
            .await,
        ServiceOutcome::RateLimited(_)
    ));
}

#[tokio::test]
async fn roles_and_live_access_control_changes_invalidate_cache() {
    let _serial = AUTH_TESTS.lock().await;
    let mut principal = Principal {
        id: "same-id".into(),
        clearance_level: 3,
        compartment_grants: vec![],
        roles: vec!["acl_admin".into()],
    };
    let principals = Arc::new(RwLock::new(PrincipalStore::new([], principal.clone())));
    let svc = service().with_principal_store(principals.clone());
    assert!(has_secret(&response(
        svc.handle("adapter", request()).await
    )));
    principal.roles.clear();
    principals.write().unwrap().set_default_principal(principal);
    assert!(!has_secret(&response(
        svc.handle("adapter", request()).await
    )));
    svc.orchestrator
        .replace_access_control(AccessControlConfig::default());
    assert!(has_secret(&response(
        svc.handle("adapter", request()).await
    )));
    svc.orchestrator
        .replace_access_control(AccessControlConfig {
            enabled: true,
            ..Default::default()
        });
    assert!(!has_secret(&response(
        svc.handle("adapter", request()).await
    )));
}

#[tokio::test]
async fn branch_and_response_shape_filters_get_separate_service_cache_entries() {
    let svc = service();
    response(svc.handle("adapter", request()).await);
    assert_eq!(
        response(svc.handle("adapter", request()).await)
            .budget
            .cache,
        "hit"
    );
    let mut variants = Vec::new();
    let mut branch = request();
    branch.branch = Some("other-branch".into());
    variants.push(branch);
    let mut limit = request();
    limit.limit = 1;
    variants.push(limit);
    let mut bytes = request();
    bytes.budget_bytes = Some(1024);
    variants.push(bytes);
    let mut include = request();
    include.include = vec![];
    variants.push(include);
    let mut history = request();
    history.include_history = Some(true);
    variants.push(history);
    for variant in variants {
        assert_eq!(
            response(svc.handle("adapter", variant).await).budget.cache,
            "miss"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn blocked_policy_resolution_does_not_block_executor_or_outer_deadline() {
    let _serial = AUTH_TESTS.lock().await;
    let principals = Arc::new(RwLock::new(PrincipalStore::deny_by_default()));
    let svc = service().with_principal_store(principals.clone());
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _guard = principals.write().unwrap();
        ready_tx.send(()).unwrap();
        let _ = release_rx.recv_timeout(Duration::from_millis(500));
    });
    ready_rx.recv().unwrap();
    let start = Instant::now();
    let result = tokio::time::timeout(
        Duration::from_millis(30),
        svc.handle_with_headers("adapter", request(), &HeaderMap::new()),
    )
    .await;
    let elapsed = start.elapsed();
    let _ = release_tx.send(());
    holder.join().unwrap();
    assert!(
        result.is_err(),
        "outer deadline must cancel stalled principal resolution"
    );
    assert!(
        elapsed < Duration::from_millis(200),
        "executor blocked for {elapsed:?}"
    );
}
