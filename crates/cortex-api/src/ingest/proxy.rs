//! Local durable capture behind the legacy `POST /v1/ingest` surface.
use crate::{
    daily_memory::{authorize, unavailable},
    http::ApiState,
};
use axum::{
    extract::{Json, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use cortex_core::events::Kind;
use cortex_storage::daily_memory::{
    MemoryCapture, MemoryCategory, MemoryOrigin, MemoryRetryKey, MemoryScope, MemorySource,
    MemoryStatus, OriginAttribution,
};
use serde::Deserialize;
use serde_json::json;

/// Maximum explicit captured body size.
pub const MAX_BODY_BYTES: usize = 8192;
/// Legacy endpoint constant; local capture does not contact it.
pub const DEFAULT_INGESTION_URL: &str = "http://127.0.0.1:17010";

/// Legacy capture shape plus additive typed local-memory metadata.
#[derive(Debug, Clone, Deserialize)]
pub struct IngestRequest {
    /// Legacy evidence kind or typed decision.
    pub kind: String,
    /// Exact user-authored content.
    pub body: String,
    /// Exact repository identity.
    pub repo: String,
    /// Opt in to external TypeSafe candidate-only classification after commit.
    /// False or omitted keeps capture local, even when provider credentials exist.
    #[serde(default)]
    pub classify: bool,
    /// Legacy topic, also used as default title.
    pub topic: Option<String>,
    /// Legacy severity hint.
    pub severity: Option<String>,
    /// Explicit short title.
    pub title: Option<String>,
    /// Create-only identity.
    #[serde(alias = "id")]
    pub memory_id: Option<String>,
    /// Initial canonical status.
    pub status: Option<MemoryStatus>,
    /// Client-reported origin, never authority.
    pub agent: Option<String>,
    /// Client-reported session, if observed.
    pub session_id: Option<String>,
    /// Client-reported turn, if observed.
    pub turn_id: Option<String>,
    /// Caller-scoped stable write retry key.
    pub retry_key: Option<String>,
    /// Branch applicability.
    pub branch: Option<String>,
    /// Relative path applicability.
    pub path: Option<String>,
    /// Explicit all-prompts applicability.
    #[serde(default)]
    pub pinned: bool,
    /// Original evidence references.
    #[serde(default)]
    pub sources: Vec<MemorySource>,
    /// Original legacy source identity, if supplied.
    pub event_id: Option<String>,
    /// Scope revision observed by writer.
    pub expected_revision: Option<u64>,
    /// Explicit accepted same-scope predecessor.
    pub supersedes: Option<String>,
    /// Explicit global write, requiring authenticated administrator.
    #[serde(default)]
    pub global: bool,
    /// Typed scope; repository identity must agree with `repo`.
    pub scope: Option<MemoryScope>,
}

/// Commit locally before acknowledging; indexing availability is irrelevant.
pub async fn handle_ingest(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(req): Json<IngestRequest>,
) -> Response {
    if req.body.len() > MAX_BODY_BYTES {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(json!({"error":"body_too_large","max_bytes":MAX_BODY_BYTES})),
        )
            .into_response();
    }
    let normalized_kind = req.kind.trim().to_ascii_lowercase();
    let kind = match normalized_kind.as_str() {
        "memory" => Kind::Memory,
        "knowledge" => Kind::Knowledge,
        "learning" => Kind::Learning,
        "decision" => Kind::Decision,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error":"unsupported_kind"})),
            )
                .into_response()
        }
    };
    let global = req.global || matches!(req.scope, Some(MemoryScope::Global));
    let caller = match authorize(&state, &headers, Some(&req.repo), global).await {
        Ok(c) => c,
        Err(r) => return r,
    };
    if matches!(&req.scope, Some(MemoryScope::Repository(repo)) if repo != &req.repo)
        || (req.global && matches!(req.scope, Some(MemoryScope::Repository(_))))
    {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({"error":"scope_mismatch"})),
        )
            .into_response();
    }
    let Some(local) = &state.daily_memory else {
        return unavailable();
    };
    let reported = req.agent.is_some() || req.session_id.is_some() || req.turn_id.is_some();
    let mut sources = req.sources;
    if let Some(event_id) = req.event_id {
        sources.push(MemorySource {
            event_id,
            kind: Some(kind),
            reference: None,
            content_hash: None,
            revision: None,
        });
    }
    let capture = MemoryCapture {
        id: req.memory_id,
        scope: if global {
            MemoryScope::Global
        } else {
            MemoryScope::Repository(req.repo)
        },
        title: req
            .title
            .or(req.topic)
            .unwrap_or_else(|| format!("Captured {normalized_kind}")),
        body: req.body,
        category: if kind == Kind::Decision {
            MemoryCategory::Decision
        } else {
            MemoryCategory::Lesson
        },
        capture_kind: Some(kind),
        status: req.status.unwrap_or(MemoryStatus::Accepted),
        origin: MemoryOrigin {
            agent: req.agent,
            session_id: req.session_id,
            turn_id: req.turn_id,
            attribution: if reported {
                OriginAttribution::ClientReported
            } else {
                OriginAttribution::Unknown
            },
        },
        sources,
        branch: req.branch,
        path: req.path,
        pinned: req.pinned,
        retry: req.retry_key.map(|key| MemoryRetryKey { caller, key }),
        expected_revision: req.expected_revision,
        supersedes: req.supersedes,
    };
    let scope = capture.scope.clone();
    match local
        .run(move |store| store.capture_daily_memory(&capture))
        .await
    {
        Ok(receipt) => {
            let advice = if req.classify {
                local.capture_advice(scope, receipt.clone()).await
            } else {
                crate::memory_advice::unavailable("not_requested")
            };
            (StatusCode::ACCEPTED, Json(json!({
            "event_id":receipt.event_id,"content_hash":receipt.content_hash,"indexed_at":null,
            "captured_at":receipt.captured_at,"memory_id":receipt.id,"id":receipt.id,"version":receipt.version,"scope_revision":receipt.scope_revision,
            "durability":"local_committed","mirror_status":"not_attempted","index_complete":false,
            "advice":advice
        }))).into_response()
        }
        Err(response) => response,
    }
}
