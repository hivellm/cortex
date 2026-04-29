//! Labeled query-set loader.
//!
//! The fixture lives at `tests/relevance/queries.toml` and pairs each
//! query with one or more `expected_doc_ids` — the strings the harness
//! tries to find in the top-10 snippets returned by `/v1/query`.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{anyhow, Context, Result};
use cortex_api::types::{IncludeField, Intent, Scope};
use serde::{Deserialize, Serialize};

/// One labeled query in the fixture set.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabeledQuery {
    /// Stable identifier (`rel-NNN`) used to diff regressions.
    pub id: String,
    /// Intent the harness will dispatch with.
    pub intent: Intent,
    /// Scope filter.
    #[serde(default)]
    pub scope: Scope,
    /// Free-text query string.
    pub query: String,
    /// Expected doc-id matchers — recall counts the query as
    /// satisfied when the top-10 contains *any* expected id.
    pub expected_doc_ids: Vec<String>,
    /// Optional curator note (kept in the report for triage).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Optional include-field override; when omitted the harness
    /// uses [`default_include`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<Vec<IncludeField>>,
}

impl LabeledQuery {
    /// Snake-case intent label used for aggregation / report keys.
    pub fn intent_label(&self) -> &'static str {
        self.intent.label()
    }
}

/// Wrapper carrying every labeled query plus a fixture-format version
/// stamp so the report can record which schema produced it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuerySet {
    /// `1` today — bumped if the fixture schema breaks.
    #[serde(default = "default_version")]
    pub version: u32,
    /// The labeled queries.
    pub queries: Vec<LabeledQuery>,
}

fn default_version() -> u32 {
    1
}

impl QuerySet {
    /// Load + validate a query set from disk.
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read query set {}", path.display()))?;
        Self::parse(&raw, path)
    }

    /// Parse a TOML query set from a string. Validates ids are
    /// unique, that every entry has at least one expected doc id,
    /// and that the query is non-empty.
    pub fn parse(raw: &str, path: &Path) -> Result<Self> {
        let set: QuerySet = toml::from_str(raw)
            .with_context(|| format!("parse query set {}", path.display()))?;
        set.validate()?;
        Ok(set)
    }

    fn validate(&self) -> Result<()> {
        if self.queries.is_empty() {
            return Err(anyhow!("query set is empty"));
        }
        let mut seen = BTreeMap::<&str, usize>::new();
        for (idx, q) in self.queries.iter().enumerate() {
            if q.id.trim().is_empty() {
                return Err(anyhow!("entry #{idx}: empty id"));
            }
            if let Some(&prev) = seen.get(q.id.as_str()) {
                return Err(anyhow!(
                    "duplicate id {:?} at entries #{prev} and #{idx}",
                    q.id
                ));
            }
            seen.insert(&q.id, idx);
            if q.query.trim().is_empty() {
                return Err(anyhow!("entry {} ({:?}): empty query", idx, q.id));
            }
            if q.expected_doc_ids.is_empty() {
                return Err(anyhow!(
                    "entry {} ({:?}): expected_doc_ids must list at least one match",
                    idx,
                    q.id
                ));
            }
        }
        Ok(())
    }

    /// Group entries by intent label — used for per-intent buckets.
    pub fn by_intent(&self) -> BTreeMap<&'static str, Vec<&LabeledQuery>> {
        let mut out: BTreeMap<&'static str, Vec<&LabeledQuery>> = BTreeMap::new();
        for q in &self.queries {
            out.entry(q.intent_label()).or_default().push(q);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fake_path() -> PathBuf {
        PathBuf::from("tests/relevance/queries.toml")
    }

    #[test]
    fn parse_minimal() {
        let raw = r#"
            version = 1
            [[queries]]
            id = "rel-001"
            intent = "pre_change_context"
            query = "the meili fan-out worker offset"
            expected_doc_ids = ["evt:cortex-Cortex-misc:01ABC"]
            [queries.scope]
            repo = "Cortex"
        "#;
        let set = QuerySet::parse(raw, &fake_path()).unwrap();
        assert_eq!(set.queries.len(), 1);
        assert_eq!(set.queries[0].intent, Intent::PreChangeContext);
    }

    #[test]
    fn rejects_duplicate_ids() {
        let raw = r#"
            [[queries]]
            id = "rel-001"
            intent = "explain"
            query = "x"
            expected_doc_ids = ["a"]
            [[queries]]
            id = "rel-001"
            intent = "explain"
            query = "y"
            expected_doc_ids = ["b"]
        "#;
        let err = QuerySet::parse(raw, &fake_path()).unwrap_err().to_string();
        assert!(err.contains("duplicate id"), "{err}");
    }

    #[test]
    fn rejects_empty_expected() {
        let raw = r#"
            [[queries]]
            id = "rel-001"
            intent = "explain"
            query = "x"
            expected_doc_ids = []
        "#;
        let err = QuerySet::parse(raw, &fake_path()).unwrap_err().to_string();
        assert!(err.contains("expected_doc_ids"), "{err}");
    }

    #[test]
    fn rejects_empty_query() {
        let raw = r#"
            [[queries]]
            id = "rel-001"
            intent = "explain"
            query = ""
            expected_doc_ids = ["a"]
        "#;
        let err = QuerySet::parse(raw, &fake_path()).unwrap_err().to_string();
        assert!(err.contains("empty query"), "{err}");
    }

    #[test]
    fn by_intent_groups_correctly() {
        let raw = r#"
            [[queries]]
            id = "rel-001"
            intent = "explain"
            query = "x"
            expected_doc_ids = ["a"]
            [[queries]]
            id = "rel-002"
            intent = "law_check"
            query = "y"
            expected_doc_ids = ["b"]
            [[queries]]
            id = "rel-003"
            intent = "explain"
            query = "z"
            expected_doc_ids = ["c"]
        "#;
        let set = QuerySet::parse(raw, &fake_path()).unwrap();
        let by = set.by_intent();
        assert_eq!(by.get("explain").unwrap().len(), 2);
        assert_eq!(by.get("law_check").unwrap().len(), 1);
    }
}
