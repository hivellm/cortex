//! Phase23e §3 — Decision fusion: link doc CONTRADICTS/CITES edges against
//! the ADR `SUPERSEDES` chain and flag canonical drift.
//!
//! ## What is canonical drift?
//!
//! A doc exhibits canonical drift when it simultaneously:
//! 1. Has a `CITES` edge pointing to a **superseded** ADR.
//! 2. Has a `CONTRADICTS` edge pointing to the **current** ADR that
//!    superseded the one it cites.
//!
//! This pattern means the doc is referencing the old thinking while arguing
//! against the new canonical decision — a concrete sign of stale documentation.
//!
//! ## How the detector works
//!
//! The detector is pure logic — no graph queries, no LLM calls. It receives:
//! - A slice of edges from the doc annotator output (CONTRADICTS + CITES edges).
//! - A [`DecisionRegistry`] built by the caller from known Decision nodes.
//!
//! It returns a list of [`DriftFinding`] records that the caller can surface to
//! the pre-thinking band or write as graph metadata.

use std::collections::HashMap;

use crate::graph::patch::EdgeOp;

// ── Decision registry ─────────────────────────────────────────────────────────

/// Status of a single ADR in the registry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecisionStatus {
    /// Decision is in an active lifecycle state (`proposed` / `accepted`).
    Active,
    /// Decision has been superseded by a newer one.
    Superseded,
    /// Decision is deprecated and no longer applicable.
    Deprecated,
}

impl DecisionStatus {
    fn from_str(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "superseded" => Self::Superseded,
            "deprecated" => Self::Deprecated,
            _ => Self::Active,
        }
    }
}

/// One entry in the decision registry.
#[derive(Debug, Clone)]
pub struct DecisionRecord {
    /// Decision id (e.g. `"DEC-0042"`).
    pub decision_id: String,
    /// Current lifecycle status.
    pub status: DecisionStatus,
    /// When `Some`, this decision was superseded by `superseded_by`.
    pub superseded_by: Option<String>,
}

/// Registry of known ADR/Decision records.
///
/// Built by the caller from the Decision nodes in the current graph batch
/// (or from a static scan of the `.rulebook/decisions/` directory).
#[derive(Debug, Default)]
pub struct DecisionRegistry {
    /// Key = decision id (e.g. `"DEC-0042"`).
    pub records: HashMap<String, DecisionRecord>,
    /// Map from path fragment (lowercase) → decision id.
    /// Allows matching an article path like `"decisions/DEC-0042-foo.md"`
    /// back to the decision id `"DEC-0042"`.
    pub path_to_id: HashMap<String, String>,
}

impl DecisionRegistry {
    /// Register a decision with its optional supersession info.
    pub fn add(
        &mut self,
        decision_id: impl Into<String>,
        status: impl AsRef<str>,
        superseded_by: Option<String>,
    ) {
        let id = decision_id.into();
        let record = DecisionRecord {
            decision_id: id.clone(),
            status: DecisionStatus::from_str(status.as_ref()),
            superseded_by,
        };
        // Register path fragments for fuzzy matching.
        let id_lower = id.to_ascii_lowercase();
        self.path_to_id.insert(id_lower, id.clone());
        self.records.insert(id, record);
    }

    /// Look up a decision by id (exact match).
    pub fn get(&self, id: &str) -> Option<&DecisionRecord> {
        self.records.get(id)
    }

    /// Try to resolve a `to_key` (article natural key or path fragment) to a
    /// decision id. Tries exact match first, then a substring scan.
    ///
    /// `to_key` is typically `"{repo}|{path}"` from the annotator output.
    pub fn resolve_to_decision<'a>(&'a self, to_key: &str) -> Option<&'a DecisionRecord> {
        // Strip the repo prefix if present (`"repo|path"` → `"path"`).
        let path_part = to_key.split_once('|').map(|(_, p)| p).unwrap_or(to_key);
        let path_lower = path_part.to_ascii_lowercase();

        // Exact path → id lookup.
        if let Some(id) = self.path_to_id.get(&path_lower) {
            return self.records.get(id);
        }

        // Substring scan: does the path contain a known decision id?
        for (fragment, id) in &self.path_to_id {
            if path_lower.contains(fragment.as_str()) {
                return self.records.get(id);
            }
        }
        None
    }
}

// ── Drift findings ────────────────────────────────────────────────────────────

/// Classification of a detected drift pattern.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DriftKind {
    /// The doc has a CITES edge to a superseded ADR.
    CitesSuperseded {
        /// Id of the superseded ADR.
        superseded_id: String,
        /// Id of the current ADR that supersedes it (if known).
        current_id: Option<String>,
    },
    /// Canonical drift: doc cites a superseded ADR while CONTRADICTING the
    /// current ADR that superseded it.
    CanonicalDrift {
        /// Id of the superseded ADR being cited.
        superseded_id: String,
        /// Id of the current ADR being contradicted.
        current_id: String,
    },
}

/// A single drift finding for a document.
#[derive(Debug, Clone)]
pub struct DriftFinding {
    /// Natural key of the Article that exhibits drift (`"{repo}|{path}"`).
    pub doc_key: String,
    /// What kind of drift was detected.
    pub kind: DriftKind,
}

// ── DocDrift detector ─────────────────────────────────────────────────────────

/// Phase23e §3 drift detector.
///
/// Call [`DocDrift::detect`] with the edge list from the doc annotator and
/// a populated [`DecisionRegistry`] to get back a list of [`DriftFinding`].
pub struct DocDrift<'r> {
    registry: &'r DecisionRegistry,
}

impl<'r> DocDrift<'r> {
    /// Create a detector backed by `registry`.
    pub fn new(registry: &'r DecisionRegistry) -> Self {
        Self { registry }
    }

    /// Scan `edges` for canonical-drift patterns.
    ///
    /// Returns all [`DriftFinding`] records discovered. Pure logic — no I/O.
    pub fn detect(&self, edges: &[EdgeOp]) -> Vec<DriftFinding> {
        // Build per-doc maps of what it cites and what it contradicts.
        // Key = from_key (article natural key).
        let mut cited_by_doc: HashMap<String, Vec<String>> = HashMap::new();
        let mut contradicted_by_doc: HashMap<String, Vec<String>> = HashMap::new();

        for edge in edges {
            match edge.edge_type.as_str() {
                "CITES" => {
                    cited_by_doc
                        .entry(edge.from_key.clone())
                        .or_default()
                        .push(edge.to_key.clone());
                }
                "CONTRADICTS" => {
                    contradicted_by_doc
                        .entry(edge.from_key.clone())
                        .or_default()
                        .push(edge.to_key.clone());
                }
                _ => {}
            }
        }

        let mut findings = Vec::new();

        // Collect all docs that appear in either set.
        let all_docs: std::collections::HashSet<String> = cited_by_doc
            .keys()
            .chain(contradicted_by_doc.keys())
            .cloned()
            .collect();

        for doc_key in all_docs {
            let cited = cited_by_doc.get(&doc_key).map(Vec::as_slice).unwrap_or(&[]);
            let contradicted = contradicted_by_doc
                .get(&doc_key)
                .map(Vec::as_slice)
                .unwrap_or(&[]);

            // Check each cited target: is it a superseded ADR?
            for cite_target in cited {
                if let Some(record) = self.registry.resolve_to_decision(cite_target) {
                    if record.status == DecisionStatus::Superseded {
                        let current_id = record.superseded_by.clone();

                        // §3.2 Canonical drift check: does this doc ALSO contradict the
                        // current ADR (the one that superseded the cited one)?
                        let is_canonical_drift = current_id.as_deref().is_some_and(|cur_id| {
                            contradicted.iter().any(|ct| {
                                self.registry
                                    .resolve_to_decision(ct)
                                    .is_some_and(|r| r.decision_id == cur_id)
                            })
                        });

                        if is_canonical_drift {
                            findings.push(DriftFinding {
                                doc_key: doc_key.clone(),
                                kind: DriftKind::CanonicalDrift {
                                    superseded_id: record.decision_id.clone(),
                                    current_id: current_id.unwrap(),
                                },
                            });
                        } else {
                            findings.push(DriftFinding {
                                doc_key: doc_key.clone(),
                                kind: DriftKind::CitesSuperseded {
                                    superseded_id: record.decision_id.clone(),
                                    current_id,
                                },
                            });
                        }
                    }
                }
            }
        }

        findings
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::graph::patch::{EdgeConfidence, EdgeOp};

    fn cites_edge(from: &str, to: &str) -> EdgeOp {
        EdgeOp {
            edge_type: "CITES".to_string(),
            from_label: "Article".to_string(),
            from_key: from.to_string(),
            to_label: "Article".to_string(),
            to_key: to.to_string(),
            props: BTreeMap::new(),
            ..Default::default()
        }
        .with_confidence(EdgeConfidence::Inferred, None)
    }

    fn contradicts_edge(from: &str, to: &str) -> EdgeOp {
        EdgeOp {
            edge_type: "CONTRADICTS".to_string(),
            from_label: "Article".to_string(),
            from_key: from.to_string(),
            to_label: "Article".to_string(),
            to_key: to.to_string(),
            props: BTreeMap::new(),
            ..Default::default()
        }
        .with_confidence(EdgeConfidence::Inferred, None)
    }

    fn build_registry() -> DecisionRegistry {
        let mut reg = DecisionRegistry::default();
        // DEC-0001: active
        reg.add("DEC-0001", "accepted", None);
        // DEC-0002: superseded by DEC-0003
        reg.add("DEC-0002", "superseded", Some("DEC-0003".to_string()));
        // DEC-0003: active, the current ADR
        reg.add("DEC-0003", "accepted", None);
        reg
    }

    /// §3.1 — doc cites a superseded ADR → CitesSuperseded finding
    #[test]
    fn doc_cites_superseded_adr_is_flagged() {
        let reg = build_registry();
        let detector = DocDrift::new(&reg);

        // Register path fragment for DEC-0002
        // (registry already has "dec-0002" in path_to_id from add())
        let edges = vec![cites_edge(
            "repo|docs/architecture/review.md",
            "repo|decisions/DEC-0002-old-auth.md",
        )];
        let findings = detector.detect(&edges);
        assert_eq!(findings.len(), 1);
        match &findings[0].kind {
            DriftKind::CitesSuperseded {
                superseded_id,
                current_id,
            } => {
                assert_eq!(superseded_id, "DEC-0002");
                assert_eq!(current_id.as_deref(), Some("DEC-0003"));
            }
            other => panic!("expected CitesSuperseded, got {other:?}"),
        }
    }

    /// §3.2 — doc cites superseded AND contradicts current → CanonicalDrift
    #[test]
    fn canonical_drift_detected() {
        let reg = build_registry();
        let detector = DocDrift::new(&reg);

        let doc = "repo|docs/my-doc.md";
        let edges = vec![
            // cites the old superseded ADR
            cites_edge(doc, "repo|decisions/DEC-0002-old-auth.md"),
            // contradicts the new current ADR
            contradicts_edge(doc, "repo|decisions/DEC-0003-new-auth.md"),
        ];
        let findings = detector.detect(&edges);
        assert_eq!(findings.len(), 1);
        match &findings[0].kind {
            DriftKind::CanonicalDrift {
                superseded_id,
                current_id,
            } => {
                assert_eq!(superseded_id, "DEC-0002");
                assert_eq!(current_id, "DEC-0003");
            }
            other => panic!("expected CanonicalDrift, got {other:?}"),
        }
    }

    /// Doc cites an active ADR — no drift.
    #[test]
    fn cites_active_adr_no_drift() {
        let reg = build_registry();
        let detector = DocDrift::new(&reg);

        let edges = vec![cites_edge(
            "repo|docs/my-doc.md",
            "repo|decisions/DEC-0001-active.md",
        )];
        let findings = detector.detect(&edges);
        assert!(
            findings.is_empty(),
            "citing an active ADR must not trigger drift"
        );
    }

    /// Contradicts-only edge (no CITES) — not canonical drift on its own.
    #[test]
    fn contradicts_without_cites_no_drift() {
        let reg = build_registry();
        let detector = DocDrift::new(&reg);

        let edges = vec![contradicts_edge(
            "repo|docs/my-doc.md",
            "repo|decisions/DEC-0003-new-auth.md",
        )];
        let findings = detector.detect(&edges);
        assert!(
            findings.is_empty(),
            "contradicting current ADR without citing a superseded one must not trigger drift"
        );
    }

    /// Doc cites superseded but contradicts an unrelated doc (not the current ADR)
    /// → CitesSuperseded only, not CanonicalDrift.
    #[test]
    fn cites_superseded_but_contradicts_unrelated() {
        let reg = build_registry();
        let detector = DocDrift::new(&reg);

        let doc = "repo|docs/my-doc.md";
        let edges = vec![
            cites_edge(doc, "repo|decisions/DEC-0002-old.md"),
            contradicts_edge(doc, "repo|docs/some-other-doc.md"),
        ];
        let findings = detector.detect(&edges);
        assert_eq!(findings.len(), 1);
        assert!(
            matches!(findings[0].kind, DriftKind::CitesSuperseded { .. }),
            "should be CitesSuperseded, not CanonicalDrift"
        );
    }

    /// Registry path resolution: `"{repo}|{path}"` containing a decision id
    /// substring resolves to the correct record.
    #[test]
    fn registry_resolves_article_key_with_repo_prefix() {
        let reg = build_registry();
        let record = reg.resolve_to_decision("myrepo|decisions/DEC-0002-old-auth.md");
        assert!(record.is_some());
        assert_eq!(record.unwrap().decision_id, "DEC-0002");
    }

    /// Unknown path → None.
    #[test]
    fn registry_returns_none_for_unknown_path() {
        let reg = build_registry();
        let record = reg.resolve_to_decision("myrepo|docs/unrelated.md");
        assert!(record.is_none());
    }
}
