//! Public contracts for the local decision/lesson projection.

use cortex_core::events::{DecisionPayload, Kind};
use serde::{Deserialize, Serialize};

/// Explicit scope; an unknown repository never becomes global.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScope {
    /// Exact repository identity, without collection-slug normalization.
    Repository(String),
    /// Explicit global scope; authorization remains the caller's responsibility.
    Global,
}

/// Content category, sharing the canonical decision lifecycle.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryCategory {
    /// Architectural or implementation decision.
    Decision,
    /// Verified lesson or correction.
    Lesson,
}

/// Canonical statuses from `DecisionPayload`; active means accepted and eligible.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MemoryStatus {
    /// Not yet authoritative.
    Proposed,
    /// Explicitly accepted advice.
    Accepted,
    /// Replaced by an explicit successor.
    Superseded,
    /// Explicitly retired advice.
    Deprecated,
}

/// How origin attribution was obtained, not an authorization grant.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OriginAttribution {
    /// No observed origin metadata.
    #[default]
    Unknown,
    /// Client-reported label, not authenticated identity.
    ClientReported,
    /// Origin resolved by a trusted transport/adapter.
    TrustedTransport,
}

/// Origin already resolved by the API/adapter; storage never guesses Claude.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryOrigin {
    /// Observed agent label, when known.
    pub agent: Option<String>,
    /// Observed source session, when known.
    pub session_id: Option<String>,
    /// Observed source turn, when known.
    pub turn_id: Option<String>,
    /// Attribution provenance, not access rights.
    pub attribution: OriginAttribution,
}

/// Evidence reference retained without rewriting its original identity/kind.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemorySource {
    /// Original source event ID.
    pub event_id: String,
    /// Original event kind, including legacy memory/knowledge/learning.
    pub kind: Option<Kind>,
    /// Source URL or file reference.
    pub reference: Option<String>,
    /// Referenced content fingerprint when available.
    pub content_hash: Option<String>,
    /// Source commit or revision when available.
    pub revision: Option<String>,
}

/// Retry identity scoped by effective scope and caller namespace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRetryKey {
    /// Transport-resolved caller namespace, not model authority.
    pub caller: String,
    /// Stable key reused for the same write attempt.
    pub key: String,
}

/// Explicit create or correction, validated before a single atomic transaction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryCapture {
    /// Optional create-only ID; absent IDs are generated once and retained on retry.
    pub id: Option<String>,
    /// Explicit effective scope.
    pub scope: MemoryScope,
    /// Short title used for lookup and citations.
    pub title: String,
    /// Exact captured body, at most 8 KiB.
    pub body: String,
    /// Decision or lesson, not a new lifecycle.
    pub category: MemoryCategory,
    /// Observed explicit capture kind; canonical lifecycle evidence stays Decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_kind: Option<Kind>,
    /// Initial canonical status; direct creation as superseded is invalid.
    pub status: MemoryStatus,
    /// Already-resolved origin; absent fields remain unknown.
    pub origin: MemoryOrigin,
    /// Original evidence references.
    pub sources: Vec<MemorySource>,
    /// Optional exact branch restriction.
    pub branch: Option<String>,
    /// Optional repository-relative path or directory restriction, using `/`.
    pub path: Option<String>,
    /// Explicitly applicable to every prompt within its effective scope.
    pub pinned: bool,
    /// Stable retry identity, when provided by the client.
    pub retry: Option<MemoryRetryKey>,
    /// Scope revision observed by the writer; mandatory for supersession.
    pub expected_revision: Option<u64>,
    /// Same-scope accepted predecessor replaced by this accepted successor.
    pub supersedes: Option<String>,
}

impl MemoryCapture {
    /// Construct an explicit accepted repository memory with honestly unknown origin.
    #[must_use]
    pub fn new(repo: &str, title: &str, body: &str, category: MemoryCategory) -> Self {
        Self {
            id: None,
            scope: MemoryScope::Repository(repo.to_owned()),
            title: title.to_owned(),
            body: body.to_owned(),
            category,
            capture_kind: None,
            status: MemoryStatus::Accepted,
            origin: MemoryOrigin::default(),
            sources: Vec::new(),
            branch: None,
            path: None,
            pinned: false,
            retry: None,
            expected_revision: None,
            supersedes: None,
        }
    }
}

/// Durable receipt snapshot; retry returns this original value even after correction.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryReceipt {
    /// Stable memory/decision ID.
    pub id: String,
    /// Canonical decision evidence event ID, not an invented source session.
    pub event_id: String,
    /// SHA-256 of the canonical typed decision payload.
    pub content_hash: String,
    /// Server capture timestamp, RFC 3339 UTC.
    pub captured_at: String,
    /// Record version at the committed operation.
    pub version: u64,
    /// Scope revision after the operation.
    pub scope_revision: u64,
}

/// Current projection, including historical/stale rows for explicit lookup.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryRecord {
    /// Stable ID shared with canonical `DecisionPayload.decision_id`.
    pub id: String,
    /// Exact effective scope.
    pub scope: MemoryScope,
    /// User-authored title.
    pub title: String,
    /// User-authored body.
    pub body: String,
    /// Projection category.
    pub category: MemoryCategory,
    /// Original explicit wire kind, when the capture surface reported one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_kind: Option<Kind>,
    /// Canonical decision status.
    pub status: MemoryStatus,
    /// Original capture origin, retained across lifecycle changes.
    pub origin: MemoryOrigin,
    /// Original source identities and references.
    pub sources: Vec<MemorySource>,
    /// Optional exact branch restriction.
    pub branch: Option<String>,
    /// Optional relative path applicability.
    pub path: Option<String>,
    /// Explicit all-prompts applicability within scope.
    pub pinned: bool,
    /// Deterministically invalidated source evidence, never inferred by a model.
    pub stale: bool,
    /// Version increments on lifecycle or stale-state changes.
    pub version: u64,
    /// Original canonical capture evidence ID.
    pub event_id: String,
    /// Capture timestamp, not a claim about source-event occurrence.
    pub captured_at: String,
    /// Hash of the current typed decision payload.
    pub content_hash: String,
    /// Explicit predecessor, if this record corrects one.
    pub supersedes: Option<String>,
}

/// Canonical typed lifecycle evidence without a synthetic event envelope/session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MemoryEvidence {
    /// Evidence event ID.
    pub event_id: String,
    /// Canonical event kind, always `Decision` for this projection.
    pub kind: Kind,
    /// Observed capture kind; distinct from this canonical evidence's kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub capture_kind: Option<Kind>,
    /// Record version captured by this event.
    pub version: u64,
    /// Existing core decision contract.
    pub payload: DecisionPayload,
    /// Origin of this explicit operation, distinct from original capture origin.
    pub origin: MemoryOrigin,
    /// Original source identities/kinds.
    pub sources: Vec<MemorySource>,
    /// Event timestamp, RFC 3339 UTC.
    pub occurred_at: String,
    /// Stale-source state at this event version.
    pub stale: bool,
}

/// Scoped ordinary-text lookup; user input is never interpreted as FTS syntax.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemoryQuery {
    /// Exact repository identity; mandatory even when globals are requested.
    pub repo: String,
    /// Explicit opt-in to global records; false by default.
    #[serde(default)]
    pub include_global: bool,
    /// Ordinary task text, bounded to 4 KiB and 32 distinct terms.
    pub query: String,
    /// Current branch; absent excludes branch-restricted advice.
    pub branch: Option<String>,
    /// Current repository-relative file paths for path-restricted advice.
    pub paths: Vec<String>,
    /// Result count, clamped to 50; zero returns no records.
    pub limit: usize,
}

impl MemoryQuery {
    /// Query one repository with no implicit globals and at most five results.
    #[must_use]
    pub fn new(repo: &str, query: &str) -> Self {
        Self {
            repo: repo.to_owned(),
            include_global: false,
            query: query.to_owned(),
            branch: None,
            paths: Vec::new(),
            limit: 5,
        }
    }
}

/// Records and revisions read from the same SQLite snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MemorySearch {
    /// Repository revision at the read snapshot.
    pub revision: u64,
    /// Global revision only when explicitly included.
    pub global_revision: Option<u64>,
    /// Eligible accepted records in deterministic rank order.
    pub records: Vec<MemoryRecord>,
}
