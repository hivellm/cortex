//! Top-level query service — composes orchestrator + cache + ACL +
//! rate limiter + redaction + audit. The HTTP handler and the MCP
//! tool binding call into this single entry point so behaviour stays
//! identical between transports.

use std::sync::{Arc, RwLock};

use axum::http::HeaderMap;

use crate::acl::{AclDecision, AclStore};
use crate::audit::{build_envelope_with_rewrite_context, AuditPublisher, MemoryAuditPublisher};
use crate::audit_store::AuditStore;
use crate::cache::{cache_key, Cache, InMemoryCache};
use crate::lanes::MemoryKeywordLane;
use crate::orchestrator::Orchestrator;
use crate::principal::Principal;
use crate::principal_resolver::resolve_request_principal;
use crate::principal_store::PrincipalStore;
use crate::query_rewrite::RewrittenQuery;
use crate::rate_limit::{RateConfig, RateDecision, RateLimiter};
use crate::redaction::redact_response;
use crate::storage::api_keys::ApiKeyStore;
use crate::types::{Notice, QueryRequest, QueryResponse, Scope};

/// Phase4j header — explicit repo override for direct
/// `/v1/query` callers (MCP tools, dashboard search, GUI).
pub const HEADER_CORTEX_REPO: &str = "x-cortex-repo";

/// Phase4j header — caller's working directory; the resolver
/// derives a repo slug from the basename via
/// [`cortex_storage::names::slug_for_repo`].
pub const HEADER_CORTEX_CWD: &str = "x-cortex-cwd";

/// Phase6d header — keyword that triggered the intent selector
/// (e.g. `how does`). The pre-thinking pipeline sets this when it
/// dispatches to `/v1/query`; direct callers (MCP, dashboard,
/// GUI) leave it empty and the audit envelope records
/// `intent_trigger: null`. Stamped on every audit emit so the
/// dashboard can attribute routing changes to the specific rule
/// that fired.
pub const HEADER_CORTEX_INTENT_TRIGGER: &str = "x-cortex-intent-trigger";

/// Env var honouring the legacy "fall back to the unknown slug"
/// behaviour. When set to `1`, the resolver does NOT reject a
/// missing scope; it logs a deprecation warning and returns
/// [`ScopeResolution::RejectedLegacy`] so downstream code keeps
/// the empty-corpus path. Removed at the harness gate (phase6e).
pub const ENV_ALLOW_UNKNOWN_SCOPE: &str = "CORTEX_ALLOW_UNKNOWN_SCOPE";

/// Which lane resolved `scope.repo`. Stamped on every audit
/// envelope so the dashboard can surface "this caller forgot to
/// set the repo and got a 422" before the relevance scores degrade.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeResolution {
    /// `request.scope.repo` carried a non-empty value.
    Explicit,
    /// Resolved from the `x-cortex-repo` request header.
    Header,
    /// Resolved from the `x-cortex-cwd` request header (slugified
    /// via `slug_for_repo` over the path basename).
    Cwd,
    /// `CORTEX_ALLOW_UNKNOWN_SCOPE=1` was set and the request
    /// would otherwise have been rejected. Acts as the
    /// one-week deprecation hatch — the audit envelope stamps
    /// this value so operators can grep for the count of remaining
    /// callers that still rely on the legacy fallback.
    RejectedLegacy,
}

impl ScopeResolution {
    /// Stable string identifier for the audit envelope and spec
    /// docs.
    pub fn as_str(self) -> &'static str {
        match self {
            ScopeResolution::Explicit => "explicit",
            ScopeResolution::Header => "header",
            ScopeResolution::Cwd => "cwd",
            ScopeResolution::RejectedLegacy => "rejected_legacy",
        }
    }
}

/// Failure mode for [`resolve_scope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeError {
    /// No lane produced a non-empty repo and the legacy escape
    /// hatch was not enabled.
    Missing,
}

/// Phase6a — derive `request.scope.repo` from the request headers
/// when the body did not carry an explicit value, in the order
/// declared by the spec:
///
/// 1. `request.scope.repo` (explicit) — round-trip unchanged.
/// 2. `x-cortex-repo` header.
/// 3. `x-cortex-cwd` header → `slug_for_repo(path-basename)`.
/// 4. Reject with [`ScopeError::Missing`] (the HTTP handler maps
///    the error to `422 scope_repo_required`).
///
/// The legacy `CORTEX_ALLOW_UNKNOWN_SCOPE=1` hatch flips the
/// rejection into a warn-and-allow, marking the resolution as
/// [`ScopeResolution::RejectedLegacy`] so the audit envelope still
/// records the lane that fired.
pub fn resolve_scope(
    req: &mut QueryRequest,
    headers: &HeaderMap,
) -> Result<ScopeResolution, ScopeError> {
    if req
        .scope
        .repo
        .as_deref()
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
    {
        return Ok(ScopeResolution::Explicit);
    }

    if let Some(value) = headers
        .get(HEADER_CORTEX_REPO)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        req.scope.repo = Some(value.to_string());
        return Ok(ScopeResolution::Header);
    }

    if let Some(cwd) = headers
        .get(HEADER_CORTEX_CWD)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        // Take the last path component, splitting on BOTH `/` and `\`.
        // `std::path::Path::file_name` is platform-dependent: the daemon
        // runs on Linux (docker) but the caller's cwd is frequently a
        // Windows path (`E:\HiveLLM\Rulebook`), and Linux's `Path` does
        // not treat `\` as a separator — so it would slug the whole
        // string to `e-hivellm-rulebook` instead of `rulebook` (issue #4
        // Bug 2). Splitting on both separators is correct regardless of
        // which OS the daemon runs on. `find(non-empty)` skips a trailing
        // separator (`foo/` -> `foo`).
        let basename = cwd
            .rsplit(['/', '\\'])
            .find(|s| !s.is_empty())
            .unwrap_or(cwd);
        let slug = cortex_storage::names::slug_for_repo(basename);
        if !slug.is_empty() {
            req.scope.repo = Some(slug);
            return Ok(ScopeResolution::Cwd);
        }
    }

    if std::env::var(ENV_ALLOW_UNKNOWN_SCOPE)
        .map(|v| matches!(v.as_str(), "1" | "true" | "TRUE" | "yes" | "on"))
        .unwrap_or(false)
    {
        tracing::warn!(
            caller = "phase6a",
            "scope_unresolved_fallback: caller did not set scope.repo and CORTEX_ALLOW_UNKNOWN_SCOPE=1 — \
             passing through to the unknown slug. The escape hatch is removed at the phase6e gate."
        );
        return Ok(ScopeResolution::RejectedLegacy);
    }

    Err(ScopeError::Missing)
}

/// Project the request-side scope into the canonical form the lanes
/// actually filter on. The `repo` field is slugified through
/// `cortex_storage::names::slug_for_repo` so it matches the
/// `cortex-{slug}-{family}` index / collection naming the strategies
/// produce; everything else round-trips verbatim. Dashboards reading
/// `scope_resolved.repo` see exactly the slug the per-project lane
/// hit, not the raw request input.
fn canonicalise_scope(req_scope: &Scope) -> Scope {
    let repo = req_scope
        .repo
        .as_deref()
        .filter(|s| !s.is_empty())
        .map(cortex_storage::names::slug_for_repo);
    // phase10h §1.2 — every input dimension normalises before
    // it lands in `scope_resolved`: empty / whitespace strings
    // drop out, topics dedupe, RFC-3339 `since` round-trips
    // verbatim (the lane filters parse it on demand).
    let files: Vec<String> = req_scope
        .files
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    let topics: Vec<String> = {
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut out: Vec<String> = Vec::new();
        for t in &req_scope.topics {
            let normalised = t.trim().to_ascii_lowercase();
            if normalised.is_empty() {
                continue;
            }
            if seen.insert(normalised.clone()) {
                out.push(normalised);
            }
        }
        out
    };
    let since = req_scope
        .since
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    // Phase11i §3.3-§3.5 — round-trip the new scope filters
    // verbatim. Empty vectors compare equal to "no filter"; an
    // explicit value passes through canonicalisation untouched.
    // Each value is trimmed but otherwise preserved so
    // case-sensitive matches against Meili's filter grammar
    // stay byte-stable.
    let dedup_trim = |v: &[String]| -> Vec<String> {
        let mut out = Vec::with_capacity(v.len());
        let mut seen = std::collections::BTreeSet::new();
        for s in v {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                continue;
            }
            if seen.insert(trimmed.to_string()) {
                out.push(trimmed.to_string());
            }
        }
        out
    };
    Scope {
        repo,
        files,
        topics,
        since,
        // Phase11i §3.1 — recency_decay round-trips verbatim.
        // Canonicalisation does not invent or strip a value;
        // the orchestrator falls back to the per-intent default
        // when this stays None.
        recency_decay: req_scope.recency_decay,
        // Phase11i §3.2 — cross_repo_boost round-trips verbatim
        // for the same reason. The orchestrator picks
        // DEFAULT_CROSS_REPO_BOOST when None.
        cross_repo_boost: req_scope.cross_repo_boost,
        models: dedup_trim(&req_scope.models),
        tools: dedup_trim(&req_scope.tools),
        session_id: req_scope
            .session_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from),
        session_cohort: dedup_trim(&req_scope.session_cohort),
        outcomes: dedup_trim(&req_scope.outcomes),
        exclude_outcomes: dedup_trim(&req_scope.exclude_outcomes),
    }
}

/// Outcome surfaced by [`QueryService::handle`]. The HTTP handler
/// translates `Denied` / `RateLimited` into the spec-defined status
/// codes; the MCP binding maps them to MCP error shapes.
#[derive(Debug)]
pub enum ServiceOutcome {
    /// Request answered.
    Ok(Box<QueryResponse>),
    /// 400 — empty query.
    EmptyQuery,
    /// 422 — phase6a: `scope.repo` could not be resolved from the
    /// body, the `x-cortex-repo` header, or the `x-cortex-cwd`
    /// header. The HTTP handler maps this to `422 scope_repo_required`.
    EmptyScope,
    /// 403 — caller / scope ACL.
    Denied,
    /// 429 — rate limit hit.
    RateLimited(std::time::Duration),
}

/// Top-level service handle.
pub struct QueryService {
    /// Orchestrator wired to the lane clients.
    pub orchestrator: Orchestrator,
    /// Cache backend (defaults to [`InMemoryCache`]).
    pub cache: Arc<dyn Cache>,
    /// ACL store.
    pub acl: Arc<AclStore>,
    /// Rate limiter.
    pub rate_limiter: Arc<RateLimiter>,
    /// Audit publisher.
    pub audit: Arc<dyn AuditPublisher>,
    /// Phase10j — in-memory ring buffer indexed by `query_id` so
    /// `GET /v1/audit/{query_id}` can answer synchronously without
    /// waiting for synap to round-trip the envelope back. Recorded
    /// in lockstep with [`Self::audit`]; both sinks see the same
    /// envelope.
    pub audit_store: Arc<AuditStore>,
    /// Snapshot of indexed-repo slugs the daemon currently sees. Read
    /// from the same `MemoryKeywordLane` the dashboard uses to derive
    /// `repos_indexed`, so the answer in `/v1/status.indexed_repos`
    /// and the trigger for `notice.code = "repo_not_indexed"` come
    /// from one source. `None` for unit tests that don't care about
    /// the lookup; production wires the lane through `main.rs`.
    pub indexed_repos: Option<Arc<MemoryKeywordLane>>,
    /// Phase11e §6 — boot-time / periodic coverage audit snapshot.
    /// Populated by `audit_coverage_at_boot` in `main.rs` (and
    /// optionally a periodic refresh loop) so `/v1/status` and
    /// `cortex_status` MCP can report honest per-backend coverage
    /// without re-running the full diff on every call. `None` when
    /// the daemon was started without coverage wiring (unit tests,
    /// cold dev stack); `Some(None)` when the audit ran but every
    /// backend was unconfigured.
    pub coverage_snapshot:
        Option<Arc<tokio::sync::RwLock<Option<crate::coverage::CoverageResponse>>>>,
    /// Phase21 §4.4 — RBAC role-binding store used to resolve an
    /// API key's role → `Principal`. `None` when access control is
    /// not configured (local dev); in that case all callers receive
    /// the default principal (see [`PrincipalStore::pass_through`]).
    pub principal_store: Option<Arc<RwLock<PrincipalStore>>>,
    /// Phase21 §4.4 — API-key store, forwarded to the principal
    /// resolver so `verify_with_role` can map a Bearer token → role.
    /// Shares the same `Arc` as the auth middleware's store.
    pub api_key_store: Option<Arc<ApiKeyStore>>,
}

impl QueryService {
    /// Build a service with an [`InMemoryCache`] + a recording audit
    /// publisher — the shape used by the unit tests + the
    /// integration suite.
    pub fn with_memory_defaults(orchestrator: Orchestrator) -> Self {
        Self {
            orchestrator,
            cache: Arc::new(InMemoryCache::new()),
            acl: Arc::new(AclStore::new()),
            rate_limiter: Arc::new(RateLimiter::new(RateConfig::default_for_spec_11())),
            audit: Arc::new(MemoryAuditPublisher::new()),
            audit_store: Arc::new(AuditStore::new()),
            indexed_repos: None,
            coverage_snapshot: None,
            principal_store: None,
            api_key_store: None,
        }
    }

    /// Phase21 §4.4 — attach the principal-store (role bindings). When set,
    /// every request is resolved to a `Principal` via the key → role → store
    /// chain. When `None` (default), all callers receive the pass-through
    /// super-admin principal so backward compat is preserved.
    pub fn with_principal_store(mut self, ps: Arc<RwLock<PrincipalStore>>) -> Self {
        self.principal_store = Some(ps);
        self
    }

    /// Phase21 §4.4 — attach the API-key store so the resolver can call
    /// `verify_with_role` to map a Bearer token to a role.
    pub fn with_api_key_store(mut self, aks: Arc<ApiKeyStore>) -> Self {
        self.api_key_store = Some(aks);
        self
    }

    /// Attach an indexed-repos snapshot. Returns `self` so callers can
    /// chain the wiring at startup (`QueryService::with_memory_defaults(...).with_indexed_repos(lane)`).
    pub fn with_indexed_repos(mut self, lane: Arc<MemoryKeywordLane>) -> Self {
        self.indexed_repos = Some(lane);
        self
    }

    /// Phase11e §6 — attach a coverage-snapshot handle. The
    /// boot-time audit (and any future periodic refresh) writes the
    /// latest [`crate::coverage::CoverageResponse`] into this Arc;
    /// `/v1/status` and `cortex_status` MCP read from it without
    /// re-running the diff. Returns `self` so callers can chain.
    pub fn with_coverage_snapshot(
        mut self,
        snapshot: Arc<tokio::sync::RwLock<Option<crate::coverage::CoverageResponse>>>,
    ) -> Self {
        self.coverage_snapshot = Some(snapshot);
        self
    }

    /// Build a [`Notice`] when the canonicalised scope's repo is set
    /// but the indexed-repos snapshot does not contain it. Returns
    /// `None` when the request omitted `scope.repo`, when the repo
    /// IS in the snapshot, or when the lane is unwired.
    fn build_repo_not_indexed_notice(&self, canonical: &Scope) -> Option<Notice> {
        let repo = canonical.repo.as_deref()?;
        let lane = self.indexed_repos.as_ref()?;
        let snapshot = lane.indexed_repos();
        if snapshot.iter().any(|s| s == repo) {
            return None;
        }
        Some(Notice {
            code: "repo_not_indexed".to_string(),
            message: format!(
                "scope.repo `{repo}` is not present in the cortex-api indexed-repo snapshot"
            ),
            hint: "run `cortex-bootstrap --repo <path>` to seed the daemon for this repo, \
                   then retry. See `/v1/status.indexed_repos` for the current set."
                .to_string(),
        })
    }

    /// Build a [`Notice`] when the canonicalised scope omitted
    /// `scope.repo`. The strategies layer slugifies an empty repo to
    /// the `unknown` family and the lanes return zero hits — without
    /// this notice the caller sees a silent empty success and has no
    /// way to know the request never targeted a real index. Returns
    /// `None` when the repo IS set (the existing
    /// `repo_not_indexed` path covers the unknown-but-set case).
    fn build_scope_unset_notice(canonical: &Scope) -> Option<Notice> {
        if canonical.repo.is_some() {
            return None;
        }
        Some(Notice {
            code: "scope_unset".to_string(),
            message: "scope.repo is missing — query targeted the `unknown` family and \
                      returned zero hits"
                .to_string(),
            hint: "set `scope.repo` to one of the indexed repos (see \
                   `/v1/status.indexed_repos`); MCP callers can pass \
                   `scope: { repo: \"<repo>\" }` in the tool arguments."
                .to_string(),
        })
    }

    /// Run one request through the full pipeline. Use this entry
    /// when the caller has already resolved the scope (or doesn't
    /// have `HeaderMap` access — e.g., the legacy MCP binding).
    /// Direct `/v1/query` callers should prefer
    /// [`QueryService::handle_with_headers`] so the audit envelope
    /// records the resolution lane.
    pub async fn handle(&self, caller: &str, req: QueryRequest) -> ServiceOutcome {
        // No headers available — use pass-through principal (backward compat).
        let principal = self.resolve_principal(&HeaderMap::new());
        self.handle_resolved(caller, req, ScopeResolution::Explicit, None, principal)
            .await
    }

    /// Phase6a entry point. Resolves the scope from `headers`
    /// (with the body taking priority), then runs the pipeline. The
    /// audit envelope stamps `scope_resolution` so the dashboard
    /// can flag misconfigured callers.
    pub async fn handle_with_headers(
        &self,
        caller: &str,
        mut req: QueryRequest,
        headers: &HeaderMap,
    ) -> ServiceOutcome {
        let resolution = match resolve_scope(&mut req, headers) {
            Ok(r) => r,
            Err(ScopeError::Missing) => return ServiceOutcome::EmptyScope,
        };
        let intent_trigger = headers
            .get(HEADER_CORTEX_INTENT_TRIGGER)
            .and_then(|v| v.to_str().ok())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        // Phase21 §4.4 — resolve the caller's principal from the API key
        // and the role-binding store. Falls back to pass-through when AC
        // is not configured (principal_store is None).
        let principal = self.resolve_principal(headers);
        self.handle_resolved(caller, req, resolution, intent_trigger, principal)
            .await
    }

    /// Phase21 §4.4 — resolve the caller's `Principal` from the
    /// request headers. Returns a pass-through super-admin when the
    /// service was not wired with a `principal_store` (backward compat).
    pub fn resolve_principal(&self, headers: &HeaderMap) -> Principal {
        match (&self.principal_store, &self.api_key_store) {
            (Some(ps_lock), aks) => {
                let ps = ps_lock.read().expect("principal_store RwLock poisoned");
                resolve_request_principal(headers, aks.as_deref(), &ps)
            }
            (None, _) => Principal::super_admin("system"),
        }
    }

    async fn handle_resolved(
        &self,
        caller: &str,
        req: QueryRequest,
        resolution: ScopeResolution,
        intent_trigger: Option<String>,
        principal: Principal,
    ) -> ServiceOutcome {
        if req.query.trim().is_empty() {
            return ServiceOutcome::EmptyQuery;
        }
        match self.acl.decide(caller, req.scope.repo.as_deref()) {
            AclDecision::Deny => return ServiceOutcome::Denied,
            AclDecision::Unknown | AclDecision::Allow => {}
        }
        match self.rate_limiter.admit(caller) {
            RateDecision::Limit { retry_after } => {
                return ServiceOutcome::RateLimited(retry_after);
            }
            RateDecision::Admit { .. } => {}
        }

        let key = cache_key(&req);
        if let Some(mut hit) = self.cache.get(&key).await {
            // Sync hit envelope's intent + cache state with the
            // request — the cached entry was stamped with whatever
            // the previous caller saw.
            hit.intent = req.intent.label().to_string();
            // Phase6f §5.2 — even on cache hit we stamp the
            // rewriter context so the dashboard can correlate
            // hit/miss rates with the active strategy. The cache
            // doesn't store the per-call RewrittenQuery (it pre-
            // dates phase6f), so we re-run the rewriter against
            // the original request to keep the audit field
            // populated. The rewriter's own cache (when Sonnet)
            // collapses this back to a hash lookup.
            let rewritten = self
                .orchestrator
                .rewriter
                .rewrite(&req.query, req.intent)
                .await
                .unwrap_or_else(|_| RewrittenQuery::passthrough(&req.query));
            let fusion_snapshot = self.orchestrator.current_fusion();
            let envelope = build_envelope_with_rewrite_context(
                caller,
                hit.intent.as_str(),
                &hit,
                resolution.as_str(),
                &fusion_snapshot,
                intent_trigger.as_deref(),
                &rewritten,
            );
            // Phase10j — dual sink: in-memory store backs the
            // synchronous `/v1/audit/{query_id}` lookup; the
            // publisher continues to fan the envelope out to synap
            // for the dashboard / streaming consumers.
            self.audit_store.record(&envelope);
            self.audit.publish(envelope).await;
            return ServiceOutcome::Ok(Box::new(hit));
        }

        // Phase21 §5.2 — run with the resolved principal's ACL context
        // so each lane injects the Bell-LaPadula filter server-side.
        tracing::debug!(
            principal_id = %principal.id,
            clearance_level = principal.clearance_level,
            compartments = ?principal.compartment_grants,
            "query executing under resolved principal"
        );
        let (mut response, rewritten) = self.orchestrator.run_with_principal(&req, &principal).await;
        redact_response(&mut response);
        // Stamp ACL + scope echo before caching so the audit layer
        // sees the canonical view. The echoed `repo` carries the
        // slug the lanes actually filter on (matches the
        // `cortex-{slug}-{family}` index/collection names) — not
        // the raw request input — so dashboards can show how the
        // query landed in the per-project surfaces.
        response.scope_resolved = canonicalise_scope(&req.scope);
        // Attach a `repo_not_indexed` notice when the resolved scope
        // points at a repo the daemon has never seen — see
        // [`QueryService::build_repo_not_indexed_notice`]. Cached so
        // the notice survives subsequent cache hits with the same
        // scope; callers asking the question repeatedly get the same
        // structured remediation hint instead of an empty success.
        if response.notice.is_none() {
            response.notice = self.build_repo_not_indexed_notice(&response.scope_resolved);
        }
        // Fallback diagnostic: when the caller forgot scope.repo
        // entirely we still want them to see an actionable hint
        // instead of `results: {}`. The repo-set-but-unknown path
        // above already produces `repo_not_indexed`, so this only
        // fires when `scope.repo` is `None` after canonicalisation.
        if response.notice.is_none() {
            response.notice = Self::build_scope_unset_notice(&response.scope_resolved);
        }
        // Phase11c — apply the byte-budget clipper before caching so
        // every cache hit also respects the cap. The default keeps
        // the wire payload under the MCP transport's hard limit;
        // callers tighten or loosen via `QueryRequest::budget_bytes`.
        let budget_bytes = req
            .budget_bytes
            .unwrap_or(crate::budget::DEFAULT_BUDGET_BYTES);
        let report = crate::budget::clip_response_to_budget(&mut response, budget_bytes);
        if crate::budget::report_is_meaningful(&report) {
            response.clipped = Some(report);
        }
        self.cache.put(&key, response.clone()).await;
        let fusion_snapshot = self.orchestrator.current_fusion();
        let envelope = build_envelope_with_rewrite_context(
            caller,
            req.intent.label(),
            &response,
            resolution.as_str(),
            &fusion_snapshot,
            intent_trigger.as_deref(),
            &rewritten,
        );
        // Phase10j — dual sink (see hit-path comment above).
        self.audit_store.record(&envelope);
        self.audit.publish(envelope).await;
        ServiceOutcome::Ok(Box::new(response))
    }

    /// Helper for the cache-invalidation event consumer. The cache
    /// stores the canonicalised slug (the form the lanes actually
    /// filter on after `canonicalise_scope` runs in `handle`); the
    /// caller may pass the raw repo id, so we slugify here so
    /// `invalidate_repo("Vectorizer")` and
    /// `invalidate_repo("vectorizer")` both clear the same entries.
    pub async fn invalidate_repo(&self, repo: &str) {
        let slug = cortex_storage::names::slug_for_repo(repo);
        self.cache.invalidate_repo(&slug).await;
    }
}

/// Minimal HTTP body shape returned for non-200 outcomes. Mirrors
/// spec 11 §Failure modes.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ErrorBody {
    /// `empty_query` | `scope_forbidden` | `rate_limited`.
    pub reason: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lanes::{LaneHit, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane};
    use crate::types::{IncludeField, Intent, Scope};

    fn build_orchestrator() -> Orchestrator {
        let v = Arc::new(MemoryVectorLane::new());
        let k = Arc::new(MemoryKeywordLane::new());
        let g = Arc::new(MemoryGraphLane::new());
        v.seed(
            "cortex-code",
            vec![LaneHit {
                doc_id: "h1".into(),
                text: "hello".into(),
                repo: Some("R".into()),
                path: Some("src/lib.rs".into()),
                symbol: None,
                content_hash: None,
                score: 0.9,
                ts: 100,
                severity: None,
                extras: Default::default(),
                overlay: crate::lanes::Overlay::default(),
            }],
        );
        Orchestrator::new(v, k, g)
    }

    fn req(query: &str) -> QueryRequest {
        QueryRequest {
            intent: Intent::PreChangeContext,
            scope: Scope::default(),
            query: query.into(),
            limit: 20,
            k: 50,
            include: vec![IncludeField::Snippets],
            budget_ms: 500,
            budget_bytes: None,
            as_of: None,
            branch: None,
            projects: None,
            include_history: None,
            include_future: None,
            include_branches: None,
            principal: None,
        }
    }

    // ---- phase6a — `resolve_scope` lane coverage ----

    fn empty_headers() -> HeaderMap {
        HeaderMap::new()
    }

    fn headers_with(name: &str, value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(
            axum::http::header::HeaderName::from_bytes(name.as_bytes()).unwrap(),
            axum::http::header::HeaderValue::from_str(value).unwrap(),
        );
        h
    }

    #[test]
    fn resolve_scope_explicit_round_trips_request_repo() {
        let mut r = req("x");
        r.scope.repo = Some("Vectorizer".into());
        let res = resolve_scope(&mut r, &empty_headers()).expect("explicit must succeed");
        assert_eq!(res, ScopeResolution::Explicit);
        assert_eq!(r.scope.repo.as_deref(), Some("Vectorizer"));
    }

    #[test]
    fn resolve_scope_falls_back_to_x_cortex_repo_header() {
        let mut r = req("x");
        let res = resolve_scope(&mut r, &headers_with(HEADER_CORTEX_REPO, "Cortex"))
            .expect("header lane must succeed");
        assert_eq!(res, ScopeResolution::Header);
        assert_eq!(r.scope.repo.as_deref(), Some("Cortex"));
    }

    #[test]
    fn resolve_scope_derives_repo_from_x_cortex_cwd_basename() {
        let mut r = req("x");
        let res = resolve_scope(
            &mut r,
            &headers_with(HEADER_CORTEX_CWD, "/home/user/work/Cortex"),
        )
        .expect("cwd lane must succeed");
        assert_eq!(res, ScopeResolution::Cwd);
        // basename → slug_for_repo lower-cases.
        assert_eq!(r.scope.repo.as_deref(), Some("cortex"));
    }

    #[test]
    fn resolve_scope_derives_repo_from_windows_cwd_on_linux_daemon() {
        // issue #4 Bug 2 regression: the daemon runs on Linux but the
        // caller's cwd is a Windows path. `\` must be treated as a
        // separator so we get `rulebook`, not `e-hivellm-rulebook`.
        let mut r = req("x");
        let res = resolve_scope(
            &mut r,
            &headers_with(HEADER_CORTEX_CWD, "E:\\HiveLLM\\Rulebook"),
        )
        .expect("cwd lane must succeed");
        assert_eq!(res, ScopeResolution::Cwd);
        assert_eq!(r.scope.repo.as_deref(), Some("rulebook"));
    }

    #[test]
    fn resolve_scope_cwd_basename_handles_trailing_separator() {
        let mut r = req("x");
        resolve_scope(&mut r, &headers_with(HEADER_CORTEX_CWD, "/home/user/Cortex/"))
            .expect("cwd lane must succeed");
        assert_eq!(r.scope.repo.as_deref(), Some("cortex"));
    }

    #[test]
    fn resolve_scope_rejects_when_every_lane_misses() {
        let mut r = req("x");
        // Make sure the legacy escape hatch is off for this test;
        // tests run in the same process so we explicitly clear it.
        std::env::remove_var(ENV_ALLOW_UNKNOWN_SCOPE);
        let err = resolve_scope(&mut r, &empty_headers()).expect_err("missing must reject");
        assert_eq!(err, ScopeError::Missing);
        assert!(r.scope.repo.is_none());
    }

    #[test]
    fn resolve_scope_legacy_hatch_passes_through_when_env_set() {
        let mut r = req("x");
        std::env::set_var(ENV_ALLOW_UNKNOWN_SCOPE, "1");
        let res = resolve_scope(&mut r, &empty_headers()).expect("legacy hatch must allow");
        assert_eq!(res, ScopeResolution::RejectedLegacy);
        assert!(
            r.scope.repo.is_none(),
            "legacy hatch must NOT synthesise a repo — the unknown slug is the audit signal"
        );
        std::env::remove_var(ENV_ALLOW_UNKNOWN_SCOPE);
    }

    #[test]
    fn resolve_scope_explicit_wins_over_headers() {
        // Order matters — request body explicit MUST beat any header,
        // otherwise a misconfigured caller could shadow itself.
        let mut r = req("x");
        r.scope.repo = Some("Cortex".into());
        let mut h = headers_with(HEADER_CORTEX_REPO, "Vectorizer");
        h.insert(
            axum::http::header::HeaderName::from_bytes(HEADER_CORTEX_CWD.as_bytes()).unwrap(),
            axum::http::header::HeaderValue::from_static("/tmp/Synap"),
        );
        let res = resolve_scope(&mut r, &h).unwrap();
        assert_eq!(res, ScopeResolution::Explicit);
        assert_eq!(r.scope.repo.as_deref(), Some("Cortex"));
    }

    #[tokio::test]
    async fn empty_query_short_circuits_with_400_outcome() {
        let svc = QueryService::with_memory_defaults(build_orchestrator());
        match svc.handle("dash", req("   ")).await {
            ServiceOutcome::EmptyQuery => (),
            other => panic!("expected EmptyQuery, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn deny_when_scope_repo_outside_acl() {
        let svc = QueryService::with_memory_defaults(build_orchestrator());
        svc.acl.set_allowed("dash", vec!["OnlyThisRepo".into()]);
        let mut r = req("x");
        r.scope.repo = Some("ForbiddenRepo".into());
        match svc.handle("dash", r).await {
            ServiceOutcome::Denied => (),
            other => panic!("expected Denied, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn scope_resolved_echoes_canonical_slug() {
        let svc = QueryService::with_memory_defaults(build_orchestrator());
        // Allow the repo through the ACL so the request reaches the
        // orchestrator instead of bouncing as Denied.
        svc.acl.set_allowed("dash", vec!["Vectorizer".into()]);
        let mut r = req("hello");
        r.scope.repo = Some("Vectorizer".into());
        match svc.handle("dash", r).await {
            ServiceOutcome::Ok(resp) => {
                // The slug runs through `slug_for_repo` so a
                // request that types "Vectorizer" resolves to the
                // lowercase ASCII form the per-project lane keys.
                assert_eq!(
                    resp.scope_resolved.repo.as_deref(),
                    Some("vectorizer"),
                    "scope_resolved.repo should be the slug the lanes filter on"
                );
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn scope_resolved_stays_empty_when_request_omits_repo() {
        let svc = QueryService::with_memory_defaults(build_orchestrator());
        match svc.handle("dash", req("hello")).await {
            ServiceOutcome::Ok(resp) => {
                assert!(resp.scope_resolved.repo.is_none());
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn cache_hit_carries_hit_label_and_skips_orchestrator() {
        let svc = QueryService::with_memory_defaults(build_orchestrator());
        let r = req("first");
        let _ = svc.handle("dash", r.clone()).await;
        match svc.handle("dash", r).await {
            ServiceOutcome::Ok(resp) => assert_eq!(resp.budget.cache, "hit"),
            other => panic!("expected Ok hit, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn audit_publishes_one_envelope_per_request() {
        let orchestrator = build_orchestrator();
        let audit = Arc::new(MemoryAuditPublisher::new());
        let svc = QueryService {
            orchestrator,
            cache: Arc::new(InMemoryCache::new()),
            acl: Arc::new(AclStore::new()),
            rate_limiter: Arc::new(RateLimiter::new(RateConfig::default_for_spec_11())),
            audit: audit.clone(),
            audit_store: Arc::new(AuditStore::new()),
            indexed_repos: None,
            coverage_snapshot: None,
            principal_store: None,
            api_key_store: None,
        };
        let _ = svc.handle("dash", req("x")).await;
        let snap = audit.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0]["caller"], "dash");
    }

    #[tokio::test]
    async fn audit_store_records_envelopes_alongside_publisher() {
        // Phase10j — the in-memory store backs `/v1/audit/{query_id}`
        // synchronously. Both sinks must see every envelope so the
        // dual lookup paths (synap-driven dashboard and
        // synchronous MCP audit tool) stay consistent.
        let orchestrator = build_orchestrator();
        let audit = Arc::new(MemoryAuditPublisher::new());
        let store = Arc::new(AuditStore::new());
        let svc = QueryService {
            orchestrator,
            cache: Arc::new(InMemoryCache::new()),
            acl: Arc::new(AclStore::new()),
            rate_limiter: Arc::new(RateLimiter::new(RateConfig::default_for_spec_11())),
            audit: audit.clone(),
            audit_store: store.clone(),
            indexed_repos: None,
            coverage_snapshot: None,
            principal_store: None,
            api_key_store: None,
        };
        let mut r = req("hello-world");
        r.scope.repo = Some("Vectorizer".into());
        let outcome = svc.handle("dash", r).await;
        let resp = match outcome {
            ServiceOutcome::Ok(b) => b,
            other => panic!("expected Ok, got {other:?}"),
        };
        let stored = store
            .get(&resp.query_id)
            .expect("audit store should retain envelope by query_id");
        assert_eq!(stored["caller"], "dash");
        assert_eq!(stored["query_id"], resp.query_id);
        // Publisher saw the same envelope.
        let snap = audit.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0]["query_id"], resp.query_id);
    }

    #[tokio::test]
    async fn notice_fires_when_scope_repo_missing_from_indexed_snapshot() {
        // The lane snapshot only knows about `cortex` (slug-form). A
        // request scoped to a different repo must surface a
        // `repo_not_indexed` notice so the caller can distinguish
        // empty results from "we never saw this repo".
        let lane = Arc::new(MemoryKeywordLane::new());
        lane.seed(
            "cortex-code",
            vec![LaneHit {
                doc_id: "h1".into(),
                text: "indexed".into(),
                repo: Some("Cortex".into()),
                path: None,
                symbol: None,
                content_hash: None,
                score: 0.9,
                ts: 0,
                severity: None,
                extras: Default::default(),
                overlay: crate::lanes::Overlay::default(),
            }],
        );
        let svc = QueryService::with_memory_defaults(build_orchestrator()).with_indexed_repos(lane);
        let mut r = req("anything");
        r.scope.repo = Some("UnknownRepo".into());
        match svc.handle("dash", r).await {
            ServiceOutcome::Ok(resp) => {
                let n = resp.notice.expect("expected repo_not_indexed notice");
                assert_eq!(n.code, "repo_not_indexed");
                assert!(
                    n.message.contains("unknownrepo"),
                    "notice should reference the canonicalised slug: {}",
                    n.message
                );
                assert!(
                    n.hint.contains("cortex-bootstrap"),
                    "hint should point at the bootstrap CLI: {}",
                    n.hint
                );
            }
            other => panic!("expected Ok with notice, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn notice_absent_when_scope_repo_is_in_indexed_snapshot() {
        let lane = Arc::new(MemoryKeywordLane::new());
        lane.seed(
            "cortex-code",
            vec![LaneHit {
                doc_id: "h1".into(),
                text: "indexed".into(),
                repo: Some("Cortex".into()),
                path: None,
                symbol: None,
                content_hash: None,
                score: 0.9,
                ts: 0,
                severity: None,
                extras: Default::default(),
                overlay: crate::lanes::Overlay::default(),
            }],
        );
        let svc = QueryService::with_memory_defaults(build_orchestrator()).with_indexed_repos(lane);
        let mut r = req("anything");
        r.scope.repo = Some("Cortex".into());
        match svc.handle("dash", r).await {
            ServiceOutcome::Ok(resp) => {
                assert!(
                    resp.notice.is_none(),
                    "no notice expected when scope.repo matches the snapshot, got {:?}",
                    resp.notice
                );
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn free_search_response_clipped_to_caller_budget_bytes() {
        // Phase11c — free_search is the intent that historically blew
        // past the MCP transport cap. The service-level clipper runs
        // before audit + cache, so a tiny `budget_bytes` on the
        // request must produce a serialised response inside the cap
        // and stamp `clipped` describing the trim.
        use crate::lanes::LaneHit;

        let v = Arc::new(MemoryVectorLane::new());
        let k = Arc::new(MemoryKeywordLane::new());
        let g = Arc::new(MemoryGraphLane::new());
        // Seed enough fat hits that the unclipped response easily
        // overshoots a 4 KiB budget.
        let big_text = "x".repeat(800);
        let mut hits = Vec::new();
        for i in 0..30 {
            hits.push(LaneHit {
                doc_id: format!("doc-{i}"),
                text: big_text.clone(),
                repo: Some("Cortex".into()),
                path: Some(format!("src/file_{i}.rs")),
                symbol: None,
                content_hash: None,
                score: 1.0 - (i as f64 * 0.01),
                ts: 0,
                severity: None,
                extras: Default::default(),
                overlay: crate::lanes::Overlay::default(),
            });
        }
        v.seed("cortex-cortex-code", hits);
        let svc = QueryService::with_memory_defaults(Orchestrator::new(v, k, g));

        let r = QueryRequest {
            intent: Intent::FreeSearch,
            scope: Scope {
                repo: Some("Cortex".into()),
                ..Scope::default()
            },
            query: "anything".into(),
            limit: 30,
            k: 30,
            include: vec![IncludeField::Snippets],
            budget_ms: 500,
            budget_bytes: Some(4096),
            as_of: None,
            branch: None,
            projects: None,
            include_history: None,
            include_future: None,
            include_branches: None,
            principal: None,
        };
        match svc.handle("dash", r).await {
            ServiceOutcome::Ok(resp) => {
                let serialised = serde_json::to_vec(&*resp).expect("serialisable");
                assert!(
                    serialised.len() <= 4096,
                    "clipped response must fit caller budget: got {} bytes for cap 4096",
                    serialised.len()
                );
                let report = resp
                    .clipped
                    .as_ref()
                    .expect("clipper must stamp report when it removes anything");
                assert!(
                    report.removed_snippets > 0 || report.snippets_text_clipped > 0,
                    "clipper should record what was trimmed: {report:?}"
                );
                assert_eq!(report.budget_bytes, 4096);
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn scope_unset_notice_fires_when_request_omits_scope_repo() {
        // Spec 11 §Failure modes — a missing `scope.repo` makes the
        // strategies layer slugify to the `unknown` family, so every
        // lane returns zero hits. The service surfaces that as a
        // `scope_unset` notice (instead of a silent empty success)
        // so MCP / dashboard callers can render an actionable hint.
        let lane = Arc::new(MemoryKeywordLane::new());
        let svc = QueryService::with_memory_defaults(build_orchestrator()).with_indexed_repos(lane);
        match svc.handle("dash", req("anything")).await {
            ServiceOutcome::Ok(resp) => {
                let n = resp.notice.expect("expected scope_unset notice");
                assert_eq!(n.code, "scope_unset");
                assert!(
                    !n.hint.is_empty(),
                    "scope_unset notice must carry a remediation hint"
                );
            }
            other => panic!("expected Ok, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn rate_limit_drops_calls_beyond_burst() {
        let orchestrator = build_orchestrator();
        let svc = QueryService {
            orchestrator,
            cache: Arc::new(InMemoryCache::new()),
            acl: Arc::new(AclStore::new()),
            rate_limiter: Arc::new(RateLimiter::new(RateConfig {
                rps_sustained: 1,
                rps_burst: 2,
            })),
            audit: Arc::new(MemoryAuditPublisher::new()),
            audit_store: Arc::new(AuditStore::new()),
            indexed_repos: None,
            coverage_snapshot: None,
            principal_store: None,
            api_key_store: None,
        };
        // Use distinct queries so the cache doesn't short-circuit
        // (cache hits reuse the rate-limit token on the spec-11 path
        // — the limiter still admits before lookup).
        let _ = svc.handle("c", req("a")).await;
        let _ = svc.handle("c", req("b")).await;
        let third = svc.handle("c", req("c")).await;
        match third {
            ServiceOutcome::RateLimited(_) => (),
            other => panic!("expected RateLimited, got {other:?}"),
        }
    }
}
