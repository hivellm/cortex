//! Transport-independent canonical records and bounded query contracts.
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const MAX_RECORD_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_BODY_BYTES: usize = 1024 * 1024;
pub const MAX_BATCH_RECORDS: usize = 256;
pub const MAX_BATCH_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_SEARCH_RESULTS: usize = 100;
pub const MAX_GRAPH_RESULTS: usize = 200;
pub const MAX_GRAPH_DEPTH: usize = 5;

#[derive(Debug, thiserror::Error)]
pub enum LocalError {
    #[error("local I/O: {0}")]
    Io(#[from] std::io::Error),
    #[error("local index: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("canonical JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid local record: {0}")]
    Invalid(String),
    #[error("record conflict: {0}")]
    Conflict(String),
    #[error("record not found: {0}")]
    NotFound(String),
}

pub type Result<T> = std::result::Result<T, LocalError>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordKind {
    Event,
    Decision,
    Lesson,
    Preference,
    #[default]
    Note,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Authority {
    #[default]
    Observation,
    Proposed,
    Accepted,
    Deprecated,
    Superseded,
}

impl Authority {
    pub(crate) fn active(self) -> bool {
        !matches!(self, Self::Deprecated | Self::Superseded)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    RelatedTo,
    Supports,
    Contradicts,
    DerivedFrom,
    Supersedes,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypedLink {
    pub target_id: String,
    pub relation: Relation,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Source {
    pub kind: String,
    pub locator: String,
    pub source_id: Option<String>,
    pub captured_at: Option<String>,
    pub source_hash: Option<String>,
    #[serde(default)]
    pub payload: Value,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CaptureRequest {
    pub id: Option<String>,
    pub repository: String,
    pub kind: RecordKind,
    pub authority: Authority,
    pub title: String,
    pub body: String,
    pub source: Source,
    #[serde(default)]
    pub links: Vec<TypedLink>,
    #[serde(default)]
    pub metadata: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub schema_version: u32,
    pub id: String,
    pub repository: String,
    pub revision: u64,
    pub kind: RecordKind,
    pub authority: Authority,
    pub title: String,
    pub body: String,
    pub source: Source,
    pub links: Vec<TypedLink>,
    pub metadata: Value,
    pub created_at: String,
    pub updated_at: String,
    pub content_sha256: String,
}

impl Record {
    pub(crate) fn recall_eligible(&self) -> bool {
        let applicability = &self.metadata["applicability"];
        self.authority.active()
            && (applicability["stale"].is_null() || applicability["stale"].as_bool() == Some(false))
            && applicability["branch"].is_null()
            && applicability["path"].is_null()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevisionRequest {
    pub title: String,
    pub body: String,
    pub authority: Authority,
    pub links: Vec<TypedLink>,
    pub metadata: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteOutcome {
    pub record: Record,
    pub created: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BatchOutcome {
    pub records: Vec<WriteOutcome>,
    pub created: usize,
    pub duplicates: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchQuery {
    pub repository: String,
    pub text: String,
    pub limit: usize,
    #[serde(default)]
    pub include_inactive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchHit {
    pub record: Record,
    pub score: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphQuery {
    pub repository: String,
    pub id: String,
    pub depth: usize,
    pub limit: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GraphEdge {
    pub source_id: String,
    pub target_id: String,
    pub relation: Relation,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphResult {
    pub records: Vec<Record>,
    pub edges: Vec<GraphEdge>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReindexReport {
    pub files: usize,
    pub revisions: usize,
    pub records: usize,
    pub dangling_links: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StoreStatus {
    pub records: usize,
    pub revisions: usize,
    pub repositories: usize,
    pub events: usize,
    pub memories: usize,
    pub links: usize,
}
