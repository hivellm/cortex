//! Phase22 §3.3 — Nexus write→read property persistence IT.
//!
//! Confirms nexus#4 is fixed in Nexus 2.3.4: properties set on a MERGE
//! are no longer silently lost; a subsequent MATCH reads them back intact.
//!
//! **Write-path param binding finding (phase22 §3.3):**
//! Nexus 2.3.4 nexus#3 fix applies to READ-only queries only. Inline
//! property maps `{ id: $param }` fail with "Complex expressions not
//! supported in CREATE properties" when they appear in write-path queries
//! (MERGE or MATCH...MERGE). The production node/edge write path therefore
//! retains inline literals. This IT uses inline literals for the write side
//! (mirroring production) and $param binding for the read side (proving
//! nexus#3 READ is usable for property-presence checks).
//!
//! Gated on `CORTEX_NEXUS_WRITE_READ_IT=1`.
//!
//! URL chain: CORTEX_GRAPH_NEXUS_URL → NEXUS_LIVE_HOST → http://127.0.0.1:17002

use nexus_sdk::{NexusClient, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

static UNIQ: AtomicU64 = AtomicU64::new(0);

fn it_enabled() -> bool {
    std::env::var("CORTEX_NEXUS_WRITE_READ_IT")
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

fn unique_id() -> String {
    let n = UNIQ.fetch_add(1, Ordering::SeqCst);
    let pid = std::process::id();
    format!("cortex-phase22-wr-{pid}-{n}")
}

fn escape_literal(s: &str) -> String {
    s.chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            '\n' => vec!['\\', 'n'],
            '\r' => vec!['\\', 'r'],
            '\t' => vec!['\\', 't'],
            c if (c as u32) < 0x20 => vec![' '],
            c => vec![c],
        })
        .collect()
}

/// Node MERGE with inline literals (production pattern), then read-back via $param.
/// Confirms nexus#4 is fixed: all SET properties survive the write path.
#[tokio::test]
async fn node_properties_persist_after_merge_set() {
    if !it_enabled() {
        return;
    }
    let client = client();
    let test_id = unique_id();
    let test_kind = "TestWriteRead";
    let test_repo = "cortex";
    let test_label = format!("Phase22WriteReadIT-{test_id}");

    let cypher = format!(
        "MERGE (n:TestNode {{ id: \"{id}\" }}) \
         SET n.kind = \"{kind}\", n.repo = \"{repo}\", n.label = \"{label}\" \
         RETURN count(n) AS written",
        id = escape_literal(&test_id),
        kind = escape_literal(test_kind),
        repo = escape_literal(test_repo),
        label = escape_literal(&test_label),
    );
    let write_result = client
        .execute_cypher(&cypher, None)
        .await
        .expect("write execute_cypher failed");

    assert!(
        write_result.error.is_none(),
        "write query error: {:?}",
        write_result.error
    );

    // Read back via $param WHERE clause (nexus#3 READ confirmed working).
    let mut read_params: HashMap<String, Value> = HashMap::new();
    read_params.insert("id".to_string(), Value::String(test_id.clone()));

    let read_result = client
        .execute_cypher(
            "MATCH (n:TestNode) WHERE n.id = $id \
             RETURN n.id AS id, n.kind AS kind, n.repo AS repo, n.label AS label",
            Some(read_params),
        )
        .await
        .expect("read execute_cypher failed");

    assert!(
        read_result.error.is_none(),
        "read query error: {:?}",
        read_result.error
    );
    assert_eq!(
        read_result.rows.len(),
        1,
        "expected exactly one row for id={test_id}"
    );

    let row = read_result.rows[0].as_array().expect("row must be array");
    assert_eq!(row.len(), 4, "expected 4 columns (id, kind, repo, label)");

    let got_id = row[0].as_str().unwrap_or("");
    let got_kind = row[1].as_str().unwrap_or("");
    let got_repo = row[2].as_str().unwrap_or("");
    let got_label = row[3].as_str().unwrap_or("");

    assert_eq!(got_id, test_id, "id mismatch — nexus#4 property loss?");
    assert_eq!(got_kind, test_kind, "kind property lost — nexus#4?");
    assert_eq!(got_repo, test_repo, "repo property lost — nexus#4?");
    assert_eq!(got_label, test_label, "label property lost — nexus#4?");

    // Cleanup: remove the test node so re-runs don't accumulate stale data.
    let del = format!(
        "MATCH (n:TestNode {{ id: \"{}\" }}) DETACH DELETE n",
        escape_literal(&test_id)
    );
    let _ = client.execute_cypher(&del, None).await;
}
