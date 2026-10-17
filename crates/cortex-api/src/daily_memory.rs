//! Durable local daily memory, independent of optional search services.
use crate::{
    http::ApiState,
    security::{acl::AclDecision, principal::can_read},
};
use axum::{
    extract::{Json, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use cortex_storage::{
    daily_memory::{
        MemoryDeliveredItem, MemoryDelivery, MemoryDeliveryRequest, MemoryError, MemoryQuery,
        MemoryRecord, MemoryScope,
    },
    MetadataStore,
};
use serde::Deserialize;
use serde_json::json;
use std::{
    num::NonZeroUsize,
    path::PathBuf,
    sync::{Arc, LazyLock, Mutex},
    time::Duration,
};
use tokio::sync::Semaphore;

const QUERY_CACHE_ENTRIES: usize = 256;

#[derive(Clone, Hash, PartialEq, Eq)]
struct MemoryCacheKey {
    caller: String,
    repo: String,
    branch: Option<String>,
    paths: Vec<String>,
    query: String,
    include_global: bool,
    limit: usize,
    budget_bytes: usize,
    revision: u64,
    global_revision: Option<u64>,
}

impl MemoryCacheKey {
    fn new(
        request: &MemoryQueryRequest,
        caller: &str,
        revision: u64,
        global_revision: Option<u64>,
    ) -> Self {
        Self {
            caller: caller.to_owned(),
            repo: request.repo.clone(),
            branch: request.branch.clone(),
            paths: request.paths.clone(),
            query: request.query.clone(),
            include_global: request.include_global,
            limit: request.limit.min(5),
            budget_bytes: request.budget_bytes.min(1536),
            revision,
            global_revision,
        }
    }
}

#[derive(Clone)]
struct CachedMemoryContext {
    context: String,
    memories: Vec<MemoryDeliveredItem>,
}

/// Explicit file-backed metadata location and bounded operation admission.
#[derive(Clone)]
pub struct DailyMemoryState {
    path: Arc<PathBuf>,
    slots: Arc<Semaphore>,
    advice: Option<Arc<crate::memory_advice::MemoryAdviceState>>,
    query_cache: Arc<Mutex<lru::LruCache<MemoryCacheKey, CachedMemoryContext>>>,
}
impl DailyMemoryState {
    pub(crate) fn database_path(&self) -> PathBuf {
        self.path.as_ref().clone()
    }

    /// Configure durable storage, without an in-memory fallback.
    #[must_use]
    pub fn new(path: PathBuf) -> Self {
        Self {
            path: Arc::new(path),
            slots: Arc::new(Semaphore::new(4)),
            advice: None,
            query_cache: Arc::new(Mutex::new(lru::LruCache::new(
                NonZeroUsize::new(QUERY_CACHE_ENTRIES).expect("cache capacity is nonzero"),
            ))),
        }
    }
    /// Explicit production opt-in; ordinary/test constructors never inspect credentials.
    #[must_use]
    pub fn with_advice_from_env(mut self) -> Self {
        self.advice = Some(Arc::new(crate::memory_advice::MemoryAdviceState::from_env()));
        self
    }

    pub(crate) async fn capture_advice(
        &self,
        scope: cortex_storage::daily_memory::MemoryScope,
        receipt: cortex_storage::daily_memory::MemoryReceipt,
    ) -> serde_json::Value {
        match &self.advice {
            Some(advice) => advice.enrich(self, scope, receipt).await,
            None => crate::memory_advice::unavailable("disabled"),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_advice_state(
        mut self,
        advice: crate::memory_advice::MemoryAdviceState,
    ) -> Self {
        self.advice = Some(Arc::new(advice));
        self
    }
    /// Run bounded SQLite work off the async executor.
    pub(crate) async fn run<T, F>(&self, operation: F) -> Result<T, Response>
    where
        T: Send + 'static,
        F: FnOnce(&mut MetadataStore) -> Result<T, MemoryError> + Send + 'static,
    {
        if self.path.as_os_str().is_empty() || self.path.to_str() == Some(":memory:") {
            return Err(unavailable());
        }
        let permit = tokio::time::timeout(
            Duration::from_millis(100),
            self.slots.clone().acquire_owned(),
        )
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?;
        let path = self.path.clone();
        let operation = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            let mut store = MetadataStore::open_with_busy_timeout(&path, Duration::from_millis(50))
                .map_err(|_| Box::new(unavailable()))?;
            operation(&mut store).map_err(|error| Box::new(memory_error(error)))
        });
        tokio::time::timeout(Duration::from_millis(250), operation)
            .await
            .map_err(|_| unavailable())?
            .map_err(|_| unavailable())?
            .map_err(|error| *error)
    }
}
pub(crate) fn unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(json!({"error":"daily_memory_unavailable"})),
    )
        .into_response()
}
fn memory_error(error: MemoryError) -> Response {
    let status = match &error {
        MemoryError::InvalidInput(_) => StatusCode::BAD_REQUEST,
        MemoryError::NotFound => StatusCode::NOT_FOUND,
        MemoryError::AlreadyExists
        | MemoryError::RevisionConflict { .. }
        | MemoryError::IdempotencyConflict => StatusCode::CONFLICT,
        MemoryError::Sqlite(_) | MemoryError::Json(_) => return unavailable(),
    };
    (
        status,
        Json(json!({"error":"daily_memory_rejected", "message":error.to_string()})),
    )
        .into_response()
}
/// Identity resolved once from transport credentials, never client agent labels.
pub(crate) struct MemoryIdentity {
    principal: crate::security::principal::Principal,
    caller: String,
    authenticated: bool,
}

static AUTH_SLOTS: LazyLock<Arc<Semaphore>> = LazyLock::new(|| Arc::new(Semaphore::new(4)));

/// Resolve expensive authentication on bounded blocking workers, outside Tokio executors.
pub(crate) async fn resolve_identity(
    state: &ApiState,
    headers: &HeaderMap,
) -> Result<MemoryIdentity, Response> {
    let permit = tokio::time::timeout(
        Duration::from_millis(100),
        AUTH_SLOTS.clone().acquire_owned(),
    )
    .await
    .map_err(|_| unavailable())?
    .map_err(|_| unavailable())?;
    let state = state.clone();
    let headers = headers.clone();
    let operation = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        resolve_identity_blocking(&state, &headers).map_err(Box::new)
    });
    tokio::time::timeout(Duration::from_secs(2), operation)
        .await
        .map_err(|_| unavailable())?
        .map_err(|_| unavailable())?
        .map_err(|error| *error)
}

/// Authorize transport identity for an optional repository and explicit global operation.
pub(crate) async fn authorize(
    state: &ApiState,
    headers: &HeaderMap,
    repo: Option<&str>,
    global: bool,
) -> Result<String, Response> {
    let identity = resolve_identity(state, headers).await?;
    check_access(state, &identity, repo, global)
}

#[allow(clippy::result_large_err)]
fn resolve_identity_blocking(
    state: &ApiState,
    headers: &HeaderMap,
) -> Result<MemoryIdentity, Response> {
    let unauthorized = || {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error":"authentication_required"})),
        )
            .into_response()
    };
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "));
    let trusted_header = crate::security::principal_resolver::trust_principal_header_enabled()
        && headers
            .get("x-cortex-principal")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|s| !s.trim().is_empty())
        && state.service.principal_store.is_some();
    let verified_key = if headers.contains_key("authorization") {
        let token = token.filter(|s| !s.is_empty()).ok_or_else(unauthorized)?;
        let store = state
            .service
            .api_key_store
            .as_ref()
            .ok_or_else(unauthorized)?;
        Some(store.verify_with_role(token).map_err(|_| unauthorized())?)
    } else {
        None
    };
    let authenticated = verified_key.is_some() || trusted_header;
    if !authenticated
        && state.cfg.access_control.enabled
        && state.cfg.access_control.deny_on_missing_principal
    {
        return Err(unauthorized());
    }
    let principal = if trusted_header {
        state.service.resolve_principal(headers)
    } else if let Some((key_id, role)) = &verified_key {
        match &state.service.principal_store {
            Some(store) => store
                .read()
                .map_err(|_| unavailable())?
                .resolve(key_id, &role.iter().cloned().collect::<Vec<_>>()),
            None => crate::security::principal::Principal::public_only(key_id),
        }
    } else {
        state.service.resolve_principal(headers)
    };
    let caller = if trusted_header {
        principal.id.clone()
    } else if let Some((id, _)) = verified_key {
        id
    } else {
        "transport:local".to_owned()
    };
    Ok(MemoryIdentity {
        principal,
        caller,
        authenticated,
    })
}

/// Recheck scope without repeating expensive credential verification.
#[allow(clippy::result_large_err)]
pub(crate) fn check_access(
    state: &ApiState,
    identity: &MemoryIdentity,
    repo: Option<&str>,
    global: bool,
) -> Result<String, Response> {
    let forbidden = || {
        (
            StatusCode::FORBIDDEN,
            Json(json!({"error":"scope_forbidden"})),
        )
            .into_response()
    };
    if global
        && (!identity.authenticated
            || state.service.principal_store.is_none()
            || !identity.principal.is_acl_admin())
    {
        return Err(forbidden());
    }
    if state.cfg.access_control.enabled
        && !can_read(
            &identity.principal,
            state.cfg.access_control.default_level,
            &state.cfg.access_control.default_compartments,
        )
    {
        return Err(forbidden());
    }
    if let Some(repo) = repo {
        if matches!(
            state.service.acl.decide(&identity.caller, Some(repo)),
            AclDecision::Deny
        ) {
            return Err(forbidden());
        }
    }
    Ok(identity.caller.clone())
}
/// Bounded explicit repository lookup.
#[derive(Debug, Deserialize)]
pub struct MemoryQueryRequest {
    /// Exact repository identity.
    pub repo: String,
    /// Current task text.
    pub query: String,
    /// Current branch.
    pub branch: Option<String>,
    /// Current repository-relative paths.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Explicit authorized global opt-in.
    #[serde(default)]
    pub include_global: bool,
    /// Maximum rendered records, capped at five.
    #[serde(default = "default_limit")]
    pub limit: usize,
    /// UTF-8 context budget including citations, capped at 1536.
    #[serde(default = "default_budget")]
    pub budget_bytes: usize,
}
fn default_limit() -> usize {
    5
}
fn default_budget() -> usize {
    1536
}

fn label(value: &str, bytes: usize) -> String {
    let encoded = serde_json::to_string(value).expect("string serialization cannot fail");
    if encoded.len() <= bytes {
        encoded
    } else {
        let mut clipped = String::new();
        let mut length = 2;
        for character in value.chars() {
            let encoded_bytes = match character {
                '"' | '\\' | '\n' | '\r' | '\t' | '\u{0008}' | '\u{000c}' => 2,
                c if c < ' ' => 6,
                c => c.len_utf8(),
            };
            if length + encoded_bytes + 3 > bytes {
                break;
            }
            clipped.push(character);
            length += encoded_bytes;
        }
        clipped.push_str("...");
        serde_json::to_string(&clipped).expect("string serialization cannot fail")
    }
}
/// Persist exact prepared delivery before returning cited context.
pub async fn handle_query(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<MemoryQueryRequest>,
) -> Response {
    let caller = match authorize(&state, &headers, Some(&req.repo), req.include_global).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    let Some(local) = &state.daily_memory else {
        return unavailable();
    };
    let cache = local.query_cache.clone();
    let result = local
        .run(move |store| {
            let (revision, global_revision) =
                current_revisions(store, &req.repo, req.include_global)?;
            let observed_key = MemoryCacheKey::new(&req, &caller, revision, global_revision);
            let cached = cache
                .lock()
                .ok()
                .and_then(|mut entries| entries.get(&observed_key).cloned());
            if let Some(rendered) = cached {
                match prepare_delivery(store, &observed_key, &rendered) {
                    Ok(delivery) => return Ok(query_response(rendered, delivery, true)),
                    // A writer changed the snapshot after the revision read. Rebuild once.
                    Err(MemoryError::RevisionConflict { .. }) => {}
                    Err(error) => return Err(error),
                }
            }
            let search = store.query_daily_memories(&MemoryQuery {
                repo: req.repo.clone(),
                query: req.query,
                branch: req.branch,
                paths: req.paths,
                include_global: req.include_global,
                limit: req.limit.min(5),
            })?;
            // The query can observe a newer snapshot than the initial cache lookup.
            let key = MemoryCacheKey {
                revision: search.revision,
                global_revision: search.global_revision,
                ..observed_key
            };
            let rendered = render_memories(search.records, key.budget_bytes)?;
            let delivery = prepare_delivery(store, &key, &rendered)?;
            if let Ok(mut entries) = cache.lock() {
                entries.put(key, rendered.clone());
            }
            Ok(query_response(rendered, delivery, false))
        })
        .await;
    match result {
        Ok(body) => Json(body).into_response(),
        Err(response) => response,
    }
}

fn current_revisions(
    store: &MetadataStore,
    repo: &str,
    include_global: bool,
) -> Result<(u64, Option<u64>), MemoryError> {
    let tx = store.conn().unchecked_transaction()?;
    let revision = store.daily_memory_revision(&MemoryScope::Repository(repo.to_owned()))?;
    let global_revision = if include_global {
        Some(store.daily_memory_revision(&MemoryScope::Global)?)
    } else {
        None
    };
    tx.commit()?;
    Ok((revision, global_revision))
}

fn prepare_delivery(
    store: &mut MetadataStore,
    key: &MemoryCacheKey,
    rendered: &CachedMemoryContext,
) -> Result<MemoryDelivery, MemoryError> {
    store.record_daily_memory_delivery(&MemoryDeliveryRequest {
        repo: key.repo.clone(),
        caller: key.caller.clone(),
        revision: key.revision,
        global_revision: key.global_revision,
        memories: rendered.memories.clone(),
        context_bytes: rendered.context.len(),
        channel: "http".to_owned(),
    })
}

fn query_response(
    rendered: CachedMemoryContext,
    delivery: MemoryDelivery,
    cache_hit: bool,
) -> serde_json::Value {
    json!({"context":rendered.context,"memories":rendered.memories,"revision":delivery.request.revision,"global_revision":delivery.request.global_revision,"delivery_id":delivery.delivery_id,"query_id":delivery.delivery_id,"stage":delivery.stage,"prepared_at":delivery.prepared_at,"cache_hit":cache_hit})
}

fn render_memories(
    records: Vec<MemoryRecord>,
    budget: usize,
) -> Result<CachedMemoryContext, MemoryError> {
    let mut context = String::new();
    let mut memories = Vec::new();
    for record in records {
        let scope = match &record.scope {
            MemoryScope::Repository(repo) => format!("repo:{}", label(repo, 280)),
            MemoryScope::Global => "global".to_owned(),
        };
        let source = record
            .sources
            .first()
            .map(|source| {
                format!(
                    " source={} kind={} ref={} revision={} sources={}",
                    label(&source.event_id, 280),
                    serde_json::to_string(&source.kind).unwrap_or_default(),
                    label(source.reference.as_deref().unwrap_or("unknown"), 96),
                    label(source.revision.as_deref().unwrap_or("unknown"), 48),
                    record.sources.len()
                )
            })
            .unwrap_or_default();
        let prefix = format!("[memory {} v{} event {} scope={}] origin={:?} agent={} session={} turn={} title={}{}\n", serde_json::to_string(&record.id)?, record.version, record.event_id, scope, record.origin.attribution,
            label(record.origin.agent.as_deref().unwrap_or("unknown"), 64), label(record.origin.session_id.as_deref().unwrap_or("unknown"), 48), label(record.origin.turn_id.as_deref().unwrap_or("unknown"), 48), label(&record.title, 80), source);
        let separator = if context.is_empty() { "" } else { "\n" };
        let remaining = budget.saturating_sub(context.len() + separator.len() + prefix.len() + 1);
        if remaining == 0 {
            continue;
        }
        let truncated = record.body.len() > remaining;
        let body_budget = if truncated {
            remaining.saturating_sub(3)
        } else {
            remaining
        };
        let mut end = record.body.len().min(body_budget);
        while !record.body.is_char_boundary(end) {
            end -= 1;
        }
        if end == 0 {
            continue;
        }
        context.push_str(separator);
        context.push_str(&prefix);
        context.push_str(&record.body[..end]);
        if truncated {
            context.push_str("...");
        }
        context.push('\n');
        memories.push(MemoryDeliveredItem {
            scope: record.scope,
            id: record.id,
            version: record.version,
            event_id: record.event_id,
        });
    }
    Ok(CachedMemoryContext { context, memories })
}
