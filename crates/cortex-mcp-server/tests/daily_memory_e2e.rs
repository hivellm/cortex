//! Required offline E2E: real MCP dispatch, TCP/Axum API and file-backed SQLite.
//! Ingestion is unavailable and bulk lanes fail; installed client harnesses are not exercised.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use cortex_api::{
    build_router_with_auth_and_cfg, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane,
    Orchestrator, QueryService,
};
use cortex_mcp_server::{Server, ToolContext};
use cortex_storage::daily_memory::{
    MemoryDeliveredItem, MemoryDeliveryStage, MemoryScope, MemoryStatus, OriginAttribution,
};
use cortex_storage::MetadataStore;
use serde_json::{json, Value};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

struct RunningApi {
    url: String,
    mcp: Server,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl RunningApi {
    async fn start(metadata: &Path) -> Self {
        let mut cfg = cortex_config::Config::default();
        cfg.ingestion.metadata_db = Some(metadata.to_string_lossy().into_owned());
        cfg.ingestion.ingestion_url = Some("http://127.0.0.1:1".to_owned());
        let service = QueryService::with_memory_defaults(Orchestrator::new(
            Arc::new(MemoryVectorLane::new().with_fail()),
            Arc::new(MemoryKeywordLane::new().with_fail()),
            Arc::new(MemoryGraphLane::new().with_fail()),
        ));
        let router = build_router_with_auth_and_cfg(Arc::new(service), None, None, Arc::new(cfg));
        Self::serve(router, None).await
    }

    async fn serve(router: axum::Router, api_key: Option<String>) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (shutdown, receiver) = oneshot::channel();
        let task = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(async {
                    let _ = receiver.await;
                })
                .await
                .unwrap();
        });
        let mut context = ToolContext::new(&url);
        if let Some(key) = api_key {
            context = context.with_api_key(key);
        }
        Self {
            mcp: Server::new(context),
            url,
            shutdown: Some(shutdown),
            task: Some(task),
        }
    }

    async fn stop(mut self) {
        self.shutdown.take().unwrap().send(()).unwrap();
        let mut task = self.task.take().unwrap();
        match tokio::time::timeout(Duration::from_secs(5), &mut task).await {
            Ok(result) => result.unwrap(),
            Err(_) => {
                task.abort();
                panic!("Axum did not stop before restart deadline");
            }
        }
    }
}

impl Drop for RunningApi {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn rpc(server: &Server, method: &str, params: Value) -> Value {
    let frame = serde_json::to_vec(&json!({
        "jsonrpc": "2.0", "id": 1, "method": method, "params": params
    }))
    .unwrap();
    let raw = tokio::time::timeout(Duration::from_secs(10), server.handle_frame(&frame))
        .await
        .expect("MCP call exceeded E2E deadline")
        .unwrap();
    let response: Value = serde_json::from_slice(&raw).unwrap();
    assert!(response.get("error").is_none(), "MCP error: {response}");
    response["result"].clone()
}

async fn invoke(server: &Server, tool: &str, args: Value) -> Value {
    let result = rpc(
        server,
        "tools/call",
        json!({ "name": tool, "arguments": args }),
    )
    .await;
    assert_eq!(result["isError"], false, "{tool} failed: {result}");
    serde_json::from_str(result["content"][0]["text"].as_str().unwrap()).unwrap()
}

fn query() -> Value {
    json!({ "repo": "Exact/Repo", "query": "dailyrecall", "branch": "main", "paths": ["src/memory/store.rs"] })
}

#[tokio::test]
async fn ordinary_prethinking_delivers_current_memory_across_sessions_cache_and_restart() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("metadata.sqlite");
    let repo = directory.path().join("repository");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::write(repo.join("cortex.toml"), "[cortex]\nid = \"Exact/Repo\"\n").unwrap();
    let request = json!({"user_prompt":"Implement memorychain in src/store.rs", "cwd":repo,
        "budget_ms":600,"session_id":"fresh-observed-session"});
    let api = RunningApi::start(&database).await;
    invoke(&api.mcp,"cortex_capture_memory",json!({"kind":"decision","repo":"Exact/Repo",
        "memory_id":"original-memorychain","body":"memorychain: retain the old context after correction.",
        "branch":"main","path":"src","agent":"session-a"})).await;
    invoke(
        &api.mcp,
        "cortex_capture_memory",
        json!({"kind":"decision","repo":"other-project",
        "memory_id":"foreign-memorychain","body":"memorychain: advice from another project."}),
    )
    .await;
    let first = invoke(&api.mcp, "cortex_pre_thinking", request.clone()).await;
    assert!(
        first["bundle"]
            .as_str()
            .unwrap()
            .contains("original-memorychain"),
        "{first}"
    );
    assert!(!first["bundle"]
        .as_str()
        .unwrap()
        .contains("foreign-memorychain"));
    let first_id = first["memory_delivery_id"].as_str().unwrap();
    let second = invoke(&api.mcp, "cortex_pre_thinking", request.clone()).await;
    assert_eq!(second["memory_cache_hit"], true);
    assert_ne!(second["memory_delivery_id"], first_id);
    let receipt = MetadataStore::open(&database)
        .unwrap()
        .daily_memory_delivery("Exact/Repo", first_id)
        .unwrap()
        .unwrap();
    assert_eq!(receipt.request.memories.len(), 1);
    assert_eq!(receipt.request.memories[0].id, "original-memorychain");
    assert!(receipt.request.context_bytes <= 1536);
    invoke(&api.mcp,"cortex_capture_memory",json!({"kind":"decision","repo":"Exact/Repo",
        "memory_id":"corrected-memorychain","body":"memorychain: invalidate context after committed correction.",
        "expected_revision":second["memory_revision"],"supersedes":"original-memorychain",
        "branch":"main","path":"src","agent":"session-b"})).await;
    let corrected = invoke(&api.mcp, "cortex_pre_thinking", request.clone()).await;
    let context = corrected["bundle"].as_str().unwrap();
    assert!(context.contains("corrected-memorychain"), "{corrected}");
    assert!(context.contains("invalidate context after committed correction"));
    assert!(!context.contains("original-memorychain"));
    assert!(!context.contains("foreign-memorychain"));
    assert_eq!(corrected["memory_cache_hit"], false);
    api.stop().await;
    let restarted = RunningApi::start(&database).await;
    let result = invoke(&restarted.mcp, "cortex_pre_thinking", request).await;
    assert!(result["bundle"]
        .as_str()
        .unwrap()
        .contains("corrected-memorychain"));
    assert!(!result["bundle"]
        .as_str()
        .unwrap()
        .contains("original-memorychain"));
    restarted.stop().await;
}

#[tokio::test]
async fn authenticated_prethinking_recalls_memory_within_production_deadline() {
    use cortex_api::storage::api_keys::ApiKeyStore;
    use cortex_api::{PrincipalStore, RoleBinding};
    use cortex_storage::daily_memory::{MemoryCapture, MemoryCategory};
    use std::sync::RwLock;
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("metadata.sqlite");
    let repo = directory.path().join("repository");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    std::fs::write(repo.join("cortex.toml"), "[cortex]\nid = \"Exact/Repo\"\n").unwrap();
    let mut capture = MemoryCapture::new(
        "Exact/Repo",
        "Authrecall",
        "authrecall: preserve corrections across authenticated sessions.",
        MemoryCategory::Lesson,
    );
    capture.id = Some("authenticated-memory".into());
    MetadataStore::open(&database)
        .unwrap()
        .capture_daily_memory(&capture)
        .unwrap();
    let keys = Arc::new(ApiKeyStore::open_in_memory().unwrap());
    for label in ["other-agent-a", "other-agent-b", "other-agent-c"] {
        keys.issue("memory", label).unwrap();
    }
    let key = keys
        .issue_with_role("memory", "fixture-reader", Some("reader"))
        .unwrap();
    let mut principals = PrincipalStore::deny_by_default();
    principals.upsert_binding(
        "reader",
        RoleBinding {
            clearance_level: 1,
            compartments: vec![],
        },
    );
    let mut service = QueryService::with_memory_defaults(Orchestrator::new(
        Arc::new(MemoryVectorLane::new().with_fail()),
        Arc::new(MemoryKeywordLane::new().with_fail()),
        Arc::new(MemoryGraphLane::new().with_fail()),
    ));
    service.principal_store = Some(Arc::new(RwLock::new(principals)));
    service.acl.allow(&key.id, "Exact/Repo");
    let mut cfg = cortex_config::Config::default();
    cfg.ingestion.metadata_db = Some(database.to_string_lossy().into_owned());
    cfg.access_control.enabled = true;
    cfg.access_control.deny_on_missing_principal = true;
    let router = build_router_with_auth_and_cfg(Arc::new(service), None, Some(keys), Arc::new(cfg));
    let api = RunningApi::serve(router, Some(key.cleartext)).await;
    let started = std::time::Instant::now();
    let result = invoke(
        &api.mcp,
        "cortex_pre_thinking",
        json!({
            "user_prompt":"Implement authrecall", "cwd":repo,"budget_ms":600
        }),
    )
    .await;
    let elapsed = started.elapsed();
    assert!(
        result["bundle"]
            .as_str()
            .unwrap()
            .contains("authenticated-memory"),
        "{result}"
    );
    assert!(result["memory_delivery_id"].is_string());
    assert!(
        elapsed < Duration::from_millis(750),
        "deadline including scheduling tolerance exceeded: {elapsed:?}"
    );
    eprintln!(
        "authenticated_prethinking_elapsed_ms={}",
        elapsed.as_millis()
    );
    api.stop().await;
}

fn assert_prepared(metadata: &Path, result: &Value, id: &str, body: &str) -> String {
    let context = result["context"].as_str().unwrap();
    assert!(context.len() <= 1536);
    assert!(context.contains(id), "missing ID in context: {context}");
    assert!(
        context.contains(body),
        "missing exact memory in context: {context}"
    );
    let items: Vec<MemoryDeliveredItem> =
        serde_json::from_value(result["memories"].clone()).unwrap();
    assert_eq!(
        items.len(),
        1,
        "foreign or irrelevant advice leaked: {result}"
    );
    assert_eq!(items[0].id, id);
    assert_eq!(items[0].version, 1);
    assert_eq!(
        items[0].scope,
        MemoryScope::Repository("Exact/Repo".to_owned())
    );
    let delivery_id = result["delivery_id"].as_str().unwrap();
    assert_eq!(result["query_id"], delivery_id);
    let store = MetadataStore::open(metadata).unwrap();
    let delivery = store
        .daily_memory_delivery("Exact/Repo", delivery_id)
        .unwrap()
        .unwrap();
    assert_eq!(delivery.stage, MemoryDeliveryStage::Prepared);
    assert_eq!(delivery.request.memories, items);
    assert_eq!(delivery.request.context_bytes, context.len());
    assert_eq!(
        delivery.request.revision,
        result["revision"].as_u64().unwrap()
    );
    assert_eq!(delivery.request.global_revision, None);
    delivery_id.to_owned()
}

#[tokio::test]
async fn real_mcp_api_sqlite_capture_restart_correction_and_version_linked_feedback() {
    let dir = tempfile::tempdir().unwrap();
    let database = dir.path().join("metadata.sqlite");
    let scope = MemoryScope::Repository("Exact/Repo".to_owned());
    let old_body = "dailyrecall: keep cached advice after a correction.";
    let new_body = "dailyrecall: invalidate cached advice after SQLite correction commits.";
    let original_id = "LESSON-RECOVERY-ORIGINAL";
    let successor_id = "ADR-RECOVERY-CORRECTED";
    let capture = json!({
        "kind": "memory", "body": old_body, "repo": "Exact/Repo", "memory_id": original_id,
        "title": "Dailyrecall original", "retry_key": "stable-original-capture",
        "branch": "main", "path": "src/memory",
        "sources": [{ "event_id": "original-learning-event", "kind": "learning", "reference": "src/memory/store.rs", "revision": "observed-commit" }]
    });

    let first = RunningApi::start(&database).await;
    rpc(&first.mcp, "initialize", json!({})).await;
    let accepted = invoke(&first.mcp, "cortex_capture_memory", capture.clone()).await;
    assert_eq!(accepted["memory_id"], original_id);
    assert_eq!(accepted["id"], original_id);
    assert_eq!(accepted["version"], 1);
    assert_eq!(accepted["durability"], "local_committed");
    assert_eq!(accepted["mirror_status"], "not_attempted");
    assert_eq!(accepted["index_complete"], false);
    assert!(accepted["indexed_at"].is_null());
    assert!(!accepted["captured_at"].as_str().unwrap().is_empty());
    invoke(
        &first.mcp,
        "cortex_capture_memory",
        json!({
            "kind": "knowledge", "body": "dailyrecall: unrelated repository advice.",
            "repo": "other-repo", "memory_id": original_id
        }),
    )
    .await;
    let original = {
        let store = MetadataStore::open(&database).unwrap();
        let record = store.daily_memory(&scope, original_id).unwrap().unwrap();
        assert_eq!(record.body, old_body);
        assert_eq!(accepted["event_id"].as_str().unwrap(), record.event_id);
        assert_eq!(
            accepted["content_hash"].as_str().unwrap(),
            record.content_hash
        );
        assert_eq!(record.origin.agent, None);
        assert_eq!(record.origin.session_id, None);
        assert_eq!(record.origin.attribution, OriginAttribution::Unknown);
        let source = record
            .sources
            .iter()
            .find(|source| source.event_id == "original-learning-event")
            .expect("original source identity must be retained");
        assert_eq!(serde_json::to_value(source.kind).unwrap(), "learning");
        assert_eq!(source.reference.as_deref(), Some("src/memory/store.rs"));
        assert_eq!(source.revision.as_deref(), Some("observed-commit"));
        record
    };
    let initial_read = invoke(&first.mcp, "cortex_memory_read", query()).await;
    let initial_delivery = assert_prepared(&database, &initial_read, original_id, old_body);
    assert_eq!(initial_read["memories"][0]["event_id"], original.event_id);
    first.stop().await;

    let second = RunningApi::start(&database).await;
    rpc(
        &second.mcp,
        "initialize",
        json!({ "clientInfo": { "name": "codex-e2e", "version": "test" } }),
    )
    .await;
    let after_restart = invoke(&second.mcp, "cortex_memory_read", query()).await;
    assert_prepared(&database, &after_restart, original_id, old_body);
    assert_eq!(after_restart["context"], initial_read["context"]);
    let observed_revision = after_restart["revision"].as_u64().unwrap();
    invoke(&second.mcp, "cortex_capture_memory", json!({
        "kind": "decision", "body": new_body, "repo": "Exact/Repo", "memory_id": successor_id,
        "title": "Dailyrecall corrected", "status": "accepted", "supersedes": original_id,
        "expected_revision": observed_revision, "agent": "untrusted-argument",
        "sources": [{ "event_id": original.event_id, "kind": "decision", "reference": "docs/recovery.md" }]
    })).await;
    let corrected_read = invoke(&second.mcp, "cortex_memory_read", query()).await;
    assert_prepared(&database, &corrected_read, successor_id, new_body);
    assert!(!corrected_read["context"]
        .as_str()
        .unwrap()
        .contains(old_body));
    assert_eq!(corrected_read["revision"], observed_revision + 1);
    {
        let store = MetadataStore::open(&database).unwrap();
        let prior = store.daily_memory(&scope, original_id).unwrap().unwrap();
        assert_eq!(prior.status, MemoryStatus::Superseded);
        assert_eq!(prior.version, 2);
        let successor = store.daily_memory(&scope, successor_id).unwrap().unwrap();
        assert_eq!(successor.origin.agent.as_deref(), Some("codex-e2e"));
        assert_eq!(successor.origin.session_id, None);
        assert_eq!(
            successor.origin.attribution,
            OriginAttribution::ClientReported
        );
        assert_eq!(successor.supersedes.as_deref(), Some(original_id));
        assert_eq!(
            store
                .daily_memory_evidence(&scope, original_id)
                .unwrap()
                .len(),
            2
        );
    }

    // An equivalent legacy retry retains unknown origin even though another
    // initialized client performed the correction between attempts.
    let legacy_retry = Server::new(ToolContext::new(&second.url));
    rpc(&legacy_retry, "initialize", json!({})).await;
    let retried = invoke(&legacy_retry, "cortex_capture_memory", capture).await;
    for field in [
        "memory_id",
        "id",
        "event_id",
        "content_hash",
        "version",
        "scope_revision",
        "captured_at",
    ] {
        assert_eq!(retried[field], accepted[field], "retry changed {field}");
    }
    drop(legacy_retry);
    let feedback = invoke(&second.mcp, "cortex_feedback_record", json!({
        "query_id": initial_delivery, "helpful": false, "rating": 2, "free_text": "Earlier advice was explicitly corrected."
    })).await;
    assert_eq!(feedback["query_id"], initial_delivery);
    second.stop().await;

    let third = RunningApi::start(&database).await;
    rpc(&third.mcp, "initialize", json!({})).await;
    let final_read = invoke(&third.mcp, "cortex_memory_read", query()).await;
    assert_prepared(&database, &final_read, successor_id, new_body);
    assert_eq!(final_read["revision"], corrected_read["revision"]);
    let signals = invoke(&third.mcp, "cortex_feedback_signals", json!({ "limit": 5 })).await;
    assert_eq!(signals["total"], 1);
    assert_eq!(signals["rows"][0]["query_id"], initial_delivery);
    assert_eq!(signals["rows"][0]["helpful"], false);
    assert_eq!(signals["rows"][0]["rating"], 2);
    {
        let store = MetadataStore::open(&database).unwrap();
        let feedback = store
            .pre_thinking_feedback(&initial_delivery)
            .unwrap()
            .unwrap();
        assert_eq!(feedback.0, initial_delivery);
        assert!(!feedback.2);
        assert_eq!(feedback.4, Some(2));
        assert_eq!(
            feedback.5.as_deref(),
            Some("Earlier advice was explicitly corrected.")
        );
        let original_delivery = store
            .daily_memory_delivery("Exact/Repo", &initial_delivery)
            .unwrap()
            .unwrap();
        assert_eq!(original_delivery.request.memories[0].id, original_id);
        assert_eq!(original_delivery.request.memories[0].version, 1);
        assert_eq!(
            store
                .daily_memory(&scope, original_id)
                .unwrap()
                .unwrap()
                .status,
            MemoryStatus::Superseded
        );
        assert_eq!(
            store
                .daily_memory_evidence(&scope, original_id)
                .unwrap()
                .len(),
            2
        );
    }
    third.stop().await;
}
