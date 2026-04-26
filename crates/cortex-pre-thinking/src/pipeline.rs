//! Pre-thinking pipeline — orchestrates scope → intent → query →
//! format → clip with fail-open semantics. Spec 12 §Pipeline.
//!
//! The query call is delegated through a [`QueryFn`] callable so the
//! pipeline stays decoupled from the live `cortex-api` HTTP client.
//! The Claude Code adapter wires its `SyncClient::pre_thinking`
//! into the callable; tests inject canned responses without
//! standing up a server.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use cortex_api::{Intent, QueryRequest, QueryResponse};

use crate::budget::{clip_to_budget, ClippedBundle, TrimStep};
use crate::intent_select::select as select_intent;
use crate::metrics::Metrics;
use crate::scope::{derive as derive_scope, RecentFile};

/// Caller-controlled budget passed in from the adapter. Mirrors the
/// spec-12 `PreThinkingBudget` struct.
#[derive(Debug, Clone, Copy)]
pub struct PreThinkingBudget {
    /// Maximum bundle size in bytes.
    pub bundle_bytes: u32,
    /// Maximum end-to-end wall-clock budget in milliseconds.
    pub time_ms: u32,
}

impl PreThinkingBudget {
    /// Spec-12 defaults: 32 KB, 600 ms.
    pub const fn default_for_spec_12() -> Self {
        Self {
            bundle_bytes: 32 * 1024,
            time_ms: 600,
        }
    }
}

/// Adapter-supplied input.
#[derive(Debug, Clone)]
pub struct PreThinkingInput<'a> {
    /// Session id for the audit envelope.
    pub session_id: &'a str,
    /// Active turn id for the audit envelope.
    pub turn_id: &'a str,
    /// Verbatim user prompt.
    pub user_prompt: &'a str,
    /// Working directory at the time the hook fired.
    pub cwd: &'a Path,
    /// Recent files from `git status` (TTL-cached upstream).
    pub recent_files: &'a [RecentFile],
    /// Budget knobs.
    pub budget: PreThinkingBudget,
}

/// Callable surface so the pipeline doesn't depend on a particular
/// transport.
#[async_trait]
pub trait QueryFn: Send + Sync {
    /// Issue a query and return the response. Implementations MUST
    /// return their fail-open default (`Ok(empty_response)`) on
    /// timeout / network failure — the pipeline never re-implements
    /// fail-open here.
    async fn query(&self, req: QueryRequest) -> Option<QueryResponse>;
}

/// Adapter wrapper for an `async fn` closure.
pub struct ClosureQueryFn<F>(pub F);

#[async_trait]
impl<F, Fut> QueryFn for ClosureQueryFn<F>
where
    F: Fn(QueryRequest) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = Option<QueryResponse>> + Send,
{
    async fn query(&self, req: QueryRequest) -> Option<QueryResponse> {
        (self.0)(req).await
    }
}

/// Outcome of running the pre-thinking pipeline.
#[derive(Debug, Clone)]
pub struct PreThinkingOutput {
    /// Markdown bundle ready to embed as `additionalContext`. Empty
    /// when the bundle would be empty per spec 12 Decision 4.
    pub bundle: String,
    /// Resolved intent.
    pub intent: Intent,
    /// `query_id` echoed from the API response for audit
    /// correlation. `None` when the API call failed.
    pub query_id: Option<String>,
    /// Trim steps the budget clipper applied.
    pub steps_applied: Vec<TrimStep>,
    /// Wall-clock latency.
    pub latency_ms: u64,
    /// `true` when the pipeline took the fail-open path (timeout /
    /// API error / panic).
    pub fail_open: bool,
}

/// Run the spec-12 pipeline.
pub async fn run<Q: QueryFn>(
    input: &PreThinkingInput<'_>,
    query_fn: Arc<Q>,
    metrics: Arc<Metrics>,
) -> PreThinkingOutput {
    let started = Instant::now();
    let derived = derive_scope(input.user_prompt, input.cwd, input.recent_files);
    let intent = select_intent(input.user_prompt);
    metrics.incr_calls(intent.label());

    let req = QueryRequest {
        intent,
        scope: derived.scope.clone(),
        query: input.user_prompt.to_string(),
        limit: 20,
        k: 50,
        include: vec![
            cortex_api::IncludeField::Snippets,
            cortex_api::IncludeField::Decisions,
            cortex_api::IncludeField::Violations,
            cortex_api::IncludeField::SimilarTurns,
        ],
        budget_ms: input.budget.time_ms.max(1) as u64,
    };

    let total_budget = Duration::from_millis(input.budget.time_ms.max(1) as u64);
    let resp_opt = match tokio::time::timeout(total_budget, query_fn.query(req)).await {
        Ok(opt) => opt,
        Err(_) => {
            metrics.incr_timeouts();
            None
        }
    };
    let response = match resp_opt {
        Some(r) => r,
        None => {
            return PreThinkingOutput {
                bundle: String::new(),
                intent,
                query_id: None,
                steps_applied: Vec::new(),
                latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                fail_open: true,
            };
        }
    };

    let clipped: ClippedBundle = clip_to_budget(
        intent.label(),
        &response,
        input.budget.bundle_bytes as usize,
    );

    if clipped.bundle.is_empty() {
        metrics.incr_empty_bundle();
    } else {
        metrics.observe_bundle_bytes(clipped.bytes as u32);
    }
    for step in &clipped.steps {
        metrics.incr_truncation_step(*step);
    }
    metrics.observe_section_count("laws", response.laws_active.len() as u32);
    metrics.observe_section_count("decisions", response.results.decisions.len() as u32);
    metrics.observe_section_count(
        "similar_turns",
        response.results.similar_turns.len() as u32,
    );
    metrics.observe_section_count("snippets", response.results.snippets.len() as u32);
    metrics.observe_section_count(
        "graph_neighbors",
        response.results.graph_neighbors.len() as u32,
    );

    let latency_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    metrics.observe_latency_ms(latency_ms);

    tracing::info!(
        session_id = %input.session_id,
        turn_id = %input.turn_id,
        intent = %intent.label(),
        query_id = %response.query_id,
        bundle_bytes = clipped.bytes,
        sections = response_section_summary(&response),
        steps = ?clipped.steps,
        latency_ms = latency_ms,
        "pre-thinking bundle assembled"
    );

    PreThinkingOutput {
        bundle: clipped.bundle,
        intent,
        query_id: Some(response.query_id),
        steps_applied: clipped.steps,
        latency_ms,
        fail_open: false,
    }
}

fn response_section_summary(r: &QueryResponse) -> String {
    format!(
        "laws={} decisions={} turns={} snippets={} graph={}",
        r.laws_active.len(),
        r.results.decisions.len(),
        r.results.similar_turns.len(),
        r.results.snippets.len(),
        r.results.graph_neighbors.len(),
    )
}
