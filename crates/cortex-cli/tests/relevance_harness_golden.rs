//! Integration test — drives [`run_harness`] end-to-end against a
//! deterministic in-memory fetcher and asserts the emitted report
//! matches a stable golden shape. Verifies F-008 contract: per-intent
//! buckets aggregate, global recall + MRR are computed, and id
//! ordering is stable across runs.

use std::collections::BTreeMap;

use cortex_api::types::{
    BudgetReport, DebugInfo, Intent, QueryResponse, ResultsBag, Scope, Snippet,
};
use cortex_cli::relevance_eval::{
    harness::{run_harness, HarnessOptions, ScoredQuery, SnippetFetcher, StatusSnapshot},
    queries::{LabeledQuery, QuerySet},
    report::IntentScores,
};

fn snip(rank: usize, path: &str) -> Snippet {
    Snippet {
        rank,
        source: "vector".into(),
        collection: Some("cortex-Cortex-code".into()),
        repo: Some("Cortex".into()),
        path: Some(path.into()),
        symbol: None,
        content_hash: None,
        text: format!("snippet {path}"),
        score: 1.0 / rank as f64,
        why: None,
    }
}

fn lq(id: &str, intent: Intent, query: &str, expected: &[&str]) -> LabeledQuery {
    LabeledQuery {
        id: id.into(),
        intent,
        scope: Scope {
            repo: Some("Cortex".into()),
            ..Scope::default()
        },
        query: query.into(),
        expected_doc_ids: expected.iter().map(|s| (*s).to_string()).collect(),
        notes: None,
        include: None,
    }
}

fn response(snippets: Vec<Snippet>) -> QueryResponse {
    QueryResponse {
        intent: "explain".into(),
        query_id: "01TEST".into(),
        scope_resolved: Scope::default(),
        results: ResultsBag {
            snippets,
            ..Default::default()
        },
        laws_active: Vec::new(),
        budget: BudgetReport {
            used_ms: 0,
            cap_ms: 500,
            cache: "miss".into(),
        },
        debug: DebugInfo::default(),
        notice: None,
    }
}

struct FakeFetcher {
    per_query: BTreeMap<String, QueryResponse>,
}

#[async_trait::async_trait]
impl SnippetFetcher for FakeFetcher {
    async fn fetch(
        &self,
        query: &LabeledQuery,
    ) -> anyhow::Result<QueryResponse> {
        self.per_query
            .get(&query.id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no fake response for {}", query.id))
    }
    async fn status_snapshot(&self) -> StatusSnapshot {
        StatusSnapshot {
            indexed_repos: ["Cortex".to_string()].into_iter().collect(),
            api_version: Some("0.1.0".into()),
            unreachable: false,
        }
    }
}

#[tokio::test]
async fn end_to_end_matches_golden_shape() {
    // Three queries: two intents, one miss, one perfect-rank-1 hit,
    // one rank-3 hit. Locks the report shape across runs.
    let mut per_query = BTreeMap::new();
    per_query.insert(
        "rel-001".to_string(),
        response(vec![snip(1, "crates/cortex-api/src/strategies.rs")]),
    );
    per_query.insert(
        "rel-002".to_string(),
        response(vec![
            snip(1, "decoy-1.rs"),
            snip(2, "decoy-2.rs"),
            snip(3, "crates/cortex-api/src/fusion.rs"),
        ]),
    );
    per_query.insert(
        "rel-003".to_string(),
        response(vec![snip(1, "irrelevant.rs")]),
    );

    let fetcher = FakeFetcher { per_query };

    let set = QuerySet {
        version: 1,
        queries: vec![
            lq(
                "rel-001",
                Intent::PreChangeContext,
                "scope.repo defaulting to None in the query strategies",
                &["crates/cortex-api/src/strategies.rs"],
            ),
            lq(
                "rel-002",
                Intent::Explain,
                "how does the rrf fusion blend lane scores",
                &["crates/cortex-api/src/fusion.rs"],
            ),
            lq(
                "rel-003",
                Intent::Explain,
                "what does the analyzer module do",
                &["crates/cortex-api/src/analyzer.rs"],
            ),
        ],
    };

    let report = run_harness(&fetcher, &set, &HarnessOptions::default(), "fixed-sha")
        .await
        .expect("harness ran");

    // Header invariants
    assert_eq!(report.git_sha, "fixed-sha");
    assert_eq!(report.api_version.as_deref(), Some("0.1.0"));
    assert!(report.omitted_intents.is_empty());

    // Global aggregate: 2 hits / 3 = 66.67%, MRR = (1.0 + 1/3 + 0.0)/3
    assert_eq!(report.global.total, 3);
    assert_eq!(report.global.matches, 2);
    assert!(
        (report.global.recall_at_10_pct - 66.6666).abs() < 0.01,
        "got {}",
        report.global.recall_at_10_pct
    );
    let expected_mrr = (1.0 + (1.0 / 3.0) + 0.0) / 3.0;
    assert!(
        (report.global.mrr_avg - expected_mrr).abs() < 1e-9,
        "got {} expected {}",
        report.global.mrr_avg,
        expected_mrr
    );

    // Per-intent buckets
    let pcc: &IntentScores = report
        .per_intent
        .get("pre_change_context")
        .expect("pre_change_context bucket");
    assert_eq!(pcc.total, 1);
    assert_eq!(pcc.matches, 1);
    let exp = report.per_intent.get("explain").expect("explain bucket");
    assert_eq!(exp.total, 2);
    assert_eq!(exp.matches, 1);

    // Stable id ordering
    let ids: Vec<&str> = report.queries.iter().map(|q| q.id.as_str()).collect();
    assert_eq!(ids, vec!["rel-001", "rel-002", "rel-003"]);

    // Per-query specifics
    let r2 = report.queries.iter().find(|q| q.id == "rel-002").unwrap();
    assert!(r2.recall_at_10);
    assert_eq!(r2.matched_rank, Some(3));
    assert!((r2.mrr - (1.0 / 3.0)).abs() < 1e-9);
    assert_eq!(
        r2.matched_doc_id.as_deref(),
        Some("crates/cortex-api/src/fusion.rs")
    );

    let r3 = report.queries.iter().find(|q| q.id == "rel-003").unwrap();
    assert!(!r3.recall_at_10);
    assert_eq!(r3.mrr, 0.0);
}

#[tokio::test]
async fn omits_intents_when_scope_repo_not_in_indexed_snapshot() {
    // Status reports only repo "Other", so the rel-Cortex-scoped
    // query must end up in omitted_intents and the run otherwise
    // succeeds with a zero-row report bucket.
    struct OtherRepoFetcher;
    #[async_trait::async_trait]
    impl SnippetFetcher for OtherRepoFetcher {
        async fn fetch(
            &self,
            _q: &LabeledQuery,
        ) -> anyhow::Result<QueryResponse> {
            unreachable!("must be omitted before fetch");
        }
        async fn status_snapshot(&self) -> StatusSnapshot {
            StatusSnapshot {
                indexed_repos: ["Other".to_string()].into_iter().collect(),
                api_version: None,
                unreachable: false,
            }
        }
    }

    let set = QuerySet {
        version: 1,
        queries: vec![lq(
            "rel-001",
            Intent::Explain,
            "anything",
            &["whatever"],
        )],
    };
    let report = run_harness(
        &OtherRepoFetcher,
        &set,
        &HarnessOptions::default(),
        "fixed-sha",
    )
    .await
    .unwrap();
    assert_eq!(report.omitted_intents, vec!["explain".to_string()]);
    assert_eq!(report.global.total, 0);
}

#[tokio::test]
async fn unreachable_status_records_full_omission() {
    struct DownFetcher;
    #[async_trait::async_trait]
    impl SnippetFetcher for DownFetcher {
        async fn fetch(
            &self,
            _q: &LabeledQuery,
        ) -> anyhow::Result<QueryResponse> {
            unreachable!("must be omitted before fetch");
        }
        async fn status_snapshot(&self) -> StatusSnapshot {
            StatusSnapshot {
                unreachable: true,
                ..Default::default()
            }
        }
    }
    let set = QuerySet {
        version: 1,
        queries: vec![lq("rel-001", Intent::Explain, "x", &["y"])],
    };
    let report = run_harness(
        &DownFetcher,
        &set,
        &HarnessOptions::default(),
        "fixed-sha",
    )
    .await
    .unwrap();
    assert!(report.omitted_intents.contains(&"explain".to_string()));
    assert!(report.queries.is_empty());
    assert_eq!(report.global.total, 0);
}

// silences unused-import warning when filtering test scope
#[allow(dead_code)]
fn _touch_scored(_s: ScoredQuery) {}
