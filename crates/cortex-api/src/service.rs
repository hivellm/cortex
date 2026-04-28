//! Top-level query service — composes orchestrator + cache + ACL +
//! rate limiter + redaction + audit. The HTTP handler and the MCP
//! tool binding call into this single entry point so behaviour stays
//! identical between transports.

use std::sync::Arc;

use crate::acl::{AclDecision, AclStore};
use crate::audit::{build_envelope, AuditPublisher, MemoryAuditPublisher};
use crate::cache::{cache_key, Cache, InMemoryCache};
use crate::lanes::MemoryKeywordLane;
use crate::orchestrator::Orchestrator;
use crate::rate_limit::{RateConfig, RateDecision, RateLimiter};
use crate::redaction::redact_response;
use crate::types::{Notice, QueryRequest, QueryResponse, Scope};

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

    /// Run one request through the full pipeline.
    pub async fn handle(&self, caller: &str, req: QueryRequest) -> ServiceOutcome {
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
                .publish(build_envelope(caller, hit.intent.as_str(), &hit))
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
            .publish(build_envelope(caller, req.intent.label(), &response))
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
