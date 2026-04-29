//! Top-level query service — composes orchestrator + cache + ACL +
//! rate limiter + redaction + audit. The HTTP handler and the MCP
//! tool binding call into this single entry point so behaviour stays
//! identical between transports.

use std::sync::Arc;

use axum::http::HeaderMap;

use crate::acl::{AclDecision, AclStore};
use crate::audit::{build_envelope_with_audit_context, AuditPublisher, MemoryAuditPublisher};
use crate::cache::{cache_key, Cache, InMemoryCache};
use crate::lanes::MemoryKeywordLane;
use crate::orchestrator::Orchestrator;
use crate::rate_limit::{RateConfig, RateDecision, RateLimiter};
use crate::redaction::redact_response;
use crate::types::{Notice, QueryRequest, QueryResponse, Scope};

/// Phase4j header — explicit repo override for direct
/// `/v1/query` callers (MCP tools, dashboard search, GUI).
pub const HEADER_CORTEX_REPO: &str = "x-cortex-repo";

/// Phase4j header — caller's working directory; the resolver
/// derives a repo slug from the basename via
/// [`cortex_storage::names::slug_for_repo`].
pub const HEADER_CORTEX_CWD: &str = "x-cortex-cwd";

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
        let basename = std::path::Path::new(cwd)
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
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
    Scope {
        repo,
        files: req_scope.files.clone(),
        topics: req_scope.topics.clone(),
        since: req_scope.since.clone(),
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
    /// Snapshot of indexed-repo slugs the daemon currently sees. Read
    /// from the same `MemoryKeywordLane` the dashboard uses to derive
    /// `repos_indexed`, so the answer in `/v1/status.indexed_repos`
    /// and the trigger for `notice.code = "repo_not_indexed"` come
    /// from one source. `None` for unit tests that don't care about
    /// the lookup; production wires the lane through `main.rs`.
    pub indexed_repos: Option<Arc<MemoryKeywordLane>>,
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
            indexed_repos: None,
        }
    }

    /// Attach an indexed-repos snapshot. Returns `self` so callers can
    /// chain the wiring at startup (`QueryService::with_memory_defaults(...).with_indexed_repos(lane)`).
    pub fn with_indexed_repos(mut self, lane: Arc<MemoryKeywordLane>) -> Self {
        self.indexed_repos = Some(lane);
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
        self.handle_resolved(caller, req, ScopeResolution::Explicit).await
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
        self.handle_resolved(caller, req, resolution).await
    }

    async fn handle_resolved(
        &self,
        caller: &str,
        req: QueryRequest,
        resolution: ScopeResolution,
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
            self.audit
                .publish(build_envelope_with_audit_context(
                    caller,
                    hit.intent.as_str(),
                    &hit,
                    resolution.as_str(),
                    &self.orchestrator.fusion,
                ))
                .await;
            return ServiceOutcome::Ok(Box::new(hit));
        }

        let mut response = self.orchestrator.run(&req).await;
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
        self.cache.put(&key, response.clone()).await;
        self.audit
            .publish(build_envelope_with_audit_context(
                caller,
                req.intent.label(),
                &response,
                resolution.as_str(),
                &self.orchestrator.fusion,
            ))
            .await;
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
    use crate::lanes::{
        LaneHit, MemoryGraphLane, MemoryKeywordLane, MemoryVectorLane,
    };
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
            indexed_repos: None,
        };
        let _ = svc.handle("dash", req("x")).await;
        let snap = audit.snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0]["caller"], "dash");
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
            }],
        );
        let svc = QueryService::with_memory_defaults(build_orchestrator())
            .with_indexed_repos(lane);
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
            }],
        );
        let svc = QueryService::with_memory_defaults(build_orchestrator())
            .with_indexed_repos(lane);
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
    async fn scope_unset_notice_fires_when_request_omits_scope_repo() {
        // Spec 11 §Failure modes — a missing `scope.repo` makes the
        // strategies layer slugify to the `unknown` family, so every
        // lane returns zero hits. The service surfaces that as a
        // `scope_unset` notice (instead of a silent empty success)
        // so MCP / dashboard callers can render an actionable hint.
        let lane = Arc::new(MemoryKeywordLane::new());
        let svc = QueryService::with_memory_defaults(build_orchestrator())
            .with_indexed_repos(lane);
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
            indexed_repos: None,
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
