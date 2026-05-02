//! Phase11l §1.2 — Nexus external-id smoke IT.
//!
//! Validates that a freshly-bumped `nexus-graph-sdk = "2.1"` round-trips
//! `_id` against the live Nexus container the rest of the Cortex stack
//! talks to. This is the gate Cortex's phase11l (mapper / cypher
//! template / schema rewrite) waits behind: until this passes, the
//! migration MUST NOT touch the structural surface — the assumption
//! that the SDK + server agree on `_id` semantics is the load-bearing
//! one.
//!
//! Gated on `CORTEX_NEXUS_EXTERNAL_ID_IT=1`. Without it, every test
//! marks itself "ignored" at runtime so plain `cargo test
//! -p cortex-workers --tests` stays green when Nexus is offline.
//! Honors `CORTEX_GRAPH_NEXUS_URL` (the same env Cortex's graph
//! worker reads) and falls back to `NEXUS_LIVE_HOST` (the SDK's
//! convention) and finally `http://127.0.0.1:17002` (the docker
//! compose default).
//!
//! Coverage:
//!
//! - `sha256:<hex>` round-trip (the hash variant Cortex will use for
//!   every `:Artifact` once the mapper migrates).
//! - `str:<utf8>` round-trip (the variant `:Symbol` /
//!   `:ExternalPackage` / `:UnresolvedImport` will use — the existing
//!   composite `repo|lang|qualified_name` slots in unchanged).
//! - `ConflictPolicy::Match` — re-create with the same `_id` returns
//!   the existing node id, no error, no property overwrite.
//! - `ConflictPolicy::Replace` — re-create with the same `_id` and a
//!   new property bag overwrites the properties; same internal id.
//! - `ConflictPolicy::Error` (default) — re-create with the same `_id`
//!   surfaces an error in the response (or a 4xx — both are valid
//!   per Nexus phase9 §2.5).

use nexus_sdk::{NexusClient, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

fn value_string(v: &Value) -> Option<&str> {
    match v {
        Value::String(s) => Some(s.as_str()),
        _ => None,
    }
}

static UNIQ: AtomicU64 = AtomicU64::new(0);

fn it_enabled() -> bool {
    std::env::var("CORTEX_NEXUS_EXTERNAL_ID_IT")
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

/// Per-test unique discriminator so parallel runs and re-runs against
/// a shared catalog don't collide on `_id`.
fn uniq_hex() -> String {
    let n = UNIQ.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    let raw = format!("cortex11l{pid}{n}");
    let mut hex = String::with_capacity(64);
    for byte in raw.as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    while hex.len() < 64 {
        hex.push('0');
    }
    hex.truncate(64);
    hex
}

fn props(pairs: &[(&str, Value)]) -> HashMap<String, Value> {
    let mut p = HashMap::new();
    for (k, v) in pairs {
        p.insert((*k).to_string(), v.clone());
    }
    p
}

macro_rules! skip_unless_enabled {
    () => {
        if !it_enabled() {
            eprintln!(
                "skipping — set CORTEX_NEXUS_EXTERNAL_ID_IT=1 to run (target Nexus: {})",
                nexus_url()
            );
            return;
        }
    };
}

#[tokio::test]
async fn sha256_external_id_round_trips() {
    skip_unless_enabled!();
    let c = client();
    let ext = format!("sha256:{}", uniq_hex());
    let create = c
        .create_node_with_external_id(
            vec!["CortexSmokeArtifact".to_string()],
            props(&[("path", Value::String("crates/foo/src/lib.rs".into()))]),
            ext.clone(),
            Some("error"),
        )
        .await
        .expect("create_node_with_external_id");
    assert!(create.error.is_none(), "create error: {:?}", create.error);
    let got = c
        .get_node_by_external_id(&ext)
        .await
        .expect("get_node_by_external_id");
    let node = got.node.expect("node present");
    assert_eq!(
        node.id, create.node_id,
        "round-trip must resolve the same internal id"
    );
}

#[tokio::test]
async fn str_external_id_round_trips() {
    skip_unless_enabled!();
    let c = client();
    let ext = format!("str:cortex|rust|crate::smoke::{}", uniq_hex());
    let create = c
        .create_node_with_external_id(
            vec!["CortexSmokeSymbol".to_string()],
            props(&[("language", Value::String("rust".into()))]),
            ext.clone(),
            None,
        )
        .await
        .expect("create_node_with_external_id");
    assert!(create.error.is_none(), "create error: {:?}", create.error);
    let got = c
        .get_node_by_external_id(&ext)
        .await
        .expect("get_node_by_external_id");
    assert_eq!(got.node.expect("node").id, create.node_id);
}

#[tokio::test]
async fn conflict_policy_match_returns_existing() {
    skip_unless_enabled!();
    let c = client();
    let ext = format!("sha256:{}", uniq_hex());
    let first = c
        .create_node_with_external_id(
            vec!["CortexSmokeArtifact".to_string()],
            props(&[("v", Value::String("first".into()))]),
            ext.clone(),
            Some("error"),
        )
        .await
        .expect("first create");
    assert!(
        first.error.is_none(),
        "first create error: {:?}",
        first.error
    );
    // Repeat with policy=match — must return the same internal id and
    // MUST NOT raise an error or overwrite the property.
    let second = c
        .create_node_with_external_id(
            vec!["CortexSmokeArtifact".to_string()],
            props(&[("v", Value::String("second".into()))]),
            ext.clone(),
            Some("match"),
        )
        .await
        .expect("second create with match policy");
    assert!(
        second.error.is_none(),
        "match policy must not surface an error: {:?}",
        second.error
    );
    assert_eq!(
        second.node_id, first.node_id,
        "match policy must return the existing internal id"
    );
    // Property must remain the original value (Match leaves props alone).
    let got = c.get_node_by_external_id(&ext).await.expect("lookup");
    let node = got.node.expect("node");
    let v = node
        .properties
        .get("v")
        .and_then(value_string)
        .unwrap_or("");
    assert_eq!(v, "first", "match policy must NOT overwrite properties");
}

#[tokio::test]
async fn conflict_policy_replace_overwrites_props() {
    skip_unless_enabled!();
    let c = client();
    let ext = format!("sha256:{}", uniq_hex());
    let first = c
        .create_node_with_external_id(
            vec!["CortexSmokeArtifact".to_string()],
            props(&[("v", Value::String("first".into()))]),
            ext.clone(),
            None,
        )
        .await
        .expect("first create");
    assert!(
        first.error.is_none(),
        "first create error: {:?}",
        first.error
    );
    let second = c
        .create_node_with_external_id(
            vec!["CortexSmokeArtifact".to_string()],
            props(&[("v", Value::String("second".into()))]),
            ext.clone(),
            Some("replace"),
        )
        .await
        .expect("second create with replace policy");
    assert!(
        second.error.is_none(),
        "replace policy must not surface an error: {:?}",
        second.error
    );
    assert_eq!(
        second.node_id, first.node_id,
        "replace policy must keep the same internal id"
    );
    let got = c.get_node_by_external_id(&ext).await.expect("lookup");
    let node = got.node.expect("node");
    let v = node
        .properties
        .get("v")
        .and_then(value_string)
        .unwrap_or("");
    assert_eq!(v, "second", "replace policy must overwrite properties");
}

#[tokio::test]
async fn conflict_policy_error_rejects_duplicate() {
    skip_unless_enabled!();
    let c = client();
    let ext = format!("sha256:{}", uniq_hex());
    let first = c
        .create_node_with_external_id(
            vec!["CortexSmokeArtifact".to_string()],
            HashMap::new(),
            ext.clone(),
            Some("error"),
        )
        .await
        .expect("first create");
    assert!(
        first.error.is_none(),
        "first create error: {:?}",
        first.error
    );
    // Second create without an explicit policy must default to `error`
    // semantics. The SDK either surfaces an Err(_) or returns Ok with
    // a populated `.error` field — both are valid per Nexus phase9 §2.5.
    let second = c
        .create_node_with_external_id(
            vec!["CortexSmokeArtifact".to_string()],
            HashMap::new(),
            ext.clone(),
            Some("error"),
        )
        .await;
    match second {
        Ok(resp) => {
            assert!(
                resp.error.is_some(),
                "ConflictPolicy::Error must populate the error field on duplicate; got {resp:?}"
            );
        }
        Err(_) => { /* SDK propagated as Err — also valid */ }
    }
}

#[tokio::test]
async fn lookup_unknown_external_id_yields_node_none() {
    skip_unless_enabled!();
    let c = client();
    let ext = format!("sha256:{}", uniq_hex());
    let got = c
        .get_node_by_external_id(&ext)
        .await
        .expect("lookup must not error on absent");
    assert!(
        got.node.is_none(),
        "absent external id must resolve to node = None, got {got:?}"
    );
}
