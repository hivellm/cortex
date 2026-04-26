//! Nexus label + relationship vocabulary and bootstrap Cypher.
//!
//! The actual Cypher execution lives in the graph-writer worker (spec 07);
//! this module just declares what to execute.

/// Every label the graph writer `MERGE`s on.
pub const LABELS: &[&str] = &[
    "Session",
    "Turn",
    "ToolCall",
    "AgentCall",
    "Memory",
    "Decision",
    "Analysis",
    "Law",
    "LawViolation",
    "Artifact",
    "Topic",
    "Entity",
    "Repo",
    "Model",
    "Tool",
    "User",
];

/// Every relationship type used in the graph.
pub const RELATIONSHIPS: &[&str] = &[
    "CONTAINS",
    "INVOKED",
    "READ",
    "WROTE",
    "EXECUTED",
    "DELETED",
    "PRODUCED",
    "SUPERSEDES",
    "REFERENCES",
    "ABOUT",
    "MENTIONS",
    "OF",
    "OBSERVED_IN",
    "SIMILAR_TO",
    "IN",
    "USED",
    "VIA",
    "BY",
    "LIVES_IN",
];

/// Cypher statements executed at bootstrap / worker startup. Each must be
/// idempotent (`IF NOT EXISTS`).
pub const BOOTSTRAP_STATEMENTS: &[&str] = &[
    "CREATE CONSTRAINT session_id IF NOT EXISTS FOR (s:Session) REQUIRE s.session_id IS UNIQUE",
    "CREATE CONSTRAINT turn_event_id IF NOT EXISTS FOR (t:Turn) REQUIRE t.event_id IS UNIQUE",
    "CREATE CONSTRAINT tool_call_event_id IF NOT EXISTS FOR (tc:ToolCall) REQUIRE tc.event_id IS UNIQUE",
    "CREATE CONSTRAINT agent_call_event_id IF NOT EXISTS FOR (ac:AgentCall) REQUIRE ac.event_id IS UNIQUE",
    "CREATE CONSTRAINT decision_event_id IF NOT EXISTS FOR (d:Decision) REQUIRE d.event_id IS UNIQUE",
    "CREATE CONSTRAINT analysis_event_id IF NOT EXISTS FOR (a:Analysis) REQUIRE a.event_id IS UNIQUE",
    "CREATE CONSTRAINT memory_event_id IF NOT EXISTS FOR (m:Memory) REQUIRE m.event_id IS UNIQUE",
    "CREATE CONSTRAINT violation_event_id IF NOT EXISTS FOR (v:LawViolation) REQUIRE v.event_id IS UNIQUE",
    "CREATE CONSTRAINT law_id IF NOT EXISTS FOR (l:Law) REQUIRE l.law_id IS UNIQUE",
    "CREATE CONSTRAINT artifact_hash IF NOT EXISTS FOR (a:Artifact) REQUIRE a.content_hash IS UNIQUE",
    "CREATE CONSTRAINT repo_path IF NOT EXISTS FOR (r:Repo) REQUIRE r.path IS UNIQUE",
    "CREATE CONSTRAINT topic_name IF NOT EXISTS FOR (t:Topic) REQUIRE t.name IS UNIQUE",
    "CREATE CONSTRAINT model_id IF NOT EXISTS FOR (m:Model) REQUIRE m.id IS UNIQUE",
    "CREATE CONSTRAINT tool_id IF NOT EXISTS FOR (t:Tool) REQUIRE t.id IS UNIQUE",
    "CREATE INDEX turn_occurred_at IF NOT EXISTS FOR (t:Turn) ON (t.occurred_at)",
    "CREATE INDEX tool_call_tool_name IF NOT EXISTS FOR (tc:ToolCall) ON (tc.tool_name)",
    "CREATE INDEX artifact_path IF NOT EXISTS FOR (a:Artifact) ON (a.path)",
];

