//! Prompt template + topic vocabulary.

use crate::types::EnrichmentInput;

/// Seed controlled-vocabulary for the classifier (v1). Add new terms via a new
/// prompt version only — changes require re-classifying historical events.
pub const TOPIC_VOCAB_V1: &[&str] = &[
    "code",
    "refactor",
    "test",
    "build",
    "ci",
    "deploy",
    "git",
    "git_push",
    "git_commit",
    "review",
    "docs",
    "config",
    "debug",
    "perf",
    "security",
    "pii",
    "credential",
    "retention",
    "storage",
    "ingestion",
    "classifier",
    "embedder",
    "graph",
    "fulltext",
    "query",
    "retrieval",
    "governance",
    "law",
    "analysis",
    "decision",
    "memory",
    "bootstrap",
    "schema",
    "migration",
    "error",
    "timeout",
    "cancel",
    "budget",
    "cost",
    "rate_limit",
    "idempotent",
];

/// Versioned prompt template.
pub struct PromptTemplate {
    /// Version string stamped on every output (`v1`).
    pub version: &'static str,
    /// Raw template with `{{TOPIC_VOCAB}}` + `{{EVENTS_JSON}}` placeholders.
    pub body: &'static str,
}

/// v1 prompt as specified in spec 05.
pub const PROMPT_V1: PromptTemplate = PromptTemplate {
    version: "v1",
    body: include_str!("../prompts/classifier.v1.txt"),
};

/// Phase9h — auto-memory consolidator merge prompt. Rendered with the
/// JSON-encoded source entries via [`render_consolidate_auto_memory`].
/// The prompt is referenced by name from `cortex-cli`'s
/// `memory_consolidate` subcommand so the production wiring (Sonnet
/// CLI driver) can be added without revising the template.
pub const CONSOLIDATE_AUTO_MEMORY_V1: PromptTemplate = PromptTemplate {
    version: "v1",
    body: include_str!("../prompts/consolidate_auto_memory.v1.txt"),
};

impl PromptTemplate {
    /// Render the template with the supplied inputs.
    pub fn render(&self, events: &[EnrichmentInput]) -> Result<String, serde_json::Error> {
        let vocab = TOPIC_VOCAB_V1.join(", ");
        let events_json = serde_json::to_string(&events)?;
        Ok(self
            .body
            .replace("{{TOPIC_VOCAB}}", &vocab)
            .replace("{{EVENTS_JSON}}", &events_json))
    }
}

/// Phase9h — render [`CONSOLIDATE_AUTO_MEMORY_V1`] with `entries`
/// JSON-encoded. Returns the prompt body that should be shipped to
/// the Sonnet driver. `entries` is `&[serde_json::Value]` so callers
/// can pass any frontmatter+body shape without coupling the
/// classifier crate to the consolidator's domain types.
pub fn render_consolidate_auto_memory(
    entries: &[serde_json::Value],
) -> Result<String, serde_json::Error> {
    let entries_json = serde_json::to_string(entries)?;
    Ok(CONSOLIDATE_AUTO_MEMORY_V1
        .body
        .replace("{{ENTRIES_JSON}}", &entries_json))
}
