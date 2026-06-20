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
    // Phase11k §1.1 — top-level lane projection fields. The spec-11
    // lane projection contract (`crates/cortex-api/src/meili_lane.rs`
    // → `LANE_EXTRAS_KEYS`) flattens unknown top-level Meili fields
    // into `extras_raw` and from there into `LaneHit.extras`. Stamping
    // these at the top of the document — instead of nesting them under
    // `ext.decision.*` / `ext.law_violation.*` — lets the orchestrator's
    // overlay derivers (`derive_decisions`, `derive_laws`,
    // `derive_similar_turns`) read them with no transform layer in
    // between. Each field is `Option<String>` so it stays absent on
    // documents whose kind doesn't apply.
    /// ADR identifier copied off `DecisionPayload.decision_id`. Set
    /// only when the document's `kind == "decision"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
    /// ADR title copied off `DecisionPayload.title`. Searchable so a
    /// free-text query against the title still hits the row.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_title: Option<String>,
    /// ADR status (`proposed` / `accepted` / `superseded` /
    /// `deprecated`). Filterable for status-scoped queries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_status: Option<String>,
    /// ADR id this decision supersedes, when the supersession chain
    /// stamps one. Filterable so the dashboard's chain view scopes to
    /// the parent ADR.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub decision_supersedes: Option<String>,
    /// Law identifier (`LAW-CORTEX-001`, `LAW-007`, …). Set on
    /// `kind == "law_violation"` documents. Searchable + filterable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub law_id: Option<String>,
    /// Law violation severity copied off `LawViolationPayload.severity`
    /// (`info` / `notable` / `critical`). Filterable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub law_severity: Option<String>,
    /// Law enforcement tier (1–4) rendered as a string. Filterable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub law_tier: Option<String>,
    /// Turn id (= envelope `event_id`) copied verbatim onto
    /// `kind == "turn"` documents so the spec-11 `derive_similar_turns`
    /// overlay can group hits by turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    // Phase18 §2.6 — bitemporal projection. The fields below match the
    // ADR-018..023 scoping columns the temporal classifier (§3) reads
    // at retrieval time. `project_id` / `branch_id` / `lifecycle` carry
    // the categorical axes; the `*_unix` columns are second-precision
    // epoch ints so Meili sortable + filter range ops stay cheap. The
    // RFC3339 string forms live on the graph side only — Meili never
    // sees the human-readable shape.
    /// Project scope copied off the graph stamp (lower-cased repo).
    /// Filterable so per-project retrievals scope without a
    /// `repo = "X"` clause.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// Branch scope (`"main"` until §2.12 ships per-project branches).
    /// Filterable for branch-isolated retrievals.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch_id: Option<String>,
    /// `proposed | active | superseded | deprecated | abandoned | merged`.
    /// Filterable so the classifier drops `superseded` / `abandoned`
    /// rows from default retrievals.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<String>,
    /// When the fact starts being true, epoch seconds (UTC). Filterable
    /// + sortable; range probes use `valid_from_unix <= as_of_unix`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_from_unix: Option<i64>,
    /// When the fact stops; absent = still valid. Filterable +
    /// sortable; `valid_to_unix EXISTS AND <= as_of_unix` selects
    /// EXPIRED rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub valid_to_unix: Option<i64>,
    /// When Cortex stopped believing the version; absent = still
    /// believed. Filterable + sortable; `superseded_at_unix EXISTS
    /// AND <= as_of_unix` selects SUPERSEDED rows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub superseded_at_unix: Option<i64>,
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

/// Build the doc-id for a bootstrap artifact — `bootstrap-<sha256hex>`
/// over the stable `(repo, path, content_hash)` tuple.
///
/// Stable across re-runs as long as `(repo, path, content_hash)` doesn't
/// change. Spec 08 §Identity calls this out so re-bootstrapping the same
/// repo is idempotent on the Meilisearch side.
///
/// The id MUST be a valid Meilisearch primary key — Meili accepts only
/// `[a-zA-Z0-9-_]` (≤ 511 bytes). The previous readable form
/// `bootstrap:<repo>:<path>:<hash>` contained `:` `/` `.` and was
/// **rejected at indexing**, silently dropping every bootstrap/content-
/// addressable doc (Decision/LawViolation/Knowledge/Learning). Hashing
/// the tuple yields a collision-resistant, Meili-safe key while keeping
/// determinism (same tuple → same id → idempotent upsert).
pub fn bootstrap_doc_id(repo: &str, path: &str, content_hash: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    // Unit-separator (0x1f) delimiters keep the tuple unambiguous so two
    // distinct (repo, path, hash) triples cannot hash to the same key by
    // concatenation aliasing.
    hasher.update(repo.as_bytes());
    hasher.update([0x1f]);
    hasher.update(path.as_bytes());
    hasher.update([0x1f]);
    hasher.update(content_hash.as_bytes());
    let digest = hasher.finalize();
    let mut id = String::with_capacity(74);
    id.push_str("bootstrap-");
    for byte in digest {
        id.push_str(&format!("{byte:02x}"));
    }
    id
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
