//! Phase11r §2 — heuristic contradiction detector.
//!
//! [`scan`] inspects a slice of [`HydratedEvidence`] items and emits a
//! [`Vec<Contradiction>`] for any pairs that match the three detector classes:
//!
//! - **`DecisionSupersession`** — a Decision's `supersedes` field matches
//!   another Decision's `decision_id`.
//! - **`LawViolationMismatch`** — a LawViolation cites a law version that
//!   differs from the matched Law's `version` when that Law is active.
//! - **`OutcomeDivergence`** — two Consolidations have overlapping temporal
//!   spans and differing `outcome_majority` values.
//!
//! All detectors are heuristic — they never block a rewrite, they only surface
//! contradictions for the synthesiser and the pre-thinking renderer to act on.

use cortex_core::events::{Contradiction, ContradictionKind, ContradictionStatus, EvidenceRef};

/// Kind-specific facts attached to a hydrated evidence item.
#[derive(Debug, Clone)]
pub enum EvidenceFacts {
    /// A `Kind::Decision` envelope.
    Decision {
        /// The decision's stable id (e.g. `DEC-042`).
        decision_id: String,
        /// If this decision supersedes another, its id.
        supersedes: Option<String>,
    },
    /// A `Kind::LawViolation` envelope.
    LawViolation {
        /// The law the violation cites.
        law_id: String,
        /// The version of the law cited in the violation.
        law_version: String,
    },
    /// A `Kind::Law` envelope (or equivalent).
    Law {
        /// The law's stable id.
        law_id: String,
        /// Current law version string.
        version: String,
        /// Whether the law is currently active.
        active: bool,
    },
    /// A `Kind::Consolidation` envelope.
    Consolidation {
        /// Start of the temporal span covered by this consolidation (ms since epoch).
        temporal_start_ms: i64,
        /// End of the temporal span (ms since epoch).
        temporal_end_ms: i64,
        /// The majority outcome label, if any (e.g. `"success"`, `"blocked_by_law"`).
        outcome_majority: Option<String>,
    },
    /// A `Kind::Turn` envelope (carries no extra discriminating facts).
    Turn,
}

/// A hydrated evidence item — the raw [`EvidenceRef`] plus its kind-specific facts.
#[derive(Debug, Clone)]
pub struct HydratedEvidence {
    /// The original reference from the topic card.
    pub evidence: EvidenceRef,
    /// Kind-specific facts needed by the detectors.
    pub kind_specific: EvidenceFacts,
}

/// Scan `hydrated` evidence for contradictions and return all detected pairs.
///
/// Each returned [`Contradiction`] carries `surfaced_at_rev = at_rev` and
/// `status = Open`. Duplicate pairs (A, B) and (B, A) are deduplicated — only
/// (A, B) is emitted (where A appears earlier in `hydrated` than B).
pub fn scan(hydrated: &[HydratedEvidence], at_rev: u32) -> Vec<Contradiction> {
    let mut out = Vec::new();
    detect_decision_supersession(hydrated, at_rev, &mut out);
    detect_law_violation_mismatch(hydrated, at_rev, &mut out);
    detect_outcome_divergence(hydrated, at_rev, &mut out);
    out
}

fn detect_decision_supersession(
    hydrated: &[HydratedEvidence],
    at_rev: u32,
    out: &mut Vec<Contradiction>,
) {
    for (i, a) in hydrated.iter().enumerate() {
        if let EvidenceFacts::Decision {
            supersedes: Some(ref supersedes_id),
            ..
        } = a.kind_specific
        {
            for b in &hydrated[..i] {
                if let EvidenceFacts::Decision {
                    decision_id: ref b_id,
                    ..
                } = b.kind_specific
                {
                    if b_id == supersedes_id {
                        out.push(Contradiction {
                            kind: ContradictionKind::DecisionSupersession,
                            evidence_a: b.evidence.id.clone(),
                            evidence_b: a.evidence.id.clone(),
                            surfaced_at_rev: at_rev,
                            status: ContradictionStatus::Open,
                        });
                    }
                }
            }
        }
    }
}

fn detect_law_violation_mismatch(
    hydrated: &[HydratedEvidence],
    at_rev: u32,
    out: &mut Vec<Contradiction>,
) {
    for violation in hydrated {
        if let EvidenceFacts::LawViolation {
            ref law_id,
            ref law_version,
        } = violation.kind_specific
        {
            for law in hydrated {
                if let EvidenceFacts::Law {
                    law_id: ref lid,
                    ref version,
                    active,
                } = law.kind_specific
                {
                    if lid == law_id && active && version != law_version {
                        out.push(Contradiction {
                            kind: ContradictionKind::LawViolationMismatch,
                            evidence_a: violation.evidence.id.clone(),
                            evidence_b: law.evidence.id.clone(),
                            surfaced_at_rev: at_rev,
                            status: ContradictionStatus::Open,
                        });
                    }
                }
            }
        }
    }
}

fn detect_outcome_divergence(
    hydrated: &[HydratedEvidence],
    at_rev: u32,
    out: &mut Vec<Contradiction>,
) {
    for (i, a) in hydrated.iter().enumerate() {
        if let EvidenceFacts::Consolidation {
            temporal_start_ms: a_start,
            temporal_end_ms: a_end,
            outcome_majority: Some(ref a_outcome),
        } = a.kind_specific
        {
            for b in &hydrated[..i] {
                if let EvidenceFacts::Consolidation {
                    temporal_start_ms: b_start,
                    temporal_end_ms: b_end,
                    outcome_majority: Some(ref b_outcome),
                } = b.kind_specific
                {
                    let overlaps = a_start < b_end && b_start < a_end;
                    if overlaps && a_outcome != b_outcome {
                        out.push(Contradiction {
                            kind: ContradictionKind::OutcomeDivergence,
                            evidence_a: b.evidence.id.clone(),
                            evidence_b: a.evidence.id.clone(),
                            surfaced_at_rev: at_rev,
                            status: ContradictionStatus::Open,
                        });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cortex_core::events::EvidenceKind;

    fn ev(id: &str, kind: EvidenceKind, facts: EvidenceFacts) -> HydratedEvidence {
        HydratedEvidence {
            evidence: EvidenceRef {
                kind,
                id: id.to_string(),
                weight: None,
                cited_at_rev: 1,
            },
            kind_specific: facts,
        }
    }

    // --- DecisionSupersession ---

    #[test]
    fn decision_supersession_detected_when_a_supersedes_b() {
        let hydrated = vec![
            ev(
                "DEC-001",
                EvidenceKind::Decision,
                EvidenceFacts::Decision {
                    decision_id: "DEC-001".into(),
                    supersedes: None,
                },
            ),
            ev(
                "DEC-002",
                EvidenceKind::Decision,
                EvidenceFacts::Decision {
                    decision_id: "DEC-002".into(),
                    supersedes: Some("DEC-001".into()),
                },
            ),
        ];
        let cs = scan(&hydrated, 3);
        assert_eq!(cs.len(), 1, "one contradiction expected");
        assert_eq!(cs[0].kind, ContradictionKind::DecisionSupersession);
        assert_eq!(cs[0].evidence_a, "DEC-001");
        assert_eq!(cs[0].evidence_b, "DEC-002");
        assert_eq!(cs[0].surfaced_at_rev, 3);
        assert_eq!(cs[0].status, ContradictionStatus::Open);
    }

    #[test]
    fn decision_no_supersession_when_supersedes_is_none() {
        let hydrated = vec![
            ev(
                "DEC-001",
                EvidenceKind::Decision,
                EvidenceFacts::Decision {
                    decision_id: "DEC-001".into(),
                    supersedes: None,
                },
            ),
            ev(
                "DEC-002",
                EvidenceKind::Decision,
                EvidenceFacts::Decision {
                    decision_id: "DEC-002".into(),
                    supersedes: None,
                },
            ),
        ];
        let cs = scan(&hydrated, 1);
        assert!(cs.is_empty(), "no contradictions expected");
    }

    #[test]
    fn decision_supersession_boundary_single_item() {
        let hydrated = vec![ev(
            "DEC-001",
            EvidenceKind::Decision,
            EvidenceFacts::Decision {
                decision_id: "DEC-001".into(),
                supersedes: None,
            },
        )];
        let cs = scan(&hydrated, 1);
        assert!(cs.is_empty());
    }

    // --- LawViolationMismatch ---

    #[test]
    fn law_violation_mismatch_detected_when_versions_differ() {
        let hydrated = vec![
            ev(
                "VIOLATION-001",
                EvidenceKind::Law,
                EvidenceFacts::LawViolation {
                    law_id: "LAW-AUTH".into(),
                    law_version: "1.0".into(),
                },
            ),
            ev(
                "LAW-AUTH-v2",
                EvidenceKind::Law,
                EvidenceFacts::Law {
                    law_id: "LAW-AUTH".into(),
                    version: "2.0".into(),
                    active: true,
                },
            ),
        ];
        let cs = scan(&hydrated, 2);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].kind, ContradictionKind::LawViolationMismatch);
        assert_eq!(cs[0].evidence_a, "VIOLATION-001");
        assert_eq!(cs[0].evidence_b, "LAW-AUTH-v2");
    }

    #[test]
    fn law_violation_no_mismatch_when_versions_match() {
        let hydrated = vec![
            ev(
                "VIOLATION-001",
                EvidenceKind::Law,
                EvidenceFacts::LawViolation {
                    law_id: "LAW-AUTH".into(),
                    law_version: "2.0".into(),
                },
            ),
            ev(
                "LAW-AUTH-v2",
                EvidenceKind::Law,
                EvidenceFacts::Law {
                    law_id: "LAW-AUTH".into(),
                    version: "2.0".into(),
                    active: true,
                },
            ),
        ];
        let cs = scan(&hydrated, 2);
        assert!(cs.is_empty());
    }

    #[test]
    fn law_violation_no_mismatch_when_law_is_inactive() {
        let hydrated = vec![
            ev(
                "VIOLATION-001",
                EvidenceKind::Law,
                EvidenceFacts::LawViolation {
                    law_id: "LAW-AUTH".into(),
                    law_version: "1.0".into(),
                },
            ),
            ev(
                "LAW-AUTH-old",
                EvidenceKind::Law,
                EvidenceFacts::Law {
                    law_id: "LAW-AUTH".into(),
                    version: "2.0".into(),
                    active: false, // inactive — no mismatch
                },
            ),
        ];
        let cs = scan(&hydrated, 2);
        assert!(cs.is_empty(), "inactive law must not trigger mismatch");
    }

    // --- OutcomeDivergence ---

    #[test]
    fn outcome_divergence_detected_for_overlapping_consolidations_with_different_outcomes() {
        let hydrated = vec![
            ev(
                "CONS-001",
                EvidenceKind::Consolidation,
                EvidenceFacts::Consolidation {
                    temporal_start_ms: 0,
                    temporal_end_ms: 1000,
                    outcome_majority: Some("success".into()),
                },
            ),
            ev(
                "CONS-002",
                EvidenceKind::Consolidation,
                EvidenceFacts::Consolidation {
                    temporal_start_ms: 500,
                    temporal_end_ms: 1500,
                    outcome_majority: Some("blocked_by_law".into()),
                },
            ),
        ];
        let cs = scan(&hydrated, 4);
        assert_eq!(cs.len(), 1);
        assert_eq!(cs[0].kind, ContradictionKind::OutcomeDivergence);
    }

    #[test]
    fn outcome_no_divergence_when_outcomes_match() {
        let hydrated = vec![
            ev(
                "CONS-001",
                EvidenceKind::Consolidation,
                EvidenceFacts::Consolidation {
                    temporal_start_ms: 0,
                    temporal_end_ms: 1000,
                    outcome_majority: Some("success".into()),
                },
            ),
            ev(
                "CONS-002",
                EvidenceKind::Consolidation,
                EvidenceFacts::Consolidation {
                    temporal_start_ms: 500,
                    temporal_end_ms: 1500,
                    outcome_majority: Some("success".into()),
                },
            ),
        ];
        let cs = scan(&hydrated, 4);
        assert!(cs.is_empty());
    }

    #[test]
    fn outcome_no_divergence_when_spans_do_not_overlap() {
        let hydrated = vec![
            ev(
                "CONS-001",
                EvidenceKind::Consolidation,
                EvidenceFacts::Consolidation {
                    temporal_start_ms: 0,
                    temporal_end_ms: 1000,
                    outcome_majority: Some("success".into()),
                },
            ),
            ev(
                "CONS-002",
                EvidenceKind::Consolidation,
                EvidenceFacts::Consolidation {
                    temporal_start_ms: 1000, // adjacent but not overlapping
                    temporal_end_ms: 2000,
                    outcome_majority: Some("blocked_by_law".into()),
                },
            ),
        ];
        let cs = scan(&hydrated, 4);
        assert!(
            cs.is_empty(),
            "adjacent but non-overlapping spans must not trigger"
        );
    }
}
