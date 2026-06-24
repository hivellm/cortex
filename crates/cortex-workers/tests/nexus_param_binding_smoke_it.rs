//! Phase22 §3.2 — Nexus param-binding smoke IT.
//!
//! Validates that Nexus 2.3.4 correctly binds $param in Cypher queries.
//! This is the gate that unlocks §3.3–§3.5: removal of inline-literal
//! Cypher workarounds introduced for nexus#3.
//!
//! Gated on `CORTEX_NEXUS_PARAM_BINDING_IT=1`. Without it, every test
//! returns early so plain `cargo test -p cortex-workers --tests` stays
//! green when Nexus is offline.
//!
//! URL chain: CORTEX_GRAPH_NEXUS_URL → NEXUS_LIVE_HOST → http://127.0.0.1:17002
//!
//! Coverage:
//! - `RETURN $name AS val` with `{name:"param-binding-test"}` → val=="param-binding-test"
//! - `MATCH (d:Decision) WHERE d.id = $id RETURN d.id AS id LIMIT 1` with a
//!   known Decision node → returns one row with the expected id.

use nexus_sdk::{NexusClient, Value};
use std::collections::HashMap;

fn it_enabled() -> bool {
    std::env::var("CORTEX_NEXUS_PARAM_BINDING_IT")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn nexus_url() -> String {
    if let Ok(u) = std::env::var("CORTEX_GRAPH_NEXUS_URL") {
        return u;
    }
    if let Ok(u) = std::env::var("NEXUS_LIVE_HOST") {
        return u;
    }
    "http://127.0.0.1:17002".to_string()
}

fn client() -> NexusClient {
    NexusClient::new(&nexus_url()).expect("nexus client init")
}

/// Scalar RETURN $param round-trip: the simplest param-binding test.
/// Gate: nexus#3 (param binding in expression position).
#[tokio::test]
async fn return_scalar_param() {
    if !it_enabled() {
        return;
    }
    let client = client();
    let mut params = HashMap::new();
    params.insert(
        "name".to_string(),
        Value::String("param-binding-test".to_string()),
    );
    let result = client
        .execute_cypher("RETURN $name AS val", Some(params))
        .await
        .expect("execute_cypher failed");

    assert!(result.error.is_none(), "query error: {:?}", result.error);
    assert_eq!(result.columns, vec!["val"], "expected column 'val'");
    assert_eq!(result.rows.len(), 1, "expected one row");
    let val = result.rows[0]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .expect("row[0] should be a string");
    assert_eq!(
        val, "param-binding-test",
        "param binding returned wrong value"
    );
}

/// MATCH + $param in WHERE predicate: tests binding in filter position.
/// Gate: nexus#3 (param binding in WHERE clause).
///
/// Seeds the fixture id from a live MATCH so the test stays self-contained
/// regardless of which Decision nodes happen to be in the graph. If no
/// Decision with a non-null id exists (nexus#4 property-loss straggler
/// cohort), the test is skipped rather than failed — §3.6 handles that.
#[tokio::test]
async fn match_where_param() {
    if !it_enabled() {
        return;
    }
    let client = client();

    let seed = client
        .execute_cypher(
            "MATCH (d:Decision) WHERE d.id IS NOT NULL RETURN d.id AS id LIMIT 1",
            None,
        )
        .await
        .expect("seed query failed");

    if seed.rows.is_empty() {
        eprintln!(
            "skip match_where_param: no Decision node with id property \
             (nexus#4 straggler cohort — verified in §3.6)"
        );
        return;
    }

    let known_id = seed.rows[0]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .expect("seed row[0] should be a string")
        .to_string();

    let mut params = HashMap::new();
    params.insert("id".to_string(), Value::String(known_id.clone()));
    let result = client
        .execute_cypher(
            "MATCH (d:Decision) WHERE d.id = $id RETURN d.id AS id LIMIT 1",
            Some(params),
        )
        .await
        .expect("execute_cypher failed");

    assert!(result.error.is_none(), "query error: {:?}", result.error);
    assert_eq!(result.columns, vec!["id"], "expected column 'id'");
    assert_eq!(result.rows.len(), 1, "expected one row for known id");
    let returned_id = result.rows[0]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v.as_str())
        .expect("row[0] should be a string");
    assert_eq!(
        returned_id, known_id,
        "MATCH WHERE $param returned wrong id"
    );
}
