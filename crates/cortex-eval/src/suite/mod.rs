//! Phase14c — suite registry. Each submodule ships the row type,
//! the metric pipeline, and the acceptance-gate helper.
//!
//! Suites depend only on `cortex-api` over HTTP — the harness MUST
//! be able to run against a remote stack (CI runners hit the dev
//! compose) without pulling the cortex-api crate as a build dep.

pub mod classification;
pub mod consolidation;
pub mod retrieval;

/// Canonical suite identifier. The CLI's `--suite` flag parses
/// into this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuiteName {
    /// `tests/golden/retrieval.csv` → MRR@10 + recall@5 against
    /// `/v1/query`.
    Retrieval,
    /// `tests/golden/consolidation.csv` → entity-recall +
    /// fact-recall against the consolidation index.
    Consolidation,
    /// `tests/golden/classification.csv` → per-kind F1 + macro-F1
    /// against the classifier worker.
    Classification,
}

impl SuiteName {
    /// Parse a stable lower-case string into a [`SuiteName`].
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "retrieval" => Some(Self::Retrieval),
            "consolidation" => Some(Self::Consolidation),
            "classification" => Some(Self::Classification),
            _ => None,
        }
    }

    /// Stable lower-case label used in reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retrieval => "retrieval",
            Self::Consolidation => "consolidation",
            Self::Classification => "classification",
        }
    }
}

/// Aggregate verdict the acceptance helpers return: which metrics
/// hit / missed their floors, plus a single `passed` flag.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptanceVerdict {
    /// `true` when every metric with a floor met or exceeded it.
    pub passed: bool,
    /// Names of metrics that fell below their floor (empty when
    /// `passed`).
    pub failed_metrics: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn suite_name_parse_round_trip() {
        for s in ["retrieval", "consolidation", "classification"] {
            assert_eq!(SuiteName::parse(s).unwrap().as_str(), s);
        }
        assert!(SuiteName::parse("garbage").is_none());
        assert!(SuiteName::parse("  RETRIEVAL  ").is_some());
    }
}
