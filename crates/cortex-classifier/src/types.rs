//! Classifier types.

use async_trait::async_trait;
use cortex_core::events::Kind;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Severity label attached to every classified event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Routine, low-signal activity.
    Info,
    /// Worth surfacing in the dashboard timeline.
    Notable,
    /// Must alert — security, broken contract, law violation, data loss.
    Critical,
}

impl Severity {
    /// Lowercase label.
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Notable => "notable",
            Severity::Critical => "critical",
        }
    }
}

/// PII risk label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PiiRisk {
    /// No personal data, no secrets.
    Low,
    /// Usernames, emails, internal paths, repo/branch names.
    Medium,
    /// Credentials, tokens, keys, financial data, customer PII.
    High,
}

impl PiiRisk {
    /// Lowercase label.
    pub fn as_str(self) -> &'static str {
        match self {
            PiiRisk::Low => "low",
            PiiRisk::Medium => "medium",
            PiiRisk::High => "high",
        }
    }
}

/// Redaction suggestion from the classifier — a secret the static redactor missed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RedactionSuggestion {
    /// Short classification tag.
    pub pattern_class: String,
    /// JSON pointer into the payload.
    pub json_pointer: String,
    /// Free-form explanation from the classifier.
    pub rationale: String,
}

/// Which backend produced a [`ClassifierOutput`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClassifierSource {
    /// Haiku invoked via Claude Code CLI.
    HaikuCli,
    /// Haiku invoked via the Anthropic SDK.
    HaikuSdk,
    /// Content-hash cache hit.
    Cache,
    /// Rule-based fallback (budget halt or offline).
    StaticFallback,
}

impl ClassifierSource {
    /// Lowercase label.
    pub fn as_str(self) -> &'static str {
        match self {
            ClassifierSource::HaikuCli => "haiku_cli",
            ClassifierSource::HaikuSdk => "haiku_sdk",
            ClassifierSource::Cache => "cache",
            ClassifierSource::StaticFallback => "static_fallback",
        }
    }
}

/// Classifier invocation mode (selected per worker process).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClassifierMode {
    /// Invoke Claude Code CLI (`claude -p ...`).
    Cli,
    /// Invoke the Anthropic SDK directly.
    Sdk,
}

/// Input to a [`Classifier`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnrichmentInput {
    /// Source event id.
    pub event_id: String,
    /// Coarse event kind (from the envelope).
    pub kind: Kind,
    /// Pre-redaction content hash — the cache key.
    pub content_hash: String,
    /// Post-redaction payload the classifier sees.
    pub redacted_payload: Value,
    /// Optional repo context hint.
    pub context_repo: Option<String>,
}

/// Output of a [`Classifier`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClassifierOutput {
    /// Echo of the input event id.
    pub event_id: String,
    /// More precise kind (e.g. `git_push`), when the classifier can refine.
    pub kind_refinement: Option<String>,
    /// Multi-label topics drawn from the controlled vocab.
    pub topics: Vec<String>,
    /// Severity label.
    pub severity: Severity,
    /// PII risk.
    pub pii_risk: PiiRisk,
    /// Extra redaction suggestions.
    pub redaction_suggestions: Vec<RedactionSuggestion>,
    /// Short summary; mandatory if redacted payload >4 KB.
    pub summary: Option<String>,
    /// Which backend produced this record.
    pub source: ClassifierSource,
    /// Prompt template version (`v1`, …).
    pub prompt_version: String,
    /// Model id (`claude-haiku-4-5`, `static-v1`, …).
    pub model: String,
    /// Wall-clock latency to produce this record.
    pub latency_ms: u32,
    /// Input token count (0 for cache / static fallback).
    pub tokens_in: u32,
    /// Output token count (0 for cache / static fallback).
    pub tokens_out: u32,
}

/// Classifier trait. Implementations may batch-call a backend or shortcut via a cache.
#[async_trait]
pub trait Classifier: Send + Sync {
    /// Classify a batch of enrichment inputs; order of the result must match the input order.
    async fn classify_batch(
        &self,
        events: &[EnrichmentInput],
    ) -> Result<Vec<ClassifierOutput>, crate::errors::ClassifierError>;
}

#[async_trait]
impl<T: Classifier + ?Sized> Classifier for Box<T> {
    async fn classify_batch(
        &self,
        events: &[EnrichmentInput],
    ) -> Result<Vec<ClassifierOutput>, crate::errors::ClassifierError> {
        (**self).classify_batch(events).await
    }
}
