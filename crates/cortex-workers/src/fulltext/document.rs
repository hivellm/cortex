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
    /// Filterable ancestor paths for prefix-scoped queries
    /// (phase11b — issue: Meili rejects `path STARTS WITH ...`).
    /// Contains every ancestor directory plus the full path, each
    /// directory entry keeping its trailing `/`. Empty when `path`
    /// is missing.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub path_prefixes: Vec<String>,
    /// Per-kind extensions keyed by kind family — schema is
    /// `ext.<family>.<field>`. Missing extensions are absent (no
    /// null-padding per spec 08).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ext: BTreeMap<String, Value>,
}

/// Compute every ancestor prefix plus the full path for a given
/// repo-relative path. Used by the indexer to populate the
/// filterable `path_prefixes` array (phase11b §1.2).
///
/// Each directory entry keeps its trailing `/` so a `scope.files`
/// caller can pass `crates/cortex-api/src/` and match on the same
/// shape the indexer wrote. The full path is appended last with no
/// trailing slash, matching the on-disk form.
///
/// Returns an empty vector for an empty / whitespace-only / `/`-only
/// input — there is nothing to scope by.
pub fn compute_path_prefixes(path: &str) -> Vec<String> {
    let trimmed = path.trim().trim_matches('/');
    if trimmed.is_empty() {
        return Vec::new();
    }
    let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(segments.len());
    let mut acc = String::new();
    for (i, seg) in segments.iter().enumerate() {
        acc.push_str(seg);
        if i + 1 < segments.len() {
            acc.push('/');
            out.push(acc.clone());
        } else {
            out.push(acc.clone());
        }
    }
    out
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

#[cfg(test)]
mod tests {
    use super::compute_path_prefixes;

    #[test]
    fn deep_nested_path_yields_each_ancestor_with_trailing_slash() {
        // Phase11b §1.4 — spec scenario "Deep nested file": the
        // ancestor list keeps the trailing `/` on every directory
        // entry and ends with the full path with no slash.
        assert_eq!(
            compute_path_prefixes("crates/cortex-api/src/meili_lane.rs"),
            vec![
                "crates/".to_string(),
                "crates/cortex-api/".to_string(),
                "crates/cortex-api/src/".to_string(),
                "crates/cortex-api/src/meili_lane.rs".to_string(),
            ],
        );
    }

    #[test]
    fn single_segment_path_returns_only_the_file() {
        // Phase11b §1.4 — spec scenario "Single-segment path".
        assert_eq!(
            compute_path_prefixes("README.md"),
            vec!["README.md".to_string()],
        );
    }

    #[test]
    fn empty_path_returns_empty() {
        // Phase11b §1.4 — spec scenario "Empty / missing path".
        assert!(compute_path_prefixes("").is_empty());
        assert!(compute_path_prefixes("   ").is_empty());
        assert!(compute_path_prefixes("/").is_empty());
        assert!(compute_path_prefixes("///").is_empty());
    }

    #[test]
    fn trailing_and_leading_slashes_are_trimmed() {
        // Phase11b §1.4 — `cortex_query` callers may send
        // `crates/cortex-api/src/` with a trailing slash; the helper
        // must produce the same output as the slash-less form so the
        // generator and indexer stay aligned.
        assert_eq!(
            compute_path_prefixes("/crates/cortex-api/src/"),
            compute_path_prefixes("crates/cortex-api/src"),
        );
    }

    #[test]
    fn directory_only_input_treats_last_segment_as_leaf() {
        // Phase11b §1.4 — when the input ends in `/`, the trimmed
        // form has no leaf-vs-dir distinction. Document this so a
        // future reader knows the helper is path-shape agnostic: it
        // only cares about segments, not whether the leaf is a file.
        assert_eq!(
            compute_path_prefixes("crates/cortex-api/src/"),
            vec![
                "crates/".to_string(),
                "crates/cortex-api/".to_string(),
                "crates/cortex-api/src".to_string(),
            ],
        );
    }
}
