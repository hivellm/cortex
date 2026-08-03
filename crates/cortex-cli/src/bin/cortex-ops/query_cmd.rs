//! Phase18 §4.3 — `cortex-ops {query, history, supersession}`.
//!
//! These three commands give operators a CLI that mirrors the
//! retrieval surface §4.4 will expose over HTTP:
//!
//! - `query "X" [--as-of] [--branch] [--projects]` posts to
//!   `cortex-api`'s `/v1/query` (the orchestrator applies the
//!   phase18 §3.3 wedge automatically; the new optional fields
//!   round-trip through the request body so §4.5 can add them
//!   without a CLI rev).
//! - `history <entity-id> [--as-of]` reads the `TimelineEvent`
//!   rows tagged with the entity from Nexus, ordered by
//!   `valid_from_unix DESC`.
//! - `supersession <entity-id>` walks the `SUPERSEDES` chain
//!   off the entity (ADR-023 §1.6) and prints the lineage.

use std::collections::HashMap;
use std::process::ExitCode;

use chrono::DateTime;
use nexus_sdk::Value as NexusValue;
use serde_json::{json, Value};

/// Phase18 §4.3 — `cortex-ops query`.
#[allow(clippy::too_many_arguments)]
pub(super) fn query(
    query_text: String,
    as_of: Option<String>,
    branch: Option<String>,
    projects: Vec<String>,
    api_url: Option<String>,
    intent: Option<String>,
    limit: u32,
    json_out: bool,
) -> ExitCode {
    let api = resolve_api_url(api_url);
    let as_of_normalized = match as_of.as_deref().map(parse_rfc3339_or_date).transpose() {
        Ok(v) => v.flatten(),
        Err(e) => {
            eprintln!("ERROR: --as-of: {e}");
            return ExitCode::from(2);
        }
    };
    let intent_value = intent.unwrap_or_else(|| "pre_change_context".to_string());

    let mut body = json!({
        "query": query_text,
        "intent": intent_value,
        "limit": limit,
        "k": 50_u32.max(limit),
        "include": ["snippets", "decisions", "similar_turns"],
        "budget_ms": 2000_u64,
        "scope": {},
    });
    if let Some(obj) = body.as_object_mut() {
        if let Some(a) = as_of_normalized {
            obj.insert("as_of".to_string(), Value::String(a));
        }
        if let Some(b) = branch {
            obj.insert("branch".to_string(), Value::String(b));
        }
        if !projects.is_empty() {
            obj.insert(
                "projects".to_string(),
                Value::Array(projects.iter().map(|p| Value::String(p.clone())).collect()),
            );
        }
    }

    let runtime = match build_runtime() {
        Ok(rt) => rt,
        Err(code) => return code,
    };
    let url = format!("{}/v1/query", api.trim_end_matches('/'));
    let response: Result<Value, String> = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| format!("build http client: {e}"))?;
        let resp = client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("POST {url}: {e}"))?;
        let status = resp.status();
        let bytes = resp.bytes().await.map_err(|e| format!("read body: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "{status}: {}",
                String::from_utf8_lossy(&bytes)
                    .chars()
                    .take(256)
                    .collect::<String>()
            ));
        }
        serde_json::from_slice::<Value>(&bytes).map_err(|e| format!("parse response: {e}"))
    });
    let response = match response {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ERROR: {e}");
            return ExitCode::from(2);
        }
    };

    if json_out {
        println!(
            "{}",
            serde_json::to_string_pretty(&response).unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }
    print_query_response(&response);
    ExitCode::SUCCESS
}

/// Phase18 §4.3 — `cortex-ops history <entity-id> [--as-of]`.
pub(super) fn history(
    entity_id: String,
    as_of: Option<String>,
    nexus: Option<String>,
    limit: u32,
    json_out: bool,
) -> ExitCode {
    let (client, nexus_url) = match connect_nexus(nexus) {
        Ok(c) => c,
        Err(code) => return code,
    };
    let as_of_unix = match as_of.as_deref().map(parse_rfc3339_to_unix).transpose() {
        Ok(v) => v.flatten(),
        Err(e) => {
            eprintln!("ERROR: --as-of: {e}");
            return ExitCode::from(2);
        }
    };
    let mut hist_params: HashMap<String, NexusValue> = HashMap::new();
    hist_params.insert(
        "entity_id".to_string(),
        NexusValue::String(entity_id.clone()),
    );
    let mut clauses: Vec<String> = vec!["t.entity_id = $entity_id".to_string()];
    if let Some(a) = as_of_unix {
        clauses.push(format!("t.recorded_at_unix <= {a}"));
    }
    let cypher = format!(
        "MATCH (t:TimelineEvent) WHERE {where_} \
         RETURN t.id AS id, t.kind AS kind, t.branch_id AS branch_id, \
                t.valid_from_unix AS valid_from_unix, \
                t.recorded_at_unix AS recorded_at_unix, \
                t.title AS title, t.summary AS summary \
         ORDER BY t.valid_from_unix DESC LIMIT {limit}",
        where_ = clauses.join(" AND "),
    );
    let runtime = match build_runtime() {
        Ok(rt) => rt,
        Err(code) => return code,
    };
    let rows = match runtime.block_on(async {
        client
            .sdk()
            .execute_cypher(&cypher, Some(hist_params))
            .await
    }) {
        Ok(out) => out.rows,
        Err(e) => {
            eprintln!("ERROR: nexus history: {e}");
            return ExitCode::from(2);
        }
    };
    if json_out {
        let payload = json!({
            "nexus_url": nexus_url,
            "entity_id": entity_id,
            "as_of_unix": as_of_unix,
            "events": rows,
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }
    println!(
        "cortex-ops history @ {nexus_url}\n  entity_id={entity_id}  events={count}",
        count = rows.len()
    );
    for row in &rows {
        let kind = row.get("kind").and_then(|v| v.as_str()).unwrap_or("<?>");
        let branch = row
            .get("branch_id")
            .and_then(|v| v.as_str())
            .unwrap_or("<?>");
        let vf = row
            .get("valid_from_unix")
            .and_then(|v| v.as_i64())
            .map(format_unix_day)
            .unwrap_or_else(|| "<?>".to_string());
        let title = row
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .chars()
            .take(64)
            .collect::<String>();
        println!("  {vf:<12}  {kind:<18}  {branch:<22}  {title}");
    }
    ExitCode::SUCCESS
}

/// Phase18 §4.3 — `cortex-ops supersession <entity-id>`.
pub(super) fn supersession(entity_id: String, nexus: Option<String>, json_out: bool) -> ExitCode {
    let (client, nexus_url) = match connect_nexus(nexus) {
        Ok(c) => c,
        Err(code) => return code,
    };
    // Walks the `SUPERSEDES` chain in both directions off the
    // entity: predecessors (this fact replaced what?) + successors
    // (this fact was replaced by what?). The classifier (§3.3
    // wedge) reads the same edges to derive the SUPERSEDED state.
    let mut sup_params: HashMap<String, NexusValue> = HashMap::new();
    sup_params.insert("id".to_string(), NexusValue::String(entity_id.clone()));
    let cypher =
        "OPTIONAL MATCH p_path = (this)-[:SUPERSEDES*0..10]->(predecessor) WHERE this.id = $id \
         WITH this, [n IN nodes(p_path) | { id: n.id, kind: head(labels(n)) }] AS predecessors \
         OPTIONAL MATCH s_path = (successor)-[:SUPERSEDES*0..10]->(this) \
         RETURN this.id AS id, head(labels(this)) AS label, \
                predecessors, \
                [n IN nodes(s_path) | { id: n.id, kind: head(labels(n)) }] AS successors"
            .to_string();
    let runtime = match build_runtime() {
        Ok(rt) => rt,
        Err(code) => return code,
    };
    let rows = match runtime
        .block_on(async { client.sdk().execute_cypher(&cypher, Some(sup_params)).await })
    {
        Ok(out) => out.rows,
        Err(e) => {
            eprintln!("ERROR: nexus supersession: {e}");
            return ExitCode::from(2);
        }
    };
    if rows.is_empty() {
        eprintln!("ERROR: entity {entity_id:?} not found");
        return ExitCode::from(1);
    }
    if json_out {
        let payload = json!({
            "nexus_url": nexus_url,
            "entity_id": entity_id,
            "lineage": rows.first(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
        return ExitCode::SUCCESS;
    }
    let row = &rows[0];
    println!("cortex-ops supersession @ {nexus_url}\n  entity_id={entity_id}",);
    let pred = row.get("predecessors").cloned().unwrap_or(Value::Null);
    let succ = row.get("successors").cloned().unwrap_or(Value::Null);
    println!(
        "  predecessors: {}",
        serde_json::to_string(&pred).unwrap_or_default()
    );
    println!(
        "  successors:   {}",
        serde_json::to_string(&succ).unwrap_or_default()
    );
    ExitCode::SUCCESS
}

// ---- helpers ----------------------------------------------------------------

fn connect_nexus(
    nexus: Option<String>,
) -> Result<(cortex_workers::graph::nexus_client::LiveNexusClient, String), ExitCode> {
    use cortex_workers::graph::config::GraphConfig;
    use cortex_workers::graph::nexus_client::LiveNexusClient;

    let nexus_url = nexus
        .or_else(|| {
            cortex_config::Config::load()
                .ok()
                .and_then(|c| c.nexus.nexus_url)
        })
        .unwrap_or_else(|| "http://127.0.0.1:17002".to_string());
    let cfg = GraphConfig {
        nexus_url: nexus_url.clone(),
        ..GraphConfig::default()
    };
    LiveNexusClient::new(cfg)
        .map(|c| (c, nexus_url.clone()))
        .map_err(|e| {
            eprintln!("ERROR: connect to Nexus at {nexus_url}: {e}");
            ExitCode::from(2)
        })
}

fn resolve_api_url(opt: Option<String>) -> String {
    opt.or_else(|| {
        cortex_config::Config::load()
            .unwrap_or_default()
            .dashboard
            .api_url
    })
    .unwrap_or_else(|| "http://127.0.0.1:17081".to_string())
}

fn build_runtime() -> Result<tokio::runtime::Runtime, ExitCode> {
    tokio::runtime::Runtime::new().map_err(|e| {
        eprintln!("ERROR: build tokio runtime: {e}");
        ExitCode::from(1)
    })
}

fn parse_rfc3339_or_date(raw: &str) -> Result<Option<String>, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let normalized = if s.len() == 10 && s.chars().filter(|c| *c == '-').count() == 2 {
        format!("{s}T00:00:00Z")
    } else {
        s.to_string()
    };
    DateTime::parse_from_rfc3339(&normalized)
        .map(|dt| {
            Some(
                dt.with_timezone(&chrono::Utc)
                    .format("%Y-%m-%dT%H:%M:%SZ")
                    .to_string(),
            )
        })
        .map_err(|e| format!("parse {normalized:?}: {e}"))
}

fn parse_rfc3339_to_unix(raw: &str) -> Result<Option<i64>, String> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let normalized = if s.len() == 10 && s.chars().filter(|c| *c == '-').count() == 2 {
        format!("{s}T00:00:00Z")
    } else {
        s.to_string()
    };
    DateTime::parse_from_rfc3339(&normalized)
        .map(|dt| Some(dt.timestamp()))
        .map_err(|e| format!("parse {normalized:?}: {e}"))
}

fn format_unix_day(unix: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp(unix, 0)
        .map(|dt| dt.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| format!("@{unix}"))
}

fn print_query_response(resp: &Value) {
    let snippets = resp
        .get("results")
        .and_then(|r| r.get("snippets"))
        .and_then(|s| s.as_array());
    let count = snippets.map(|s| s.len()).unwrap_or(0);
    let used_ms = resp
        .get("budget")
        .and_then(|b| b.get("used_ms"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    println!("cortex-ops query  snippets={count}  used_ms={used_ms}");
    if let Some(arr) = snippets {
        for s in arr {
            let rank = s.get("rank").and_then(|v| v.as_u64()).unwrap_or(0);
            let source = s.get("source").and_then(|v| v.as_str()).unwrap_or("");
            let path = s.get("path").and_then(|v| v.as_str()).unwrap_or("");
            let symbol = s.get("symbol").and_then(|v| v.as_str()).unwrap_or("");
            println!("  [{rank}] ({source}) {path} :: {symbol}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_api_url_prefers_explicit_arg() {
        assert_eq!(resolve_api_url(Some("http://x".into())), "http://x");
    }

    #[test]
    fn parse_rfc3339_or_date_handles_day_precision() {
        assert_eq!(
            parse_rfc3339_or_date("2026-04-01").unwrap().as_deref(),
            Some("2026-04-01T00:00:00Z")
        );
    }

    #[test]
    fn parse_rfc3339_or_date_returns_none_on_empty() {
        assert!(parse_rfc3339_or_date("").unwrap().is_none());
        assert!(parse_rfc3339_or_date("   ").unwrap().is_none());
    }

    #[test]
    fn parse_rfc3339_to_unix_round_trips() {
        let unix = parse_rfc3339_to_unix("2026-04-01T00:00:00Z")
            .unwrap()
            .unwrap();
        assert_eq!(format_unix_day(unix), "2026-04-01");
    }
}
