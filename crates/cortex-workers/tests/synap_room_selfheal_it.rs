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

// ── Phase29c — durable-offset stale-ahead self-heal (graph consumer) ─

mod stale_offset {
    use std::sync::{Arc, Mutex};

    use cortex_storage::MetadataStore;
    use cortex_workers::graph::worker::{
        LiveSynapConsumer as GraphConsumer, SynapHandle as GraphHandle,
    };
    use cortex_workers::graph::SynapConsumer as GraphSynapConsumer;
    use wiremock::matchers::{body_string_contains, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn ok_empty_consume() -> serde_json::Value {
        serde_json::json!({
            "success": true, "request_id": "x", "error": null,
            "payload": { "events": [] },
        })
    }

    fn ok_stats_head_zero() -> serde_json::Value {
        // A post-wipe room: one message at offset 0.
        serde_json::json!({
            "success": true, "request_id": "x", "error": null,
            "payload": {
                "name": "cortex.events.enriched",
                "message_count": 1, "max_offset": 0,
                "min_offset": 0, "total_published": 1,
                "total_consumed": 0, "subscriber_count": 1, "dropped": 0,
            },
        })
    }

    fn ok_consume_offset_zero_event() -> serde_json::Value {
        let envelope = serde_json::json!({ "event_id": "evt-stale-heal" });
        let bytes: Vec<u8> = serde_json::to_vec(&envelope).expect("serialize");
        serde_json::json!({
            "success": true, "request_id": "x", "error": null,
            "payload": { "events": [{
                "offset": 0, "event": "enriched", "data": bytes,
                "timestamp": 1_700_000_000_000u64,
            }] },
        })
    }

    #[tokio::test]
    async fn durable_cursor_beyond_room_head_resets_and_resumes() {
        // The live incident: a synap restart wiped the room back to
        // offset 0 while the durable SQLite ledger still held a large
        // pre-wipe offset — the consumer read past the head forever.
        let server = MockServer::start().await;

        // Poll at the stale cursor → empty.
        Mock::given(method("POST"))
            .and(path("/api/v1/command"))
            .and(body_string_contains("stream.consume"))
            .and(body_string_contains("\"from_offset\":5000"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_empty_consume()))
            .mount(&server)
            .await;
        // The bounded stats probe reveals the post-wipe head.
        Mock::given(method("POST"))
            .and(path("/api/v1/command"))
            .and(body_string_contains("stream.stats"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats_head_zero()))
            .expect(1)
            .mount(&server)
            .await;
        // The healed poll from 0 delivers the event.
        Mock::given(method("POST"))
            .and(path("/api/v1/command"))
            .and(body_string_contains("stream.consume"))
            .and(body_string_contains("\"from_offset\":0"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_consume_offset_zero_event()))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("meta.sqlite");
        {
            let store = MetadataStore::open(&db).expect("open store");
            store
                .consumer_offset_upsert("battery-graph", "cortex.events.enriched", 4999, None)
                .expect("seed stale offset");
        }
        let metadata = Arc::new(Mutex::new(MetadataStore::open(&db).expect("reopen")));
        let handle = Arc::new(GraphHandle::new(&server.uri()).expect("handle"));
        let consumer = GraphConsumer::with_persistent_offset(
            handle,
            metadata.clone(),
            "battery-graph",
            "cortex.events.enriched",
        )
        .expect("consumer");
        assert_eq!(consumer.tracker().current(), 5000, "resumed at stale+1");

        // Poll 1: empty at the stale cursor → self-heal fires (stats
        // probed once), cursor resets to 0, reset persisted.
        let batch = consumer
            .next_batch("cortex.events.enriched", 10)
            .await
            .expect("poll 1");
        assert!(batch.is_empty());
        assert_eq!(consumer.tracker().current(), 0, "cursor healed to 0");
        {
            let guard = metadata.lock().unwrap();
            let row = guard
                .consumer_offset_lookup("battery-graph", "cortex.events.enriched")
                .expect("lookup")
                .expect("row");
            assert_eq!(row.last_offset, 0, "reset persisted to the ledger");
        }

        // Poll 2: consumes the post-wipe event from offset 0.
        let batch = consumer
            .next_batch("cortex.events.enriched", 10)
            .await
            .expect("poll 2");
        assert_eq!(batch.len(), 1, "healed consumer resumes delivery");
        assert_eq!(batch[0].event_id.as_deref(), Some("evt-stale-heal"));
    }

    // ── Phase29c §port — fulltext consumer (single tracker, in-memory) ──

    mod fulltext_case {
        use std::sync::Arc;

        use cortex_workers::fulltext::{
            LiveSynapConsumer as FulltextConsumer, SynapConsumer as FulltextSynapConsumer,
            SynapHandle as FulltextHandle, STREAM_ENRICHED,
        };
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        fn ok_empty_consume() -> serde_json::Value {
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": { "events": [] },
            })
        }

        fn ok_stats_head_zero() -> serde_json::Value {
            // A post-wipe room: one message at offset 0.
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": {
                    "name": STREAM_ENRICHED,
                    "message_count": 1, "max_offset": 0,
                    "min_offset": 0, "total_published": 1,
                    "total_consumed": 0, "subscriber_count": 1, "dropped": 0,
                },
            })
        }

        fn ok_consume_offset_zero_event() -> serde_json::Value {
            let envelope = serde_json::json!({ "event_id": "evt-fulltext-stale-heal" });
            let bytes: Vec<u8> = serde_json::to_vec(&envelope).expect("serialize");
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": { "events": [{
                    "offset": 0, "event": "enriched", "data": bytes,
                    "timestamp": 1_700_000_000_000u64,
                }] },
            })
        }

        #[tokio::test]
        async fn in_memory_cursor_beyond_room_head_resets_and_resumes() {
            // Same live incident as the graph consumer, ported: a synap
            // restart wipes the room back to offset 0 while the
            // fulltext consumer's in-memory cursor keeps counting from
            // where it left off — consume() then reads past the head
            // forever.
            let server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.consume"))
                .and(body_string_contains("\"from_offset\":5000"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_empty_consume()))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.stats"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats_head_zero()))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.consume"))
                .and(body_string_contains("\"from_offset\":0"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(ok_consume_offset_zero_event()),
                )
                .mount(&server)
                .await;

            let handle = Arc::new(FulltextHandle::new(&server.uri()).expect("handle"));
            let consumer = FulltextConsumer::new(handle);
            consumer.tracker().seed(5000);

            // Poll 1: empty at the stale cursor → self-heal fires
            // (stats probed once), cursor resets to 0.
            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 1");
            assert!(batch.is_empty(), "stale poll must read empty");
            assert_eq!(consumer.tracker().current(), 0, "cursor healed to 0");

            // Poll 2: consumes the post-wipe event from offset 0.
            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 2");
            assert_eq!(batch.len(), 1, "healed consumer resumes delivery");
            assert_eq!(
                batch[0].event_id.as_deref(),
                Some("evt-fulltext-stale-heal")
            );
        }
    }

    // ── Phase29c §port — embedder consumer (single tracker, in-memory) ──

    mod embedder_case {
        use std::sync::Arc;

        use cortex_workers::embedder::{
            LiveSynapConsumer as EmbedderConsumer, SynapConsumer as EmbedderSynapConsumer,
            SynapHandle as EmbedderHandle, STREAM_ENRICHED,
        };
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        fn ok_empty_consume() -> serde_json::Value {
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": { "events": [] },
            })
        }

        fn ok_stats_head_zero() -> serde_json::Value {
            // A post-wipe room: one message at offset 0.
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": {
                    "name": STREAM_ENRICHED,
                    "message_count": 1, "max_offset": 0,
                    "min_offset": 0, "total_published": 1,
                    "total_consumed": 0, "subscriber_count": 1, "dropped": 0,
                },
            })
        }

        fn ok_consume_offset_zero_event() -> serde_json::Value {
            let envelope = serde_json::json!({ "event_id": "evt-embedder-stale-heal" });
            let bytes: Vec<u8> = serde_json::to_vec(&envelope).expect("serialize");
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": { "events": [{
                    "offset": 0, "event": "enriched", "data": bytes,
                    "timestamp": 1_700_000_000_000u64,
                }] },
            })
        }

        #[tokio::test]
        async fn in_memory_cursor_beyond_room_head_resets_and_resumes() {
            let server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.consume"))
                .and(body_string_contains("\"from_offset\":5000"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_empty_consume()))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.stats"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats_head_zero()))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.consume"))
                .and(body_string_contains("\"from_offset\":0"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(ok_consume_offset_zero_event()),
                )
                .mount(&server)
                .await;

            let handle = Arc::new(EmbedderHandle::new(&server.uri()).expect("handle"));
            let consumer = EmbedderConsumer::new(handle);
            consumer.tracker().seed(5000);

            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 1");
            assert!(batch.is_empty(), "stale poll must read empty");
            assert_eq!(consumer.tracker().current(), 0, "cursor healed to 0");

            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 2");
            assert_eq!(batch.len(), 1, "healed consumer resumes delivery");
            assert_eq!(
                batch[0].event_id.as_deref(),
                Some("evt-embedder-stale-heal")
            );
        }
    }

    // ── Phase29c §port — classifier consumer (per-room trackers) ────────

    mod classifier_case {
        use std::sync::Arc;

        use cortex_workers::classifier_worker::{
            LiveSynapConsumer as ClassifierConsumer, SynapConsumer as ClassifierSynapConsumer,
            SynapHandle as ClassifierHandle, STREAM_BOOTSTRAP, STREAM_RAW,
        };
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        fn ok_empty_consume() -> serde_json::Value {
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": { "events": [] },
            })
        }

        fn ok_stats_head_zero() -> serde_json::Value {
            // A post-wipe room: one message at offset 0.
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": {
                    "name": STREAM_RAW,
                    "message_count": 1, "max_offset": 0,
                    "min_offset": 0, "total_published": 1,
                    "total_consumed": 0, "subscriber_count": 1, "dropped": 0,
                },
            })
        }

        fn ok_consume_offset_zero_event() -> serde_json::Value {
            let envelope = serde_json::json!({ "event_id": "evt-classifier-stale-heal" });
            let bytes: Vec<u8> = serde_json::to_vec(&envelope).expect("serialize");
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": { "events": [{
                    "offset": 0, "event": "raw", "data": bytes,
                    "timestamp": 1_700_000_000_000u64,
                }] },
            })
        }

        #[tokio::test]
        async fn raw_room_cursor_beyond_head_resets_without_touching_bootstrap() {
            // The live incident this ports: room `cortex.events.raw`
            // showed message_count=1, total_consumed=0 while the
            // classifier idled — its raw-room cursor had drifted past
            // the post-restart head. The bootstrap room's independent
            // tracker must be untouched by the raw room's heal.
            let server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.consume"))
                .and(body_string_contains("\"from_offset\":5000"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_empty_consume()))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.stats"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats_head_zero()))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.consume"))
                .and(body_string_contains("\"from_offset\":0"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(ok_consume_offset_zero_event()),
                )
                .mount(&server)
                .await;

            let handle = Arc::new(ClassifierHandle::new(&server.uri()).expect("handle"));
            let consumer = ClassifierConsumer::new(handle);
            consumer.tracker_for(STREAM_RAW).seed(5000);
            // Distinct non-zero value so a later "healed to 0" on the
            // raw tracker cannot be mistaken for the bootstrap tracker
            // having been reset too.
            consumer.tracker_for(STREAM_BOOTSTRAP).seed(42);

            // Poll 1 (raw): empty at the stale cursor → self-heal fires
            // (stats probed once, scoped to STREAM_RAW), raw cursor
            // resets to 0.
            let batch = consumer
                .next_batch(STREAM_RAW, 10)
                .await
                .expect("poll 1 (raw)");
            assert!(batch.is_empty(), "stale poll must read empty");
            assert_eq!(
                consumer.tracker_for(STREAM_RAW).current(),
                0,
                "raw cursor healed to 0"
            );
            assert_eq!(
                consumer.tracker_for(STREAM_BOOTSTRAP).current(),
                42,
                "bootstrap tracker must be untouched by the raw room's heal"
            );

            // Poll 2 (raw): consumes the post-wipe event from offset 0.
            let batch = consumer
                .next_batch(STREAM_RAW, 10)
                .await
                .expect("poll 2 (raw)");
            assert_eq!(batch.len(), 1, "healed consumer resumes delivery");
            assert_eq!(
                batch[0].event_id.as_deref(),
                Some("evt-classifier-stale-heal")
            );
            assert_eq!(
                consumer.tracker_for(STREAM_BOOTSTRAP).current(),
                42,
                "bootstrap tracker still untouched after the raw room resumed"
            );
        }
    }

    // ── Phase29d — published-decrease wipe detection (equality gap +
    // lagging-consumer blind spots the phase29c `>` check misses).
    // Ported once against the fulltext consumer per the proposal —
    // the heal is identical byte-for-byte across all four consumers,
    // already proven by the phase29c per-consumer duplication above.

    mod published_decrease {
        use std::sync::Arc;
        use std::time::Duration;

        use cortex_workers::fulltext::{
            LiveSynapConsumer as FulltextConsumer, SynapConsumer as FulltextSynapConsumer,
            SynapHandle as FulltextHandle, STREAM_ENRICHED,
        };
        use wiremock::matchers::{body_string_contains, method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        fn ok_empty_consume() -> serde_json::Value {
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": { "events": [] },
            })
        }

        fn ok_stats(
            message_count: u64,
            max_offset: u64,
            total_published: u64,
        ) -> serde_json::Value {
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": {
                    "name": STREAM_ENRICHED,
                    "message_count": message_count, "max_offset": max_offset,
                    "min_offset": 0, "total_published": total_published,
                    "total_consumed": 0, "subscriber_count": 1, "dropped": 0,
                },
            })
        }

        fn ok_consume_one_event_at(offset: u64, event_id: &str) -> serde_json::Value {
            let envelope = serde_json::json!({ "event_id": event_id });
            let bytes: Vec<u8> = serde_json::to_vec(&envelope).expect("serialize");
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": { "events": [{
                    "offset": offset, "event": "enriched", "data": bytes,
                    "timestamp": 1_700_000_000_000u64,
                }] },
            })
        }

        /// The residual hole this proposal documents (synap#257 — see
        /// `.rulebook/tasks/phase29d_synap-wipe-equality-gap/proposal.md`):
        /// `stream.stats` carries no room-generation discriminator, so
        /// a wipe that happens to refill the room to EXACTLY the
        /// stale cursor's value is indistinguishable from "healthy,
        /// caught up" — neither the phase29c `cursor > head_next`
        /// check nor the phase29d published-decrease check can fire.
        /// The second half of this test shows the fix's actual power:
        /// once a PRIOR probe remembered a higher `total_published`,
        /// a later decrease against THAT baseline still heals even
        /// though `cursor > head_next` is false on every poll.
        #[tokio::test]
        async fn equality_at_stale_check_documents_residual_gap_then_decrease_heals_via_remembered_baseline(
        ) {
            // ---- Scenario A: cursor == post-wipe head_next on the
            // very first probe (no remembered baseline yet) — the
            // documented residual gap. No heal fires.
            {
                let server = MockServer::start().await;

                Mock::given(method("POST"))
                    .and(path("/api/v1/command"))
                    .and(body_string_contains("stream.consume"))
                    .and(body_string_contains("\"from_offset\":1"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(ok_empty_consume()))
                    .mount(&server)
                    .await;
                Mock::given(method("POST"))
                    .and(path("/api/v1/command"))
                    .and(body_string_contains("stream.stats"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats(1, 0, 1)))
                    .expect(1)
                    .mount(&server)
                    .await;

                let handle = Arc::new(FulltextHandle::new(&server.uri()).expect("handle"));
                let consumer = FulltextConsumer::new(handle);
                consumer.tracker().seed(1);

                let batch = consumer
                    .next_batch(STREAM_ENRICHED, 10)
                    .await
                    .expect("poll 1");
                assert!(
                    batch.is_empty(),
                    "post-wipe room has nothing at offset 1 yet"
                );
                assert_eq!(
                    consumer.tracker().current(),
                    1,
                    "documented residual hole: cursor == head_next == baseline never heals"
                );
            }

            // ---- Scenario B: a PRIOR probe remembered published=5;
            // the CURRENT probe reports published=1 — the decrease
            // against the remembered baseline heals even though
            // `cursor(1) > head_next(1)` is false on every poll.
            {
                let server = MockServer::start().await;

                Mock::given(method("POST"))
                    .and(path("/api/v1/command"))
                    .and(body_string_contains("stream.consume"))
                    .and(body_string_contains("\"from_offset\":1"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(ok_empty_consume()))
                    .mount(&server)
                    .await;
                // Probe #1 remembers a HIGHER published count.
                Mock::given(method("POST"))
                    .and(path("/api/v1/command"))
                    .and(body_string_contains("stream.stats"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats(1, 0, 5)))
                    .up_to_n_times(1)
                    .mount(&server)
                    .await;
                // Probe #2 sees the decrease.
                Mock::given(method("POST"))
                    .and(path("/api/v1/command"))
                    .and(body_string_contains("stream.stats"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats(1, 0, 1)))
                    .mount(&server)
                    .await;
                Mock::given(method("POST"))
                    .and(path("/api/v1/command"))
                    .and(body_string_contains("stream.consume"))
                    .and(body_string_contains("\"from_offset\":0"))
                    .respond_with(
                        ResponseTemplate::new(200)
                            .set_body_json(ok_consume_one_event_at(0, "evt-decrease-heal")),
                    )
                    .mount(&server)
                    .await;

                let handle = Arc::new(FulltextHandle::new(&server.uri()).expect("handle"));
                // Phase29d test hook: a real 60s wait between probes
                // is not viable in a unit-speed IT, so the interval is
                // collapsed to zero — the due-check still runs, it
                // just never blocks the second probe.
                let consumer =
                    FulltextConsumer::new(handle).with_stale_check_interval(Duration::ZERO);
                consumer.tracker().seed(1);

                // Poll 1: empty; probe #1 remembers published=5 (>=
                // baseline 1) — no heal yet.
                let batch = consumer
                    .next_batch(STREAM_ENRICHED, 10)
                    .await
                    .expect("poll 1");
                assert!(batch.is_empty());
                assert_eq!(
                    consumer.tracker().current(),
                    1,
                    "no heal yet — published(5) >= baseline(1)"
                );

                // Poll 2: empty again; probe #2 sees published=1 <
                // remembered baseline 5 → heal.
                let batch = consumer
                    .next_batch(STREAM_ENRICHED, 10)
                    .await
                    .expect("poll 2");
                assert!(batch.is_empty());
                assert_eq!(
                    consumer.tracker().current(),
                    0,
                    "decrease against the remembered baseline healed the cursor to 0"
                );

                // Poll 3: resumes from 0 and delivers the post-wipe event.
                let batch = consumer
                    .next_batch(STREAM_ENRICHED, 10)
                    .await
                    .expect("poll 3");
                assert_eq!(batch.len(), 1, "healed consumer resumes delivery");
                assert_eq!(batch[0].event_id.as_deref(), Some("evt-decrease-heal"));
            }
        }

        /// The other phase29c blind spot: a synap wipe while the
        /// consumer LAGS. The refilled room's head can be AHEAD of
        /// the stale cursor, so `consume(from_offset=cursor)` returns
        /// events from the NEW generation at overlapping offsets —
        /// the poll is never empty, so the phase29c empty-poll fast
        /// path never fires and the cross-generation skip is silent.
        /// Phase29d's promoted "probe on ANY poll" trigger closes
        /// this: the published-decrease check still runs on a
        /// non-empty poll.
        #[tokio::test]
        async fn lagging_consumer_nonempty_poll_still_probes_and_heals_on_published_decrease() {
            let server = MockServer::start().await;

            // The cursor never advances in this test (no ack call),
            // so both polls request the same stale offset.
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.consume"))
                .and(body_string_contains("\"from_offset\":2"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(ok_consume_one_event_at(2, "evt-lag-old-gen")),
                )
                .mount(&server)
                .await;
            // Probe #1: well ahead, no decrease yet — just remembers
            // published=6.
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.stats"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats(6, 5, 6)))
                .up_to_n_times(1)
                .mount(&server)
                .await;
            // Probe #2: published dropped to 3 (< remembered 6) → heal.
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.stats"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats(3, 2, 3)))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.consume"))
                .and(body_string_contains("\"from_offset\":0"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(ok_consume_one_event_at(0, "evt-lag-heal")),
                )
                .mount(&server)
                .await;

            let handle = Arc::new(FulltextHandle::new(&server.uri()).expect("handle"));
            let consumer = FulltextConsumer::new(handle).with_stale_check_interval(Duration::ZERO);
            consumer.tracker().seed(2);

            // Poll 1: non-empty (one stale-generation event at offset
            // 2), but the promoted ANY-poll probe still fires.
            // published(6) >= baseline(max(0,2)=2) → no heal yet;
            // remembered updates to 6.
            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 1");
            assert_eq!(batch.len(), 1, "non-empty poll must still be delivered");
            assert_eq!(batch[0].event_id.as_deref(), Some("evt-lag-old-gen"));
            assert_eq!(consumer.tracker().current(), 2, "no heal yet on poll 1");

            // Poll 2: consume STILL returns the stale-offset event
            // (masking the wipe locally) — but probe #2 sees
            // published=3 < remembered baseline 6 → the decrease
            // heals the cursor to 0 regardless of what this poll's
            // consume result looked like.
            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 2");
            assert_eq!(
                batch.len(),
                1,
                "poll 2 is still non-empty; the wipe is invisible locally"
            );
            assert_eq!(
                consumer.tracker().current(),
                0,
                "published-decrease healed the cursor to 0 despite the non-empty poll"
            );

            // Poll 3: resumes from 0 and delivers the real post-wipe event.
            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 3");
            assert_eq!(batch.len(), 1);
            assert_eq!(batch[0].event_id.as_deref(), Some("evt-lag-heal"));
        }
    }
}
