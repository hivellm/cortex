//! Nexus label + relationship vocabulary and bootstrap Cypher.
//!
//! The actual Cypher execution lives in the graph-writer worker (spec 07);
//! this module just declares what to execute.

/// Typed node-kind vocabulary. Covers all labels Cortex writes to Nexus:
/// UA-adopted kinds (code / non-code / knowledge groups from the
/// Understand-Anything analysis, ADR #35) and Cortex-only kinds
/// (session/memory/decision layer that UA entirely lacks).
///
/// `as_label()` returns the Nexus `PascalCase` label string.
/// `from_label()` does the reverse for deserialization.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    // ── UA-adopted: code group ──────────────────────────────────────────
    /// Source file (UA `file`).
    File,
    /// Function or method (UA `function`).
    Function,
    /// Class or struct (UA `class`).
    Class,
    /// Crate / package / module (UA `module`).
    Module,
    // ── UA-adopted: non-code group ─────────────────────────────────────
    /// Configuration file — TOML / YAML / env (UA `config`).
    Config,
    /// Documentation file or page (UA `document`).
    Document,
    /// Network service / microservice (UA `service`).
    Service,
    /// Database table or collection (UA `table`).
    Table,
    /// HTTP / gRPC / GraphQL endpoint (UA `endpoint`).
    Endpoint,
    /// CI / data pipeline (UA `pipeline`).
    Pipeline,
    /// Schema definition — protobuf / GraphQL DDL / SQL DDL (UA `schema`).
    Schema,
    /// IaC resource — Terraform / CDK (UA `resource`).
    Resource,
    // ── UA-adopted: knowledge group ────────────────────────────────────
    /// Wiki / doc article (UA `article`).
    Article,
    /// Named entity extracted from docs (UA `entity`).
    Entity,
    /// Topic / category (UA `topic`); unifies Cortex `TopicCard`.
    Topic,
    /// First-class claim node — enables graph-layer contradiction (UA `claim`).
    Claim,
    /// Citation source (UA `source`).
    Source,
    // ── Cortex-only: session / event layer ────────────────────────────
    /// Conversation session.
    Session,
    /// Single turn within a session.
    Turn,
    /// Tool invocation event.
    ToolCall,
    /// Sub-agent invocation event.
    AgentCall,
    /// Memory/knowledge capture event.
    Memory,
    /// Architectural decision record.
    Decision,
    /// Analysis artifact.
    Analysis,
    /// Project law / rule definition.
    Law,
    /// Observed law violation.
    LawViolation,
    // ── Cortex-only: code/doc graph ───────────────────────────────────
    /// Source artifact (file + content hash).
    Artifact,
    /// Repository root.
    Repo,
    /// Code symbol (function / class / trait / type).
    Symbol,
    /// Section within a markdown document.
    DocSection,
    /// External (out-of-workspace) package dependency.
    ExternalPackage,
    /// Import the resolver could not place — sentinel node for coverage.
    UnresolvedImport,
    // ── Cortex-only: knowledge pipeline ──────────────────────────────
    /// Rulebook knowledge entry.
    Knowledge,
    /// Rulebook learning entry.
    Learning,
    /// Distilled consolidation (session / topic / decision summary).
    Consolidation,
    /// Living-synthesis topic card.
    TopicCard,
    // ── Cortex-only: identity ─────────────────────────────────────────
    /// LLM model identity node.
    Model,
    /// Tool identity node.
    Tool,
    /// User identity node.
    User,
}

impl NodeKind {
    /// Nexus `PascalCase` label string.
    pub fn as_label(self) -> &'static str {
        match self {
            NodeKind::File => "File",
            NodeKind::Function => "Function",
            NodeKind::Class => "Class",
            NodeKind::Module => "Module",
            NodeKind::Config => "Config",
            NodeKind::Document => "Document",
            NodeKind::Service => "Service",
            NodeKind::Table => "Table",
            NodeKind::Endpoint => "Endpoint",
            NodeKind::Pipeline => "Pipeline",
            NodeKind::Schema => "Schema",
            NodeKind::Resource => "Resource",
            NodeKind::Article => "Article",
            NodeKind::Entity => "Entity",
            NodeKind::Topic => "Topic",
            NodeKind::Claim => "Claim",
            NodeKind::Source => "Source",
            NodeKind::Session => "Session",
            NodeKind::Turn => "Turn",
            NodeKind::ToolCall => "ToolCall",
            NodeKind::AgentCall => "AgentCall",
            NodeKind::Memory => "Memory",
            NodeKind::Decision => "Decision",
            NodeKind::Analysis => "Analysis",
            NodeKind::Law => "Law",
            NodeKind::LawViolation => "LawViolation",
            NodeKind::Artifact => "Artifact",
            NodeKind::Repo => "Repo",
            NodeKind::Symbol => "Symbol",
            NodeKind::DocSection => "DocSection",
            NodeKind::ExternalPackage => "ExternalPackage",
            NodeKind::UnresolvedImport => "UnresolvedImport",
            NodeKind::Knowledge => "Knowledge",
            NodeKind::Learning => "Learning",
            NodeKind::Consolidation => "Consolidation",
            NodeKind::TopicCard => "TopicCard",
            NodeKind::Model => "Model",
            NodeKind::Tool => "Tool",
            NodeKind::User => "User",
        }
    }

    /// Parse a label string back to a [`NodeKind`]. Returns `None` for
    /// unrecognised labels (forward-compat: callers should treat `None` as
    /// an unknown-but-valid node, not an error).
    pub fn from_label(label: &str) -> Option<Self> {
        Some(match label {
            "File" => NodeKind::File,
            "Function" => NodeKind::Function,
            "Class" => NodeKind::Class,
            "Module" => NodeKind::Module,
            "Config" => NodeKind::Config,
            "Document" => NodeKind::Document,
            "Service" => NodeKind::Service,
            "Table" => NodeKind::Table,
            "Endpoint" => NodeKind::Endpoint,
            "Pipeline" => NodeKind::Pipeline,
            "Schema" => NodeKind::Schema,
            "Resource" => NodeKind::Resource,
            "Article" => NodeKind::Article,
            "Entity" => NodeKind::Entity,
            "Topic" => NodeKind::Topic,
            "Claim" => NodeKind::Claim,
            "Source" => NodeKind::Source,
            "Session" => NodeKind::Session,
            "Turn" => NodeKind::Turn,
            "ToolCall" => NodeKind::ToolCall,
            "AgentCall" => NodeKind::AgentCall,
            "Memory" => NodeKind::Memory,
            "Decision" => NodeKind::Decision,
            "Analysis" => NodeKind::Analysis,
            "Law" => NodeKind::Law,
            "LawViolation" => NodeKind::LawViolation,
            "Artifact" => NodeKind::Artifact,
            "Repo" => NodeKind::Repo,
            "Symbol" => NodeKind::Symbol,
            "DocSection" => NodeKind::DocSection,
            "ExternalPackage" => NodeKind::ExternalPackage,
            "UnresolvedImport" => NodeKind::UnresolvedImport,
            "Knowledge" => NodeKind::Knowledge,
            "Learning" => NodeKind::Learning,
            "Consolidation" => NodeKind::Consolidation,
            "TopicCard" => NodeKind::TopicCard,
            "Model" => NodeKind::Model,
            "Tool" => NodeKind::Tool,
            "User" => NodeKind::User,
            _ => return None,
        })
    }

    /// Whether this kind belongs to the UA-adopted vocabulary.
    /// `false` for Cortex-only kinds.
    pub fn is_ua_adopted(self) -> bool {
        matches!(
            self,
            NodeKind::File
                | NodeKind::Function
                | NodeKind::Class
                | NodeKind::Module
                | NodeKind::Config
                | NodeKind::Document
                | NodeKind::Service
                | NodeKind::Table
                | NodeKind::Endpoint
                | NodeKind::Pipeline
                | NodeKind::Schema
                | NodeKind::Resource
                | NodeKind::Article
                | NodeKind::Entity
                | NodeKind::Topic
                | NodeKind::Claim
                | NodeKind::Source
        )
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every NodeKind round-trips through as_label → from_label.
    #[test]
    fn node_kind_round_trips() {
        let all = [
            NodeKind::File,
            NodeKind::Function,
            NodeKind::Class,
            NodeKind::Module,
            NodeKind::Config,
            NodeKind::Document,
            NodeKind::Service,
            NodeKind::Table,
            NodeKind::Endpoint,
            NodeKind::Pipeline,
            NodeKind::Schema,
            NodeKind::Resource,
            NodeKind::Article,
            NodeKind::Entity,
            NodeKind::Topic,
            NodeKind::Claim,
            NodeKind::Source,
            NodeKind::Session,
            NodeKind::Turn,
            NodeKind::ToolCall,
            NodeKind::AgentCall,
            NodeKind::Memory,
            NodeKind::Decision,
            NodeKind::Analysis,
            NodeKind::Law,
            NodeKind::LawViolation,
            NodeKind::Artifact,
            NodeKind::Repo,
            NodeKind::Symbol,
            NodeKind::DocSection,
            NodeKind::ExternalPackage,
            NodeKind::UnresolvedImport,
            NodeKind::Knowledge,
            NodeKind::Learning,
            NodeKind::Consolidation,
            NodeKind::TopicCard,
            NodeKind::Model,
            NodeKind::Tool,
            NodeKind::User,
        ];
        for kind in all {
            let label = kind.as_label();
            let parsed = NodeKind::from_label(label);
            assert_eq!(
                parsed,
                Some(kind),
                "NodeKind::{kind:?} round-trip failed: as_label={label:?}, from_label={parsed:?}"
            );
        }
    }

    /// UA-adopted node kinds are flagged correctly; Cortex-only ones are not.
    #[test]
    fn node_kind_ua_adopted_flag() {
        assert!(NodeKind::File.is_ua_adopted());
        assert!(NodeKind::Table.is_ua_adopted());
        assert!(NodeKind::Claim.is_ua_adopted());
        assert!(!NodeKind::Session.is_ua_adopted());
        assert!(!NodeKind::Decision.is_ua_adopted());
        assert!(!NodeKind::Artifact.is_ua_adopted());
    }

    /// from_label returns None for unknown strings (forward-compat).
    #[test]
    fn node_kind_unknown_label_returns_none() {
        assert!(NodeKind::from_label("UnknownFutureKind").is_none());
        assert!(NodeKind::from_label("").is_none());
    }
}
