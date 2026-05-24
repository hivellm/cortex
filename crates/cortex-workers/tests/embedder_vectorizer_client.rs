//! Integration tests for `cortex_workers::embedder::vectorizer_client`.

use cortex_core::events::Kind;
use cortex_workers::classifier::Severity;
use cortex_workers::embedder::{
    with_retry, Chunk, ChunkMetadata, ChunkSource, MemoryVectorizerClient, VectorizerClient,
    VectorizerClientError,
};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

fn make_chunk(id: &str) -> Chunk {
    Chunk {
        dedup_key: id.to_string(),
        parent_event_id: "evt".into(),
        parent_content_hash: "hash".into(),
        chunk_content_hash: format!("hash-{id}"),
        collection: "cortex-code".into(),
        text: "hello".into(),
        metadata: ChunkMetadata {
            kind: Kind::ToolCall,
            topics: vec![],
            severity: Severity::Info,
            repo: None,
            path: None,
            symbol: None,
            byte_range: Some((0, 5)),
            language: Some("rust".into()),
            source: ChunkSource::Code,
            prompt_version: None,
        },
    }
}

#[tokio::test]
async fn memory_client_reports_deduped_on_replay() {
    let client = MemoryVectorizerClient::default();
    let chunks = vec![make_chunk("ulid1"), make_chunk("ulid2")];

    let first = client.upsert_chunks("cortex-code", &chunks).await.unwrap();
    assert_eq!(first.written, 2);
    assert_eq!(first.deduped, 0);
    assert_eq!(first.new_entries.len(), 2);
    for entry in &first.new_entries {
        assert!(
            !entry.server_id.is_empty(),
            "memory client must mint a server id"
        );
    }

    let second = client.upsert_chunks("cortex-code", &chunks).await.unwrap();
    assert_eq!(second.written, 0);
    assert_eq!(second.deduped, 2);
    assert!(second.new_entries.is_empty());

    assert_eq!(client.stored_count_for("cortex-code"), 2);
}

#[tokio::test]
async fn memory_client_exists_by_dedup_key_returns_stored_subset() {
    let client = MemoryVectorizerClient::default();
    let chunks = vec![make_chunk("a"), make_chunk("b")];
    client.upsert_chunks("cortex-code", &chunks).await.unwrap();

    let query = vec!["a".to_string(), "b".into(), "c".into()];
    let present = client
        .exists_by_dedup_key("cortex-code", &query)
        .await
        .unwrap();
    assert!(present.contains("a"));
    assert!(present.contains("b"));
    assert!(!present.contains("c"));
    assert_eq!(present.len(), 2);
}

#[tokio::test]
async fn with_retry_recovers_after_transient_transport_errors() {
    let attempts = Arc::new(AtomicU32::new(0));
    let attempts2 = attempts.clone();
    let result: Result<&'static str, VectorizerClientError> = with_retry(3, move || {
        let attempts = attempts2.clone();
        async move {
            let n = attempts.fetch_add(1, Ordering::SeqCst);
            if n < 2 {
                Err(VectorizerClientError::Transport(
                    "connection refused".into(),
                ))
            } else {
                Ok("ok")
            }
        }
    })
    .await;
    assert_eq!(result.unwrap(), "ok");
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn with_retry_does_not_retry_non_retriable() {
    let attempts = Arc::new(AtomicU32::new(0));
    let attempts2 = attempts.clone();
    let result: Result<(), VectorizerClientError> = with_retry(3, move || {
        let attempts = attempts2.clone();
        async move {
            attempts.fetch_add(1, Ordering::SeqCst);
            Err(VectorizerClientError::Other("boom".into()))
        }
    })
    .await;
    assert!(matches!(result, Err(VectorizerClientError::Other(_))));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}
