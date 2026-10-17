//! Synchronous hook paths.
//!
//! Spec 10 §Synchronous paths: hooks that block on `cortex-api`:
//!
//! - `UserPromptSubmit` drives `cortex_pre_thinking::pipeline::run`,
//!   which POSTs a real `cortex_api::QueryRequest` to `/v1/query`,
//!   formats the response into a Markdown bundle, and clips it to the
//!   configured budget. The bundle string ships back as
//!   `additionalContext`.
//! - `PreToolUse` calls `/v1/laws/check`; critical violations route
//!   to `permissionDecision: deny`.
//! - `SessionStart` (phase30 §1.3) calls `GET
//!   /v1/dashboard/active-work` so a fresh session can surface
//!   in-flight Rulebook tasks without the agent calling
//!   `cortex_active_work` explicitly.
//!
//! All three fail-open on timeout / error: the session continues
//! unchanged.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cortex_api::active_work::{ActiveTaskRow, ActiveWorkReport};
use cortex_api::{QueryRequest, QueryResponse};
use cortex_pre_thinking::pipeline::{
    run_with_memory as pre_thinking_run, ClosureQueryFn, PreThinkingBudget, PreThinkingInput,
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
    /// Whether the API reused revision-checked memory content. Its delivery
    /// receipt and authorization are checked afresh for this prompt.
    pub cache_hit: bool,
    /// Local retrieval outcome, independent from optional archive availability.
    pub memory_outcome: &'static str,
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

/// Phase30 (continuity §1.3) — fixed short timeout for the
/// active-work sync call. Not config-driven (unlike `pre_thinking` /
/// `laws`, which expose `timeout_ms` knobs in `adapter.toml`):
/// `SessionStart` already fails open on any hiccup and there is no
/// operator-facing tuning need yet. Sized to the same order of
/// magnitude as `default_laws_timeout` (300 ms) since both gate a
/// session-boundary hook that must never stall Claude Code.
const ACTIVE_WORK_TIMEOUT_MS: u64 = 300;

/// Result of the active-work sync call (phase30 §1.3). `SessionStart`
/// consumes `tasks` to render the "active work" Markdown block.
#[derive(Debug, Clone, Default)]
pub struct ActiveWorkOutcome {
    /// Active-task rows from `.rulebook/tasks/`, verbatim from the
    /// daemon's `/v1/dashboard/active-work` response (already sorted
    /// by phase and with `next_unchecked_item` pre-truncated to 256
    /// bytes server-side — see `cortex_api::active_work`).
    pub tasks: Vec<ActiveTaskRow>,
    /// `true` when the call timed out, errored, or returned a
    /// non-2xx status and we fail-opened to an empty task list. The
    /// caller can't (and doesn't need to) distinguish "call failed"
    /// from "genuinely nothing in flight" — either way `SessionStart`
    /// renders nothing rather than blocking or failing the session.
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
    /// Transport credential, separate from prompt metadata and printable config.
    api_key: Option<String>,
    /// Shared pre-thinking metrics — accumulated across calls so
    /// the health source can read cache hit/miss counters.
    prethink_metrics: Arc<PreThinkingMetrics>,
}

impl SyncClient {
    /// Build a sync client from config.
    pub fn new(cfg: &AdapterSection, metrics: Arc<Metrics>) -> Self {
        let client = Client::builder()
            .timeout(Duration::from_millis(cfg.timeout_ms))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("reqwest client builder"); // SAFETY: Client::builder().build() only fails on TLS init failure; panicking at process boot when TLS is unusable is the correct behaviour and grandfathered by the phase14i §3.2 gate.
        let prethink_metrics = Arc::new(PreThinkingMetrics::new());
        Self {
            client,
            api_endpoint: cfg.api_endpoint.trim_end_matches('/').to_string(),
            pre_thinking_timeout: Duration::from_millis(cfg.pre_thinking.timeout_ms.min(600)),
            pre_thinking_enabled: cfg.pre_thinking.enabled,
            pre_thinking_max_bytes: cfg.pre_thinking.max_bundle_kb.saturating_mul(1024),
            laws_timeout: Duration::from_millis(cfg.laws.timeout_ms),
            laws_block_on_critical: cfg.laws.block_on_critical,
            metrics,
            api_key: None,
            prethink_metrics,
        }
    }

    /// Configure authenticated API access without deriving identity from agent labels.
    pub fn with_api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    /// Expose the shared pre-thinking metrics handle so the health
    /// source can read cache counters from the same registry the
    /// cache bumps.
    pub fn prethink_metrics(&self) -> Arc<PreThinkingMetrics> {
        self.prethink_metrics.clone()
    }

    /// Total configured context cap, shared by memory and lifecycle metadata.
    pub fn bundle_budget(&self) -> usize {
        usize::try_from(self.pre_thinking_max_bytes).unwrap_or(usize::MAX)
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
        self.context(prompt, session_id, turn_id, cwd, false).await
    }

    /// Session start has no task query: retrieve only explicitly pinned advice.
    pub async fn session_start_memory(
        &self,
        session_id: &str,
        cwd: Option<&str>,
    ) -> PreThinkingResult {
        self.context("", session_id, None, cwd, true).await
    }

    async fn context(
        &self,
        prompt: &str,
        session_id: &str,
        turn_id: Option<&str>,
        cwd: Option<&str>,
        local_only: bool,
    ) -> PreThinkingResult {
        if !self.pre_thinking_enabled {
            return PreThinkingResult {
                bundle: String::new(),
                fail_open: true,
                cache_hit: false,
                memory_outcome: "disabled",
            };
        }
        // Hook origin must be observed and absolute. Relative paths would be
        // resolved against the daemon rather than the emitting client's CWD.
        let Some(cwd_str) = cwd.filter(|cwd| Path::new(cwd).is_absolute()) else {
            return PreThinkingResult {
                bundle: String::new(),
                fail_open: true,
                cache_hit: false,
                memory_outcome: "scope_unresolved",
            };
        };
        let timeout = self.pre_thinking_timeout;
        let max_bytes = self.pre_thinking_max_bytes;
        let url = format!("{}/v1/query", self.api_endpoint.trim_end_matches('/'));
        let http = self.client.clone();
        let metrics = self.metrics.clone();
        let api_key = self.api_key.clone();

        let query_fn = Arc::new(ClosureQueryFn(move |req: QueryRequest| {
            let url = url.clone();
            let http = http.clone();
            let metrics = metrics.clone();
            let api_key = api_key.clone();
            async move {
                // Match the pipeline's bounded query inside its shared deadline.
                let trigger = cortex_pre_thinking::intent_select::select_matched(&req.query)
                    .trigger
                    .unwrap_or("");
                let send_started = Instant::now();
                let mut http_req = http.post(&url);
                if let Some(key) = api_key {
                    http_req = http_req.bearer_auth(key);
                }
                if !trigger.is_empty() {
                    http_req = http_req.header("x-cortex-intent-trigger", trigger);
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
                    Ok(Ok(mut resp)) => {
                        let mut body = Vec::new();
                        while let Some(chunk) = resp.chunk().await.ok()? {
                            if body.len().saturating_add(chunk.len()) > 256 * 1024 {
                                return None;
                            }
                            body.extend_from_slice(&chunk);
                        }
                        serde_json::from_slice::<QueryResponse>(&body).ok()
                    }
                }
            }
        }));

        let cwd_path = Path::new(cwd_str);

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
            principal: None,
        };

        let memory_fn = Arc::new(cortex_pre_thinking::HttpDailyMemory::new(
            self.client.clone(),
            self.api_endpoint.clone(),
            self.api_key.clone(),
        ));
        let output = if local_only {
            cortex_pre_thinking::pipeline::run_memory_only(
                &input,
                memory_fn,
                self.prethink_metrics.clone(),
            )
            .await
        } else {
            pre_thinking_run(&input, query_fn, memory_fn, self.prethink_metrics.clone()).await
        };
        let bundle_bytes = u32::try_from(output.bundle.len()).unwrap_or(u32::MAX);
        self.metrics.observe_bundle_bytes(bundle_bytes);

        PreThinkingResult {
            bundle: output.bundle,
            fail_open: output.fail_open,
            cache_hit: output.memory_cache_hit,
            memory_outcome: output.memory_outcome,
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

    /// Run the active-work sync call (phase30 §1.3). Fail-open on any
    /// timeout / transport error / non-2xx: `SessionStart` must never
    /// block or fail a session over a `.rulebook/` snapshot going
    /// stale. `repo` scopes the daemon's response to one workspace
    /// slug; `None` (or an empty slug) requests the unfiltered
    /// snapshot.
    pub async fn active_work(&self, repo: Option<&str>) -> ActiveWorkOutcome {
        let started = Instant::now();
        let Some(repo) = repo.filter(|repo| !repo.is_empty()) else {
            return ActiveWorkOutcome::default();
        };
        let url = format!("{}/v1/dashboard/active-work", self.api_endpoint);
        let mut req = self.client.get(&url).query(&[("repo", repo)]);
        if let Some(token) = &self.api_key {
            req = req.bearer_auth(token);
        }
        let timeout = Duration::from_millis(ACTIVE_WORK_TIMEOUT_MS);
        let resp = tokio::time::timeout(timeout, async {
            let mut response = req.send().await.ok()?;
            if !response.status().is_success() {
                return None;
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response.chunk().await.ok()? {
                if bytes.len().saturating_add(chunk.len()) > 64 * 1024 {
                    return None;
                }
                bytes.extend_from_slice(&chunk);
            }
            serde_json::from_slice::<ActiveWorkReport>(&bytes).ok()
        })
        .await;
        let latency_ms = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
        self.metrics
            .observe_sync_latency("SessionStart", latency_ms);
        let body: Option<ActiveWorkReport> = match resp {
            Err(_elapsed) => {
                self.metrics.incr_sync_timeout("SessionStart");
                None
            }
            Ok(body) => body,
        };
        match body {
            Some(report) => ActiveWorkOutcome {
                tasks: report.active_tasks,
                fail_open: false,
            },
            None => ActiveWorkOutcome {
                tasks: Vec::new(),
                fail_open: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AdapterSection;
    use serde_json::json;

    fn explicit_test_repo() -> tempfile::TempDir {
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        std::fs::write(
            repo.path().join("cortex.toml"),
            "[cortex]\nid='Sync/Test'\n",
        )
        .unwrap();
        repo
    }

    #[tokio::test]
    async fn active_work_authenticates_exact_repo_and_skips_unresolved_scope() {
        use wiremock::matchers::{header, method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/dashboard/active-work"))
            .and(query_param("repo", "exact-repo"))
            .and(header("authorization", "Bearer synthetic-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "active_tasks":[{"id":"phase34","status":"in-progress"}],
                "in_progress_count":1,"blocked_count":0,"recent_archives":[]
            })))
            .expect(1)
            .mount(&server)
            .await;
        let client = SyncClient::new(
            &AdapterSection {
                api_endpoint: server.uri(),
                ..Default::default()
            },
            Arc::new(Metrics::new()),
        )
        .with_api_key("synthetic-token");
        assert!(client.active_work(None).await.tasks.is_empty());
        assert!(client.active_work(Some("")).await.tasks.is_empty());
        let result = client.active_work(Some("exact-repo")).await;
        assert!(!result.fail_open);
        assert_eq!(result.tasks[0].id, "phase34");
        server.verify().await;
    }

    #[tokio::test]
    async fn active_work_rejects_oversized_response() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/v1/dashboard/active-work"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "active_tasks":[{"id":"phase34","status":"blocked","blocked_reason":"x".repeat(64*1024)}],
                "in_progress_count":0,"blocked_count":1,"recent_archives":[]
            }))).mount(&server).await;
        let client = SyncClient::new(
            &AdapterSection {
                api_endpoint: server.uri(),
                ..Default::default()
            },
            Arc::new(Metrics::new()),
        );
        let result = client.active_work(Some("exact-repo")).await;
        assert!(result.fail_open);
        assert!(result.tasks.is_empty());
    }

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
        let repo = explicit_test_repo();
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
            .pre_thinking("hello", "sess", None, repo.path().to_str())
            .await;
        assert!(r.fail_open);
        assert!(r.bundle.is_empty());
        // Windows may time out connecting to a closed port before reporting
        // refusal. Both are transport failures, unlike an unresolved CWD.
        assert!(
            matches!(r.memory_outcome, "unavailable" | "timeout"),
            "unexpected outcome: {}",
            r.memory_outcome
        );
    }

    #[tokio::test]
    async fn unavailable_memory_and_query_are_never_replayed_from_adapter_cache() {
        use wiremock::{
            matchers::{method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        let repo = explicit_test_repo();
        for route in ["/v1/memory/query", "/v1/query"] {
            Mock::given(method("POST"))
                .and(path(route))
                .respond_with(ResponseTemplate::new(503))
                .expect(2)
                .mount(&server)
                .await;
        }
        let cfg = AdapterSection {
            api_endpoint: server.uri(),
            pre_thinking: crate::config::PreThinkingSection {
                timeout_ms: 50,
                ..Default::default()
            },
            ..Default::default()
        };
        let client = SyncClient::new(&cfg, Arc::new(Metrics::new()));
        for _ in 0..2 {
            let result = client
                .pre_thinking("same prompt", "s", None, repo.path().to_str())
                .await;
            assert!(result.fail_open);
            assert!(!result.cache_hit);
            assert!(result.bundle.is_empty());
            assert_eq!(result.memory_outcome, "unavailable");
        }
        assert_eq!(client.prethink_metrics().cache_counters(), (0, 0));
        server.verify().await;
    }

    #[tokio::test]
    async fn fresh_scoped_memory_survives_optional_outage_and_auth_denial_drops_old_context() {
        use wiremock::{
            matchers::{header, method, path},
            Mock, MockServer, ResponseTemplate,
        };
        let server = MockServer::start().await;
        let repo = tempfile::tempdir().unwrap();
        std::fs::create_dir(repo.path().join(".git")).unwrap();
        std::fs::write(repo.path().join(".git/HEAD"), "ref: refs/heads/current\n").unwrap();
        std::fs::write(
            repo.path().join("cortex.toml"),
            "[cortex]\nid = \"Exact/Repo\"\n",
        )
        .unwrap();
        let cfg = AdapterSection {
            api_endpoint: server.uri(),
            ..Default::default()
        };
        let client =
            SyncClient::new(&cfg, Arc::new(Metrics::new())).with_api_key("synthetic-token");
        for (revision, id) in [(1, "original"), (2, "corrected")] {
            server.reset().await;
            Mock::given(method("POST")).and(path("/v1/memory/query"))
                .and(header("authorization","Bearer synthetic-token"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "context":format!("[memory \"{id}\" v1 event evt-{id} scope=repo:Exact/Repo]\nUse transaction {id}.\n"),
                    "memories":[{"scope":{"repository":"Exact/Repo"},"id":id,"version":1,"event_id":format!("evt-{id}")}],
                    "revision":revision,"global_revision":null,"delivery_id":format!("delivery-{revision}"),
                    "stage":"prepared","cache_hit":false
                }))).expect(1).mount(&server).await;
            Mock::given(method("POST"))
                .and(path("/v1/query"))
                .and(header("authorization", "Bearer synthetic-token"))
                .respond_with(ResponseTemplate::new(503))
                .expect(1)
                .mount(&server)
                .await;
            let result = client
                .pre_thinking(
                    "Use transaction",
                    "observed-session",
                    None,
                    repo.path().to_str(),
                )
                .await;
            assert!(
                result.bundle.contains(id),
                "local memory lost: {}",
                result.bundle
            );
            assert!(result.fail_open);
            assert!(!result.cache_hit);
            if revision == 2 {
                assert!(!result.bundle.contains("original"));
            }
            let requests = server.received_requests().await.unwrap();
            let request = requests
                .iter()
                .find(|req| req.url.path() == "/v1/memory/query")
                .unwrap();
            let body: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(body["repo"], "Exact/Repo");
            assert_eq!(body["branch"], "current");
            assert_eq!(body["include_global"], false);
            server.verify().await;
        }
        server.reset().await;
        Mock::given(method("POST"))
            .and(path("/v1/memory/query"))
            .respond_with(ResponseTemplate::new(403))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v1/query"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;
        let result = client
            .pre_thinking(
                "Use transaction",
                "observed-session",
                None,
                repo.path().to_str(),
            )
            .await;
        assert!(!result.bundle.contains("original"));
        assert!(!result.bundle.contains("corrected"));
        assert!(!result.cache_hit);
    }
}
