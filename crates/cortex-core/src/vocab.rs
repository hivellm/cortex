//! Controlled vocabularies for the envelope.
//!
//! These must stay in sync with the `enum` lists in `schemas/envelope.schema.json`.
//! The test `vocab_matches_schema` in `tests/schema_alignment.rs` enforces this.

/// Adapter identifiers allowed in [`crate::events::Envelope::tool`].
pub const TOOL_IDS: &[&str] = &[
    "claude-code",
    "cursor",
    "codex",
    "gemini",
    "copilot",
    "windsurf",
    "cortex-cli",
    "cortex-bootstrap",
    "git-hook",
    "fs-watcher",
];

/// Event discriminators allowed in [`crate::events::Envelope::kind`].
pub const KIND_IDS: &[&str] = &[
    "turn",
    "tool_call",
    "agent_call",
    "memory",
    "decision",
    "analysis",
    "law_violation",
    "artifact",
];

/// Streams the ingestion router accepts.
pub const STREAM_IDS: &[&str] = &["live", "bootstrap"];

/// Platform identifiers allowed in [`crate::events::Context::platform`].
pub const PLATFORM_IDS: &[&str] = &["win32", "darwin", "linux"];
