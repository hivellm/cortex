//! Phase11i §4.5 — relevance gold-set IT.
//!
//! Drives the hand-curated `tests/fixtures/relevance-gold.json` fixture
//! against a live `cortex-api` daemon. Computes recall@10, MRR@10, and
//! NDCG@10 across 30 entries and fails when MRR@10 falls below the
//! 0.75 acceptance threshold the proposal pinned in §4.5.
//!
//! The HTTP path is gated behind `CORTEX_RELEVANCE_IT=1` so a vanilla
//! `cargo test` run on a host without a live daemon prints a one-line
//! "gate not set" notice and exits without making network calls. The
//! scoring math (`first_match_rank` + the per-query mean) is also
//! exercised by deterministic unit tests at the bottom of this file
//! so a regression in the matcher surfaces without requiring an
//! operator to flip the env var.
//!
//! Operator workflow:
//!
//! 1. Boot the live stack (`docker-compose up cortex-api meili synap …`).
//! 2. Trigger an indexer pass so the gold-set's expected docs are
//!    actually present in the corpus (`cortex-bootstrap` over the
//!    `cortex` repo + relevant siblings).
//! 3. `CORTEX_RELEVANCE_IT=1 cargo test -p cortex-api --test relevance_eval_it -- --nocapture`.
//!
//! When `CORTEX_API_URL` is set the harness honours it; otherwise it
//! defaults to `http://127.0.0.1:17000` (the spec-11 documented bind
//! address).

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use cortex_api::types::{IncludeField, Intent, QueryRequest, QueryResponse, Scope, Snippet};
use serde::Deserialize;

const FIXTURE_PATH: &str = "tests/fixtures/relevance-gold.json";
const ENV_GATE: &str = "CORTEX_RELEVANCE_IT";
const ENV_API_URL: &str = "CORTEX_API_URL";
const DEFAULT_API_URL: &str = "http://127.0.0.1:17000";
const TOP_K: usize = 10;
const MRR_THRESHOLD: f64 = 0.75;
const PER_REQUEST_BUDGET_MS: u64 = 1_500;

#[derive(Debug, Deserialize)]
struct GoldSet {
    #[allow(dead_code)]
    #[serde(default = "default_version")]
    version: u32,
    queries: Vec<GoldEntry>,
}

fn default_version() -> u32 {
    1
}

#[derive(Debug, Deserialize, Clone)]
struct GoldEntry {
    id: String,
    intent: Intent,
    query: String,
    #[serde(default)]
    scope: Scope,
    expected_doc_ids: Vec<String>,
    #[serde(default)]
    #[allow(dead_code)]
    notes: Option<String>,
}

fn load_gold() -> GoldSet {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_else(|_| ".".into());
    let path = PathBuf::from(manifest).join(FIXTURE_PATH);
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read gold fixture {}: {err}", path.display()));
    serde_json::from_str(&raw)
        .unwrap_or_else(|err| panic!("parse gold fixture {}: {err}", path.display()))
}

fn doc_id_for(s: &Snippet) -> String {
    format!(
        "{}|{}|{}",
        s.repo.as_deref().unwrap_or("_"),
        s.path.as_deref().unwrap_or("_"),
        s.content_hash.as_deref().unwrap_or("_"),
    )
}

fn snippet_match_fields(s: &Snippet) -> [Option<&str>; 5] {
    [
        s.repo.as_deref(),
        s.path.as_deref(),
        s.symbol.as_deref(),
        s.content_hash.as_deref(),
        s.collection.as_deref(),
    ]
}

/// 1-based rank of the first snippet that matches any expected id, or
/// `None` when nothing in the top-`k` matches. Mirrors
/// `cortex-cli/src/relevance_eval/harness.rs::first_match_rank` so the
/// IT and the operator-facing CLI agree on what "match" means:
/// composite id → exact field match → substring of `path` / `symbol`.
fn first_match_rank(snippets: &[Snippet], expected: &[String], k: usize) -> Option<usize> {
    let limit = snippets.len().min(k);
    for (idx, snippet) in snippets[..limit].iter().enumerate() {
        let canonical = doc_id_for(snippet);
        for exp in expected {
            if exp == &canonical {
                return Some(idx + 1);
            }
            for field in snippet_match_fields(snippet).iter().flatten() {
                if exp == field {
                    return Some(idx + 1);
                }
            }
            for field in [snippet.path.as_deref(), snippet.symbol.as_deref()]
                .iter()
                .flatten()
            {
                if !exp.is_empty() && field.contains(exp.as_str()) {
                    return Some(idx + 1);
                }
            }
        }
    }
    None
}

fn mrr(rank: Option<usize>) -> f64 {
    rank.map(|r| 1.0 / r as f64).unwrap_or(0.0)
}

/// Binary-relevance NDCG at the matched rank — the gold set has at
/// most one accepted hit per query, so DCG = 1 / log2(rank + 1) and
/// the ideal DCG (everything top-1) is 1.0 → NDCG@k = DCG itself.
fn ndcg(rank: Option<usize>) -> f64 {
    rank.map(|r| 1.0 / ((r as f64) + 1.0).log2()).unwrap_or(0.0)
}

#[derive(Debug, Default)]
struct AggregateScores {
    mrr_sum: f64,
    ndcg_sum: f64,
    hit_count: usize,
    per_intent: BTreeMap<&'static str, IntentBucket>,
}

#[derive(Debug, Default)]
struct IntentBucket {
    mrr_sum: f64,
    n: usize,
}

impl AggregateScores {
    fn record(&mut self, intent: Intent, rank: Option<usize>) {
        self.mrr_sum += mrr(rank);
        self.ndcg_sum += ndcg(rank);
        if rank.is_some() {
            self.hit_count += 1;
        }
        let bucket = self.per_intent.entry(intent.label()).or_default();
        bucket.mrr_sum += mrr(rank);
        bucket.n += 1;
    }
    fn mean_mrr(&self, n: usize) -> f64 {
        if n == 0 {
            0.0
        } else {
            self.mrr_sum / n as f64
        }
    }
    fn mean_ndcg(&self, n: usize) -> f64 {
        if n == 0 {
            0.0
        } else {
            self.ndcg_sum / n as f64
        }
    }
    fn recall(&self, n: usize) -> f64 {
        if n == 0 {
            0.0
        } else {
            self.hit_count as f64 / n as f64
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn relevance_gold_set_meets_mrr_threshold() {
    if std::env::var(ENV_GATE).ok().as_deref() != Some("1") {
        eprintln!(
            "{ENV_GATE} not set — skipping live relevance IT. \
             Set {ENV_GATE}=1 (and {ENV_API_URL} when the daemon is not on {DEFAULT_API_URL}) to run."
        );
        return;
    }
    let base = std::env::var(ENV_API_URL).unwrap_or_else(|_| DEFAULT_API_URL.into());
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(PER_REQUEST_BUDGET_MS + 1_000))
        .build()
        .expect("build reqwest client");

    let gold = load_gold();
    assert_eq!(
        gold.queries.len(),
        30,
        "phase11i §4.4 pinned the gold set at 30 entries"
    );

    let mut scores = AggregateScores::default();
    let mut failures: Vec<String> = Vec::new();

    for entry in &gold.queries {
        let req = QueryRequest {
            intent: entry.intent,
            scope: entry.scope.clone(),
            query: entry.query.clone(),
            limit: 20,
            k: 50,
            include: vec![IncludeField::Snippets],
            budget_ms: PER_REQUEST_BUDGET_MS,
            budget_bytes: None,
        };
        let url = format!("{}/v1/query", base.trim_end_matches('/'));
        let resp = client
            .post(&url)
            .header("x-cortex-caller", "cortex-relevance-it")
            .json(&req)
            .send()
            .await
            .unwrap_or_else(|err| panic!("POST {url} for {}: {err}", entry.id));
        let status = resp.status();
        let body = resp
            .text()
            .await
            .unwrap_or_else(|err| panic!("read response for {}: {err}", entry.id));
        assert!(
            status.is_success(),
            "{} returned {status}: {body}",
            entry.id
        );
        let parsed: QueryResponse = serde_json::from_str(&body)
            .unwrap_or_else(|err| panic!("parse response for {}: {err}\nbody={body}", entry.id));
        let rank = first_match_rank(&parsed.results.snippets, &entry.expected_doc_ids, TOP_K);
        if rank.is_none() {
            failures.push(format!(
                "{} (intent={}): no match in top-{TOP_K}; query={:?} expected={:?}",
                entry.id,
                entry.intent.label(),
                entry.query,
                entry.expected_doc_ids,
            ));
        }
        scores.record(entry.intent, rank);
    }

    let n = gold.queries.len();
    let mean_mrr = scores.mean_mrr(n);
    let mean_ndcg = scores.mean_ndcg(n);
    let recall = scores.recall(n);
    eprintln!(
        "relevance gold-set: n={n} recall@{TOP_K}={recall:.3} mrr@{TOP_K}={mean_mrr:.3} ndcg@{TOP_K}={mean_ndcg:.3}"
    );
    for (intent, bucket) in &scores.per_intent {
        let m = if bucket.n == 0 {
            0.0
        } else {
            bucket.mrr_sum / bucket.n as f64
        };
        eprintln!("  {intent}: mrr@{TOP_K}={m:.3} (n={})", bucket.n);
    }
    if !failures.is_empty() {
        eprintln!("misses ({}/{}):", failures.len(), n);
        for f in &failures {
            eprintln!("  - {f}");
        }
    }

    assert!(
        mean_mrr >= MRR_THRESHOLD,
        "phase11i §4.5 acceptance gate: mrr@{TOP_K}={mean_mrr:.3} below threshold {MRR_THRESHOLD}"
    );
}

#[cfg(test)]
mod scoring_math {
    use super::*;
    use std::collections::BTreeMap;

    fn mk_snippet(rank: usize, repo: &str, path: &str, symbol: Option<&str>) -> Snippet {
        Snippet {
            rank,
            source: "vector".into(),
            collection: None,
            repo: Some(repo.into()),
            path: Some(path.into()),
            symbol: symbol.map(String::from),
            content_hash: None,
            text: String::new(),
            body_truncated: false,
            score: 1.0 / rank as f64,
            why: None,
        }
    }

    #[test]
    fn first_match_returns_rank_one_for_top_hit() {
        let snippets = vec![
            mk_snippet(1, "cortex", "crates/cortex-api/src/strategies.rs", None),
            mk_snippet(2, "cortex", "crates/cortex-api/src/lanes.rs", None),
        ];
        let expected = vec!["crates/cortex-api/src/strategies.rs".to_string()];
        assert_eq!(first_match_rank(&snippets, &expected, 10), Some(1));
    }

    #[test]
    fn first_match_returns_none_when_nothing_matches() {
        let snippets = vec![mk_snippet(
            1,
            "cortex",
            "crates/cortex-api/src/lib.rs",
            None,
        )];
        let expected = vec!["crates/cortex-other/src/foo.rs".to_string()];
        assert_eq!(first_match_rank(&snippets, &expected, 10), None);
    }

    #[test]
    fn first_match_honours_substring_against_path() {
        // The matcher accepts a partial path (the curator's substring
        // shortcut) so the fixture survives chunk-hash drift.
        let snippets = vec![mk_snippet(
            1,
            "cortex",
            "crates/cortex-api/src/fusion.rs",
            None,
        )];
        let expected = vec!["fusion.rs".to_string()];
        assert_eq!(first_match_rank(&snippets, &expected, 10), Some(1));
    }

    #[test]
    fn first_match_caps_at_top_k() {
        let snippets = vec![
            mk_snippet(1, "cortex", "a.rs", None),
            mk_snippet(2, "cortex", "b.rs", None),
            mk_snippet(3, "cortex", "c.rs", None),
            mk_snippet(4, "cortex", "match.rs", None),
        ];
        let expected = vec!["match.rs".to_string()];
        assert_eq!(first_match_rank(&snippets, &expected, 3), None);
        assert_eq!(first_match_rank(&snippets, &expected, 4), Some(4));
    }

    #[test]
    fn mrr_and_ndcg_match_their_definitions() {
        assert!((mrr(Some(1)) - 1.0).abs() < f64::EPSILON);
        assert!((mrr(Some(2)) - 0.5).abs() < f64::EPSILON);
        assert!((mrr(Some(10)) - 0.1).abs() < f64::EPSILON);
        assert!((mrr(None) - 0.0).abs() < f64::EPSILON);
        // NDCG at rank 1: 1 / log2(2) = 1.0; at rank 3: 1 / log2(4) = 0.5.
        assert!((ndcg(Some(1)) - 1.0).abs() < 1e-9);
        assert!((ndcg(Some(3)) - 0.5).abs() < 1e-9);
        assert!((ndcg(None) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn aggregate_scores_average_per_intent() {
        let mut s = AggregateScores::default();
        s.record(Intent::PreChangeContext, Some(1)); // mrr=1
        s.record(Intent::PreChangeContext, Some(2)); // mrr=0.5
        s.record(Intent::DecisionLookup, None); // mrr=0
        assert!((s.mean_mrr(3) - (1.5 / 3.0)).abs() < 1e-9);
        assert!((s.recall(3) - (2.0 / 3.0)).abs() < 1e-9);
        let by: BTreeMap<&str, f64> = s
            .per_intent
            .iter()
            .map(|(k, v)| (*k, v.mrr_sum / v.n as f64))
            .collect();
        assert!((by["pre_change_context"] - 0.75).abs() < 1e-9);
        assert!((by["decision_lookup"] - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn gold_fixture_loads_and_matches_intent_breakdown() {
        // Sanity: every entry parses, ids are unique, and the
        // intent split agrees with the §4.4 commitment.
        let gold = load_gold();
        assert_eq!(gold.queries.len(), 30);
        let mut by_intent: BTreeMap<&'static str, usize> = BTreeMap::new();
        let mut seen_ids: std::collections::BTreeSet<&str> = Default::default();
        for q in &gold.queries {
            assert!(seen_ids.insert(q.id.as_str()), "duplicate id {}", q.id);
            assert!(!q.expected_doc_ids.is_empty(), "{} has no expected", q.id);
            *by_intent.entry(q.intent.label()).or_default() += 1;
        }
        assert_eq!(by_intent.get("pre_change_context").copied(), Some(10));
        assert_eq!(by_intent.get("decision_lookup").copied(), Some(5));
        assert_eq!(by_intent.get("similar_problems").copied(), Some(10));
        assert_eq!(by_intent.get("law_check").copied(), Some(3));
        assert_eq!(by_intent.get("free_search").copied(), Some(2));
    }
}
