//! Phase6b regression guard for the spec-11 lane-projection contract.
//!
//! Every overlay derivation in [`crate::orchestrator`]
//! (`derive_decisions`, `derive_similar_turns`, `derive_laws`)
//! reads its inputs out of [`crate::lanes::LaneHit::extras`]. The
//! orchestrator unit tests have always passed because the in-tree
//! `MemoryKeywordLane` double seeds those keys directly — but the
//! live lanes (`MeiliKeywordLane`, `VectorizerLane`) used to drop
//! every contract key on the floor, so production bundles never
//! carried a "Recent decisions" / "Similar past turns" / "Law
//! violations" section even when the underlying data matched.
//!
//! These tests pin the live projections against a fixture upstream
//! document that carries every key in
//! [`crate::lanes::LANE_EXTRAS_KEYS`] and assert each lands on
//! `LaneHit.extras` 1:1. They also lock in a "missing keys
//! round-trip as absent" case — the orchestrator's overlay
//! derivers depend on `extras.get(key) -> None` for the no-match
//! branch and a stray `Value::Null` would let bogus `DecisionRef`
//! rows through.

use serde_json::{json, Map as JsonMap, Value};

use crate::lanes::{LaneHit, LANE_EXTRAS_KEYS};
use crate::meili_lane::project_doc;
use crate::types::Scope;
use crate::vectorizer_lane::{project_search_result, WireSearchHit};

fn keyword_req(index: &str) -> crate::lanes::KeywordRequest {
    crate::lanes::KeywordRequest {
        index: index.to_string(),
        query: "fixture".to_string(),
        limit: 5,
        scope: Scope::default(),
    }
}

fn vector_req(collection: &str) -> crate::lanes::VectorRequest {
    crate::lanes::VectorRequest {
        collection: collection.to_string(),
        query: "fixture".to_string(),
        k: 5,
        scope: Scope::default(),
    }
}

/// Every contract key set on a single fixture document, with
/// distinguishable values so a swap regression (e.g. `decision_id`
/// landing under the `turn_id` slot) is caught by the assertion.
fn full_contract_values() -> Vec<(&'static str, Value)> {
    vec![
        ("decision_id", json!("DEC-0001")),
        ("decision_status", json!("accepted")),
        ("supersedes", json!(["DEC-0000"])),
        ("turn_id", json!("01HTURN0000000000000000000")),
        ("model", json!("claude-sonnet-4-6")),
        ("summary", json!("a turn worth surfacing")),
        ("law_id", json!("LAW-0001")),
        ("severity", json!("warn")),
    ]
}

#[test]
fn contract_keys_constant_matches_documented_set() {
    // Sanity: the constant's order is part of the spec — if a
    // future edit drops a key the regression test below will
    // miss it, so pin the membership separately.
    let keys: std::collections::BTreeSet<&str> = LANE_EXTRAS_KEYS.iter().copied().collect();
    for expected in [
        "decision_id",
        "decision_status",
        "supersedes",
        "turn_id",
        "model",
        "summary",
        "law_id",
        "severity",
    ] {
        assert!(
            keys.contains(expected),
            "LANE_EXTRAS_KEYS missing {expected:?}"
        );
    }
}

#[test]
fn meili_projects_every_contract_key_from_top_level() {
    let mut doc = json!({
        "id": "doc-1",
        "event_id": "01HEVT0000000000000000000A",
        "kind": "decision",
        "repo": "Cortex",
        "path": "decisions/0001-x.md",
        "title": "Decision 0001",
        "summary": "the chosen approach",
        "ts": 1_777_400_000,
        "_rankingScore": 0.42,
    });
    for (k, v) in full_contract_values() {
        doc[k] = v;
    }
    let hit: LaneHit = project_doc(doc, &keyword_req("cortex-cortex-decisions"))
        .expect("fixture deserialises");
    for (k, v) in full_contract_values() {
        assert_eq!(
            hit.extras.get(k),
            Some(&v),
            "Meili projection lost contract key {k:?}"
        );
    }
    // Lane label invariant — the keyword lane must keep stamping
    // `source = "keyword"` even after the contract sweep.
    assert_eq!(
        hit.extras.get("source").and_then(|v| v.as_str()),
        Some("keyword")
    );
}

#[test]
fn meili_prefers_meta_over_top_level_when_both_are_set() {
    // During a fulltext-worker rollout a doc can carry both shapes
    // simultaneously. The contract pins `_meta.<key>` as the
    // canonical source so writers can migrate the legacy nesting
    // out without coordinating a flag day with the api.
    let doc = json!({
        "id": "doc-1",
        "event_id": "01HEVT0000000000000000000A",
        "kind": "decision",
        "title": "Decision 0001",
        "decision_id": "DEC-LEGACY",
        "_meta": {
            "decision_id": "DEC-CANONICAL",
        },
    });
    let hit: LaneHit = project_doc(doc, &keyword_req("cortex-cortex-decisions"))
        .expect("fixture deserialises");
    assert_eq!(
        hit.extras.get("decision_id").and_then(|v| v.as_str()),
        Some("DEC-CANONICAL")
    );
}

#[test]
fn meili_missing_contract_keys_round_trip_as_absent() {
    let doc = json!({
        "id": "doc-1",
        "event_id": "01HEVT0000000000000000000A",
        "kind": "code",
        "repo": "Cortex",
        "title": "no decision here",
        "body": "irrelevant snippet",
    });
    let hit: LaneHit = project_doc(doc, &keyword_req("cortex-cortex-code"))
        .expect("fixture deserialises");
    for k in LANE_EXTRAS_KEYS {
        assert!(
            !hit.extras.contains_key(*k),
            "expected contract key {k:?} absent on a doc that doesn't carry it"
        );
    }
}

#[test]
fn vectorizer_projects_every_contract_key_from_payload_top_level() {
    let mut payload: JsonMap<String, Value> = JsonMap::new();
    payload.insert("repo".into(), json!("Cortex"));
    payload.insert("kind".into(), json!("turn"));
    payload.insert("body".into(), json!("fused snippet body"));
    for (k, v) in full_contract_values() {
        payload.insert(k.to_string(), v);
    }
    let r = WireSearchHit {
        id: "vec-1".into(),
        score: 0.91,
        payload,
        vector: None,
    };
    let hit: LaneHit = project_search_result(r, &vector_req("cortex-cortex-turns"));
    for (k, v) in full_contract_values() {
        assert_eq!(
            hit.extras.get(k),
            Some(&v),
            "Vectorizer projection lost contract key {k:?}"
        );
    }
    assert_eq!(
        hit.extras.get("source").and_then(|v| v.as_str()),
        Some("vector")
    );
}

#[test]
fn vectorizer_falls_back_to_nested_payload_when_top_level_lacks_the_key() {
    // phase11d — older embedder-worker builds nested the contract
    // keys under `payload.payload.<key>`. A mixed corpus during a
    // worker bump must still surface decisions / turns / laws.
    let mut nested: JsonMap<String, Value> = JsonMap::new();
    nested.insert("turn_id".into(), json!("01HTURN_PAYLOAD_NESTED_0000"));
    nested.insert("model".into(), json!("claude-haiku-4-5"));
    let mut payload: JsonMap<String, Value> = JsonMap::new();
    payload.insert("repo".into(), json!("Cortex"));
    payload.insert("kind".into(), json!("turn"));
    payload.insert("body".into(), json!("legacy nested fixture"));
    payload.insert("payload".into(), Value::Object(nested));

    let r = WireSearchHit {
        id: "vec-1".into(),
        score: 0.55,
        payload,
        vector: None,
    };
    let hit: LaneHit = project_search_result(r, &vector_req("cortex-cortex-turns"));
    assert_eq!(
        hit.extras.get("turn_id").and_then(|v| v.as_str()),
        Some("01HTURN_PAYLOAD_NESTED_0000")
    );
    assert_eq!(
        hit.extras.get("model").and_then(|v| v.as_str()),
        Some("claude-haiku-4-5")
    );
}

#[test]
fn vectorizer_top_level_wins_over_nested_payload_when_both_are_set() {
    let mut nested: JsonMap<String, Value> = JsonMap::new();
    nested.insert("turn_id".into(), json!("01HTURN_NESTED_LEGACY_00000"));
    let mut payload: JsonMap<String, Value> = JsonMap::new();
    payload.insert("turn_id".into(), json!("01HTURN_TOP_LEVEL_CURRENT_0"));
    payload.insert("body".into(), json!("dual-shape fixture"));
    payload.insert("payload".into(), Value::Object(nested));

    let r = WireSearchHit {
        id: "vec-1".into(),
        score: 0.55,
        payload,
        vector: None,
    };
    let hit: LaneHit = project_search_result(r, &vector_req("cortex-cortex-turns"));
    assert_eq!(
        hit.extras.get("turn_id").and_then(|v| v.as_str()),
        Some("01HTURN_TOP_LEVEL_CURRENT_0")
    );
}

#[test]
fn vectorizer_missing_contract_keys_round_trip_as_absent() {
    let mut payload: JsonMap<String, Value> = JsonMap::new();
    payload.insert("repo".into(), json!("Cortex"));
    payload.insert("kind".into(), json!("code"));
    payload.insert("body".into(), json!("no contract keys"));
    let r = WireSearchHit {
        id: "vec-1".into(),
        score: 0.5,
        payload,
        vector: None,
    };
    let hit: LaneHit = project_search_result(r, &vector_req("cortex-cortex-code"));
    for k in LANE_EXTRAS_KEYS {
        assert!(
            !hit.extras.contains_key(*k),
            "expected contract key {k:?} absent on a vector hit that doesn't carry it"
        );
    }
}
