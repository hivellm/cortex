//! Phase14c — suite registry. Each submodule ships the row type,
//! the metric pipeline, and the acceptance-gate helper.
//!
//! Suites depend only on `cortex-api` over HTTP — the harness MUST
//! be able to run against a remote stack (CI runners hit the dev
//! compose) without pulling the cortex-api crate as a build dep.

pub mod access_control;
pub mod classification;
pub mod consolidation;
pub mod mcp_search;
pub mod retrieval;

/// Canonical suite identifier. The CLI's `--suite` flag parses
/// into this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuiteName {
    /// `crates/cortex-eval/tests/golden/retrieval.csv` → MRR@10 + recall@5 against
    /// `/v1/query`.
    Retrieval,
    /// `crates/cortex-eval/tests/golden/consolidation.csv` → entity-recall +
    /// fact-recall against the consolidation index.
    Consolidation,
    /// `crates/cortex-eval/tests/golden/classification.csv` → per-kind F1 + macro-F1
    /// against the classifier worker.
    Classification,
    /// Phase19 §5.3 — `crates/cortex-eval/tests/golden/mcp_search.csv` → recall@5
    /// against each granular MCP tool (entity / topic / file-touched
    /// lookups + the rest).
    McpSearch,
    /// Phase21 §9.1 — `crates/cortex-eval/tests/golden/access_control.csv` → zero
    /// false-grants across the clearance×level×compartment matrix.
    AccessControl,
}

impl SuiteName {
    /// Parse a stable lower-case string into a [`SuiteName`].
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "retrieval" => Some(Self::Retrieval),
            "consolidation" => Some(Self::Consolidation),
            "classification" => Some(Self::Classification),
            "mcp_search" => Some(Self::McpSearch),
            "access_control" => Some(Self::AccessControl),
            _ => None,
        }
    }

    /// Stable lower-case label used in reports.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Retrieval => "retrieval",
            Self::Consolidation => "consolidation",
            Self::Classification => "classification",
            Self::McpSearch => "mcp_search",
            Self::AccessControl => "access_control",
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
        for s in [
            "retrieval",
            "consolidation",
            "classification",
            "mcp_search",
            "access_control",
        ] {
            assert_eq!(SuiteName::parse(s).unwrap().as_str(), s);
        }
        assert!(SuiteName::parse("garbage").is_none());
        assert!(SuiteName::parse("  RETRIEVAL  ").is_some());
    }
}
