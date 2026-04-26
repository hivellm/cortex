//! Document types and identity rules for the full-text indexer.
//!
//! Mirrors `docs/specs/08-fulltext-indexer.md` §Document schema. Every
//! Meilisearch document follows the shared-core layout plus an optional
//! per-kind extension under `ext.<kind>`. The doc-id rule is explicit so
//! both live ingest and bootstrap flows produce stable, dedup-friendly
//! ids.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Severity label echoed onto the document — copied verbatim from the
/// classifier output so filterable queries match the upstream vocab.
pub type Severity = String;

/// PII-risk label echoed onto the document.
pub type PiiRisk = String;

/// One full-text document destined for Meilisearch.
///
/// `id` is the document key Meilisearch dedupes by. `body` holds the
/// primary searchable text after the body-selection rule (spec 08
/// §Body selection). All field names match the JSON keys the spec
/// declares so downstream filters work without remapping.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Document {
    /// Document key. `event_id` for live, `bootstrap:<repo>:<path>:<hash>`
    /// for bootstrap (architecture §6).
    pub id: String,
    /// Source event id — preserved verbatim so the dashboard can join
    /// docs back to their envelope.
    pub event_id: String,
    /// Coarse event kind (matches the schema-discriminator in spec 01).
    pub kind: String,
    /// Pre-redaction content hash from the envelope.
    pub content_hash: String,
    /// Event timestamp in milliseconds since the epoch (filterable +
    /// sortable per spec 08 §Index configuration).
    pub ts: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Repo hint from the envelope context.
    pub repo: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Path hint from the envelope context.
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    /// Classifier topics (controlled vocab) — filterable.
    pub topics: Vec<String>,
    /// Classifier severity label.
    pub severity: Severity,
    /// Classifier PII-risk label.
    pub pii_risk: PiiRisk,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Short summary from the classifier; preferred body when set.
    pub summary: Option<String>,
    /// Short identifier for snippet rendering (symbol for code,
    /// first H1 for docs, first 80 chars otherwise).
    pub title: String,
    /// Primary searchable text — already redacted.
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    /// Tree-sitter language identifier when known.
    pub language: Option<String>,
    /// `true` when `body` was truncated to honour the size cap.
    #[serde(default)]
    pub truncated: bool,
    /// Per-kind extensions keyed by kind family — schema is
    /// `ext.<family>.<field>`. Missing extensions are absent (no
    /// null-padding per spec 08).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

/// Build the doc-id for a live event — the envelope's own `event_id`.
pub fn live_doc_id(event_id: &str) -> String {
    event_id.to_string()
}

/// Build the doc-id for a bootstrap artifact — `bootstrap:<repo>:<path>:<hash>`.
///
/// Stable across re-runs as long as `(repo, path, content_hash)` doesn't
/// change. Spec 08 §Identity calls this out explicitly so re-bootstrapping
/// the same repo is idempotent on the Meilisearch side.
pub fn bootstrap_doc_id(repo: &str, path: &str, content_hash: &str) -> String {
    format!("bootstrap:{repo}:{path}:{content_hash}")
}
