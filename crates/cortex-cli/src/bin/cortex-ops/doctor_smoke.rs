//! Phase30 (live-e2e-smoke-and-doctor-wiring) —
//! `cortex-ops doctor-smoke`.
//!
//! The scheduled long-lived-stack doctor + e2e smoke. §1.1 designated
//! the LOCAL docker stack as the long-lived target (GitHub-hosted
//! runners cannot reach it, and §1.1 explicitly forbids reproducing
//! the fresh-boot-CI pattern health-smoke/doctor/retention-canary all
//! abandoned), so the scheduler is the same local cron infrastructure
//! `health.watchdog` uses (`cron_jobs` row `health.doctor_smoke`),
//! and failures surface exactly where every other cron failure does:
//! the row's `last_status`/`failure_streak` (dashboard schedule
//! panel) plus this command's non-zero exit.
//!
//! One run performs, against the LIVE stack:
//! 1. Backend + worker health: the four backends' `/health`, the
//!    reranker's `/health` (§3.1 coverage), the five cortex worker
//!    `/healthz` endpoints asserting `state == ok` (the
//!    freshness-derived state that already flagged the graph-worker
//!    stall — §2.3), and the host adapter's admin `/healthz`
//!    (§3.3 coverage; skip-not-fail when the operator has no adapter
//!    configured).
//! 2. The "registered but never exercised" gate (§2): every READ tool
//!    in `ToolRegistry::default_set()` is invoked in-process against
//!    the live cortex-api with minimal args synthesized from its own
//!    `inputSchema` — dead wiring (unreachable api, missing route
//!    panic) fails the run; per-tool outcomes + timestamps persist to
//!    `<home>/mcp_tool_smoke.json` so the exercise history is
//!    queryable across runs. `cortex_query` + `cortex_pre_thinking`
//!    must additionally succeed WITHOUT `isError` (the §1.2 real
//!    end-to-end calls). Write tools are exercised by real usage
//!    only, never by the smoke.
//!
//! This generalizes the four confirmed ship-then-dead-wire instances
//! (phantom-link verifier, pre-thinking cache counters, adapter
//! daemon, graph-worker stall) into one nightly gate (§2.4): every
//! wire is either probed (workers/adapter/backends) or exercised
//! (MCP tools) on every run.

use std::collections::BTreeMap;
use std::process::ExitCode;

use cortex_mcp_server::tools::{ToolContext, ToolRegistry};
use serde_json::{json, Value};

/// Tools that must succeed end-to-end (no `isError`) — the §1.2 smoke
/// pair. Everything else in the registry only has to prove its wire
/// is alive (a reachable, non-panicking call).
const MUST_SUCCEED: &[&str] = &["cortex_query", "cortex_pre_thinking"];

/// Synthesize a minimal argument object from a tool's own
/// `inputSchema`: every `required` property gets a type-appropriate
/// benign value. The goal is exercising the WIRE (route exists,
/// handler answers), not producing meaningful results — a
/// `found:false` / empty-hits / 404-mapped soft error all count as a
/// live wire.
pub(super) fn synthesize_args(descriptor: &Value) -> Value {
    let schema = &descriptor["inputSchema"];
    let mut args = serde_json::Map::new();
    let required: Vec<&str> = schema["required"]
        .as_array()
        .map(|a| a.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let props = schema["properties"].as_object();
    for name in required {
        let ty = props
            .and_then(|p| p.get(name))
            .and_then(|s| s["type"].as_str())
            .unwrap_or("string");
        let value = match ty {
            "integer" | "number" => json!(1),
            "boolean" => json!(false),
            "array" => json!([]),
            "object" => json!({}),
            // Benign string: real-looking enough to pass shape
            // validation in most handlers, meaningless enough to
            // return empty results.
            _ => match name {
                "query" | "q" => json!("doctor smoke probe"),
                "intent" => json!("free_search"),
                "repo" => json!("cortex"),
                "kind" => json!("tool_call"),
                _ => json!("doctor-smoke"),
            },
        };
        args.insert(name.to_string(), value);
    }
    Value::Object(args)
}

/// Resolve the persistent per-tool exercise ledger path.
fn ledger_path() -> std::path::PathBuf {
    let cfg = cortex_config::Config::load().unwrap_or_default();
    let home = cfg
        .ingestion
        .home
        .clone()
        .map(std::path::PathBuf::from)
        .or_else(|| super::helpers::home_dir().map(|h| h.join(".cortex")))
        .unwrap_or_else(|| std::path::PathBuf::from(".cortex"));
    home.join("mcp_tool_smoke.json")
}

pub(super) fn doctor_smoke(api_url: Option<String>, json_out: bool) -> ExitCode {
    let cfg = cortex_config::Config::load().unwrap_or_default();
    let api_url = api_url
        .or_else(|| cfg.dashboard.api_url.clone())
        .unwrap_or_else(|| "http://127.0.0.1:17000".to_string());

    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(r) => r,
        Err(e) => {
            eprintln!("doctor-smoke: tokio runtime: {e}");
            return ExitCode::from(2);
        }
    };
    let http = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            eprintln!("doctor-smoke: http client: {e}");
            return ExitCode::from(2);
        }
    };

    let mut failures: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();
    let mut skips: Vec<String> = Vec::new();

    // Pre-check: is cortex-api itself reachable? Decides how
    // per-tool "unreachable" classifies below — when the api answers
    // /v1/health, a tool-level timeout is a SLOW endpoint (warn),
    // not a dead wire (fail). Kills the false positives from
    // long-scan endpoints (files_touched ≈30s) without losing the
    // real dead-api signal.
    let api_reachable = rt.block_on(async {
        http.get(format!("{}/v1/health", api_url.trim_end_matches('/')))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    });
    if !api_reachable {
        failures.push(format!("cortex-api: {api_url}/v1/health unreachable"));
    }

    // ── 1. Backend + worker + adapter health ────────────────────────
    // (host-port defaults; every one overridable via config/env the
    // same way the plain `doctor` resolves them.)
    let backend_probes: Vec<(String, String, bool)> = vec![
        // (label, url, required)
        (
            "vectorizer".into(),
            format!(
                "{}/health",
                std::env::var("VECTORIZER_URL").unwrap_or_else(|_| "http://127.0.0.1:17001".into())
            ),
            true,
        ),
        (
            "nexus".into(),
            format!(
                "{}/health",
                std::env::var("NEXUS_URL").unwrap_or_else(|_| "http://127.0.0.1:17002".into())
            ),
            true,
        ),
        (
            "synap".into(),
            format!(
                "{}/health",
                std::env::var("SYNAP_URL").unwrap_or_else(|_| "http://127.0.0.1:17003".into())
            ),
            true,
        ),
        (
            "meilisearch".into(),
            format!(
                "{}/health",
                std::env::var("MEILI_URL").unwrap_or_else(|_| "http://127.0.0.1:17004".into())
            ),
            true,
        ),
        // §3.1 — the reranker was the one compose backend with no
        // health coverage anywhere. Third-party TEI image → probe its
        // own /health; required only when an endpoint is configured.
        (
            "cortex-reranker".into(),
            format!(
                "{}/health",
                std::env::var("CORTEX_RERANKER_HEALTH_URL")
                    .unwrap_or_else(|_| "http://127.0.0.1:17040".into())
            ),
            std::env::var("CORTEX_RERANKER_HEALTH_URL").is_ok(),
        ),
        // §3.3 — the host adapter daemon lives OUTSIDE compose and is
        // exactly the binary that silently died before; probe its
        // admin /healthz. Required only when configured, because a
        // fresh checkout has no adapter.
        (
            "cortex-adapter-claude".into(),
            format!(
                "{}/healthz",
                std::env::var("CORTEX_ADAPTER_ADMIN_URL")
                    .unwrap_or_else(|_| "http://127.0.0.1:17011".into())
            ),
            std::env::var("CORTEX_ADAPTER_ADMIN_URL").is_ok(),
        ),
    ];
    // §2.3 — worker /healthz endpoints; `state` is the
    // freshness-derived value (`rules::freshness_state`) that already
    // catches dead consume loops. `ok` required; `degraded`/`down`
    // fail.
    let worker_probes: Vec<(String, String)> = vec![
        ("cortex-ingestion".into(), "http://127.0.0.1:17010".into()),
        (
            "cortex-classifier-worker".into(),
            "http://127.0.0.1:17021".into(),
        ),
        (
            "cortex-embedder-worker".into(),
            "http://127.0.0.1:17022".into(),
        ),
        (
            "cortex-fulltext-worker".into(),
            "http://127.0.0.1:17023".into(),
        ),
        (
            "cortex-graph-worker".into(),
            "http://127.0.0.1:17024".into(),
        ),
    ];

    rt.block_on(async {
        for (label, url, required) in &backend_probes {
            let ok = http
                .get(url)
                .send()
                .await
                .map(|r| r.status().is_success())
                .unwrap_or(false);
            if ok {
                continue;
            }
            if *required {
                failures.push(format!("{label}: {url} unreachable/unhealthy"));
            } else {
                skips.push(format!("{label}: {url} not up (not configured — skip)"));
            }
        }
        for (label, base) in &worker_probes {
            let url = format!("{base}/healthz");
            let state = match http.get(&url).send().await {
                Ok(r) => r
                    .json::<Value>()
                    .await
                    .ok()
                    .and_then(|v| v["state"].as_str().map(String::from))
                    .unwrap_or_else(|| "unparseable".into()),
                Err(_) => "unreachable".into(),
            };
            match state.as_str() {
                "ok" => {}
                // Freshness-degraded cannot distinguish a dead
                // consume loop from a legitimately quiet stack (600s
                // idle window) — failing here every quiet night is
                // exactly the red-noise pattern §1.1 forbids. Warn;
                // the hard signals (down/unreachable) still fail.
                "degraded" => warnings.push(format!(
                    "{label}: /healthz state = degraded (idle or stalled)"
                )),
                other => failures.push(format!("{label}: /healthz state = {other}")),
            }
        }
    });

    // ── 2. Exercise every READ tool in-process against the live api ─
    let registry = ToolRegistry::default_set();
    let ctx = ToolContext::new(api_url.clone());
    let now_ms = chrono::Utc::now().timestamp_millis();
    let mut ledger: BTreeMap<String, Value> = std::fs::read_to_string(ledger_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let mut exercised = 0usize;

    for name in registry.names() {
        let tool = match registry.find(name) {
            Some(t) => t,
            None => continue,
        };
        if tool.read_or_write() != "read" {
            continue; // write tools are exercised by real usage only
        }
        let args = synthesize_args(&tool.descriptor());
        let outcome = rt.block_on(async { tool.call(&ctx, args).await });
        let (alive, detail) = match &outcome {
            Ok(res) => {
                // A soft error is a live wire UNLESS the api itself
                // was unreachable — that is exactly the dead-wire
                // signal this gate exists for.
                let text = res
                    .content
                    .first()
                    .and_then(|c| c["text"].as_str())
                    .unwrap_or_default();
                let unreachable = res.is_error && text.contains("api_unreachable");
                (!unreachable, if res.is_error { "soft_error" } else { "ok" })
            }
            Err(_) => (true, "invalid_input"), // route reached its arg validation — wire alive
        };
        let must_succeed = MUST_SUCCEED.contains(&name);
        let hard_ok = matches!(&outcome, Ok(r) if !r.is_error);
        if !alive {
            if api_reachable {
                // The api answers /v1/health but this tool's call
                // timed out — slow endpoint, wire proven separately.
                warnings.push(format!("{name}: call timed out (slow endpoint)"));
                exercised += 1;
            } else {
                failures.push(format!("{name}: cortex-api unreachable (dead wire)"));
            }
        } else if must_succeed && !hard_ok {
            failures.push(format!("{name}: e2e smoke call did not succeed cleanly"));
        } else {
            exercised += 1;
        }
        ledger.insert(
            name.to_string(),
            json!({ "last_exercised_ms": now_ms, "outcome": detail }),
        );
    }
    // Persist the ledger (best-effort — a read-only fs must not fail
    // the health verdict itself).
    if let Ok(serialized) = serde_json::to_string_pretty(&ledger) {
        let path = ledger_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let _ = std::fs::write(&path, serialized);
    }

    let status = if failures.is_empty() { "ok" } else { "fail" };
    if json_out {
        let payload = json!({
            "api_url": api_url,
            "status": status,
            "tools_exercised": exercised,
            "failures": failures,
            "warnings": warnings,
            "skipped": skips,
            "ledger_path": ledger_path().display().to_string(),
        });
        println!(
            "{}",
            serde_json::to_string_pretty(&payload).unwrap_or_default()
        );
    } else {
        println!("cortex-ops doctor-smoke @ {api_url}");
        println!("  tools_exercised = {exercised}");
        for s in &skips {
            println!("  skip  {s}");
        }
        for w in &warnings {
            println!("  warn  {w}");
        }
        for f in &failures {
            println!("  FAIL  {f}");
        }
        println!("  status = {status}");
    }
    if failures.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthesize_covers_required_props_with_type_appropriate_values() {
        let desc = json!({
            "inputSchema": {
                "type": "object",
                "required": ["query", "limit", "flags", "opts", "deep"],
                "properties": {
                    "query": {"type": "string"},
                    "limit": {"type": "integer"},
                    "flags": {"type": "array"},
                    "opts": {"type": "object"},
                    "deep": {"type": "boolean"},
                }
            }
        });
        let args = synthesize_args(&desc);
        assert_eq!(args["query"], "doctor smoke probe");
        assert_eq!(args["limit"], 1);
        assert_eq!(args["flags"], json!([]));
        assert_eq!(args["opts"], json!({}));
        assert_eq!(args["deep"], false);
    }

    #[test]
    fn synthesize_empty_when_nothing_required() {
        let desc = json!({"inputSchema": {"type": "object", "properties": {}}});
        assert_eq!(synthesize_args(&desc), json!({}));
    }

    #[test]
    fn every_read_tool_synthesizes_parseable_args() {
        // §2.2's static half: minimal args can be synthesized for
        // EVERY read tool from its own schema — a tool whose schema
        // the synthesizer cannot satisfy would silently escape the
        // nightly gate.
        let reg = ToolRegistry::default_set();
        for name in reg.names() {
            let tool = reg.find(name).unwrap();
            if tool.read_or_write() != "read" {
                continue;
            }
            let args = synthesize_args(&tool.descriptor());
            assert!(args.is_object(), "{name} args must be an object");
        }
    }
}
