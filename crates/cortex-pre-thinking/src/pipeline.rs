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
use cortex_api::{Intent, Notice, QueryRequest, QueryResponse};

use crate::breaker::{Breaker, BreakerState, FailReason};
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
    /// Structural diagnostic forwarded verbatim from
    /// `QueryResponse.notice` — the MCP shim turns this into a
    /// distinct soft-error reason (e.g. `repo_not_indexed`) so the
    /// caller can disambiguate from a generic empty bundle. See
    /// issue hivellm/cortex#1.
    pub notice: Option<Notice>,
    /// Phase14e — breaker state observed when the run completed.
    /// `None` for legacy callers using [`run`] without a shared
    /// breaker.
    pub breaker_state: Option<BreakerState>,
}

/// Phase14e sentinel embedded in the bundle on every fail-open
/// dispatch. Distinguishes an empty bundle caused by an outage
/// (`<!-- cortex: timeout reason=… -->`) from a bundle that was
/// legitimately empty because no context matched. Operators grep
/// transcripts for the marker when triaging silent agent drift.
pub const FAIL_OPEN_SENTINEL_PREFIX: &str = "<!-- cortex: timeout reason=";

/// Build the fail-open bundle sentinel. Format is intentionally
/// HTML-comment shaped so it survives markdown rendering without
/// being treated as content.
pub fn fail_open_sentinel(reason: FailReason, query_id: Option<&str>) -> String {
    match query_id {
        Some(id) if !id.is_empty() => {
            format!(
                "{FAIL_OPEN_SENTINEL_PREFIX}{} query_id={id} -->",
                reason.as_str()
            )
        }
        _ => format!("{FAIL_OPEN_SENTINEL_PREFIX}{} -->", reason.as_str()),
    }
}

/// Run the spec-12 pipeline (no breaker — kept for callers that
/// have not migrated yet). Delegates to [`run_with_breaker`] with
/// a fresh local breaker so behaviour is identical to a
/// breaker-aware caller that never sees a failure.
pub async fn run<Q: QueryFn>(
    input: &PreThinkingInput<'_>,
    query_fn: Arc<Q>,
    metrics: Arc<Metrics>,
) -> PreThinkingOutput {
    let breaker = Arc::new(Breaker::new());
    run_with_breaker(input, query_fn, metrics, breaker).await
}

/// Phase14e — run the pipeline through a shared circuit breaker.
/// Production callers (the Claude Code adapter, MCP shim) build
/// one `Arc<Breaker>` at boot and reuse it across calls so the
/// breaker observes the system-wide failure tally.
pub async fn run_with_breaker<Q: QueryFn>(
    input: &PreThinkingInput<'_>,
    query_fn: Arc<Q>,
    metrics: Arc<Metrics>,
    breaker: Arc<Breaker>,
) -> PreThinkingOutput {
    let started = Instant::now();
    let derived = derive_scope(input.user_prompt, input.cwd, input.recent_files);
    let intent = select_intent(input.user_prompt);
    metrics.incr_calls(intent.label());

    // Breaker guard — Open ⇒ instant fail-open. The
    // `breaker_open` reason carries no query_id (no upstream call
    // happened).
    let permit = match breaker.guard() {
        Ok(p) => p,
        Err(_) => {
            metrics.incr_fail_open(FailReason::BreakerOpen.as_str());
            tracing::warn!(
                session_id = %input.session_id,
                turn_id = %input.turn_id,
                intent = %intent.label(),
                reason = "breaker_open",
                "pre-thinking fail-open short-circuited by breaker"
            );
            return PreThinkingOutput {
                bundle: fail_open_sentinel(FailReason::BreakerOpen, None),
                intent,
                query_id: None,
                steps_applied: Vec::new(),
                latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                fail_open: true,
                notice: None,
                breaker_state: Some(BreakerState::Open),
            };
        }
    };

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
        // Pre-thinking does its own downstream byte clipping via
        // `cortex_pre_thinking::budget::clip_to_budget`, so we let
        // the API-side clipper run with its default cap (32 KiB) —
        // the pre-thinking trim ladder runs after this and tightens
        // further if `bundle_bytes` is smaller.
        budget_bytes: None,
        as_of: None,
        branch: None,
        projects: None,
        include_history: None,
        include_future: None,
        include_branches: None,
    };

    let total_budget = Duration::from_millis(input.budget.time_ms.max(1) as u64);
    let (resp_opt, reason) = match tokio::time::timeout(total_budget, query_fn.query(req)).await {
        Ok(opt @ Some(_)) => (opt, None),
        Ok(None) => (None, Some(FailReason::Internal)),
        Err(_) => {
            metrics.incr_timeouts();
            (None, Some(FailReason::Timeout))
        }
    };
    let response = match resp_opt {
        Some(r) => {
            // Successful upstream response — half-open probes
            // close the breaker; closed-state successes are no-ops.
            permit.record_success();
            r
        }
        None => {
            let r = reason.unwrap_or(FailReason::Internal);
            metrics.incr_fail_open(r.as_str());
            let new_state = permit.record_failure();
            tracing::warn!(
                session_id = %input.session_id,
                turn_id = %input.turn_id,
                intent = %intent.label(),
                reason = r.as_str(),
                new_state = ?new_state,
                "pre-thinking fail-open dispatched"
            );
            return PreThinkingOutput {
                bundle: fail_open_sentinel(r, None),
                intent,
                query_id: None,
                steps_applied: Vec::new(),
                latency_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                fail_open: true,
                notice: None,
                breaker_state: Some(breaker.state()),
            };
        }
    };
    let notice = response.notice.clone();

    let clipped: ClippedBundle = clip_to_budget(
        intent.label(),
        &response,
        input.budget.bundle_bytes as usize,
    );

    if clipped.bundle.is_empty() {
        metrics.incr_empty_bundle();
    } else {
        metrics.observe_bundle_bytes(clipped.bytes as u32);
        // Phase14f §4.1 — per-intent histogram sample so the
        // dashboard can render p50/p95/p99 per intent.
        metrics.observe_bundle_bytes_per_intent(intent.label(), clipped.bytes as u32);
    }
    for step in &clipped.steps {
        metrics.incr_truncation_step(*step);
    }
    metrics.observe_section_count("laws", response.laws_active.len() as u32);
    metrics.observe_section_count("decisions", response.results.decisions.len() as u32);
    metrics.observe_section_count("similar_turns", response.results.similar_turns.len() as u32);
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
        notice,
        breaker_state: Some(breaker.state()),
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::breaker::BreakerConfig;
    use std::path::PathBuf;
    use std::time::Duration as StdDuration;

    /// QueryFn that always returns `None` — drives the timeout
    /// fail-open path in tests without sleeping the full budget.
    struct AlwaysNone;
    #[async_trait]
    impl QueryFn for AlwaysNone {
        async fn query(&self, _req: QueryRequest) -> Option<QueryResponse> {
            None
        }
    }

    fn input<'a>(cwd: &'a PathBuf) -> PreThinkingInput<'a> {
        PreThinkingInput {
            session_id: "01SESS",
            turn_id: "01TURN",
            user_prompt: "investigate failing auth tests",
            cwd,
            recent_files: &[],
            budget: PreThinkingBudget {
                bundle_bytes: 4 * 1024,
                time_ms: 50,
            },
        }
    }

    #[tokio::test]
    async fn fail_open_bundle_carries_sentinel_with_reason() {
        let cwd = std::env::current_dir().unwrap();
        let metrics = Arc::new(Metrics::new());
        let breaker = Arc::new(Breaker::with_config(BreakerConfig::default()));
        let out =
            run_with_breaker(&input(&cwd), Arc::new(AlwaysNone), metrics.clone(), breaker).await;
        assert!(out.fail_open);
        assert!(
            out.bundle.starts_with(FAIL_OPEN_SENTINEL_PREFIX),
            "bundle must carry sentinel; got {:?}",
            out.bundle
        );
        let snap = metrics.fail_open_snapshot();
        assert!(snap.values().sum::<u64>() >= 1, "fail_open counter bumped");
    }

    #[tokio::test]
    async fn burst_fails_trip_breaker_and_short_circuit_subsequent_calls() {
        let cwd = std::env::current_dir().unwrap();
        let metrics = Arc::new(Metrics::new());
        let breaker = Arc::new(Breaker::with_config(BreakerConfig {
            threshold: 3,
            window: StdDuration::from_secs(60),
            cooldown: StdDuration::from_secs(30),
        }));
        // 3 failures should trip the breaker.
        for _ in 0..3 {
            let _ = run_with_breaker(
                &input(&cwd),
                Arc::new(AlwaysNone),
                metrics.clone(),
                breaker.clone(),
            )
            .await;
        }
        assert_eq!(breaker.state(), BreakerState::Open);
        // 4th call: short-circuit, reason=breaker_open, sentinel
        // present, NO upstream attempt.
        let out =
            run_with_breaker(&input(&cwd), Arc::new(AlwaysNone), metrics.clone(), breaker).await;
        assert!(out.fail_open);
        assert!(out.bundle.contains("breaker_open"));
        let snap = metrics.fail_open_snapshot();
        assert!(snap.get("breaker_open").copied().unwrap_or(0) >= 1);
    }

    #[test]
    fn fail_open_sentinel_format_is_stable() {
        let s = fail_open_sentinel(FailReason::Timeout, Some("q-1"));
        assert_eq!(s, "<!-- cortex: timeout reason=timeout query_id=q-1 -->");
        let s2 = fail_open_sentinel(FailReason::BreakerOpen, None);
        assert_eq!(s2, "<!-- cortex: timeout reason=breaker_open -->");
    }
}
