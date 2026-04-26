//! Schema bootstrap statements for the Cortex graph.
//!
//! Mirrors `docs/specs/07-graph-writer.md` §Schema bootstrapping. Every
//! statement is idempotent: re-running on an already-bootstrapped Nexus
//! instance is a no-op. Failure on any statement is fatal — the worker
//! refuses to accept events against an unknown schema.
//!
//! Statement ordering: constraints first (they are what the writer
//! relies on for `MERGE` idempotency), then secondary indexes used by
//! the read path (spec 11). Each constraint name is unique inside Nexus
//! so re-running the bootstrap is cheap.

/// Cypher statements applied at worker startup, in order.
///
/// The list mirrors the schema in spec 07 §Schema bootstrapping
/// verbatim, plus three extra constraints not in the spec body that
/// architecture §4.2 implies as `id` keys (`Memory`, `Analysis`,
/// `LawViolation`) and the `Repo.name` uniqueness constraint that the
/// `IN_REPO` edge depends on.
pub const SCHEMA_STATEMENTS: &[&str] = &[
    "CREATE CONSTRAINT session_id IF NOT EXISTS FOR (s:Session) REQUIRE s.id IS UNIQUE",
    "CREATE CONSTRAINT turn_id IF NOT EXISTS FOR (t:Turn) REQUIRE t.id IS UNIQUE",
    "CREATE CONSTRAINT tool_call_id IF NOT EXISTS FOR (tc:ToolCall) REQUIRE tc.id IS UNIQUE",
    "CREATE CONSTRAINT artifact_natural_key IF NOT EXISTS FOR (a:Artifact) REQUIRE a.natural_key IS UNIQUE",
    "CREATE CONSTRAINT decision_id IF NOT EXISTS FOR (d:Decision) REQUIRE d.id IS UNIQUE",
    "CREATE CONSTRAINT memory_id IF NOT EXISTS FOR (m:Memory) REQUIRE m.id IS UNIQUE",
    "CREATE CONSTRAINT analysis_id IF NOT EXISTS FOR (a:Analysis) REQUIRE a.id IS UNIQUE",
    "CREATE CONSTRAINT law_id IF NOT EXISTS FOR (l:Law) REQUIRE l.id IS UNIQUE",
    "CREATE CONSTRAINT violation_id IF NOT EXISTS FOR (v:LawViolation) REQUIRE v.id IS UNIQUE",
    "CREATE CONSTRAINT repo_name IF NOT EXISTS FOR (r:Repo) REQUIRE r.name IS UNIQUE",
    "CREATE INDEX artifact_repo_path IF NOT EXISTS FOR (a:Artifact) ON (a.repo, a.path)",
    "CREATE INDEX turn_ts IF NOT EXISTS FOR (t:Turn) ON (t.ts)",
    "CREATE INDEX tool_call_name IF NOT EXISTS FOR (tc:ToolCall) ON (tc.tool_name)",
];

/// Owned-string clone of [`SCHEMA_STATEMENTS`] for callers (like
/// [`crate::nexus_client::GraphClient::ensure_schema`]) that want a
/// `Vec<String>` they can push into.
pub fn statements() -> Vec<String> {
    SCHEMA_STATEMENTS.iter().map(|s| s.to_string()).collect()
}
