//! Shared helpers for integration tests.
//!
//! This module lives under `tests/common/` — the standard Rust
//! integration-test convention. Each `tests/*.rs` file compiles as its own
//! binary, so helpers only reachable here are referenced via `mod common;`
//! in every test file that needs them.

#![allow(dead_code)]

use cortex_classifier::{ClassifierOutput, ClassifierSource, PiiRisk, Severity};
use cortex_core::events::Kind;
use cortex_workers::embedder::{
    Chunk, ChunkMetadata, ChunkSource, CollectionSchema, EmbedderConfig, EnrichedEvent,
    LiveVectorizerClient, Metric,
};
use serde_json::json;
use std::time::Duration;
use ulid::Ulid;

/// Whether the live-integration suite is opted in.
pub fn it_enabled() -> bool {
    std::env::var("CORTEX_EMBEDDER_IT").as_deref() == Ok("1")
}

/// Default Vectorizer base URL for integration tests.
pub fn vectorizer_url() -> String {
    std::env::var("CORTEX_EMBEDDER_VECTORIZER_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:17001".to_string())
}

/// Optional Vectorizer admin password (SDK treats it as API key).
pub fn vectorizer_password() -> Option<String> {
    std::env::var("CORTEX_EMBEDDER_VECTORIZER_PASSWORD")
        .ok()
        .or_else(|| Some("cortex-dev-admin".to_string()))
}

/// Generate a unique collection name with the given suffix. Includes a
/// ULID so parallel runs of the same test don't collide.
pub fn unique_collection(suffix: &str) -> String {
    format!(
        "cortex-it-{}-{}",
        Ulid::new().to_string().to_lowercase(),
        suffix
    )
}

/// Build a configured [`EmbedderConfig`] for the integration suite.
pub fn it_config(prefix: Option<&str>) -> EmbedderConfig {
    EmbedderConfig {
        vectorizer_url: vectorizer_url(),
        vectorizer_user: "admin".into(),
        vectorizer_password: vectorizer_password(),
        collection_prefix: prefix.unwrap_or("cortex-it").to_string(),
        upsert_batch: 64,
        max_retry: 3,
        ..EmbedderConfig::default()
    }
}

/// Default schema used by integration tests.
///
/// Uses `dim = 512` because the `hivehub/vectorizer:3.0.0` server image
/// runs the BM25 fallback embedder, which only accepts 512-dim collections
/// on `POST /insert`. The embedder crate's in-process default (768, from
/// `CollectionSchema::default()`) reflects the production Vectorizer model
/// choice, so each test builds its own schema here.
pub fn it_schema() -> CollectionSchema {
    CollectionSchema {
        dim: 512,
        metric: Metric::Cosine,
        metadata_index: vec![
            "kind".into(),
            "topics".into(),
            "repo".into(),
            "path".into(),
            "language".into(),
            "severity".into(),
        ],
        hybrid: true,
    }
}

/// Log in to the Vectorizer dev stack and return a bearer JWT.
///
/// Uses [`LiveVectorizerClient::login`] (wrapping `vectorizer-sdk`'s
/// `login()` method — added in SDK 3.0.3). The returned `access_token`
/// rides as the SDK's `api_key`, which the 3.0.3 HTTP transport sniffs
/// as a three-segment JWT and sends as `Authorization: Bearer`.
pub async fn login_token() -> String {
    let password = vectorizer_password().unwrap_or_else(|| "cortex-dev-admin".into());
    let jwt = LiveVectorizerClient::login(&vectorizer_url(), "admin", &password)
        .await
        .expect("vectorizer login via SDK");
    jwt.access_token
}

/// Build a configured [`EmbedderConfig`] that carries an authenticated
/// bearer token in the `vectorizer_password` slot.
pub async fn it_config_authed(prefix: Option<&str>) -> EmbedderConfig {
    let token = login_token().await;
    EmbedderConfig {
        vectorizer_password: Some(token),
        ..it_config(prefix)
    }
}

/// Build a [`LiveVectorizerClient`] aimed at the dev stack, with a freshly
/// issued JWT already installed.
pub async fn live_client() -> LiveVectorizerClient {
    let config = it_config_authed(None).await;
    LiveVectorizerClient::new(config).expect("failed to build LiveVectorizerClient for IT run")
}

/// Delete a collection best-effort. Dev volume is disposable; failures are
/// logged to stderr and not propagated — per spec (round-5 brief).
pub async fn drop_collection(client: &LiveVectorizerClient, name: &str) {
    if let Err(e) = client.sdk().delete_collection(name).await {
        eprintln!("warning: failed to delete IT collection '{name}': {e}");
    }
}

/// Synthesise an enriched event with a `content` string payload.
pub fn enriched_event(
    id: &str,
    kind: Kind,
    path: Option<&str>,
    content: &str,
    summary: Option<&str>,
) -> EnrichedEvent {
    EnrichedEvent {
        event_id: id.to_string(),
        kind,
        content_hash: format!("parent-{id}"),
        redacted_payload: json!({ "content": content }),
        classifier: ClassifierOutput {
            event_id: id.to_string(),
            kind_refinement: None,
            topics: vec!["test".into()],
            severity: Severity::Info,
            pii_risk: PiiRisk::Low,
            redaction_suggestions: vec![],
            summary: summary.map(|s| s.to_string()),
            entities: Vec::new(),
            relations: Vec::new(),
            source: ClassifierSource::StaticFallback,
            prompt_version: "v1".into(),
            model: "static-v1".into(),
            latency_ms: 0,
            tokens_in: 0,
            tokens_out: 0,
        },
        context_repo: None,
        context_path: path.map(|s| s.to_string()),
        parent_event_id: None,
        session_id: None,
    }
}

/// Synthesise a chunk with a stable `dedup_key` for use in raw-upsert IT
/// tests. The server will assign its own UUID on insert; tests that need
/// to round-trip the mapping read `UpsertReport.new_entries` /
/// `EmbedReport.new_records`.
pub fn make_chunk(id: &str, collection: &str, text: &str) -> Chunk {
    Chunk {
        dedup_key: id.to_string(),
        parent_event_id: "evt_it".into(),
        parent_content_hash: "parent-it".into(),
        chunk_content_hash: format!("content-{id}"),
        collection: collection.to_string(),
        text: text.to_string(),
        metadata: ChunkMetadata {
            kind: Kind::ToolCall,
            topics: vec!["test".into()],
            severity: Severity::Info,
            repo: None,
            path: Some("test.rs".into()),
            symbol: Some(format!("sym_{id}")),
            byte_range: Some((0, text.len() as u32)),
            language: Some("rust".into()),
            source: ChunkSource::Code,
            prompt_version: None,
        },
    }
}

/// Poll `predicate` until it returns true or `deadline` elapses. Returns
/// whether the condition held before the deadline.
pub async fn wait_until<F, Fut>(mut predicate: F, deadline: Duration) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let start = std::time::Instant::now();
    let mut delay = Duration::from_millis(100);
    loop {
        if predicate().await {
            return true;
        }
        if start.elapsed() >= deadline {
            return false;
        }
        tokio::time::sleep(delay).await;
        if delay < Duration::from_millis(500) {
            delay = Duration::from_millis((delay.as_millis() as u64).saturating_mul(2));
        }
    }
}

/// Fail a test with a skipped-IT hint instead of panicking.
pub fn skip_if_not_it() -> bool {
    if !it_enabled() {
        eprintln!("skipping: CORTEX_EMBEDDER_IT != 1");
        return true;
    }
    false
}

/// Assert two sets of ids contain the same elements, showing a diff on
/// mismatch. Ordering-insensitive.
pub fn assert_same_ids(actual: &std::collections::BTreeSet<String>, expected: &[&str]) {
    let want: std::collections::BTreeSet<String> = expected.iter().map(|s| s.to_string()).collect();
    assert_eq!(actual, &want, "id-set mismatch");
}
