//! Bootstrap-event-kind -> canonical [`cortex_core::events::Kind`] mapping.
//!
//! Spec 09 emits string kinds (`artifact.code`, `turn.historical`, ...)
//! that don't line up 1:1 with `cortex_core::events::Kind`. This module
//! is the single source of truth for the mapping so the worker, tests,
//! and any future code path agree.

use cortex_core::events::Kind;

/// Failure modes for the kind mapper.
#[derive(Debug, thiserror::Error)]
pub enum KindMapError {
    /// The bootstrap envelope used a kind string the worker doesn't recognise.
    #[error("unknown bootstrap kind: {0}")]
    Unknown(String),
}

/// Map a bootstrap event kind string onto a canonical [`Kind`].
pub fn kind_from_bootstrap(kind: &str) -> Result<Kind, KindMapError> {
    let lower = kind.trim().to_ascii_lowercase();
    match lower.as_str() {
        "artifact.code" | "artifact.doc" | "artifact" => Ok(Kind::Artifact),
        "turn.historical" | "turn" => Ok(Kind::Turn),
        "decision.imported" | "decision" => Ok(Kind::Decision),
        "law.imported" | "law" | "law_violation" | "law.violation" => Ok(Kind::LawViolation),
        "memory.imported" | "memory" => Ok(Kind::Memory),
        "tool_call" | "tool.call" => Ok(Kind::ToolCall),
        "agent_call" | "agent.call" => Ok(Kind::AgentCall),
        "analysis" => Ok(Kind::Analysis),
        _ => Err(KindMapError::Unknown(kind.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_kinds_map_to_artifact() {
        assert_eq!(
            kind_from_bootstrap("artifact.code").unwrap(),
            Kind::Artifact
        );
        assert_eq!(kind_from_bootstrap("artifact.doc").unwrap(), Kind::Artifact);
    }

    #[test]
    fn turn_historical_maps_to_turn() {
        assert_eq!(kind_from_bootstrap("turn.historical").unwrap(), Kind::Turn);
    }

    #[test]
    fn decision_imported_maps_to_decision() {
        assert_eq!(
            kind_from_bootstrap("decision.imported").unwrap(),
            Kind::Decision
        );
    }

    #[test]
    fn law_imported_maps_to_law_violation() {
        assert_eq!(
            kind_from_bootstrap("law.imported").unwrap(),
            Kind::LawViolation
        );
    }

    #[test]
    fn memory_imported_maps_to_memory() {
        assert_eq!(
            kind_from_bootstrap("memory.imported").unwrap(),
            Kind::Memory
        );
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(
            kind_from_bootstrap("Artifact.Code").unwrap(),
            Kind::Artifact
        );
    }

    #[test]
    fn unknown_kind_errors() {
        assert!(kind_from_bootstrap("not_a_real_kind").is_err());
    }
}
