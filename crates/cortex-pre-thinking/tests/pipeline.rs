//! Integration tests for the pre-thinking pipeline.

use std::sync::Arc;

use async_trait::async_trait;
use cortex_api::{
    BudgetReport, DebugInfo, DecisionRef, GraphNeighbor, Intent, LaneTimings, LawRef,
    QueryRequest, QueryResponse, ResultsBag, Scope, SimilarTurn, Snippet,
};
use cortex_pre_thinking::{
    pipeline::run, FileStatus, Metrics, PreThinkingBudget, PreThinkingInput, QueryFn, RecentFile,
    TrimStep,
};

struct CannedQuery {
    response: QueryResponse,
    delay: Option<std::time::Duration>,
    captured: tokio::sync::Mutex<Vec<QueryRequest>>,
}

impl CannedQuery {
    fn new(response: QueryResponse) -> Self {
        Self {
            response,
            delay: None,
            captured: tokio::sync::Mutex::new(Vec::new()),
        }
    }
    fn with_delay(mut self, d: std::time::Duration) -> Self {
        self.delay = Some(d);
        self
    }
    async fn captured(&self) -> Vec<QueryRequest> {
        self.captured.lock().await.clone()
    }
}

#[async_trait]
impl QueryFn for CannedQuery {
    async fn query(&self, req: QueryRequest) -> Option<QueryResponse> {
        if let Some(d) = self.delay {
            tokio::time::sleep(d).await;
        }
        self.captured.lock().await.push(req);
        Some(self.response.clone())
    }
}

struct AlwaysNone;

#[async_trait]
impl QueryFn for AlwaysNone {
    async fn query(&self, _req: QueryRequest) -> Option<QueryResponse> {
        None
    }
}

fn populated_response(query_id: &str) -> QueryResponse {
    QueryResponse {
        intent: "pre_change_context".into(),
        query_id: query_id.into(),
        scope_resolved: Scope::default(),
        results: ResultsBag {
            snippets: vec![Snippet {
                rank: 1,
                source: "vector".into(),
                collection: None,
                repo: Some("Vectorizer".into()),
                path: Some("src/index/hnsw/mod.rs".into()),
                symbol: Some("hnsw_search".into()),
                content_hash: None,
                text: "pub fn hnsw_search() {}".into(),
                score: 0.9,
                why: Some("vector match".into()),
            }],
            decisions: vec![DecisionRef {
                rank: 1,
                id: "DEC-0042".into(),
                title: "Adopt Meili".into(),
                status: "accepted".into(),
                ts: 1_715_000_000_000,
                score: 0.7,
                links: vec![],
            }],
            violations: vec![],
            graph_neighbors: vec![GraphNeighbor {
                from: "T".into(),
                relation: "TOUCHED".into(),
                to: "A".into(),
                hops: 1,
            }],
            similar_turns: vec![SimilarTurn {
                turn_id: "01HX".into(),
                ts: 1_715_000_000_000,
                model: "claude".into(),
                summary: "tweak ef".into(),
                score: 0.6,
            }],
        },
        laws_active: vec![LawRef {
            id: "LAW-007".into(),
            severity: "critical".into(),
            title: "no skip hooks".into(),
        }],
        budget: BudgetReport {
            used_ms: 0,
            cap_ms: 500,
            cache: "miss".into(),
        },
        debug: DebugInfo {
            lanes: LaneTimings::default(),
            errors: Default::default(),
            truncated: false,
        },
        notice: None,
    }
}

fn input_for<'a>(prompt: &'a str, cwd: &'a std::path::Path) -> PreThinkingInput<'a> {
    PreThinkingInput {
        session_id: "s",
        turn_id: "t",
        user_prompt: prompt,
        cwd,
        recent_files: &[],
        budget: PreThinkingBudget::default_for_spec_12(),
    }
}

#[tokio::test]
async fn happy_path_returns_bundle_with_query_id() {
    let canned = Arc::new(CannedQuery::new(populated_response("01HFIXED")));
    let metrics = Arc::new(Metrics::new());
    let cwd = std::env::temp_dir();
    let out = run(
        &input_for("refactor hnsw_search to take ef per call", &cwd),
        canned.clone(),
        metrics,
    )
    .await;
    assert!(!out.bundle.is_empty());
    assert_eq!(out.intent, Intent::PreChangeContext);
    assert!(out.bundle.contains("query_id=01HFIXED"));
    assert!(!out.fail_open);
    let captured = canned.captured().await;
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0].intent, Intent::PreChangeContext);
}

#[tokio::test]
async fn intent_routing_picks_decision_lookup_for_why_prompts() {
    let canned = Arc::new(CannedQuery::new(populated_response("Q1")));
    let metrics = Arc::new(Metrics::new());
    let cwd = std::env::temp_dir();
    let out = run(
        &input_for("why did we pick 128 for ef_search?", &cwd),
        canned.clone(),
        metrics,
    )
    .await;
    assert_eq!(out.intent, Intent::DecisionLookup);
    let captured = canned.captured().await;
    assert_eq!(captured[0].intent, Intent::DecisionLookup);
}

#[tokio::test]
async fn empty_response_returns_empty_bundle_and_increments_counter() {
    let mut empty = populated_response("Q2");
    empty.results = ResultsBag::default();
    empty.laws_active.clear();
    let canned = Arc::new(CannedQuery::new(empty));
    let metrics = Arc::new(Metrics::new());
    let cwd = std::env::temp_dir();
    let out = run(
        &input_for("anything", &cwd),
        canned,
        metrics.clone(),
    )
    .await;
    assert!(out.bundle.is_empty());
    assert_eq!(
        metrics
            .empty_bundle
            .load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[tokio::test]
async fn timeout_returns_empty_bundle_and_increments_timeouts() {
    let canned = Arc::new(
        CannedQuery::new(populated_response("Q3"))
            .with_delay(std::time::Duration::from_millis(800)),
    );
    let metrics = Arc::new(Metrics::new());
    let cwd = std::env::temp_dir();
    let mut input = input_for("refactor", &cwd);
    input.budget.time_ms = 200;
    let out = run(&input, canned, metrics.clone()).await;
    assert!(out.bundle.is_empty());
    assert!(out.fail_open);
    assert_eq!(
        metrics.timeouts.load(std::sync::atomic::Ordering::Relaxed),
        1
    );
}

#[tokio::test]
async fn always_none_query_fn_returns_empty_with_fail_open() {
    let metrics = Arc::new(Metrics::new());
    let cwd = std::env::temp_dir();
    let out = run(&input_for("refactor", &cwd), Arc::new(AlwaysNone), metrics).await;
    assert!(out.bundle.is_empty());
    assert!(out.fail_open);
    assert!(out.query_id.is_none());
}

#[tokio::test]
async fn deterministic_byte_for_byte_output_across_runs() {
    let canned = Arc::new(CannedQuery::new(populated_response("DETERMINISTIC")));
    let metrics = Arc::new(Metrics::new());
    let cwd = std::env::temp_dir();
    let a = run(
        &input_for("refactor hnsw_search", &cwd),
        canned.clone(),
        metrics.clone(),
    )
    .await;
    let b = run(
        &input_for("refactor hnsw_search", &cwd),
        canned,
        metrics,
    )
    .await;
    assert_eq!(a.bundle, b.bundle);
}

#[tokio::test]
async fn overflow_response_clips_under_budget_keeping_laws() {
    let mut fat = populated_response("FAT");
    // Pump snippets up so the bundle exceeds 4 KB.
    fat.results.snippets = (0..5)
        .map(|i| Snippet {
            rank: i + 1,
            source: "vector".into(),
            collection: None,
            repo: Some("R".into()),
            path: Some(format!("src/{i}.rs")),
            symbol: Some(format!("fn_{i}")),
            content_hash: None,
            text: "x".repeat(8 * 1024),
            score: 0.5,
            why: Some("why".into()),
        })
        .collect();
    let canned = Arc::new(CannedQuery::new(fat));
    let metrics = Arc::new(Metrics::new());
    let cwd = std::env::temp_dir();
    let mut input = input_for("refactor", &cwd);
    input.budget.bundle_bytes = 4 * 1024;
    let out = run(&input, canned, metrics).await;
    assert!(out.bundle.len() <= 4 * 1024);
    assert!(out.bundle.contains("LAW-007"));
    assert!(!out.steps_applied.is_empty());
}

#[tokio::test]
async fn recent_files_within_5_min_make_it_into_scope() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path().join("R");
    std::fs::create_dir_all(repo.join(".git")).unwrap();
    let canned = Arc::new(CannedQuery::new(populated_response("Q4")));
    let metrics = Arc::new(Metrics::new());
    let recent = vec![RecentFile {
        path: repo.join("src/lib.rs"),
        status: FileStatus::Modified,
        age_seconds: 30,
    }];
    let input = PreThinkingInput {
        session_id: "s",
        turn_id: "t",
        user_prompt: "tighten the parser",
        cwd: &repo,
        recent_files: &recent,
        budget: PreThinkingBudget::default_for_spec_12(),
    };
    let _ = run(&input, canned.clone(), metrics).await;
    let captured = canned.captured().await;
    assert_eq!(captured[0].scope.repo.as_deref(), Some("R"));
    assert!(captured[0].scope.files.iter().any(|f| f.contains("lib.rs")));
}

#[tokio::test]
async fn truncation_step_metric_records_each_applied_step() {
    let mut fat = populated_response("Q5");
    fat.results.snippets = (0..5)
        .map(|i| Snippet {
            rank: i + 1,
            source: "vector".into(),
            collection: None,
            repo: Some("R".into()),
            path: Some(format!("src/{i}.rs")),
            symbol: Some(format!("fn_{i}")),
            content_hash: None,
            text: "x".repeat(8 * 1024),
            score: 0.5,
            why: Some("why".into()),
        })
        .collect();
    let canned = Arc::new(CannedQuery::new(fat));
    let metrics = Arc::new(Metrics::new());
    let cwd = std::env::temp_dir();
    let mut input = input_for("refactor", &cwd);
    input.budget.bundle_bytes = 1024;
    let out = run(&input, canned, metrics.clone()).await;
    assert!(!out.steps_applied.is_empty());
    let trimmed = metrics.truncation_applied.lock().unwrap().clone();
    assert!(!trimmed.is_empty());
    let _ = TrimStep::SlimSnippets;
}
