//! Phase29b (hive-services-update-aug2026 §2) — consumer room
//! self-heal against a fake Synap.
//!
//! Synap stream rooms are EPHEMERAL: a synap restart wipes them, and
//! until 95f32c7 the long-running consumers spammed the server with
//! `Room not found` errors on every poll until *they* were restarted.
//! The §2.1 fix makes every live consumer re-declare the room
//! (idempotent `stream.get_or_create`) and retry ONCE within the same
//! poll when the consume comes back `Room not found`.
//!
//! The fake Synap here speaks the real SDK wire shape
//! (`POST /api/v1/command`, `{"success", "payload", "error"}`):
//! the FIRST `stream.consume` answers `Room not found`, the
//! re-declare must fire, and the retried consume delivers the event —
//! all inside one `next_batch` call.

use std::sync::Arc;

use cortex_workers::fulltext::{LiveSynapConsumer, SynapConsumer, SynapHandle, STREAM_ENRICHED};
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn err_room_not_found() -> serde_json::Value {
    serde_json::json!({
        "success": false,
        "request_id": "x",
        "payload": null,
        "error": "Invalid request: Room 'cortex.events.enriched' not found",
    })
}

fn ok_get_or_create() -> serde_json::Value {
    serde_json::json!({
        "success": true,
        "request_id": "x",
        "payload": { "created": true, "room": STREAM_ENRICHED, "success": true },
        "error": null,
    })
}

fn ok_consume_one_event() -> serde_json::Value {
    // `data` as a JSON byte-array — the shape the real server emits
    // (serde_json::to_vec of the envelope) and the SDK decodes.
    let envelope = serde_json::json!({ "event_id": "evt-heal-1" });
    let bytes: Vec<u8> = serde_json::to_vec(&envelope).expect("serialize envelope");
    serde_json::json!({
        "success": true,
        "request_id": "x",
        "payload": {
            "events": [{
                "offset": 7,
                "event": "enriched",
                "data": bytes,
                "timestamp": 1_700_000_000_000u64,
            }],
        },
        "error": null,
    })
}

#[tokio::test]
async fn consume_room_not_found_redeclares_and_retries_within_one_poll() {
    let server = MockServer::start().await;

    // First consume → Room not found (exhausts after one hit).
    Mock::given(method("POST"))
        .and(path("/api/v1/command"))
        .and(body_string_contains("stream.consume"))
        .respond_with(ResponseTemplate::new(200).set_body_json(err_room_not_found()))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    // Retried consume → one event.
    Mock::given(method("POST"))
        .and(path("/api/v1/command"))
        .and(body_string_contains("stream.consume"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_consume_one_event()))
        .mount(&server)
        .await;
    // The self-heal declare — MUST fire exactly once.
    Mock::given(method("POST"))
        .and(path("/api/v1/command"))
        .and(body_string_contains("stream.get_or_create"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_get_or_create()))
        .expect(1)
        .mount(&server)
        .await;

    let handle = Arc::new(SynapHandle::new(&server.uri()).expect("synap handle"));
    let consumer = LiveSynapConsumer::new(handle);

    let batch = consumer
        .next_batch(STREAM_ENRICHED, 10)
        .await
        .expect("next_batch");
    assert_eq!(batch.len(), 1, "healed poll must deliver the event");
    assert_eq!(batch[0].offset, 7);
    assert_eq!(batch[0].event_id.as_deref(), Some("evt-heal-1"));
    // MockServer::verify on drop asserts the get_or_create expect(1).
}

#[tokio::test]
async fn consume_room_still_missing_after_redeclare_degrades_to_empty_batch() {
    // Both the initial consume AND the post-declare retry answer
    // Room-not-found: the poll must degrade to an empty batch (idle),
    // never an error, and the declare must fire only ONCE (bound).
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/api/v1/command"))
        .and(body_string_contains("stream.consume"))
        .respond_with(ResponseTemplate::new(200).set_body_json(err_room_not_found()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/command"))
        .and(body_string_contains("stream.get_or_create"))
        .respond_with(ResponseTemplate::new(200).set_body_json(ok_get_or_create()))
        .expect(1)
        .mount(&server)
        .await;

    let handle = Arc::new(SynapHandle::new(&server.uri()).expect("synap handle"));
    let consumer = LiveSynapConsumer::new(handle);

    let batch = consumer
        .next_batch(STREAM_ENRICHED, 10)
        .await
        .expect("degrades to Ok");
    assert!(batch.is_empty(), "still-missing room reads as idle");
}
