//! Acceptance gate regression pin — validates the curated golden CSVs
//! are well-formed and the recorded baseline meets the phase14c floors.
//!
//! These tests run without a live stack (pure file reads + metric logic).
//! They fail fast when:
//!   1. A golden CSV is broken or has zero rows.
//!   2. The baseline JSON is edited to report sub-floor numbers.
//!
//! For the *live* eval run that regenerates metrics against a running
//! cortex-api, set `CORTEX_EVAL_IT=1`:
//!   cargo test -p cortex-eval --test golden_set_acceptance -- --nocapture
//!
//! Live path requires `CORTEX_API_URL` (default http://127.0.0.1:17000).

use std::path::PathBuf;

use cortex_eval::suite::{
    consolidation::{
        load_csv as load_consolidation, ENTITY_RECALL_FLOOR, FACT_RECALL_FLOOR,
    },
    retrieval::{load_csv as load_retrieval, MRR_AT_10_FLOOR, RECALL_AT_5_FLOOR},
};

fn manifest_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn baseline() -> serde_json::Value {
    let path = manifest_dir().join("baselines/cdc-baseline-v1.json");
    let raw = std::fs::read_to_string(&path).expect("read cdc-baseline-v1.json");
    serde_json::from_str(&raw).expect("parse baseline json")
}

fn metric_value(suite: &serde_json::Value, name: &str) -> f64 {
    suite["metrics"]
        .as_array()
        .expect("metrics array")
        .iter()
        .find(|m| m["name"] == name)
        .and_then(|m| m["value"].as_f64())
        .unwrap_or_else(|| panic!("metric {name} missing from baseline"))
}

// ── Golden CSV structure ──────────────────────────────────────────────────────

#[test]
fn retrieval_golden_csv_is_well_formed() {
    let p = manifest_dir().join("tests/golden/retrieval.csv");
    let rows = load_retrieval(&p).expect("load retrieval golden csv");
    assert!(!rows.is_empty(), "retrieval.csv has no rows");
    for row in &rows {
        assert!(!row.id.is_empty(), "row missing id");
        assert!(!row.query.is_empty(), "row {} missing query", row.id);
        assert!(
            !row.expected_paths().is_empty(),
            "row {} has no expected paths",
            row.id
        );
    }
}

#[test]
fn consolidation_golden_csv_is_well_formed() {
    let p = manifest_dir().join("tests/golden/consolidation.csv");
    let rows = load_consolidation(&p).expect("load consolidation golden csv");
    assert!(!rows.is_empty(), "consolidation.csv has no rows");
    for row in &rows {
        assert!(!row.id.is_empty(), "row missing id");
        assert!(
            !row.consolidation_id.is_empty(),
            "row {} missing consolidation_id",
            row.id
        );
        assert!(
            !row.expected_entity_list().is_empty(),
            "row {} has no expected entities",
            row.id
        );
    }
}

// ── Baseline acceptance gates (regression pin) ────────────────────────────────

#[test]
fn baseline_retrieval_reranked_meets_mrr_floor() {
    let b = baseline();
    let mrr = metric_value(&b["retrieval_reranked"], "mrr_at_10");
    assert!(
        mrr >= MRR_AT_10_FLOOR,
        "reranked MRR@10 {mrr:.4} < floor {MRR_AT_10_FLOOR}"
    );
}

#[test]
fn baseline_retrieval_reranked_meets_recall_floor() {
    let b = baseline();
    let recall = metric_value(&b["retrieval_reranked"], "recall_at_5");
    assert!(
        recall >= RECALL_AT_5_FLOOR,
        "reranked recall@5 {recall:.4} < floor {RECALL_AT_5_FLOOR}"
    );
}

#[test]
fn baseline_consolidation_meets_entity_recall_floor() {
    let b = baseline();
    let er = metric_value(&b["consolidation"], "entity_recall");
    assert!(
        er >= ENTITY_RECALL_FLOOR,
        "consolidation entity_recall {er:.4} < floor {ENTITY_RECALL_FLOOR}"
    );
}

#[test]
fn baseline_consolidation_meets_fact_recall_floor() {
    let b = baseline();
    let fr = metric_value(&b["consolidation"], "fact_recall");
    assert!(
        fr >= FACT_RECALL_FLOOR,
        "consolidation fact_recall {fr:.4} < floor {FACT_RECALL_FLOOR}"
    );
}

// ── Arm gates (IT-pinned) ─────────────────────────────────────────────────────

#[test]
fn temporal_arm_it_pinned_in_baseline() {
    let b = baseline();
    let status = b["temporal_arm"]["status"]
        .as_str()
        .expect("temporal_arm.status missing");
    assert_eq!(
        status, "it_pinned",
        "temporal arm must be it_pinned in baseline"
    );
    let result = b["temporal_arm"]["it_result"]
        .as_str()
        .expect("temporal_arm.it_result missing");
    assert!(
        result.contains("4/4"),
        "expected 4/4 in temporal_arm.it_result, got {result}"
    );
}

#[test]
fn cross_project_arm_it_pinned_in_baseline() {
    let b = baseline();
    let status = b["cross_project_arm"]["status"]
        .as_str()
        .expect("cross_project_arm.status missing");
    assert_eq!(
        status, "it_pinned",
        "cross-project arm must be it_pinned in baseline"
    );
    let result = b["cross_project_arm"]["it_result"]
        .as_str()
        .expect("cross_project_arm.it_result missing");
    assert!(
        result.contains("4/4"),
        "expected 4/4 in cross_project_arm.it_result, got {result}"
    );
}

// ── Live eval (requires CORTEX_EVAL_IT=1 + running stack) ────────────────────

#[tokio::test]
async fn live_retrieval_suite_meets_reranked_gates() {
    if std::env::var("CORTEX_EVAL_IT").is_err() {
        return;
    }
    let api_url = std::env::var("CORTEX_API_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:17000".into());
    let golden_path = manifest_dir().join("tests/golden/retrieval.csv");
    let rows = load_retrieval(&golden_path).expect("load retrieval golden csv");

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap();

    let mut observed: Vec<Vec<String>> = Vec::with_capacity(rows.len());
    for row in &rows {
        // Match the live /v1/query contract: `intent` is required and `repo`
        // lives under `scope` (not as a top-level key).
        let body = serde_json::json!({
            "query": row.query,
            "intent": "free_search",
            "scope": { "repo": row.repo },
            "limit": 10,
        });
        let resp = match client
            .post(format!("{api_url}/v1/query"))
            .json(&body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                eprintln!("row {}: HTTP error: {e}", row.id);
                observed.push(Vec::new());
                continue;
            }
        };
        if !resp.status().is_success() {
            eprintln!("row {}: non-2xx status {}", row.id, resp.status());
            observed.push(Vec::new());
            continue;
        }
        let json: serde_json::Value = resp.json().await.unwrap_or(serde_json::Value::Null);
        let paths: Vec<String> = json
            .get("results")
            .and_then(|r| r.get("snippets"))
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|s| s.get("path").and_then(|p| p.as_str()).map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        observed.push(paths);
    }

    let report = cortex_eval::suite::retrieval::build_report(&rows, &observed);
    let verdict = cortex_eval::retrieval_acceptance(&report);
    assert!(
        verdict.passed,
        "live retrieval suite failed gates: {:?}\nreport: {:#}",
        verdict.failed_metrics,
        serde_json::to_string_pretty(&serde_json::json!({
            "metrics": report.metrics
        }))
        .unwrap()
    );
}
