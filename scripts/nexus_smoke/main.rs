//! Standalone Nexus SDK smoke — verifies whether the cypher
//! "expected value at line 1 column 1" error is a Nexus bug or
//! a cortex-api integration bug.
//!
//! Run inside the cortex-api container so the hostname `nexus`
//! resolves:
//!   docker exec cortex-api /tmp/nexus_smoke

use nexus_sdk::*;
use std::collections::HashMap;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let url = env::args().nth(1).unwrap_or_else(|| {
        env::var("CORTEX_NEXUS_URL").unwrap_or_else(|_| "nexus://nexus:15474".into())
    });
    println!("=== nexus_smoke against {url} ===");

    let client = NexusClient::new(&url)?;

    println!("\n[1] health_check");
    println!("  -> {:?}", client.health_check().await);

    println!("\n[2] stats");
    match client.get_stats().await {
        Ok(s) => println!(
            "  nodes={} rels={} labels={}",
            s.catalog.node_count, s.catalog.rel_count, s.catalog.label_count
        ),
        Err(e) => println!("  err: {e:?}"),
    }

    println!("\n[3] cypher: RETURN 1");
    println!(
        "  -> {:?}",
        client.execute_cypher("RETURN 1 AS one", None).await
    );

    println!("\n[4] cypher: MATCH (n) RETURN count(n)");
    println!(
        "  -> {:?}",
        client
            .execute_cypher("MATCH (n) RETURN count(n) AS c", None)
            .await
    );

    println!("\n[5] cypher: MATCH (s) RETURN s LIMIT 1");
    println!(
        "  -> {:?}",
        client
            .execute_cypher("MATCH (s) RETURN s LIMIT 1", None)
            .await
    );

    println!("\n[6] cypher: MATCH (s) RETURN s.event_id AS id LIMIT 3");
    println!(
        "  -> {:?}",
        client
            .execute_cypher("MATCH (s) RETURN s.event_id AS id LIMIT 3", None)
            .await
    );

    println!("\n[7] cypher param: WHERE s.event_id = $id");
    let mut params: HashMap<String, Value> = HashMap::new();
    params.insert(
        "id".to_string(),
        Value::String("01KQ8F7BQ7N8QBKY16K1K6RYBH".into()),
    );
    println!(
        "  -> {:?}",
        client
            .execute_cypher(
                "MATCH (s) WHERE s.event_id = $id RETURN s LIMIT 1",
                Some(params),
            )
            .await
    );

    println!("\n[8] list labels");
    println!("  -> {:?}", client.list_labels().await);

    for label in ["Decision", "ToolCall", "Turn", "Consolidation", "Repo", "Cor_Decision"] {
        let q = format!("MATCH (n:{label}) RETURN n LIMIT 1");
        println!("\n[9] {q}");
        match client.execute_cypher(&q, None).await {
            Ok(r) => println!("  ok rows={} err={:?}", r.rows.len(), r.error),
            Err(e) => println!("  err: {e:?}"),
        }
    }

    println!("\n[10] MATCH (n:Decision) RETURN n.event_id AS id LIMIT 3");
    match client
        .execute_cypher(
            "MATCH (n:Decision) RETURN n.event_id AS id LIMIT 3",
            None,
        )
        .await
    {
        Ok(r) => println!("  ok rows={} err={:?}", r.rows.len(), r.error),
        Err(e) => println!("  err: {e:?}"),
    }

    println!("\n[11b] Decision node keys via id+labels+keys");
    for q in [
        "MATCH (n:Decision) RETURN id(n) AS nid, n.id AS i, n.event_id AS eid, n.decision_id AS did LIMIT 3",
        "MATCH (n:Decision) RETURN id(n) AS nid, keys(n) AS k LIMIT 1",
        "MATCH (n:Decision) RETURN n LIMIT 1",
    ] {
        match client.execute_cypher(q, None).await {
            Ok(r) => println!("  {q}\n    rows={:?} err={:?}", r.rows, r.error),
            Err(e) => println!("  {q}\n    err={e:?}"),
        }
    }
    println!("\n[12] get_node_by_external_id('01KQ8F7BQ7N8QBKY16K1K6RYBH')");
    match client
        .get_node_by_external_id("01KQ8F7BQ7N8QBKY16K1K6RYBH")
        .await
    {
        Ok(r) => println!("  ok: {:?}", r),
        Err(e) => println!("  err: {e:?}"),
    }

    println!("\n[13] Decision lookup via label-scoped where");
    match client
        .execute_cypher(
            "MATCH (n:Decision) WHERE n.event_id = $id RETURN id(n) AS nid LIMIT 1",
            Some({
                let mut p = HashMap::new();
                p.insert(
                    "id".to_string(),
                    Value::String("01KQNYMYKMPG5NQS2GZRBSVJ5R".into()),
                );
                p
            }),
        )
        .await
    {
        Ok(r) => println!("  ok rows={} err={:?} rows={:?}", r.rows.len(), r.error, r.rows),
        Err(e) => println!("  err: {e:?}"),
    }

    println!("\n[14] Neighbors via label+id lookup");
    match client
        .execute_cypher(
            "MATCH (s:Decision) WHERE s.event_id = $id MATCH (s)-[r]-(n) RETURN s, type(r) AS rk, n LIMIT 10",
            Some({
                let mut p = HashMap::new();
                p.insert(
                    "id".to_string(),
                    Value::String("01KQNYMYKMPG5NQS2GZRBSVJ5R".into()),
                );
                p
            }),
        )
        .await
    {
        Ok(r) => println!("  ok rows={} err={:?}", r.rows.len(), r.error),
        Err(e) => println!("  err: {e:?}"),
    }

    println!("\n[11] MATCH (n:Decision) RETURN id(n) AS nid, labels(n) AS lbl LIMIT 3");
    match client
        .execute_cypher(
            "MATCH (n:Decision) RETURN id(n) AS nid, labels(n) AS lbl LIMIT 3",
            None,
        )
        .await
    {
        Ok(r) => println!("  ok rows={} err={:?}", r.rows.len(), r.error),
        Err(e) => println!("  err: {e:?}"),
    }

    Ok(())
}
