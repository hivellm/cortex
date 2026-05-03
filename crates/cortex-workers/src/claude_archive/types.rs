//! JSONL record shapes — phase11i §1.
//!
//! Mirrors what Claude Code v2.1.x writes to
//! `~/.claude/projects/<project>/<session>.jsonl`. Seven distinct
//! record types share a common envelope of stable fields
//! (`sessionId`, `parentUuid`, `timestamp`, `cwd`, `gitBranch`,
//! `version`, `entrypoint`, `userType`). The remaining fields are
//! discriminated by the top-level `type` (and `subtype` /
//! `attachment.type` for the multi-shape variants).

use std::fmt;

use serde::Deserialize;
use thiserror::Error;

/// Errors raised by every phase of the ingest pipeline. The
/// reader-side errors stay tolerant — corrupt lines are logged +
/// counted, never panic'd. The mapper-side errors are the only
/// ones that can fail a record outright.
#[derive(Debug, Error)]
pub enum ArchiveError {
    /// The file at `path` could not be opened / read.
    #[error("io error reading {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },
    /// A JSONL line failed to parse. The reader continues with the
    /// next line; this variant is surfaced via the `errors` counter
    /// in [`super::reader::ReadStats`].
    #[error("malformed JSONL at {path}:{line}: {source}")]
    MalformedJson {
        path: String,
        line: usize,
        #[source]
        source: serde_json::Error,
    },
    /// A record's `type` discriminant was missing or unknown. The
    /// reader counts it but does not return — the dropper logs and
    /// moves on.
    #[error("unknown record type at {path}:{line}: {tag}")]
    UnknownKind {
        path: String,
        line: usize,
        tag: String,
    },
    /// The mapper rejected an envelope projection. Surfaces in
    /// [`super::mapper::MapStats::dropped_records`].
    #[error("mapper rejection: {reason}")]
    MapperRejection { reason: String },
}

/// Top-level `type` discriminator after parsing one JSONL line.
/// `Unknown` carries the raw tag so logs can identify new variants
/// future Claude Code releases introduce.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordKind {
    User,
    Assistant,
    Attachment,
    System,
    FileHistorySnapshot,
    LastPrompt,
    QueueOperation,
    Unknown(String),
}

impl fmt::Display for RecordKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecordKind::User => write!(f, "user"),
            RecordKind::Assistant => write!(f, "assistant"),
            RecordKind::Attachment => write!(f, "attachment"),
            RecordKind::System => write!(f, "system"),
            RecordKind::FileHistorySnapshot => write!(f, "file-history-snapshot"),
            RecordKind::LastPrompt => write!(f, "last-prompt"),
            RecordKind::QueueOperation => write!(f, "queue-operation"),
            RecordKind::Unknown(tag) => write!(f, "unknown:{tag}"),
        }
    }
}

impl RecordKind {
    /// Parse from the raw `type` string. Defensive — unrecognised
    /// tags fall through to `Unknown` so the reader keeps moving.
    pub fn from_tag(tag: &str) -> Self {
        match tag {
            "user" => Self::User,
            "assistant" => Self::Assistant,
            "attachment" => Self::Attachment,
            "system" => Self::System,
            "file-history-snapshot" => Self::FileHistorySnapshot,
            "last-prompt" => Self::LastPrompt,
            "queue-operation" => Self::QueueOperation,
            other => Self::Unknown(other.to_string()),
        }
    }
}

/// Envelope-of-envelopes the reader emits per parsed line. Wraps
/// the raw `serde_json::Value` plus the typed fields the mapper
/// keys off. Keeps the structural fields cheap to reach without
/// re-walking the payload.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JsonlRecord {
    /// Top-level `type` (parsed via [`RecordKind::from_tag`]).
    /// Optional so a missing `type` field is folded into the
    /// reader's `typeless_records` counter rather than crashing
    /// the JSON parse — corrupted appends in the wild sometimes
    /// truncate the field.
    #[serde(rename = "type", default)]
    pub kind: Option<String>,

    /// Stable per-session id. Always present on the records the
    /// mapper consumes. Some transient records (`file-history-snapshot`
    /// without a parent) skip it; the mapper drops those.
    #[serde(default)]
    pub session_id: Option<String>,

    /// Per-record UUID. Used as `event_id` source.
    #[serde(default)]
    pub uuid: Option<String>,

    /// Parent UUID — pairs `user` records with their corresponding
    /// `assistant` reply; the mapper joins on this field.
    #[serde(default)]
    pub parent_uuid: Option<String>,

    /// ISO-8601 timestamp. Kept verbatim so the chrono reparse
    /// happens in the mapper layer (avoids one round-trip when
    /// reading sessions only for byte-counting / estimation).
    #[serde(default)]
    pub timestamp: Option<String>,

    /// Working directory at the time of the record. Anchors the
    /// `Envelope.context.cwd` slot.
    #[serde(default)]
    pub cwd: Option<String>,

    /// Git branch at record time. Anchors `Envelope.context.branch`.
    #[serde(default)]
    pub git_branch: Option<String>,

    /// Claude Code harness version (`"2.1.112"` etc.). Surfaces in
    /// `Envelope.context.extras.harness_version` so the
    /// classifier can flag harness-version-specific patterns.
    #[serde(default)]
    pub version: Option<String>,

    /// `claude-vscode` in the surveyed corpus.
    #[serde(default)]
    pub entrypoint: Option<String>,

    /// `external` (user) or internal.
    #[serde(default)]
    pub user_type: Option<String>,

    /// True when the record is part of a sub-agent's inner
    /// transcript (parallel/debug context). The mapper still
    /// projects these but stamps `Envelope.context.extras.sidechain
    /// = true` so retrieval can filter them out.
    #[serde(default)]
    pub is_sidechain: Option<bool>,

    /// Anthropic API request id (`req_…`). Carried into the
    /// envelope so audit trails can join back to the API logs.
    #[serde(default)]
    pub request_id: Option<String>,

    /// Discriminator for `system` records: `local_command` is the
    /// only value the corpus carries today.
    #[serde(default)]
    pub subtype: Option<String>,

    /// Free-text `content` slot for `system` records and
    /// `last-prompt`/`queue-operation` envelopes.
    #[serde(default)]
    pub content: Option<String>,

    /// Anthropic-shaped message body (only for `user` /
    /// `assistant` records). Captured verbatim — the mapper
    /// projects the relevant fields.
    #[serde(default)]
    pub message: Option<serde_json::Value>,

    /// Attachment blob (only for `attachment` records). Subtypes:
    /// `tool_result`, `hook_success`, `hook_additional_context`,
    /// `deferred_tools_delta`, `skill_listing`,
    /// `file-history-snapshot`. Captured verbatim.
    #[serde(default)]
    pub attachment: Option<serde_json::Value>,

    /// Catch-all so the mapper can read fields the typed slots
    /// above missed (e.g. `lastPrompt`, `snapshot`).
    #[serde(flatten)]
    pub extras: serde_json::Map<String, serde_json::Value>,
}
