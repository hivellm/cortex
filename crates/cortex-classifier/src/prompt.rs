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

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::events::Kind;
    use serde_json::json;

    #[test]
    fn renders_placeholders() {
        let inputs = vec![EnrichmentInput {
            event_id: "01H".to_string(),
            kind: Kind::ToolCall,
            content_hash: "sha256:abc".into(),
            redacted_payload: json!({ "tool_name": "Bash", "input": { "command": "ls" }, "outcome": "success" }),
            context_repo: Some("Cortex".into()),
        }];
        let rendered = PROMPT_V1.render(&inputs).unwrap();
        assert!(!rendered.contains("{{TOPIC_VOCAB}}"));
        assert!(!rendered.contains("{{EVENTS_JSON}}"));
        assert!(rendered.contains("\"event_id\":\"01H\""));
        assert!(rendered.contains("code"));
    }
}
