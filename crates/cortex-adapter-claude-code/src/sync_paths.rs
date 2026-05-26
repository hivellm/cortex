//! Synchronous hook paths.
//!
//! Spec 10 §Synchronous paths: two hooks block on `cortex-api`:
//!
//! - `UserPromptSubmit` drives `cortex_pre_thinking::pipeline::run`,
//!   which POSTs a real `cortex_api::QueryRequest` to `/v1/query`,
//!   formats the response into a Markdown bundle, and clips it to the
//!   configured budget. The bundle string ships back as
//!   `additionalContext`.
//! - `PreToolUse` calls `/v1/laws/check`; critical violations route
//!   to `permissionDecision: deny`.
//!
//! Both fail-open on timeout / error: the session continues
//! unchanged.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cortex_api::{QueryRequest, QueryResponse};
use cortex_pre_thinking::pipeline::{
    run as pre_thinking_run, ClosureQueryFn, PreThinkingBudget, PreThinkingInput,
};
use cortex_pre_thinking::Metrics as PreThinkingMetrics;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::AdapterSection;
use crate::metrics::Metrics;

/// Result of the pre-thinking sync path.
#[derive(Debug, Clone, Default)]
pub struct PreThinkingResult {
    /// Markdown bundle the daemon returns as `additionalContext`.
    /// Empty on timeout, error, or when the pipeline produced no
    /// content for the active intent.
    pub bundle: String,
    /// `true` when the call was throttled by `enabled = false` or
    /// timed out and we returned the empty bundle.
    pub fail_open: bool,
}

/// Result of the law-check sync path.
#[derive(Debug, Clone, Default)]
pub struct LawCheckResult {
    /// `true` when the daemon should `permissionDecision: deny` the
    /// tool call.
    pub deny: bool,
    /// Concatenated reason returned to Claude Code.
    pub reason: String,
    /// Per-violation list captured for the async event.
    pub violations: Vec<Violation>,
    /// `true` when the call timed out / errored and we fail-opened.
    pub fail_open: bool,
}

/// Shape returned by `/v1/laws/check`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Violation {
    /// Law id (e.g. `LAW-007`).
    pub law_id: String,
    /// `info` | `notable` | `critical`.
    pub severity: String,
    /// Human-readable message.
    pub message: String,
}

/// Outbound shape of the `/v1/laws/check` body.
#[derive(Debug, Clone, Serialize)]
pub struct LawCheckRequest<'a> {
    /// Tool the model is about to invoke.
    pub tool_name: &'a str,
    /// Tool input verbatim.
    pub input: &'a Value,
    /// Session id for correlation.
    pub session_id: &'a str,
    /// Active turn id, if any.
    pub turn_id: Option<&'a str>,
}

// Pre-thinking outbound body is `cortex_api::QueryRequest`. The
// adapter does not declare its own request struct so the wire shape
// can never drift from the API contract.

/// Sync-path helper that wraps the HTTP client + the hook-budget
/// timeout knobs.
pub struct SyncClient {
    client: Client,
    api_endpoint: String,
    pre_thinking_timeout: Duration,
    pre_thinking_enabled: bool,
    pre_thinking_max_bytes: u64,
    laws_timeout: Duration,
    laws_block_on_critical: bool,
    metrics: Arc<Metrics>,
}

impl SyncClient {
    /// Build a sync client from config.
    pub fn new(cfg: &AdapterSection, metrics: Arc<Metrics>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(cfg.timeout_ms))
            .build()
            .expect("reqwest client builder");
        Self {
            client,
            api_endpoint: cfg.api_endpoint.trim_end_matches('/').to_string(),
            pre_thinking_timeout: Duration::from_millis(cfg.pre_thinking.timeout_ms),
            pre_thinking_enabled: cfg.pre_thinking.enabled,
            pre_thinking_max_bytes: cfg.pre_thinking.max_bundle_kb * 1024,
            laws_timeout: Duration::from_millis(cfg.laws.timeout_ms),
            laws_block_on_critical: cfg.laws.block_on_critical,
            metrics,
        }
    }

    /// Run the pre-thinking sync call. Always returns a value —
    /// fail-open on any error. Drives the spec-12
    /// `cortex_pre_thinking::pipeline::run` pipeline so the wire shape
    /// matches `cortex_api::QueryRequest` and the resulting Markdown
    /// bundle is already clipped to the budget.
    pub async fn pre_thinking(
        &self,
        prompt: &str,
        session_id: &str,
        turn_id: Option<&str>,
        cwd: Option<&str>,
    ) -> PreThinkingResult {
        if !self.pre_thinking_enabled {
            return PreThinkingResult {
                bundle: String::new(),
                fail_open: true,
            };
        }
        let timeout = self.pre_thinking_timeout;
        let max_bytes = self.pre_thinking_max_bytes;
        let url = format!("{}/v1/query", self.api_endpoint.trim_end_matches('/'));
        let http = self.client.clone();
        let metrics = self.metrics.clone();

        // Phase6d — re-derive the intent trigger keyword off the
        // prompt so the audit envelope can record `intent_trigger`.
        // The pipeline's own `select_intent` call drops the trigger,
        // so we duplicate the (pure-function) match here. Empty
        // string means "no rule fired"; the service treats that as
        // an absent header and the audit envelope stamps
        // `intent_trigger: null`.
        let intent_trigger = cortex_pre_thinking::intent_select::select_matched(prompt)
            .trigger
            .unwrap_or("")
            .to_string();

        let query_fn = Arc::new(ClosureQueryFn(move |req: QueryRequest| {
            let url = url.clone();
            let http = http.clone();
            let metrics = metrics.clone();
            let trigger = intent_trigger.clone();
            async move {
                let send_started = Instant::now();
                let mut http_req = http.post(&url);
                if !trigger.is_empty() {
                    http_req = http_req.header("x-cortex-intent-trigger", &trigger);
                }
                let resp = tokio::time::timeout(timeout, http_req.json(&req).send()).await;
                let latency_ms =
                    u32::try_from(send_started.elapsed().as_millis()).unwrap_or(u32::MAX);
                metrics.observe_sync_latency("UserPromptSubmit", latency_ms);
                match resp {
                    Err(_elapsed) => {
                        metrics.incr_sync_timeout("UserPromptSubmit");
                        None
                    }
                    Ok(Err(_e)) => None,
                    Ok(Ok(resp)) if !resp.status().is_success() => None,
                    Ok(Ok(resp)) => resp.json::<QueryResponse>().await.ok(),
                }
            }
        }));

        let cwd_path = cwd.map(Path::new).unwrap_or_else(|| Path::new("."));
        let budget_bytes = u32::try_from(max_bytes).unwrap_or(u32::MAX);
        let budget_ms = u32::try_from(timeout.as_millis()).unwrap_or(u32::MAX);
        let input = PreThinkingInput {
            session_id,
            turn_id: turn_id.unwrap_or(""),
            user_prompt: prompt,
            cwd: cwd_path,
            recent_files: &[],
            budget: PreThinkingBudget {
                bundle_bytes: budget_bytes,
                time_ms: budget_ms,
            },
        };

        let prethink_metrics = Arc::new(PreThinkingMetrics::new());
        let output = pre_thinking_run(&input, query_fn, prethink_metrics).await;
        let bundle_bytes = u32::try_from(output.bundle.len()).unwrap_or(u32::MAX);
        self.metrics.observe_bundle_bytes(bundle_bytes);

        PreThinkingResult {
            bundle: output.bundle,
            fail_open: output.fail_open,
        }
    }

    /// Run the law-check sync call. Fail-open on timeout / error per
    /// spec 10 §Synchronous paths.
    pub async fn law_check(
        &self,
        tool_name: &str,
        input: &Value,
        session_id: &str,
        turn_id: Option<&str>,
    ) -> LawCheckResult {
        let started = Instant::now();
        let url = format!("{}/v1/laws/check", self.api_endpoint);
        let req = LawCheckRequest {
            tool_name,
            input,
            session_id,
            turn_id,
        };
        let resp =
            tokio::time::timeout(self.laws_timeout, self.client.post(&url).json(&req).send()).await;
        let latency_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
        self.metrics.observe_sync_latency("PreToolUse", latency_ms);
        let body: Option<Value> = match resp {
            Err(_) => {
                self.metrics.incr_sync_timeout("PreToolUse");
                None
            }
            Ok(Err(_)) => None,
            Ok(Ok(resp)) if !resp.status().is_success() => None,
            Ok(Ok(resp)) => resp.json().await.ok(),
        };
        let body = match body {
            Some(b) => b,
            None => {
                return LawCheckResult {
                    deny: false,
                    reason: String::new(),
                    violations: Vec::new(),
                    fail_open: true,
                };
            }
        };
        let violations: Vec<Violation> = body
            .get("violations")
            .and_then(|v| serde_json::from_value::<Vec<Violation>>(v.clone()).ok())
            .unwrap_or_default();
        let critical: Vec<&Violation> = violations
            .iter()
            .filter(|v| v.severity.eq_ignore_ascii_case("critical"))
            .collect();
        let deny = self.laws_block_on_critical && !critical.is_empty();
        if deny {
            for v in &critical {
                self.metrics.incr_law_block(&v.law_id);
            }
        }
        let reason = critical
            .iter()
            .map(|v| format!("{}: {}", v.law_id, v.message))
            .collect::<Vec<_>>()
            .join("\n");
        LawCheckResult {
            deny,
            reason,
            violations,
            fail_open: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AdapterSection;
    use serde_json::json;

    #[tokio::test]
    async fn pre_thinking_disabled_returns_empty_fail_open() {
        let cfg = AdapterSection {
            pre_thinking: crate::config::PreThinkingSection {
                enabled: false,
                ..Default::default()
            },
            ..Default::default()
        };
        let metrics = Arc::new(Metrics::new());
        let client = SyncClient::new(&cfg, metrics);
        let r = client
            .pre_thinking("hello", "sess", None, Some("/repo"))
            .await;
        assert!(r.fail_open);
        assert!(r.bundle.is_empty());
    }

    #[tokio::test]
    async fn unreachable_api_endpoint_fails_open_for_law_check() {
        let cfg = AdapterSection {
            api_endpoint: "http://127.0.0.1:1".into(),
            laws: crate::config::LawsSection {
                timeout_ms: 50,
                ..Default::default()
            },
            ..Default::default()
        };
        let metrics = Arc::new(Metrics::new());
        let client = SyncClient::new(&cfg, metrics);
        let r = client
            .law_check("Bash", &json!({ "command": "rm -rf" }), "sess", None)
            .await;
        assert!(r.fail_open);
        assert!(!r.deny);
    }

    #[tokio::test]
    async fn unreachable_api_endpoint_fails_open_for_pre_thinking() {
        let cfg = AdapterSection {
            api_endpoint: "http://127.0.0.1:1".into(),
            pre_thinking: crate::config::PreThinkingSection {
                timeout_ms: 50,
                ..Default::default()
            },
            ..Default::default()
        };
        let metrics = Arc::new(Metrics::new());
        let client = SyncClient::new(&cfg, metrics);
        let r = client
            .pre_thinking("hello", "sess", None, Some("/repo"))
            .await;
        assert!(r.fail_open);
        // Phase14e — fail-open bundles carry an HTML-comment
        // sentinel instead of being empty, so the model can
        // distinguish outage from "no results matched". The
        // marker `<!-- cortex: timeout reason=…` is the contract.
        assert!(
            r.bundle.starts_with("<!-- cortex: timeout reason="),
            "fail-open bundle missing sentinel: {:?}",
            r.bundle
        );
    }
}
