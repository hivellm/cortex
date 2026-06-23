//! Phase21 §5.6 — pre-thinking bundle ACL filter integration tests.
//!
//! Drives `run_with_breaker` with a `CannedQuery` that returns snippets at
//! mixed classification levels and compartments. Verifies that only snippets
//! the principal may read (per Bell-LaPadula) reach the assembled bundle,
//! and that a caller without a principal sees all snippets (backward compat).

use std::sync::Arc;

use async_trait::async_trait;
use cortex_api::{
    BudgetReport, DebugInfo, Intent, LaneTimings, QueryRequest, QueryResponse, ResultsBag, Scope,
    Snippet,
};
use cortex_pre_thinking::{
    breaker::Breaker, pipeline::run_with_breaker, Metrics, PreThinkingBudget, PreThinkingInput,
    QueryFn,
};
use cortex_api::Principal;

struct CannedQuery(QueryResponse);

#[async_trait]
impl QueryFn for CannedQuery {
    async fn query(&self, _req: QueryRequest) -> Option<QueryResponse> {
        Some(self.0.clone())
    }
}

fn snippet_at(id: &str, level: u8, compartments: &[&str]) -> Snippet {
    Snippet {
        rank: 1,
        source: "keyword".into(),
        collection: None,
        repo: None,
        path: Some(format!("{id}.rs")),
        symbol: None,
        content_hash: None,
        text: id.to_string(),
        body_truncated: false,
        score: 1.0,
        why: None,
        verified: None,
        verdict: None,
        class_level: if level == 0 { None } else { Some(level) },
        class_compartments: compartments.iter().map(|s| s.to_string()).collect(),
    }
}

fn response_with(snippets: Vec<Snippet>) -> QueryResponse {
    QueryResponse {
        intent: Intent::PreChangeContext.label().to_string(),
        query_id: "01HTEST".into(),
        scope_resolved: Scope::default(),
        results: ResultsBag {
            snippets,
            ..ResultsBag::default()
        },
        laws_active: vec![],
        budget: BudgetReport {
            used_ms: 0,
            cap_ms: 600,
            cache: "miss".into(),
        },
        debug: DebugInfo {
            lanes: LaneTimings::default(),
            errors: Default::default(),
            truncated: false,
            notes: Vec::new(),
        },
        notice: None,
        clipped: None,
    }
}

fn input<'a>(
    cwd: &'a std::path::Path,
    principal: Option<Principal>,
) -> PreThinkingInput<'a> {
    PreThinkingInput {
        session_id: "s",
        turn_id: "t",
        user_prompt: "investigate the auth module",
        cwd,
        recent_files: &[],
        budget: PreThinkingBudget {
            bundle_bytes: 64 * 1024,
            time_ms: 500,
        },
        principal,
    }
}

/// Phase21 §5.6 — a principal with clearance=1 must only see `public`
/// (level=0) and `internal` (level=1) snippets in the bundle.
#[tokio::test]
async fn bundle_filter_drops_snippets_above_clearance() {
    let cwd = std::env::temp_dir();
    let snippets = vec![
        snippet_at("public-doc", 0, &[]),
        snippet_at("internal-doc", 1, &[]),
        snippet_at("confidential-doc", 2, &[]),
        snippet_at("restricted-doc", 3, &[]),
    ];
    let query_fn = Arc::new(CannedQuery(response_with(snippets)));
    let principal = Principal {
        id: "low-clearance".into(),
        clearance_level: 1,
        compartment_grants: vec![],
        roles: vec![],
    };
    let out = run_with_breaker(
        &input(&cwd, Some(principal)),
        query_fn,
        Arc::new(Metrics::new()),
        Arc::new(Breaker::new()),
    )
    .await;
    assert!(!out.fail_open, "must not be fail-open");
    assert!(
        out.bundle.contains("public-doc"),
        "public must be in bundle; got: {:?}",
        &out.bundle[..out.bundle.len().min(400)]
    );
    assert!(
        out.bundle.contains("internal-doc"),
        "internal must be in bundle"
    );
    assert!(
        !out.bundle.contains("confidential-doc"),
        "confidential must be dropped"
    );
    assert!(
        !out.bundle.contains("restricted-doc"),
        "restricted must be dropped"
    );
}

/// Phase21 §5.6 — a principal without a required compartment grant must not
/// see compartment-gated snippets even when level is within clearance.
#[tokio::test]
async fn bundle_filter_drops_snippets_with_missing_compartment() {
    let cwd = std::env::temp_dir();
    let snippets = vec![
        snippet_at("no-compart", 1, &[]),
        snippet_at("financial-doc", 1, &["financial"]),
        snippet_at("hr-doc", 1, &["hr"]),
    ];
    let query_fn = Arc::new(CannedQuery(response_with(snippets)));
    let principal = Principal {
        id: "finance-analyst".into(),
        clearance_level: 2,
        compartment_grants: vec!["financial".into()],
        roles: vec![],
    };
    let out = run_with_breaker(
        &input(&cwd, Some(principal)),
        query_fn,
        Arc::new(Metrics::new()),
        Arc::new(Breaker::new()),
    )
    .await;
    assert!(!out.fail_open);
    assert!(out.bundle.contains("no-compart"), "unclassified must pass");
    assert!(
        out.bundle.contains("financial-doc"),
        "financial must pass (grant held)"
    );
    assert!(!out.bundle.contains("hr-doc"), "hr must be dropped");
}

/// Phase21 §5.6 — when no principal is supplied all snippets pass through
/// (backward compat — callers that don't set principal see full bundle).
#[tokio::test]
async fn no_principal_passes_all_snippets_through() {
    let cwd = std::env::temp_dir();
    let snippets = vec![
        snippet_at("public-doc", 0, &[]),
        snippet_at("restricted-doc", 3, &["security"]),
    ];
    let query_fn = Arc::new(CannedQuery(response_with(snippets)));
    let out = run_with_breaker(
        &input(&cwd, None),
        query_fn,
        Arc::new(Metrics::new()),
        Arc::new(Breaker::new()),
    )
    .await;
    assert!(!out.fail_open);
    assert!(
        out.bundle.contains("public-doc"),
        "public must pass without principal"
    );
    assert!(
        out.bundle.contains("restricted-doc"),
        "restricted must pass without principal (no ACL)"
    );
}
