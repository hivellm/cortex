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
    use std::time::Duration;

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
        // A post-wipe room: one message at offset 0. `generation: 0`
        // (field absent on a pre-1.3.1 server) so this mock exercises
        // the documented Phase29d §1.3 fallback path — the legacy
        // `cursor > head_next` comparison — rather than the primary
        // generation-change check.
        serde_json::json!({
            "success": true, "request_id": "x", "error": null,
            "payload": {
                "name": "cortex.events.enriched",
                "message_count": 1, "max_offset": 0,
                "min_offset": 0, "total_published": 1,
                "total_consumed": 0, "subscriber_count": 1, "dropped": 0,
                "generation": 0,
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

    /// Phase29d §1.3 — the definitive-generation heal must ALSO
    /// force-rewind the durable SQLite ledger, exactly like the
    /// phase29c cursor-beyond-head heal above. Cursor(1) ==
    /// head_next(1) here — the legacy `cursor > head_next` check
    /// could never fire on this scenario — so this also proves the
    /// generation heal covers the graph consumer's persistence path
    /// the other three (in-memory-only) consumers don't have.
    #[tokio::test]
    async fn generation_change_force_rewinds_the_durable_ledger() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/command"))
            .and(body_string_contains("stream.consume"))
            .and(body_string_contains("\"from_offset\":1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_empty_consume()))
            .mount(&server)
            .await;
        // Probe #1: records generation 1 (head_next(1) == cursor(1) —
        // the legacy check would never fire here even if it ran).
        Mock::given(method("POST"))
            .and(path("/api/v1/command"))
            .and(body_string_contains("stream.stats"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": {
                    "name": "cortex.events.enriched",
                    "message_count": 1, "max_offset": 0,
                    "min_offset": 0, "total_published": 1,
                    "total_consumed": 0, "subscriber_count": 1, "dropped": 0,
                    "generation": 1,
                },
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;
        // Probe #2: generation changed 1 -> 2 → heal + ledger rewind.
        Mock::given(method("POST"))
            .and(path("/api/v1/command"))
            .and(body_string_contains("stream.stats"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": {
                    "name": "cortex.events.enriched",
                    "message_count": 1, "max_offset": 0,
                    "min_offset": 0, "total_published": 1,
                    "total_consumed": 0, "subscriber_count": 1, "dropped": 0,
                    "generation": 2,
                },
            })))
            .mount(&server)
            .await;

        let dir = tempfile::tempdir().expect("tempdir");
        let db = dir.path().join("meta.sqlite");
        {
            let store = MetadataStore::open(&db).expect("open store");
            store
                .consumer_offset_upsert("battery-graph-gen", "cortex.events.enriched", 0, None)
                .expect("seed offset");
        }
        let metadata = Arc::new(Mutex::new(MetadataStore::open(&db).expect("reopen")));
        let handle = Arc::new(GraphHandle::new(&server.uri()).expect("handle"));
        let consumer = GraphConsumer::with_persistent_offset(
            handle,
            metadata.clone(),
            "battery-graph-gen",
            "cortex.events.enriched",
        )
        .expect("consumer")
        .with_stale_check_interval(Duration::ZERO);
        assert_eq!(consumer.tracker().current(), 1, "resumed at seeded+1");

        // Poll 1: first observation, records generation 1 — no heal.
        let batch = consumer
            .next_batch("cortex.events.enriched", 10)
            .await
            .expect("poll 1");
        assert!(batch.is_empty());
        assert_eq!(
            consumer.tracker().current(),
            1,
            "no heal on first observation"
        );

        // Poll 2: generation changed → heal fires AND force-rewinds
        // the durable ledger.
        let batch = consumer
            .next_batch("cortex.events.enriched", 10)
            .await
            .expect("poll 2");
        assert!(batch.is_empty());
        assert_eq!(
            consumer.tracker().current(),
            0,
            "in-memory cursor healed to 0 on generation change"
        );
        {
            let guard = metadata.lock().unwrap();
            let row = guard
                .consumer_offset_lookup("battery-graph-gen", "cortex.events.enriched")
                .expect("lookup")
                .expect("row");
            assert_eq!(
                row.last_offset, 0,
                "generation-change heal force-rewound the ledger"
            );
        }
    }

    /// Phase29d §1.3 — the DURABLE cursor's boot-time wipe detection.
    ///
    /// Found live, not in review: the graph consumer is the only one
    /// whose cursor outlives the process. With the generation baseline
    /// held only in memory, the first probe after a restart had
    /// nothing to compare against, adopted the live generation, and
    /// left the cursor pointing into the previous incarnation's dead
    /// offset space — `resume_at=1` against a wiped room whose head
    /// was also 1, an exact tie no count-based check can see. The
    /// worker sat at `jobs=0` and never recovered.
    ///
    /// The ledger therefore persists `room_generation` beside the
    /// offset, and boot restores it as the baseline: the very first
    /// probe then sees the change and heals.
    #[tokio::test]
    async fn persisted_generation_lets_a_durable_cursor_detect_a_wipe_on_the_first_boot_probe() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/api/v1/command"))
            .and(body_string_contains("stream.consume"))
            .and(body_string_contains("\"from_offset\":1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(ok_empty_consume()))
            .mount(&server)
            .await;
        // The live room is a NEW incarnation (generation 99) whose head
        // coincides with the resumed cursor — the tie that defeats every
        // count-based heuristic.
        Mock::given(method("POST"))
            .and(path("/api/v1/command"))
            .and(body_string_contains("stream.stats"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": {
                    "name": "cortex.events.enriched",
                    "message_count": 1, "max_offset": 0,
                    "min_offset": 0, "total_published": 1,
                    "total_consumed": 0, "subscriber_count": 1, "dropped": 0,
                    "generation": 99,
                },
            })))
            .mount(&server)
            .await;
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
            // A ledger written by a PREVIOUS process against the OLD
            // incarnation (generation 7).
            let store = MetadataStore::open(&db).expect("open store");
            store
                .consumer_offset_upsert("battery-graph-boot", "cortex.events.enriched", 0, None)
                .expect("seed offset");
            store
                .consumer_offset_generation_set("battery-graph-boot", "cortex.events.enriched", 7)
                .expect("seed generation");
        }
        let metadata = Arc::new(Mutex::new(MetadataStore::open(&db).expect("reopen")));
        let handle = Arc::new(GraphHandle::new(&server.uri()).expect("handle"));
        let consumer = GraphConsumer::with_persistent_offset(
            handle,
            metadata.clone(),
            "battery-graph-boot",
            "cortex.events.enriched",
        )
        .expect("consumer")
        .with_stale_check_interval(Duration::ZERO);
        assert_eq!(consumer.tracker().current(), 1, "resumed at seeded+1");

        // Poll 1 — the FIRST probe of this process already has a
        // baseline (7, from the ledger) to compare against 99.
        let batch = consumer
            .next_batch("cortex.events.enriched", 10)
            .await
            .expect("poll 1");
        assert!(batch.is_empty());
        assert_eq!(
            consumer.tracker().current(),
            0,
            "persisted baseline makes the wipe visible on the first boot probe"
        );
        {
            let guard = metadata.lock().unwrap();
            let row = guard
                .consumer_offset_lookup("battery-graph-boot", "cortex.events.enriched")
                .expect("lookup")
                .expect("row");
            assert_eq!(row.last_offset, 0, "ledger force-rewound");
            assert_eq!(
                row.room_generation, 99,
                "ledger adopted the new incarnation so the next boot compares against it"
            );
        }

        // Poll 2 — the healed cursor delivers the post-wipe event.
        let batch = consumer
            .next_batch("cortex.events.enriched", 10)
            .await
            .expect("poll 2");
        assert_eq!(batch.len(), 1, "healed consumer resumes delivery");
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
            // A post-wipe room: one message at offset 0. `generation:
            // 0` (field absent on a pre-1.3.1 server) so this mock
            // exercises the documented Phase29d §1.3 fallback path —
            // the legacy `cursor > head_next` comparison — rather
            // than the primary generation-change check.
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": {
                    "name": STREAM_ENRICHED,
                    "message_count": 1, "max_offset": 0,
                    "min_offset": 0, "total_published": 1,
                    "total_consumed": 0, "subscriber_count": 1, "dropped": 0,
                    "generation": 0,
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
            // A post-wipe room: one message at offset 0. `generation:
            // 0` (field absent on a pre-1.3.1 server) so this mock
            // exercises the documented Phase29d §1.3 fallback path —
            // the legacy `cursor > head_next` comparison — rather
            // than the primary generation-change check.
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": {
                    "name": STREAM_ENRICHED,
                    "message_count": 1, "max_offset": 0,
                    "min_offset": 0, "total_published": 1,
                    "total_consumed": 0, "subscriber_count": 1, "dropped": 0,
                    "generation": 0,
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
        use std::time::Duration;

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
            // A post-wipe room: one message at offset 0. `generation:
            // 0` (field absent on a pre-1.3.1 server) so this mock
            // exercises the documented Phase29d §1.3 fallback path —
            // the legacy `cursor > head_next` comparison — rather
            // than the primary generation-change check.
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": {
                    "name": STREAM_RAW,
                    "message_count": 1, "max_offset": 0,
                    "min_offset": 0, "total_published": 1,
                    "total_consumed": 0, "subscriber_count": 1, "dropped": 0,
                    "generation": 0,
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

        fn ok_stats_gen(generation: u64) -> serde_json::Value {
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": {
                    "name": STREAM_RAW,
                    "message_count": 1, "max_offset": 0,
                    "min_offset": 0, "total_published": 1,
                    "total_consumed": 0, "subscriber_count": 1, "dropped": 0,
                    "generation": generation,
                },
            })
        }

        /// Phase29d §1.3 — PER-ROOM isolation for the primary
        /// generation-change signal: a generation change on the raw
        /// room must reset ONLY that room's tracker (and remembered
        /// generation); the bootstrap room's independent tracker must
        /// be untouched. Ports the phase29c isolation guarantee
        /// (`raw_room_cursor_beyond_head_resets_without_touching_bootstrap`
        /// above) to the generation check.
        #[tokio::test]
        async fn generation_change_resets_only_the_affected_room() {
            let server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.consume"))
                .and(body_string_contains("\"from_offset\":1"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_empty_consume()))
                .mount(&server)
                .await;
            // Probe #1: records generation 1 for the raw room.
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.stats"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats_gen(1)))
                .up_to_n_times(1)
                .mount(&server)
                .await;
            // Probe #2: generation changed 1 -> 2 → heal (raw room only).
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.stats"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats_gen(2)))
                .mount(&server)
                .await;

            let handle = Arc::new(ClassifierHandle::new(&server.uri()).expect("handle"));
            let consumer =
                ClassifierConsumer::new(handle).with_stale_check_interval(Duration::ZERO);
            consumer.tracker_for(STREAM_RAW).seed(1);
            // Distinct non-zero value so a later "healed to 0" on the
            // raw tracker cannot be mistaken for the bootstrap tracker
            // having been reset too.
            consumer.tracker_for(STREAM_BOOTSTRAP).seed(42);

            // Poll 1 (raw): first observation records generation 1 —
            // no heal, bootstrap tracker untouched.
            let batch = consumer
                .next_batch(STREAM_RAW, 10)
                .await
                .expect("poll 1 (raw)");
            assert!(batch.is_empty());
            assert_eq!(
                consumer.tracker_for(STREAM_RAW).current(),
                1,
                "no heal on first observation"
            );
            assert_eq!(consumer.tracker_for(STREAM_BOOTSTRAP).current(), 42);

            // Poll 2 (raw): generation changed → heal fires, RAW
            // tracker resets to 0, bootstrap tracker still untouched.
            let batch = consumer
                .next_batch(STREAM_RAW, 10)
                .await
                .expect("poll 2 (raw)");
            assert!(batch.is_empty());
            assert_eq!(
                consumer.tracker_for(STREAM_RAW).current(),
                0,
                "raw cursor healed to 0 on generation change"
            );
            assert_eq!(
                consumer.tracker_for(STREAM_BOOTSTRAP).current(),
                42,
                "bootstrap tracker untouched by the raw room's generation-change heal"
            );
        }
    }

    // ── Phase29d §1.3 — definitive room-generation wipe detection.
    // Replaces the deleted `published_decrease` module: synap 1.3.1
    // (synap#257) now ships `stream.stats().generation`, so the
    // interim published-decrease heuristic (and its baseline
    // bookkeeping) is gone — a generation CHANGE is the primary wipe
    // signal, closing the exact cursor == head_next equality gap the
    // old heuristics could only work around, never see. Ported once
    // against the fulltext consumer per the proposal — the heal is
    // identical byte-for-byte across all four consumers, already
    // proven by the phase29c per-consumer duplication above.

    mod generation {
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

        fn ok_stats(message_count: u64, max_offset: u64, generation: u64) -> serde_json::Value {
            serde_json::json!({
                "success": true, "request_id": "x", "error": null,
                "payload": {
                    "name": STREAM_ENRICHED,
                    "message_count": message_count, "max_offset": max_offset,
                    "min_offset": 0, "total_published": message_count,
                    "total_consumed": 0, "subscriber_count": 1, "dropped": 0,
                    "generation": generation,
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

        /// THE headline case §1.3 exists to close (synap#257 — see
        /// `.rulebook/tasks/phase29d_synap-wipe-equality-gap/proposal.md`):
        /// cursor 1, post-wipe room head_next also 1 (message_count
        /// 1, max_offset 0) — the phase29c `cursor > head_next` check
        /// can never fire here (1 > 1 is false) on ANY poll, which is
        /// exactly the residual gap the deleted published-decrease
        /// heuristic could only document, not close (it degenerates
        /// to the same equal-looking counts when a room is wiped and
        /// refilled to an equal size). `generation` is the first
        /// signal that CAN see it: two probes reporting different
        /// generations against identical-looking offsets is
        /// unambiguous wipe evidence — this test passing IS the proof
        /// §1.3 closed it.
        #[tokio::test]
        async fn generation_change_heals_even_when_cursor_equals_room_head() {
            let server = MockServer::start().await;

            // Every poll requests the same stale cursor (1) — the
            // room never has anything at offset 1 on either probe.
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.consume"))
                .and(body_string_contains("\"from_offset\":1"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_empty_consume()))
                .mount(&server)
                .await;
            // Probe #1: records generation A. head_next(1) ==
            // cursor(1) — the legacy check would never fire even if
            // it still ran.
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.stats"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats(1, 0, 100)))
                .up_to_n_times(1)
                .mount(&server)
                .await;
            // Probe #2: generation changed (100 -> 200) — the room
            // was wiped and refilled to the SAME apparent head_next.
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.stats"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats(1, 0, 200)))
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.consume"))
                .and(body_string_contains("\"from_offset\":0"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(ok_consume_one_event_at(0, "evt-generation-heal")),
                )
                .mount(&server)
                .await;

            let handle = Arc::new(FulltextHandle::new(&server.uri()).expect("handle"));
            let consumer = FulltextConsumer::new(handle).with_stale_check_interval(Duration::ZERO);
            consumer.tracker().seed(1);

            // Poll 1: empty; probe #1 records generation 100 (first
            // observation — says nothing about a wipe by itself).
            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 1");
            assert!(batch.is_empty());
            assert_eq!(
                consumer.tracker().current(),
                1,
                "first observation records the baseline generation, no heal"
            );

            // Poll 2: empty; probe #2 sees generation 200 !=
            // remembered 100 → heal, DESPITE cursor(1) ==
            // head_next(1) on every poll — this is the residual gap
            // closing.
            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 2");
            assert!(batch.is_empty());
            assert_eq!(
                consumer.tracker().current(),
                0,
                "generation change healed the cursor despite cursor == head_next"
            );

            // Poll 3: resumes from 0 and delivers the post-wipe event.
            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 3");
            assert_eq!(batch.len(), 1, "healed consumer resumes delivery");
            assert_eq!(batch[0].event_id.as_deref(), Some("evt-generation-heal"));
        }

        /// Guards against a heal that fires on every poll and replays
        /// the room forever: repeated probes reporting the SAME
        /// generation must never reset a cursor that is legitimately
        /// caught up.
        #[tokio::test]
        async fn same_generation_never_resets_a_healthy_cursor() {
            let server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.consume"))
                .and(body_string_contains("\"from_offset\":3"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_empty_consume()))
                .mount(&server)
                .await;
            // Every probe reports the SAME generation.
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.stats"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats(3, 2, 42)))
                .mount(&server)
                .await;

            let handle = Arc::new(FulltextHandle::new(&server.uri()).expect("handle"));
            let consumer = FulltextConsumer::new(handle).with_stale_check_interval(Duration::ZERO);
            consumer.tracker().seed(3);

            // Poll 1: first observation, records generation 42.
            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 1");
            assert!(batch.is_empty());
            assert_eq!(
                consumer.tracker().current(),
                3,
                "no heal on first observation"
            );

            // Poll 2, 3, 4: generation unchanged every time — the
            // cursor must never be reset.
            for i in 0..3 {
                let batch = consumer
                    .next_batch(STREAM_ENRICHED, 10)
                    .await
                    .unwrap_or_else(|_| panic!("poll {i}"));
                assert!(batch.is_empty());
                assert_eq!(
                    consumer.tracker().current(),
                    3,
                    "same generation must never reset a healthy cursor (poll {i})"
                );
            }
        }

        /// The other phase29c blind spot, now closed by generation: a
        /// synap wipe while the consumer LAGS. The refilled room's
        /// head can be AHEAD of the stale cursor, so
        /// `consume(from_offset=cursor)` returns events from the NEW
        /// generation at overlapping offsets — the poll is never
        /// empty, so the phase29c empty-poll fast path never fires.
        /// Phase29d's promoted "probe on ANY poll" trigger + the
        /// generation check closes this: the generation-change heal
        /// still runs on a non-empty poll.
        #[tokio::test]
        async fn lagging_consumer_wipe_heals_on_generation_change() {
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
            // Probe #1: records generation 9 (first observation).
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.stats"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats(6, 5, 9)))
                .up_to_n_times(1)
                .mount(&server)
                .await;
            // Probe #2: generation changed 9 -> 10 → heal.
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.stats"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats(3, 2, 10)))
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
            // 2), but the promoted ANY-poll probe still fires and
            // records generation 9 (first observation — no heal yet).
            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 1");
            assert_eq!(batch.len(), 1, "non-empty poll must still be delivered");
            assert_eq!(batch[0].event_id.as_deref(), Some("evt-lag-old-gen"));
            assert_eq!(consumer.tracker().current(), 2, "no heal yet on poll 1");

            // Poll 2: consume STILL returns the stale-offset event
            // (masking the wipe locally) — but probe #2 sees
            // generation 10 != remembered 9 → the change heals the
            // cursor to 0 regardless of what this poll's consume
            // result looked like.
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
                "generation change healed the cursor to 0 despite the non-empty poll"
            );

            // Poll 3: resumes from 0 and delivers the real post-wipe event.
            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 3");
            assert_eq!(batch.len(), 1);
            assert_eq!(batch[0].event_id.as_deref(), Some("evt-lag-heal"));
        }

        /// `generation == 0` means the server predates synap 1.3.1
        /// (the field is absent from the wire payload and defaults
        /// via serde). The legacy `cursor > head_next` comparison is
        /// the only signal available then; it must still fire so an
        /// old-server deployment does not silently regress to the
        /// original phase29-pre starvation bug. Strictly weaker than
        /// the generation check — it cannot see the equality
        /// coincidence the two tests above cover.
        #[tokio::test]
        async fn zero_generation_falls_back_to_head_comparison() {
            let server = MockServer::start().await;

            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.consume"))
                .and(body_string_contains("\"from_offset\":5"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_empty_consume()))
                .mount(&server)
                .await;
            // generation: 0 (via ok_stats' default arg semantics: pass
            // 0 explicitly) — pre-1.3.1 server. head_next =
            // max_offset(0)+1 = 1, cursor(5) > head_next(1) → the
            // fallback must still heal.
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.stats"))
                .respond_with(ResponseTemplate::new(200).set_body_json(ok_stats(1, 0, 0)))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(method("POST"))
                .and(path("/api/v1/command"))
                .and(body_string_contains("stream.consume"))
                .and(body_string_contains("\"from_offset\":0"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(ok_consume_one_event_at(0, "evt-fallback-heal")),
                )
                .mount(&server)
                .await;

            let handle = Arc::new(FulltextHandle::new(&server.uri()).expect("handle"));
            let consumer = FulltextConsumer::new(handle);
            consumer.tracker().seed(5);

            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 1");
            assert!(batch.is_empty(), "post-wipe room has nothing at offset 5");
            assert_eq!(
                consumer.tracker().current(),
                0,
                "generation==0 fallback (cursor > head_next) still heals"
            );

            let batch = consumer
                .next_batch(STREAM_ENRICHED, 10)
                .await
                .expect("poll 2");
            assert_eq!(batch.len(), 1, "healed consumer resumes delivery");
            assert_eq!(batch[0].event_id.as_deref(), Some("evt-fallback-heal"));
        }
    }
}
